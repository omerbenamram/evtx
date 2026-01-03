use std::collections::HashMap;

use winstructs::guid::Guid;

use super::error::{Result, WevtManifestError};
use super::types::*;
use super::util::*;

impl<'a> CrimManifest<'a> {
    /// Parse a CRIM manifest blob (the payload stored inside a `WEVT_TEMPLATE` resource).
    ///
    /// This is the entrypoint for turning raw bytes into typed structures that can be joined
    /// against EVTX event metadata (e.g. event→template lookups for offline caches).
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        let header = parse_crim_header(data)?;
        let crim_size_usize =
            usize::try_from(header.size).map_err(|_| WevtManifestError::SizeOutOfBounds {
                what: "CRIM.size",
                offset: 0,
                size: header.size,
            })?;
        if crim_size_usize > data.len() {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "CRIM.size",
                offset: 0,
                size: header.size,
            });
        }

        let data = &data[..crim_size_usize];

        let provider_count = usize::try_from(header.provider_count).map_err(|_| {
            WevtManifestError::CountOutOfBounds {
                what: "CRIM.provider_count",
                offset: 12,
                count: header.provider_count,
            }
        })?;

        let providers_off = 16usize;
        let provider_desc_size = 20usize;
        let providers_end = providers_off
            .checked_add(provider_count.checked_mul(provider_desc_size).ok_or(
                WevtManifestError::CountOutOfBounds {
                    what: "CRIM.provider_count",
                    offset: 12,
                    count: header.provider_count,
                },
            )?)
            .ok_or(WevtManifestError::CountOutOfBounds {
                what: "CRIM.provider_count",
                offset: 12,
                count: header.provider_count,
            })?;

        if providers_end > data.len() {
            return Err(WevtManifestError::Truncated {
                what: "CRIM provider descriptor array",
                offset: 16,
                need: providers_end - providers_off,
                have: data.len().saturating_sub(providers_off),
            });
        }

        let mut providers = Vec::with_capacity(provider_count);
        for i in 0..provider_count {
            let desc_off = providers_off + i * provider_desc_size;
            let guid = read_guid_named(data, desc_off, "CRIM.provider.guid")?;
            let provider_off = read_u32_named(data, desc_off + 16, "CRIM.provider.offset")?;

            let provider = parse_provider(data, guid, provider_off)?;
            providers.push(provider);
        }

        Ok(Self {
            data,
            header,
            providers,
        })
    }

    /// Build lookup indices to support joining events/templates.
    ///
    /// This is primarily used by cache builders and tooling: it yields stable keys for mapping
    /// provider event definitions to template GUIDs, and for resolving templates by GUID.
    pub fn build_index(&'a self) -> CrimManifestIndex<'a> {
        let mut templates_by_guid: HashMap<String, Vec<&TemplateDefinition<'a>>> = HashMap::new();
        let mut event_to_template_guids: HashMap<EventKey, Vec<Guid>> = HashMap::new();

        for provider in &self.providers {
            if let Some(ttbl) = provider.wevt.elements.templates.as_ref() {
                for tpl in &ttbl.templates {
                    templates_by_guid
                        .entry(tpl.guid.to_string())
                        .or_default()
                        .push(tpl);
                }
            }

            if let Some(evnt) = provider.wevt.elements.events.as_ref() {
                for ev in &evnt.events {
                    let Some(template_offset) = ev.template_offset else {
                        continue;
                    };

                    let Some(tpl) = provider.template_by_offset(template_offset) else {
                        continue;
                    };

                    let key = EventKey {
                        provider_guid: provider.guid.to_string(),
                        event_id: ev.identifier,
                        version: ev.version,
                        channel: ev.channel,
                        level: ev.level,
                        opcode: ev.opcode,
                        task: ev.task,
                        keywords: ev.keywords,
                    };

                    let entry = event_to_template_guids.entry(key).or_default();
                    if !entry.contains(&tpl.guid) {
                        entry.push(tpl.guid.clone());
                    }
                }
            }
        }

        CrimManifestIndex {
            templates_by_guid,
            event_to_template_guids,
        }
    }
}

fn parse_crim_header(data: &[u8]) -> Result<CrimHeader> {
    let sig = read_sig_named(data, 0, "CRIM signature")?;
    if sig != *b"CRIM" {
        return Err(WevtManifestError::InvalidSignature {
            offset: 0,
            expected: *b"CRIM",
            found: sig,
        });
    }

    let size = read_u32_named(data, 4, "CRIM.size")?;
    let major_version = read_u16_named(data, 8, "CRIM.major_version")?;
    let minor_version = read_u16_named(data, 10, "CRIM.minor_version")?;
    let provider_count = read_u32_named(data, 12, "CRIM.provider_count")?;

    if size < 16 {
        return Err(WevtManifestError::SizeOutOfBounds {
            what: "CRIM.size",
            offset: 0,
            size,
        });
    }

    Ok(CrimHeader {
        size,
        major_version,
        minor_version,
        provider_count,
    })
}

