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
        let crim_size_usize = usize::try_from(header.size)
            .ok()
            .filter(|&s| s <= data.len())
            .ok_or_else(|| size_err("CRIM.size", 0, header.size))?;

        let data = &data[..crim_size_usize];

        let (provider_count, providers_bytes) =
            count_bytes(header.provider_count, 20, "CRIM.provider_count", 12)?;

        let providers_off = 16usize;
        let providers_end = providers_off
            .checked_add(providers_bytes)
            .ok_or_else(|| count_err("CRIM.provider_count", 12, header.provider_count))?;

        if providers_end > data.len() {
            return Err(trunc_err(
                "CRIM provider descriptor array",
                16,
                providers_end - providers_off,
                data.len().saturating_sub(providers_off),
            ));
        }

        let mut providers = Vec::with_capacity(provider_count);
        for i in 0..provider_count {
            let desc_off = providers_off + i * 20;
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
        return Err(sig_err(0, b"CRIM", sig));
    }

    let size = read_u32_named(data, 4, "CRIM.size")?;
    let major_version = read_u16_named(data, 8, "CRIM.major_version")?;
    let minor_version = read_u16_named(data, 10, "CRIM.minor_version")?;
    let provider_count = read_u32_named(data, 12, "CRIM.provider_count")?;

    if size < 16 {
        return Err(size_err("CRIM.size", 0, size));
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
        return Err(sig_err(provider_off, b"WEVT", sig));
    }

    let size = read_u32_named(crim, provider_off_usize + 4, "WEVT.size")?;
    let message_identifier = opt_message_id(read_u32_named(
        crim,
        provider_off_usize + 8,
        "WEVT.message_identifier",
    )?);
    let descriptor_count =
        read_u32_named(crim, provider_off_usize + 12, "WEVT.number_of_descriptors")?;
    let unknown2_count = read_u32_named(crim, provider_off_usize + 16, "WEVT.number_of_unknown2")?;

    let (desc_count_usize, desc_bytes) = count_bytes(
        descriptor_count,
        8,
        "WEVT.number_of_descriptors",
        provider_off + 12,
    )?;
    let desc_off = provider_off_usize + 20;
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

    let (unknown2_count_usize, unknown2_bytes) = count_bytes(
        unknown2_count,
        4,
        "WEVT.number_of_unknown2",
        provider_off + 16,
    )?;
    let unknown2_off = desc_off + desc_bytes;
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
                    return Err(trunc_err(
                        "unknown element header",
                        d.element_offset,
                        8,
                        crim.len().saturating_sub(off),
                    ));
                }
            }
        }
    }

    Ok(out)
}

fn sig_err(offset: u32, expected: &[u8; 4], found: [u8; 4]) -> WevtManifestError {
    WevtManifestError::InvalidSignature {
        offset,
        expected: *expected,
        found,
    }
}

fn size_err(what: &'static str, offset: u32, size: u32) -> WevtManifestError {
    WevtManifestError::SizeOutOfBounds { what, offset, size }
}

fn count_err(what: &'static str, offset: u32, count: u32) -> WevtManifestError {
    WevtManifestError::CountOutOfBounds {
        what,
        offset,
        count,
    }
}

fn trunc_err(what: &'static str, offset: u32, need: usize, have: usize) -> WevtManifestError {
    WevtManifestError::Truncated {
        what,
        offset,
        need,
        have,
    }
}

fn off_err(what: &'static str, offset: u32, len: usize) -> WevtManifestError {
    WevtManifestError::OffsetOutOfBounds { what, offset, len }
}

struct TableNames {
    offset: &'static str,
    header: &'static str,
    signature: &'static str,
    size: &'static str,
    count: &'static str,
    array: &'static str,
}

macro_rules! table_names {
    ($p:literal) => {
        table_names!($p, concat!($p, " definitions array"))
    };
    ($p:literal, $array:expr) => {
        TableNames {
            offset: concat!($p, " offset"),
            header: concat!($p, " header"),
            signature: concat!($p, " signature"),
            size: concat!($p, ".size"),
            count: concat!($p, ".count"),
            array: $array,
        }
    };
}