fn parse_provider<'a>(crim: &'a [u8], guid: Guid, provider_off: u32) -> Result<Provider<'a>> {
    let provider_off_usize = u32_to_usize(provider_off, "WEVT provider offset", crim.len())?;
    // Need at least 20 bytes for WEVT header.
    require_len(crim, provider_off_usize, 20, "WEVT header")?;

    let sig = read_sig_named(crim, provider_off_usize, "WEVT signature")?;
    if sig != *b"WEVT" {
        return Err(WevtManifestError::InvalidSignature {
            offset: provider_off,
            expected: *b"WEVT",
            found: sig,
        });
    }

    let size = read_u32_named(crim, provider_off_usize + 4, "WEVT.size")?;
    let message_identifier_raw =
        read_u32_named(crim, provider_off_usize + 8, "WEVT.message_identifier")?;
    let descriptor_count =
        read_u32_named(crim, provider_off_usize + 12, "WEVT.number_of_descriptors")?;
    let unknown2_count = read_u32_named(crim, provider_off_usize + 16, "WEVT.number_of_unknown2")?;

    let message_identifier = if message_identifier_raw == 0xffffffff {
        None
    } else {
        Some(message_identifier_raw)
    };

    let desc_count_usize =
        usize::try_from(descriptor_count).map_err(|_| WevtManifestError::CountOutOfBounds {
            what: "WEVT.number_of_descriptors",
            offset: provider_off + 12,
            count: descriptor_count,
        })?;

    let desc_off = provider_off_usize + 20;
    let desc_bytes =
        desc_count_usize
            .checked_mul(8)
            .ok_or(WevtManifestError::CountOutOfBounds {
                what: "WEVT.number_of_descriptors",
                offset: provider_off + 12,
                count: descriptor_count,
            })?;
    require_len(crim, desc_off, desc_bytes, "WEVT descriptor array")?;

    let mut element_descriptors = Vec::with_capacity(desc_count_usize);
    for i in 0..desc_count_usize {
        let off = desc_off + i * 8;
        let element_offset = read_u32_named(crim, off, "WEVT.descriptor.element_offset")?;
        let unknown = read_u32_named(crim, off + 4, "WEVT.descriptor.unknown")?;
        let element_off_usize = u32_to_usize(element_offset, "WEVT element offset", crim.len())?;
        require_len(crim, element_off_usize, 4, "WEVT element signature")?;
        let signature = read_sig_named(crim, element_off_usize, "WEVT element signature")?;
        element_descriptors.push(ProviderElementDescriptor {
            element_offset,
            unknown,
            signature,
        });
    }

    let unknown2_count_usize =
        usize::try_from(unknown2_count).map_err(|_| WevtManifestError::CountOutOfBounds {
            what: "WEVT.number_of_unknown2",
            offset: provider_off + 16,
            count: unknown2_count,
        })?;

    let unknown2_off = desc_off + desc_bytes;
    let unknown2_bytes =
        unknown2_count_usize
            .checked_mul(4)
            .ok_or(WevtManifestError::CountOutOfBounds {
                what: "WEVT.number_of_unknown2",
                offset: provider_off + 16,
                count: unknown2_count,
            })?;
    require_len(crim, unknown2_off, unknown2_bytes, "WEVT unknown2 array")?;

    let mut unknown2 = Vec::with_capacity(unknown2_count_usize);
    for i in 0..unknown2_count_usize {
        let off = unknown2_off + i * 4;
        unknown2.push(read_u32_named(crim, off, "WEVT.unknown2")?);
    }

    let elements = parse_provider_elements(crim, &element_descriptors)?;

    Ok(Provider {
        guid,
        offset: provider_off,
        wevt: WevtProvider {
            offset: provider_off,
            size,
            message_identifier,
            element_descriptors,
            unknown2,
            elements,
        },
    })
}

fn parse_provider_elements<'a>(
    crim: &'a [u8],
    descriptors: &[ProviderElementDescriptor],
) -> Result<ProviderElements<'a>> {
    let mut out = ProviderElements::default();

    for d in descriptors {
        match &d.signature {
            b"CHAN" => {
                out.channels = Some(parse_channels(crim, d.element_offset)?);
            }
            b"EVNT" => {
                out.events = Some(parse_events(crim, d.element_offset)?);
            }
            b"KEYW" => {
                out.keywords = Some(parse_keywords(crim, d.element_offset)?);
            }
            b"LEVL" => {
                out.levels = Some(parse_levels(crim, d.element_offset)?);
            }
            b"MAPS" => {
                out.maps = Some(parse_maps(crim, d.element_offset)?);
            }
            b"OPCO" => {
                out.opcodes = Some(parse_opcodes(crim, d.element_offset)?);
            }
            b"TASK" => {
                out.tasks = Some(parse_tasks(crim, d.element_offset)?);
            }
            b"TTBL" => {
                out.templates = Some(parse_ttbl(crim, d.element_offset)?);
            }
            _ => {
                // Unknown element: try to read size (offset+4) and capture the region.
                let off = u32_to_usize(d.element_offset, "provider element offset", crim.len())?;
                if off + 8 <= crim.len() {
                    let size = read_u32_named(crim, off + 4, "provider element size")?;
                    let end = u32_to_usize(
                        d.element_offset.saturating_add(size),
                        "unknown element end",
                        crim.len(),
                    )?;
                    let data = &crim[off..end];
                    out.unknown.push(UnknownElement {
                        signature: d.signature,
                        offset: d.element_offset,
                        size,
                        data,
                    });
                } else {
                    return Err(WevtManifestError::Truncated {
                        what: "unknown element header",
                        offset: d.element_offset,
                        need: 8,
                        have: crim.len().saturating_sub(off),
                    });
                }
            }
        }
    }

    Ok(out)
}

fn parse_channels(crim: &[u8], off: u32) -> Result<ChannelDefinitions> {
    let off_usize = u32_to_usize(off, "CHAN offset", crim.len())?;
    require_len(crim, off_usize, 12, "CHAN header")?;
    let sig = read_sig_named(crim, off_usize, "CHAN signature")?;
    if sig != *b"CHAN" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"CHAN",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "CHAN.size")?;
    let count = read_u32_named(crim, off_usize + 8, "CHAN.count")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "CHAN.count",
        offset: off + 8,
        count,
    })?;
    let defs_off = off_usize + 12;
    let defs_bytes = count_usize
        .checked_mul(16)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "CHAN.count",
            offset: off + 8,
            count,
        })?;
    let min_end = defs_off
        .checked_add(defs_bytes)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what: "CHAN definitions array",
            offset: off,
            size,
        })?;

    let _end = if size == 0 {
        // libfwevt accepts size==0 and uses `count` to parse the definitions array.
        min_end
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "CHAN.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "CHAN.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "CHAN definitions array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut channels = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let d_off = defs_off + i * 16;
        let identifier = read_u32_named(crim, d_off, "CHAN.identifier")?;
        let name_offset = read_u32_named(crim, d_off + 4, "CHAN.name_offset")?;
        let unknown = read_u32_named(crim, d_off + 8, "CHAN.unknown")?;
        let msg_raw = read_u32_named(crim, d_off + 12, "CHAN.message_identifier")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        let name = if name_offset == 0 {
            None
        } else {
            Some(read_sized_utf16_string(crim, name_offset, "CHAN name")?)
        };
        channels.push(ChannelDefinition {
            identifier,
            name_offset,
            unknown,
            message_identifier,
            name,
        });
    }

    Ok(ChannelDefinitions {
        offset: off,
        size,
        channels,
    })
}

fn parse_events(crim: &[u8], off: u32) -> Result<EventDefinitions> {
    let off_usize = u32_to_usize(off, "EVNT offset", crim.len())?;
    require_len(crim, off_usize, 16, "EVNT header")?;
    let sig = read_sig_named(crim, off_usize, "EVNT signature")?;
    if sig != *b"EVNT" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"EVNT",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "EVNT.size")?;
    let count = read_u32_named(crim, off_usize + 8, "EVNT.count")?;
    let unknown = read_u32_named(crim, off_usize + 12, "EVNT.unknown")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "EVNT.count",
        offset: off + 8,
        count,
    })?;
    let events_off = off_usize + 16;
    let events_bytes = count_usize
        .checked_mul(48)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "EVNT.count",
            offset: off + 8,
            count,
        })?;
    let min_end =
        events_off
            .checked_add(events_bytes)
            .ok_or(WevtManifestError::SizeOutOfBounds {
                what: "EVNT event array",
                offset: off,
                size,
            })?;

    let end = if size == 0 {
        // libfwevt accepts size==0 and uses `count` to parse the array.
        min_end
    } else {
        if size < 16 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "EVNT.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "EVNT.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "EVNT event array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut events = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let e_off = events_off + i * 48;
        let identifier = read_u16_named(crim, e_off, "EVNT.event.identifier")?;
        let version = read_u8_named(crim, e_off + 2, "EVNT.event.version")?;
        let channel = read_u8_named(crim, e_off + 3, "EVNT.event.channel")?;
        let level = read_u8_named(crim, e_off + 4, "EVNT.event.level")?;
        let opcode = read_u8_named(crim, e_off + 5, "EVNT.event.opcode")?;
        let task = read_u16_named(crim, e_off + 6, "EVNT.event.task")?;
        let keywords = read_u64_named(crim, e_off + 8, "EVNT.event.keywords")?;
        let message_identifier = read_u32_named(crim, e_off + 16, "EVNT.event.message_identifier")?;
        let template_offset_raw = read_u32_named(crim, e_off + 20, "EVNT.event.template_offset")?;
        let opcode_offset_raw = read_u32_named(crim, e_off + 24, "EVNT.event.opcode_offset")?;
        let level_offset_raw = read_u32_named(crim, e_off + 28, "EVNT.event.level_offset")?;
        let task_offset_raw = read_u32_named(crim, e_off + 32, "EVNT.event.task_offset")?;
        let unknown_count = read_u32_named(crim, e_off + 36, "EVNT.event.unknown_count")?;
        let unknown_offset = read_u32_named(crim, e_off + 40, "EVNT.event.unknown_offset")?;
        let flags = read_u32_named(crim, e_off + 44, "EVNT.event.flags")?;

        events.push(EventDefinition {
            identifier,
            version,
            channel,
            level,
            opcode,
            task,
            keywords,
            message_identifier,
            template_offset: if template_offset_raw == 0 {
                None
            } else {
                Some(template_offset_raw)
            },
            opcode_offset: if opcode_offset_raw == 0 {
                None
            } else {
                Some(opcode_offset_raw)
            },
            level_offset: if level_offset_raw == 0 {
                None
            } else {
                Some(level_offset_raw)
            },
            task_offset: if task_offset_raw == 0 {
                None
            } else {
                Some(task_offset_raw)
            },
            unknown_count,
            unknown_offset,
            flags,
        });
    }

    let trailing = if end >= events_off + events_bytes {
        crim[events_off + events_bytes..end].to_vec()
    } else {
        vec![]
    };

    Ok(EventDefinitions {
        offset: off,
        size,
        unknown,
        events,
        trailing,
    })
}