fn opt_message_id(raw: u32) -> Option<u32> {
    (raw != 0xffffffff).then_some(raw)
}

fn opt_nonzero(v: u32) -> Option<u32> {
    (v != 0).then_some(v)
}

fn read_opt_name(crim: &[u8], offset: u32, what: &'static str) -> Result<Option<String>> {
    if offset == 0 {
        Ok(None)
    } else {
        read_sized_utf16_string(crim, offset, what).map(Some)
    }
}

fn u32_count(count: u32, what: &'static str, offset: u32) -> Result<usize> {
    usize::try_from(count).map_err(|_| count_err(what, offset, count))
}

fn count_bytes(count: u32, rec: usize, what: &'static str, offset: u32) -> Result<(usize, usize)> {
    let n = u32_count(count, what, offset)?;
    let bytes = n
        .checked_mul(rec)
        .ok_or_else(|| count_err(what, offset, count))?;
    Ok((n, bytes))
}

fn read_block_header(
    crim: &[u8],
    off: u32,
    sig: &'static [u8; 4],
    header_len: usize,
    names: &TableNames,
) -> Result<(usize, u32, u32)> {
    let off_usize = u32_to_usize(off, names.offset, crim.len())?;
    require_len(crim, off_usize, header_len, names.header)?;
    let found = read_sig_named(crim, off_usize, names.signature)?;
    if found != *sig {
        return Err(sig_err(off, sig, found));
    }
    let size = read_u32_named(crim, off_usize + 4, names.size)?;
    let count = read_u32_named(crim, off_usize + 8, names.count)?;
    Ok((off_usize, size, count))
}

fn region_end(len: usize, off: u32, size: u32, min_size: u32, what: &'static str) -> Result<usize> {
    if size == 0 {
        // libfwevt accepts size==0 and parses by `count`.
        return Ok(len);
    }
    if size < min_size {
        return Err(size_err(what, off, size));
    }
    checked_end(len, off, size, what)
}

struct TableBounds {
    header_off: usize,
    size: u32,
    count: usize,
    recs_off: usize,
    recs_end: usize,
    end: usize,
}

fn parse_table_header(
    crim: &[u8],
    off: u32,
    sig: &'static [u8; 4],
    header_len: usize,
    rec_size: usize,
    names: &TableNames,
) -> Result<TableBounds> {
    let (header_off, size, count) = read_block_header(crim, off, sig, header_len, names)?;
    let (count, recs_bytes) = count_bytes(count, rec_size, names.count, off + 8)?;
    let recs_off = header_off + header_len;
    let recs_end = recs_off
        .checked_add(recs_bytes)
        .ok_or_else(|| size_err(names.array, off, size))?;

    let end = if size == 0 {
        // libfwevt accepts size==0 and uses `count` to parse the array.
        recs_end
    } else {
        let end = region_end(crim.len(), off, size, header_len as u32, names.size)?;
        if recs_end > end {
            return Err(size_err(names.array, off, size));
        }
        end
    };

    Ok(TableBounds {
        header_off,
        size,
        count,
        recs_off,
        recs_end,
        end,
    })
}

fn parse_table<T>(
    crim: &[u8],
    off: u32,
    sig: &'static [u8; 4],
    rec_size: usize,
    names: TableNames,
    read_rec: impl Fn(usize) -> Result<T>,
) -> Result<(u32, Vec<T>)> {
    let t = parse_table_header(crim, off, sig, 12, rec_size, &names)?;
    let mut recs = Vec::with_capacity(t.count);
    for i in 0..t.count {
        recs.push(read_rec(t.recs_off + i * rec_size)?);
    }
    Ok((t.size, recs))
}