fn parse_keywords(crim: &[u8], off: u32) -> Result<KeywordDefinitions> {
    let off_usize = u32_to_usize(off, "KEYW offset", crim.len())?;
    require_len(crim, off_usize, 12, "KEYW header")?;
    let sig = read_sig_named(crim, off_usize, "KEYW signature")?;
    if sig != *b"KEYW" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"KEYW",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "KEYW.size")?;
    let count = read_u32_named(crim, off_usize + 8, "KEYW.count")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "KEYW.count",
        offset: off + 8,
        count,
    })?;
    let defs_off = off_usize + 12;
    let defs_bytes = count_usize
        .checked_mul(16)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "KEYW.count",
            offset: off + 8,
            count,
        })?;
    let min_end = defs_off
        .checked_add(defs_bytes)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what: "KEYW definitions array",
            offset: off,
            size,
        })?;

    let _end = if size == 0 {
        // libfwevt accepts size==0 and uses `count` to parse the definitions array.
        min_end
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "KEYW.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "KEYW.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "KEYW definitions array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut keywords = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let d_off = defs_off + i * 16;
        let identifier = read_u64_named(crim, d_off, "KEYW.identifier")?;
        let msg_raw = read_u32_named(crim, d_off + 8, "KEYW.message_identifier")?;
        let data_offset = read_u32_named(crim, d_off + 12, "KEYW.data_offset")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        let name = if data_offset == 0 {
            None
        } else {
            Some(read_sized_utf16_string(crim, data_offset, "KEYW data")?)
        };
        keywords.push(KeywordDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        });
    }

    Ok(KeywordDefinitions {
        offset: off,
        size,
        keywords,
    })
}

fn parse_levels(crim: &[u8], off: u32) -> Result<LevelDefinitions> {
    let off_usize = u32_to_usize(off, "LEVL offset", crim.len())?;
    require_len(crim, off_usize, 12, "LEVL header")?;
    let sig = read_sig_named(crim, off_usize, "LEVL signature")?;
    if sig != *b"LEVL" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"LEVL",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "LEVL.size")?;
    let count = read_u32_named(crim, off_usize + 8, "LEVL.count")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "LEVL.count",
        offset: off + 8,
        count,
    })?;
    let defs_off = off_usize + 12;
    let defs_bytes = count_usize
        .checked_mul(12)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "LEVL.count",
            offset: off + 8,
            count,
        })?;
    let min_end = defs_off
        .checked_add(defs_bytes)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what: "LEVL definitions array",
            offset: off,
            size,
        })?;

    let _end = if size == 0 {
        min_end
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "LEVL.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "LEVL.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "LEVL definitions array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut levels = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let d_off = defs_off + i * 12;
        let identifier = read_u32_named(crim, d_off, "LEVL.identifier")?;
        let msg_raw = read_u32_named(crim, d_off + 4, "LEVL.message_identifier")?;
        let data_offset = read_u32_named(crim, d_off + 8, "LEVL.data_offset")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        let name = if data_offset == 0 {
            None
        } else {
            Some(read_sized_utf16_string(crim, data_offset, "LEVL data")?)
        };
        levels.push(LevelDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        });
    }

    Ok(LevelDefinitions {
        offset: off,
        size,
        levels,
    })
}