fn parse_channels(crim: &[u8], off: u32) -> Result<ChannelDefinitions> {
    let (size, channels) = parse_table(crim, off, b"CHAN", 16, table_names!("CHAN"), |d_off| {
        let identifier = read_u32_named(crim, d_off, "CHAN.identifier")?;
        let name_offset = read_u32_named(crim, d_off + 4, "CHAN.name_offset")?;
        let unknown = read_u32_named(crim, d_off + 8, "CHAN.unknown")?;
        let message_identifier =
            opt_message_id(read_u32_named(crim, d_off + 12, "CHAN.message_identifier")?);
        let name = read_opt_name(crim, name_offset, "CHAN name")?;
        Ok(ChannelDefinition {
            identifier,
            name_offset,
            unknown,
            message_identifier,
            name,
        })
    })?;

    Ok(ChannelDefinitions {
        offset: off,
        size,
        channels,
    })
}

fn parse_events(crim: &[u8], off: u32) -> Result<EventDefinitions> {
    let t = parse_table_header(
        crim,
        off,
        b"EVNT",
        16,
        48,
        &table_names!("EVNT", "EVNT event array"),
    )?;
    let unknown = read_u32_named(crim, t.header_off + 12, "EVNT.unknown")?;

    let mut events = Vec::with_capacity(t.count);
    for i in 0..t.count {
        let e_off = t.recs_off + i * 48;
        let identifier = read_u16_named(crim, e_off, "EVNT.event.identifier")?;
        let version = read_u8_named(crim, e_off + 2, "EVNT.event.version")?;
        let channel = read_u8_named(crim, e_off + 3, "EVNT.event.channel")?;
        let level = read_u8_named(crim, e_off + 4, "EVNT.event.level")?;
        let opcode = read_u8_named(crim, e_off + 5, "EVNT.event.opcode")?;
        let task = read_u16_named(crim, e_off + 6, "EVNT.event.task")?;
        let keywords = read_u64_named(crim, e_off + 8, "EVNT.event.keywords")?;
        let message_identifier = read_u32_named(crim, e_off + 16, "EVNT.event.message_identifier")?;
        let template_offset = opt_nonzero(read_u32_named(
            crim,
            e_off + 20,
            "EVNT.event.template_offset",
        )?);
        let opcode_offset = opt_nonzero(read_u32_named(
            crim,
            e_off + 24,
            "EVNT.event.opcode_offset",
        )?);
        let level_offset =
            opt_nonzero(read_u32_named(crim, e_off + 28, "EVNT.event.level_offset")?);
        let task_offset = opt_nonzero(read_u32_named(crim, e_off + 32, "EVNT.event.task_offset")?);
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
            template_offset,
            opcode_offset,
            level_offset,
            task_offset,
            unknown_count,
            unknown_offset,
            flags,
        });
    }

    let trailing = if t.end >= t.recs_end {
        crim[t.recs_end..t.end].to_vec()
    } else {
        vec![]
    };

    Ok(EventDefinitions {
        offset: off,
        size: t.size,
        unknown,
        events,
        trailing,
    })
}

fn parse_keywords(crim: &[u8], off: u32) -> Result<KeywordDefinitions> {
    let (size, keywords) = parse_table(crim, off, b"KEYW", 16, table_names!("KEYW"), |d_off| {
        let identifier = read_u64_named(crim, d_off, "KEYW.identifier")?;
        let message_identifier =
            opt_message_id(read_u32_named(crim, d_off + 8, "KEYW.message_identifier")?);
        let data_offset = read_u32_named(crim, d_off + 12, "KEYW.data_offset")?;
        let name = read_opt_name(crim, data_offset, "KEYW data")?;
        Ok(KeywordDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        })
    })?;

    Ok(KeywordDefinitions {
        offset: off,
        size,
        keywords,
    })
}

fn parse_levels(crim: &[u8], off: u32) -> Result<LevelDefinitions> {
    let (size, levels) = parse_table(crim, off, b"LEVL", 12, table_names!("LEVL"), |d_off| {
        let identifier = read_u32_named(crim, d_off, "LEVL.identifier")?;
        let message_identifier =
            opt_message_id(read_u32_named(crim, d_off + 4, "LEVL.message_identifier")?);
        let data_offset = read_u32_named(crim, d_off + 8, "LEVL.data_offset")?;
        let name = read_opt_name(crim, data_offset, "LEVL data")?;
        Ok(LevelDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        })
    })?;

    Ok(LevelDefinitions {
        offset: off,
        size,
        levels,
    })
}

fn parse_opcodes(crim: &[u8], off: u32) -> Result<OpcodeDefinitions> {
    let (size, opcodes) = parse_table(crim, off, b"OPCO", 12, table_names!("OPCO"), |d_off| {
        let identifier = read_u32_named(crim, d_off, "OPCO.identifier")?;
        let message_identifier =
            opt_message_id(read_u32_named(crim, d_off + 4, "OPCO.message_identifier")?);
        let data_offset = read_u32_named(crim, d_off + 8, "OPCO.data_offset")?;
        let name = read_opt_name(crim, data_offset, "OPCO data")?;
        Ok(OpcodeDefinition {
            identifier,
            message_identifier,
            data_offset,
            name,
        })
    })?;

    Ok(OpcodeDefinitions {
        offset: off,
        size,
        opcodes,
    })
}

fn parse_tasks(crim: &[u8], off: u32) -> Result<TaskDefinitions> {
    let (size, tasks) = parse_table(crim, off, b"TASK", 28, table_names!("TASK"), |d_off| {
        let identifier = read_u32_named(crim, d_off, "TASK.identifier")?;
        let message_identifier =
            opt_message_id(read_u32_named(crim, d_off + 4, "TASK.message_identifier")?);
        let mui_identifier = read_guid_named(crim, d_off + 8, "TASK.mui_identifier")?;
        let data_offset = read_u32_named(crim, d_off + 24, "TASK.data_offset")?;
        let name = read_opt_name(crim, data_offset, "TASK data")?;
        Ok(TaskDefinition {
            identifier,
            message_identifier,
            mui_identifier,
            data_offset,
            name,
        })
    })?;

    Ok(TaskDefinitions {
        offset: off,
        size,
        tasks,
    })
}