fn parse_opcodes(crim: &[u8], off: u32) -> Result<OpcodeDefinitions> {
    let off_usize = u32_to_usize(off, "OPCO offset", crim.len())?;
    require_len(crim, off_usize, 12, "OPCO header")?;
    let sig = read_sig_named(crim, off_usize, "OPCO signature")?;
    if sig != *b"OPCO" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"OPCO",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "OPCO.size")?;
    let count = read_u32_named(crim, off_usize + 8, "OPCO.count")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "OPCO.count",
        offset: off + 8,
        count,
    })?;
    let defs_off = off_usize + 12;
    let defs_bytes = count_usize
        .checked_mul(12)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "OPCO.count",
            offset: off + 8,
            count,
        })?;
    let min_end = defs_off
        .checked_add(defs_bytes)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what: "OPCO definitions array",
            offset: off,
            size,
        })?;

    let _end = if size == 0 {
        min_end
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "OPCO.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "OPCO.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "OPCO definitions array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut opcodes = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let d_off = defs_off + i * 12;
        let identifier = read_u32_named(crim, d_off, "OPCO.identifier")?;
        let msg_raw = read_u32_named(crim, d_off + 4, "OPCO.message_identifier")?;
        let data_offset = read_u32_named(crim, d_off + 8, "OPCO.data_offset")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        let name = if data_offset == 0 {
            None
        } else {
            Some(read_sized_utf16_string(crim, data_offset, "OPCO data")?)
        };
        opcodes.push(OpcodeDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        });
    }

    Ok(OpcodeDefinitions {
        offset: off,
        size,
        opcodes,
    })
}

fn parse_tasks(crim: &[u8], off: u32) -> Result<TaskDefinitions> {
    let off_usize = u32_to_usize(off, "TASK offset", crim.len())?;
    require_len(crim, off_usize, 12, "TASK header")?;
    let sig = read_sig_named(crim, off_usize, "TASK signature")?;
    if sig != *b"TASK" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"TASK",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "TASK.size")?;
    let count = read_u32_named(crim, off_usize + 8, "TASK.count")?;

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "TASK.count",
        offset: off + 8,
        count,
    })?;
    let defs_off = off_usize + 12;
    let defs_bytes = count_usize
        .checked_mul(28)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "TASK.count",
            offset: off + 8,
            count,
        })?;
    let min_end = defs_off
        .checked_add(defs_bytes)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what: "TASK definitions array",
            offset: off,
            size,
        })?;

    let _end = if size == 0 {
        min_end
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "TASK.size",
                offset: off,
                size,
            });
        }
        let end = checked_end(crim.len(), off, size, "TASK.size")?;
        if min_end > end {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "TASK definitions array",
                offset: off,
                size,
            });
        }
        end
    };

    let mut tasks = Vec::with_capacity(count_usize);
    for i in 0..count_usize {
        let d_off = defs_off + i * 28;
        let identifier = read_u32_named(crim, d_off, "TASK.identifier")?;
        let msg_raw = read_u32_named(crim, d_off + 4, "TASK.message_identifier")?;
        let mui_identifier = read_guid_named(crim, d_off + 8, "TASK.mui_identifier")?;
        let data_offset = read_u32_named(crim, d_off + 24, "TASK.data_offset")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        let name = if data_offset == 0 {
            None
        } else {
            Some(read_sized_utf16_string(crim, data_offset, "TASK data")?)
        };
        tasks.push(TaskDefinition {
            identifier,
            message_identifier,
            mui_identifier,
            data_offset,
            name,
        });
    }

    Ok(TaskDefinitions {
        offset: off,
        size,
        tasks,
    })
}

fn parse_ttbl<'a>(crim: &'a [u8], off: u32) -> Result<TemplateTable<'a>> {
    let off_usize = u32_to_usize(off, "TTBL offset", crim.len())?;
    require_len(crim, off_usize, 12, "TTBL header")?;
    let sig = read_sig_named(crim, off_usize, "TTBL signature")?;
    if sig != *b"TTBL" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"TTBL",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "TTBL.size")?;
    let count = read_u32_named(crim, off_usize + 8, "TTBL.count")?;
    let end = if size == 0 {
        // libfwevt accepts size==0 and parses by `count` and per-template sizes.
        crim.len()
    } else {
        if size < 12 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "TTBL.size",
                offset: off,
                size,
            });
        }
        checked_end(crim.len(), off, size, "TTBL.size")?
    };

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "TTBL.count",
        offset: off + 8,
        count,
    })?;

    let mut templates = Vec::with_capacity(count_usize);
    let mut cur = off_usize + 12;

    for _ in 0..count_usize {
        if cur + 40 > end {
            return Err(WevtManifestError::Truncated {
                what: "TEMP header",
                offset: usize_to_u32(cur),
                need: 40,
                have: end.saturating_sub(cur),
            });
        }
        let temp_sig = read_sig_named(crim, cur, "TEMP signature")?;
        if temp_sig != *b"TEMP" {
            return Err(WevtManifestError::InvalidSignature {
                offset: usize_to_u32(cur),
                expected: *b"TEMP",
                found: temp_sig,
            });
        }
        let temp_size = read_u32_named(crim, cur + 4, "TEMP.size")?;
        if temp_size < 40 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "TEMP.size",
                offset: usize_to_u32(cur),
                size: temp_size,
            });
        }
        let temp_end = checked_end(end, usize_to_u32(cur), temp_size, "TEMP.size")?;
        let temp_off_u32 = usize_to_u32(cur);

        let item_descriptor_count = read_u32_named(crim, cur + 8, "TEMP.item_descriptor_count")?;
        let item_name_count = read_u32_named(crim, cur + 12, "TEMP.item_name_count")?;
        let template_items_offset = read_u32_named(crim, cur + 16, "TEMP.template_items_offset")?;
        let event_type = read_u32_named(crim, cur + 20, "TEMP.event_type")?;
        let guid = read_guid_named(crim, cur + 24, "TEMP.guid")?;

        // libfwevt notes: if number_of_descriptors (and number_of_names) is 0, the template_items_offset
        // is either 0 or points to the end of the template. Treat non-zero name count in this case as invalid.
        if item_descriptor_count == 0 && item_name_count != 0 {
            return Err(WevtManifestError::CountOutOfBounds {
                what: "TEMP.item_name_count (expected 0 when item_descriptor_count == 0)",
                offset: temp_off_u32 + 12,
                count: item_name_count,
            });
        }

        let template_slice = &crim[cur..temp_end];

        // Compute binxml bounds using template_items_offset (absolute, relative to CRIM).
        let items_abs = if item_descriptor_count == 0 && template_items_offset == 0 {
            // libfwevt allows 0 in the no-items case; treat as end-of-template for binxml sizing.
            temp_off_u32.saturating_add(temp_size)
        } else {
            template_items_offset
        };

        let items_rel = if items_abs == 0 {
            // No guidance; treat items as starting at end-of-template.
            temp_size
        } else if items_abs < temp_off_u32 {
            return Err(WevtManifestError::OffsetOutOfBounds {
                what: "TEMP.template_items_offset",
                offset: items_abs,
                len: crim.len(),
            });
        } else {
            items_abs - temp_off_u32
        };

        let items_rel_usize = u32_to_usize(
            items_rel,
            "TEMP.template_items_offset (relative)",
            template_slice.len(),
        )?;
        if items_rel_usize > template_slice.len() {
            return Err(WevtManifestError::OffsetOutOfBounds {
                what: "TEMP.template_items_offset (relative)",
                offset: temp_off_u32.saturating_add(items_rel),
                len: crim.len(),
            });
        }

        let binxml_start = 40usize;
        let binxml_end = items_rel_usize.min(template_slice.len());
        let binxml = if binxml_end >= binxml_start {
            &template_slice[binxml_start..binxml_end]
        } else {
            &template_slice[binxml_start..binxml_start]
        };

        let items = parse_template_items(
            template_slice,
            temp_off_u32,
            item_descriptor_count,
            template_items_offset,
        )?;

        templates.push(TemplateDefinition {
            offset: temp_off_u32,
            size: temp_size,
            item_descriptor_count,
            item_name_count,
            template_items_offset,
            event_type,
            guid,
            binxml,
            items,
        });

        cur = temp_end;
    }

    Ok(TemplateTable {
        offset: off,
        size,
        templates,
    })
}

fn parse_template_items(
    template: &[u8],
    template_off_abs: u32,
    item_descriptor_count: u32,
    template_items_offset_abs: u32,
) -> Result<Vec<TemplateItem>> {
    let count_usize = usize::try_from(item_descriptor_count).map_err(|_| {
        WevtManifestError::CountOutOfBounds {
            what: "TEMP.item_descriptor_count",
            offset: template_off_abs + 8,
            count: item_descriptor_count,
        }
    })?;

    if count_usize == 0 {
        // Validate template_items_offset for the zero-items case.
        if template_items_offset_abs != 0
            && template_items_offset_abs != template_off_abs.saturating_add(template.len() as u32)
        {
            return Err(WevtManifestError::OffsetOutOfBounds {
                what: "TEMP.template_items_offset (expected 0 or end-of-template when item_descriptor_count==0)",
                offset: template_items_offset_abs,
                len: template_off_abs.saturating_add(template.len() as u32) as usize,
            });
        }
        return Ok(vec![]);
    }

    if template_items_offset_abs < template_off_abs {
        return Err(WevtManifestError::OffsetOutOfBounds {
            what: "TEMP.template_items_offset",
            offset: template_items_offset_abs,
            len: template_off_abs.saturating_add(template.len() as u32) as usize,
        });
    }

    let rel = template_items_offset_abs - template_off_abs;
    let rel_usize = u32_to_usize(rel, "TEMP.template_items_offset (relative)", template.len())?;
    if rel_usize < 40 || rel_usize >= template.len() {
        return Err(WevtManifestError::OffsetOutOfBounds {
            what: "TEMP.template_items_offset (relative)",
            offset: template_items_offset_abs,
            len: template_off_abs.saturating_add(template.len() as u32) as usize,
        });
    }

    let needed = count_usize
        .checked_mul(20)
        .ok_or(WevtManifestError::CountOutOfBounds {
            what: "TEMP.item_descriptor_count",
            offset: template_off_abs + 8,
            count: item_descriptor_count,
        })?;
    if rel_usize + needed > template.len() {
        return Err(WevtManifestError::Truncated {
            what: "template item descriptors",
            offset: template_items_offset_abs,
            need: needed,
            have: template.len().saturating_sub(rel_usize),
        });
    }

    let descriptor_end = rel_usize + needed;

    // First pass: parse descriptors and collect the minimal non-zero name offset (relative to template base).
    let mut items = Vec::with_capacity(count_usize);
    let mut min_name_rel: Option<usize> = None;

    for i in 0..count_usize {
        let d_off = rel_usize + i * 20;
        let unknown1 = read_u32_named(template, d_off, "TEMP.item.unknown1")?;
        let input_type = read_u8_named(template, d_off + 4, "TEMP.item.input_type")?;
        let output_type = read_u8_named(template, d_off + 5, "TEMP.item.output_type")?;
        let unknown3 = read_u16_named(template, d_off + 6, "TEMP.item.unknown3")?;
        let unknown4 = read_u32_named(template, d_off + 8, "TEMP.item.unknown4")?;
        let count = read_u16_named(template, d_off + 12, "TEMP.item.count")?;
        let length = read_u16_named(template, d_off + 14, "TEMP.item.length")?;
        let name_offset = read_u32_named(template, d_off + 16, "TEMP.item.name_offset")?;

        if name_offset != 0 {
            if name_offset < template_off_abs {
                return Err(WevtManifestError::OffsetOutOfBounds {
                    what: "template item name_offset",
                    offset: name_offset,
                    len: template_off_abs.saturating_add(template.len() as u32) as usize,
                });
            }
            let name_rel = name_offset - template_off_abs;
            let name_rel_usize = u32_to_usize(
                name_rel,
                "template item name_offset (relative)",
                template.len(),
            )?;
            min_name_rel = Some(min_name_rel.map_or(name_rel_usize, |m| m.min(name_rel_usize)));
        }

        items.push(TemplateItem {
            unknown1,
            input_type,
            output_type,
            unknown3,
            unknown4,
            count,
            length,
            name_offset,
            name: None,
        });
    }

    // libfwevt’s reader relies on a boundary between descriptors and names; enforce that at least
    // the first name (if present) starts after the descriptor table.
    if let Some(min_name_rel) = min_name_rel
        && min_name_rel < descriptor_end
    {
        return Err(WevtManifestError::OffsetOutOfBounds {
            what: "template item name_offset overlaps descriptor table",
            offset: template_off_abs.saturating_add(min_name_rel as u32),
            len: template_off_abs.saturating_add(template.len() as u32) as usize,
        });
    }

    // Second pass: resolve names.
    for item in &mut items {
        if item.name_offset == 0 {
            continue;
        }
        let name_rel = item.name_offset - template_off_abs;
        item.name = Some(read_sized_utf16_string(
            template,
            name_rel,
            "template item name",
        )?);
    }

    Ok(items)
}