fn parse_ttbl<'a>(crim: &'a [u8], off: u32) -> Result<TemplateTable<'a>> {
    let (off_usize, size, count) =
        read_block_header(crim, off, b"TTBL", 12, &table_names!("TTBL"))?;
    let end = region_end(crim.len(), off, size, 12, "TTBL.size")?;
    let count_usize = u32_count(count, "TTBL.count", off + 8)?;

    let mut templates = Vec::with_capacity(count_usize);
    let mut cur = off_usize + 12;

    for _ in 0..count_usize {
        if cur + 40 > end {
            return Err(trunc_err(
                "TEMP header",
                usize_to_u32(cur),
                40,
                end.saturating_sub(cur),
            ));
        }
        let temp_sig = read_sig_named(crim, cur, "TEMP signature")?;
        if temp_sig != *b"TEMP" {
            return Err(sig_err(usize_to_u32(cur), b"TEMP", temp_sig));
        }
        let temp_size = read_u32_named(crim, cur + 4, "TEMP.size")?;
        if temp_size < 40 {
            return Err(size_err("TEMP.size", usize_to_u32(cur), temp_size));
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
            return Err(count_err(
                "TEMP.item_name_count (expected 0 when item_descriptor_count == 0)",
                temp_off_u32 + 12,
                item_name_count,
            ));
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
            return Err(off_err("TEMP.template_items_offset", items_abs, crim.len()));
        } else {
            items_abs - temp_off_u32
        };

        let items_rel_usize = u32_to_usize(
            items_rel,
            "TEMP.template_items_offset (relative)",
            template_slice.len(),
        )?;
        if items_rel_usize > template_slice.len() {
            return Err(off_err(
                "TEMP.template_items_offset (relative)",
                temp_off_u32.saturating_add(items_rel),
                crim.len(),
            ));
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
    let count_usize = u32_count(
        item_descriptor_count,
        "TEMP.item_descriptor_count",
        template_off_abs + 8,
    )?;

    if count_usize == 0 {
        // Validate template_items_offset for the zero-items case.
        if template_items_offset_abs != 0
            && template_items_offset_abs != template_off_abs.saturating_add(template.len() as u32)
        {
            return Err(off_err(
                "TEMP.template_items_offset (expected 0 or end-of-template when item_descriptor_count==0)",
                template_items_offset_abs,
                template_off_abs.saturating_add(template.len() as u32) as usize,
            ));
        }
        return Ok(vec![]);
    }

    if template_items_offset_abs < template_off_abs {
        return Err(off_err(
            "TEMP.template_items_offset",
            template_items_offset_abs,
            template_off_abs.saturating_add(template.len() as u32) as usize,
        ));
    }

    let rel = template_items_offset_abs - template_off_abs;
    let rel_usize = u32_to_usize(rel, "TEMP.template_items_offset (relative)", template.len())?;
    if rel_usize < 40 || rel_usize >= template.len() {
        return Err(off_err(
            "TEMP.template_items_offset (relative)",
            template_items_offset_abs,
            template_off_abs.saturating_add(template.len() as u32) as usize,
        ));
    }

    let needed = count_usize.checked_mul(20).ok_or_else(|| {
        count_err(
            "TEMP.item_descriptor_count",
            template_off_abs + 8,
            item_descriptor_count,
        )
    })?;
    if rel_usize + needed > template.len() {
        return Err(trunc_err(
            "template item descriptors",
            template_items_offset_abs,
            needed,
            template.len().saturating_sub(rel_usize),
        ));
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
                return Err(off_err(
                    "template item name_offset",
                    name_offset,
                    template_off_abs.saturating_add(template.len() as u32) as usize,
                ));
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
        return Err(off_err(
            "template item name_offset overlaps descriptor table",
            template_off_abs.saturating_add(min_name_rel as u32),
            template_off_abs.saturating_add(template.len() as u32) as usize,
        ));
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
    // MAPS contains value maps (VMAP) and bitmap maps (BMAP) that define enumeration/flag types
    // for event parameters. See libfwevt documentation:
    // https://github.com/libyal/libfwevt/blob/main/documentation/Windows%20Event%20manifest%20binary%20format.asciidoc
    //
    // Layout (libfwevt struct `fwevt_template_maps`):
    //   0:4   "MAPS" signature
    //   4:4   size (including header)
    //   8:4   count (number of maps)
    //   12:4  data_offset (unused by libfwevt — we ignore it too)
    //   16:   (count-1) * 4 bytes: offsets for maps 1..count
    //   ...:  map 0 starts immediately after the offsets array (implied)
    //   ...:  map 1+ at offsets from the array
    //
    // Each VMAP has its own `size` field, so we read that to determine extent — no sorting or
    // boundary guessing needed.

    let (off_usize, size, count) =
        read_block_header(crim, off, b"MAPS", 16, &table_names!("MAPS"))?;
    // Note: bytes 12-15 are `data_offset` in the struct but libfwevt ignores it; so do we.
    let end = region_end(crim.len(), off, size, 16, "MAPS.size")?;
    let count_usize = u32_count(count, "MAPS.count", off + 8)?;

    if count_usize == 0 {
        return Ok(MapsDefinitions {
            offset: off,
            size,
            maps: Vec::new(),
        });
    }

    // Read (count-1) offsets array at MAPS+16.
    let offs_array_off = off_usize + 16;
    let offs_array_bytes = count_usize.saturating_sub(1).checked_mul(4).unwrap_or(0);
    if offs_array_off + offs_array_bytes > crim.len() {
        return Err(size_err("MAPS offsets array", off, size));
    }

    // Build map offsets deterministically:
    // - map 0: implied at MAPS + 16 + (count-1)*4
    // - map 1+: from offsets array in order
    let implied_first = (offs_array_off + offs_array_bytes) as u32;
    let mut map_offsets = Vec::with_capacity(count_usize);
    map_offsets.push(implied_first);
    for i in 0..count_usize.saturating_sub(1) {
        let o = read_u32_named(crim, offs_array_off + i * 4, "MAPS.map_offset")?;
        map_offsets.push(o);
    }

    // Parse each map. Each VMAP declares its own size; BMAP format is unknown (we capture 4 bytes).
    let mut maps = Vec::with_capacity(count_usize);
    for &map_off in &map_offsets {
        let map_off_usize = u32_to_usize(map_off, "MAPS map offset", crim.len())?;
        if map_off_usize + 4 > crim.len() {
            return Err(trunc_err(
                "MAPS map signature",
                map_off,
                4,
                crim.len().saturating_sub(map_off_usize),
            ));
        }
        let sig = read_sig_named(crim, map_off_usize, "MAPS map signature")?;

        match &sig {
            b"VMAP" => {
                // VMAP has its own size field at offset 4.
                if map_off_usize + 8 > crim.len() {
                    return Err(trunc_err(
                        "VMAP size field",
                        map_off,
                        8,
                        crim.len().saturating_sub(map_off_usize),
                    ));
                }
                let vmap_size = read_u32_named(crim, map_off_usize + 4, "VMAP.size")?;
                let vmap_size_usize = usize::try_from(vmap_size)
                    .map_err(|_| size_err("VMAP.size", map_off, vmap_size))?;
                let slice_end = map_off_usize.saturating_add(vmap_size_usize).min(end);
                let map_slice = &crim[map_off_usize..slice_end];
                maps.push(MapDefinition::ValueMap(parse_vmap(
                    crim, map_off, map_slice,
                )?));
            }
            b"BMAP" => {
                // BMAP format is undocumented (TODO in libfwevt). Capture just the signature.
                let slice_end = (map_off_usize + 4).min(end);
                maps.push(MapDefinition::Bitmap(BitmapMap {
                    offset: map_off,
                    data: &crim[map_off_usize..slice_end],
                }));
            }
            _ => {
                // Unknown map type — capture just the signature.
                let slice_end = (map_off_usize + 4).min(end);
                maps.push(MapDefinition::Unknown {
                    signature: sig,
                    offset: map_off,
                    data: &crim[map_off_usize..slice_end],
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
        return Err(trunc_err("VMAP header", off, 16, map_slice.len()));
    }
    let size = read_u32_named(map_slice, 4, "VMAP.size")?;
    let map_string_offset = read_u32_named(map_slice, 8, "VMAP.map_string_offset")?;
    let entry_count = read_u32_named(map_slice, 12, "VMAP.entry_count")?;

    let size_usize = usize::try_from(size)
        .ok()
        .filter(|&s| s >= 16 && s <= map_slice.len())
        .ok_or_else(|| size_err("VMAP.size", off, size))?;

    let (entry_count_usize, entries_bytes) =
        count_bytes(entry_count, 8, "VMAP.entry_count", off + 12)?;

    if 16 + entries_bytes > size_usize {
        return Err(size_err("VMAP entries array", off, size));
    }

    let mut entries = Vec::with_capacity(entry_count_usize);
    for i in 0..entry_count_usize {
        let e_off = 16 + i * 8;
        let identifier = read_u32_named(map_slice, e_off, "VMAP.entry.identifier")?;
        let message_identifier = opt_message_id(read_u32_named(
            map_slice,
            e_off + 4,
            "VMAP.entry.message_identifier",
        )?);
        entries.push(ValueMapEntry {
            identifier,
            message_identifier,
        });
    }

    let map_string = read_opt_name(crim, map_string_offset, "VMAP map string")?;

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