fn parse_maps<'a>(crim: &'a [u8], off: u32) -> Result<MapsDefinitions<'a>> {
    // Maps parsing in libfwevt is TODO; we implement VMAP per spec and keep others opaque.
    let off_usize = u32_to_usize(off, "MAPS offset", crim.len())?;
    require_len(crim, off_usize, 16, "MAPS header")?;
    let sig = read_sig_named(crim, off_usize, "MAPS signature")?;
    if sig != *b"MAPS" {
        return Err(WevtManifestError::InvalidSignature {
            offset: off,
            expected: *b"MAPS",
            found: sig,
        });
    }
    let size = read_u32_named(crim, off_usize + 4, "MAPS.size")?;
    let count = read_u32_named(crim, off_usize + 8, "MAPS.count")?;
    let first_map_offset = read_u32_named(crim, off_usize + 12, "MAPS.first_map_offset")?;
    let end = if size == 0 {
        // libfwevt accepts size==0 and parses by offsets/count.
        crim.len()
    } else {
        if size < 16 {
            return Err(WevtManifestError::SizeOutOfBounds {
                what: "MAPS.size",
                offset: off,
                size,
            });
        }
        checked_end(crim.len(), off, size, "MAPS.size")?
    };

    let count_usize = usize::try_from(count).map_err(|_| WevtManifestError::CountOutOfBounds {
        what: "MAPS.count",
        offset: off + 8,
        count,
    })?;

    let mut map_offsets: Vec<u32> = Vec::with_capacity(count_usize);
    if count_usize > 0 {
        // Interpret first_map_offset as map_offsets[0] when non-zero; otherwise fallback to implied offset.
        let implied_first = (off_usize + 16 + (count_usize.saturating_sub(1) * 4)) as u32;
        let first = if first_map_offset == 0 {
            implied_first
        } else {
            first_map_offset
        };
        map_offsets.push(first);
    }

    // Remaining offsets array (count-1).
    let offs_array_off = off_usize + 16;
    let offs_array_bytes = count_usize.saturating_sub(1).checked_mul(4).unwrap_or(0);
    if offs_array_off + offs_array_bytes > end {
        return Err(WevtManifestError::SizeOutOfBounds {
            what: "MAPS offsets array",
            offset: off,
            size,
        });
    }
    for i in 0..count_usize.saturating_sub(1) {
        let o = read_u32_named(crim, offs_array_off + i * 4, "MAPS.map_offset")?;
        map_offsets.push(o);
    }

    // Parse each map by offset.
    //
    // Some real-world providers (e.g. `wevtsvc.dll`) store the offsets array out-of-order. Map
    // boundaries are based on the next map in *file order*, not the next entry in the offsets
    // array, so we sort offsets before iterating.
    let mut map_offsets_sorted = map_offsets;
    map_offsets_sorted.sort_unstable();
    map_offsets_sorted.dedup();

    // Boundaries are unknown, so for unknown map types we capture until the next map offset or
    // MAPS end.
    let mut maps = Vec::with_capacity(map_offsets_sorted.len());
    for (i, &map_off) in map_offsets_sorted.iter().enumerate() {
        let map_off_usize = u32_to_usize(map_off, "MAPS map offset", crim.len())?;
        if map_off_usize + 4 > crim.len() {
            return Err(WevtManifestError::Truncated {
                what: "MAPS map signature",
                offset: map_off,
                need: 4,
                have: crim.len().saturating_sub(map_off_usize),
            });
        }
        let sig = read_sig_named(crim, map_off_usize, "MAPS map signature")?;
        let next_off = map_offsets_sorted
            .get(i + 1)
            .copied()
            .unwrap_or_else(|| usize_to_u32(end));
        let next_usize = u32_to_usize(next_off, "MAPS map end", crim.len())?;
        let slice_end = next_usize.min(end);
        if slice_end < map_off_usize {
            return Err(WevtManifestError::OffsetOutOfBounds {
                what: "MAPS map boundary",
                offset: next_off,
                len: crim.len(),
            });
        }
        let map_slice = match &sig {
            b"VMAP" => &crim[map_off_usize..slice_end],
            // Avoid capturing an unbounded tail for unknown map types when MAPS.size == 0.
            _ => &crim[map_off_usize..std::cmp::min(map_off_usize + 4, slice_end)],
        };

        match &sig {
            b"VMAP" => {
                maps.push(MapDefinition::ValueMap(parse_vmap(
                    crim, map_off, map_slice,
                )?));
            }
            b"BMAP" => {
                maps.push(MapDefinition::Bitmap(BitmapMap {
                    offset: map_off,
                    data: map_slice,
                }));
            }
            _ => {
                maps.push(MapDefinition::Unknown {
                    signature: sig,
                    offset: map_off,
                    data: map_slice,
                });
            }
        }
    }

    Ok(MapsDefinitions {
        offset: off,
        size,
        maps,
    })
}

fn parse_vmap<'a>(crim: &'a [u8], off: u32, map_slice: &'a [u8]) -> Result<ValueMap<'a>> {
    // VMAP layout (per spec):
    // 0:4 sig
    // 4:4 size (including signature)
    // 8:4 map_string_offset (relative to CRIM)
    // 12:4 entry_count
    // 16: entries (8 bytes each)
    if map_slice.len() < 16 {
        return Err(WevtManifestError::Truncated {
            what: "VMAP header",
            offset: off,
            need: 16,
            have: map_slice.len(),
        });
    }
    let size = read_u32_named(map_slice, 4, "VMAP.size")?;
    let map_string_offset = read_u32_named(map_slice, 8, "VMAP.map_string_offset")?;
    let entry_count = read_u32_named(map_slice, 12, "VMAP.entry_count")?;

    let size_usize = usize::try_from(size).map_err(|_| WevtManifestError::SizeOutOfBounds {
        what: "VMAP.size",
        offset: off,
        size,
    })?;
    if size_usize < 16 || size_usize > map_slice.len() {
        return Err(WevtManifestError::SizeOutOfBounds {
            what: "VMAP.size",
            offset: off,
            size,
        });
    }

    let entry_count_usize =
        usize::try_from(entry_count).map_err(|_| WevtManifestError::CountOutOfBounds {
            what: "VMAP.entry_count",
            offset: off + 12,
            count: entry_count,
        })?;
    let entries_bytes =
        entry_count_usize
            .checked_mul(8)
            .ok_or(WevtManifestError::CountOutOfBounds {
                what: "VMAP.entry_count",
                offset: off + 12,
                count: entry_count,
            })?;

    if 16 + entries_bytes > size_usize {
        return Err(WevtManifestError::SizeOutOfBounds {
            what: "VMAP entries array",
            offset: off,
            size,
        });
    }

    let mut entries = Vec::with_capacity(entry_count_usize);
    for i in 0..entry_count_usize {
        let e_off = 16 + i * 8;
        let identifier = read_u32_named(map_slice, e_off, "VMAP.entry.identifier")?;
        let msg_raw = read_u32_named(map_slice, e_off + 4, "VMAP.entry.message_identifier")?;
        let message_identifier = if msg_raw == 0xffffffff {
            None
        } else {
            Some(msg_raw)
        };
        entries.push(ValueMapEntry {
            identifier,
            message_identifier,
        });
    }

    let map_string = if map_string_offset == 0 {
        None
    } else {
        Some(read_sized_utf16_string(
            crim,
            map_string_offset,
            "VMAP map string",
        )?)
    };

    let trailing = &map_slice[16 + entries_bytes..size_usize];

    Ok(ValueMap {
        offset: off,
        size,
        map_string_offset,
        entries,
        map_string,
        trailing,
    })
}
