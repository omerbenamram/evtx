use crate::evtx_record::{EvtxRecord, SerializedEvtxRecord};
use crate::utils::bytes;
use crate::utils::utf16::decode_utf16le_bytes;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use thiserror::Error;

const MTA_MAGIC: &[u8; 8] = b"MTAFile\0";
const MTA_HEADER_SIZE: usize = 24;
const PAGE_PREFIX_SIZE: usize = 64;
const PAGE_METADATA_SIZE: usize = 24;
const PAGE_OFFSETS_COUNT: usize = 100;
const PAGE_OFFSETS_SIZE: usize = PAGE_OFFSETS_COUNT * 8;
const PAGE_HEADER_SIZE: usize = PAGE_PREFIX_SIZE + PAGE_METADATA_SIZE + PAGE_OFFSETS_SIZE;
const PAGE_SENTINEL: u32 = 0xffffffff;

#[derive(Debug, Error)]
pub enum MtaError {
    #[error("MTA file truncated while reading {what} at offset {offset} (need {need}, have {have})")]
    Truncated {
        what: &'static str,
        offset: usize,
        need: usize,
        have: usize,
    },

    #[error("Invalid MTA magic: {magic:?}")]
    InvalidMagic { magic: [u8; 8] },

    #[error("Unsupported MTA version {major}.{minor}")]
    UnsupportedVersion { major: u32, minor: u32 },

    #[error("Missing MTA section: {name}")]
    MissingSection { name: &'static str },

    #[error("Invalid MTA section descriptor: {message}")]
    InvalidSection { message: &'static str },

    #[error("Invalid MTA page: {message}")]
    InvalidPage { message: &'static str },

    #[error("Invalid UTF-16LE string in MTA file")]
    InvalidUtf16,

    #[error("I/O error while reading MTA file")]
    Io(#[from] std::io::Error),
}

pub type MtaResult<T> = std::result::Result<T, MtaError>;

#[derive(Debug, Clone)]
pub struct MtaFile {
    messages: Vec<Option<String>>,
    event_to_msg_index: Vec<Option<u32>>,
    event_index_to_msg_index: Vec<Option<u32>>,
}

impl MtaFile {
    pub fn from_path(path: impl AsRef<Path>) -> MtaResult<Self> {
        let mut file = File::open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    pub fn from_bytes(bytes: &[u8]) -> MtaResult<Self> {
        let magic = read_array::<8>(bytes, 0, "mta.magic")?;
        if &magic != MTA_MAGIC {
            return Err(MtaError::InvalidMagic { magic });
        }

        let major = read_u32(bytes, 8, "mta.version_major")?;
        let minor = read_u32(bytes, 12, "mta.version_minor")?;
        if major != 1 || minor != 1 {
            return Err(MtaError::UnsupportedVersion { major, minor });
        }

        let table_size = read_u32(bytes, 16, "mta.section_table_size")? as usize;
        let num_sections = read_u32(bytes, 20, "mta.num_sections")? as usize;

        let table_end = MTA_HEADER_SIZE
            .checked_add(table_size)
            .ok_or(MtaError::InvalidSection {
                message: "section table size overflow",
            })?;
        if table_end > bytes.len() {
            return Err(MtaError::Truncated {
                what: "mta.section_table",
                offset: MTA_HEADER_SIZE,
                need: table_size,
                have: bytes.len().saturating_sub(MTA_HEADER_SIZE),
            });
        }

        let mut sections = HashMap::new();
        let mut offset = MTA_HEADER_SIZE;
        for _ in 0..num_sections {
            let section_offset = read_u64(bytes, offset, "mta.section.offset")? as usize;
            let section_size = read_u64(bytes, offset + 8, "mta.section.size")? as usize;
            let name_len = read_u32(bytes, offset + 16, "mta.section.name_len")? as usize;
            let name_start = offset + 20;
            let name_end = name_start
                .checked_add(name_len)
                .ok_or(MtaError::InvalidSection {
                    message: "section name length overflow",
                })?;
            if name_end > bytes.len() {
                return Err(MtaError::Truncated {
                    what: "mta.section.name",
                    offset: name_start,
                    need: name_len,
                    have: bytes.len().saturating_sub(name_start),
                });
            }

            let name = decode_utf16le_bytes(&bytes[name_start..name_end])
                .map_err(|_| MtaError::InvalidUtf16)?;
            sections.insert(
                name,
                Section {
                    offset: section_offset,
                    size: section_size,
                },
            );
            offset = name_end;
        }

        let evt = sections
            .get("EVT")
            .ok_or(MtaError::MissingSection { name: "EVT" })?
            .slice(bytes)?;
        let msg = sections
            .get("MSG")
            .ok_or(MtaError::MissingSection { name: "MSG" })?
            .slice(bytes)?;

        let mut messages: Vec<Option<String>> = Vec::new();
        parse_paged_records(msg, |entry_index, payload| {
            if payload.len() < 4 {
                return Err(MtaError::InvalidSection {
                    message: "msg payload too small",
                });
            }
            let byte_len = read_u32(payload, 0, "msg.string_len")? as usize;
            if payload.len() < 4 + byte_len {
                return Err(MtaError::Truncated {
                    what: "msg.string_bytes",
                    offset: 4,
                    need: byte_len,
                    have: payload.len().saturating_sub(4),
                });
            }
            let raw = &payload[4..4 + byte_len];
            let string = decode_utf16le_nul(raw)?;
            let index = entry_index as usize;
            if index >= messages.len() {
                messages.resize(index + 1, None);
            }
            messages[index] = Some(string);
            Ok(())
        })?;

        let mut event_to_msg_index: Vec<Option<u32>> = Vec::new();
        let mut event_index_to_msg_index: Vec<Option<u32>> = Vec::new();
        parse_paged_records(evt, |entry_index, payload| {
            if payload.len() < 16 {
                return Err(MtaError::InvalidSection {
                    message: "evt payload too small",
                });
            }
            let event_value = read_u32(payload, 0, "evt.event_value")?;
            let msg_index = read_u32(payload, 8, "evt.msg_index")?;

            let entry = entry_index as usize;
            if entry >= event_index_to_msg_index.len() {
                event_index_to_msg_index.resize(entry + 1, None);
            }
            event_index_to_msg_index[entry] = Some(msg_index);

            let idx = event_value as usize;
            if idx >= event_to_msg_index.len() {
                event_to_msg_index.resize(idx + 1, None);
            }
            event_to_msg_index[idx] = Some(msg_index);
            Ok(())
        })?;

        Ok(MtaFile {
            messages,
            event_to_msg_index,
            event_index_to_msg_index,
        })
    }

    pub fn message_for_event_value(&self, event_value: u32) -> Option<&str> {
        let msg_index = *self.event_to_msg_index.get(event_value as usize)?;
        self.message_by_index(msg_index?)
    }

    pub fn message_for_record_id(&self, record_id: u64) -> Option<&str> {
        let event_value = u32::try_from(record_id).ok()?;
        self.message_for_event_value(event_value)
    }

    pub fn message_for_entry_index(&self, entry_index: u32) -> Option<&str> {
        let msg_index = *self.event_index_to_msg_index.get(entry_index as usize)?;
        self.message_by_index(msg_index?)
    }

    /// Look up a localized message for a serialized EVTX record using its `event_record_id`.
    pub fn message_for_record<T>(&self, record: &SerializedEvtxRecord<T>) -> Option<&str> {
        self.message_for_record_id(record.event_record_id)
    }

    /// Look up a localized message for an EVTX record using its `event_record_id`.
    pub fn message_for_evtx_record(&self, record: &EvtxRecord<'_>) -> Option<&str> {
        self.message_for_record_id(record.event_record_id)
    }

    pub fn message_by_index(&self, msg_index: u32) -> Option<&str> {
        self.messages
            .get(msg_index as usize)
            .and_then(|value| value.as_deref())
    }
}

#[derive(Debug, Clone, Copy)]
struct Section {
    offset: usize,
    size: usize,
}

impl Section {
    fn slice<'a>(&self, bytes: &'a [u8]) -> MtaResult<&'a [u8]> {
        let end = self
            .offset
            .checked_add(self.size)
            .ok_or(MtaError::InvalidSection {
                message: "section size overflow",
            })?;
        if end > bytes.len() {
            return Err(MtaError::Truncated {
                what: "mta.section",
                offset: self.offset,
                need: self.size,
                have: bytes.len().saturating_sub(self.offset),
            });
        }
        bytes
            .get(self.offset..end)
            .ok_or(MtaError::InvalidSection {
                message: "section slice out of bounds",
            })
    }
}

fn parse_paged_records<F>(section: &[u8], mut on_record: F) -> MtaResult<()>
where
    F: FnMut(u32, &[u8]) -> MtaResult<()>,
{
    if section.is_empty() || section.iter().all(|&b| b == 0) {
        return Ok(());
    }

    let mut page_offset = 0usize;
    let mut safety_counter = 0usize;

    loop {
        if page_offset >= section.len() {
            return Err(MtaError::InvalidPage {
                message: "page offset out of bounds",
            });
        }
        let _ = slice(section, page_offset, PAGE_HEADER_SIZE, "mta.page.header")?;

        let sentinel = read_u32(section, page_offset, "mta.page.sentinel")?;
        if sentinel != PAGE_SENTINEL {
            return Err(MtaError::InvalidPage {
                message: "invalid page sentinel",
            });
        }

        let footer_sentinel = read_u32(section, page_offset + 60, "mta.page.footer_sentinel")?;
        if footer_sentinel != PAGE_SENTINEL {
            return Err(MtaError::InvalidPage {
                message: "invalid page footer sentinel",
            });
        }

        let meta_offset = page_offset + PAGE_PREFIX_SIZE;
        let _page_index = read_u64(section, meta_offset, "mta.page.index")?;
        let next_offset = read_u64(section, meta_offset + 8, "mta.page.next_offset")? as usize;
        let self_offset = read_u64(section, meta_offset + 16, "mta.page.self_offset")? as usize;

        let offsets_start = meta_offset + PAGE_METADATA_SIZE;
        for idx in 0..PAGE_OFFSETS_COUNT {
            let off = read_u64(
                section,
                offsets_start + idx * 8,
                "mta.page.offset",
            )? as usize;
            if off == 0 {
                continue;
            }
            if off + 8 > section.len() {
                return Err(MtaError::Truncated {
                    what: "mta.record.header",
                    offset: off,
                    need: 8,
                    have: section.len().saturating_sub(off),
                });
            }
            let entry_index = read_u32(section, off, "mta.record.entry_index")?;
            let payload_size = read_u32(section, off + 4, "mta.record.payload_size")? as usize;
            let payload_start = off + 8;
            let payload_end = payload_start
                .checked_add(payload_size)
                .ok_or(MtaError::InvalidSection {
                    message: "record payload size overflow",
                })?;
            if payload_end > section.len() {
                return Err(MtaError::Truncated {
                    what: "mta.record.payload",
                    offset: payload_start,
                    need: payload_size,
                    have: section.len().saturating_sub(payload_start),
                });
            }
            on_record(entry_index, &section[payload_start..payload_end])?;
        }

        if next_offset == self_offset {
            break;
        }
        if next_offset <= page_offset || next_offset >= section.len() {
            return Err(MtaError::InvalidPage {
                message: "invalid next page offset",
            });
        }

        page_offset = next_offset;
        safety_counter += 1;
        if safety_counter > 10_000 {
            return Err(MtaError::InvalidPage {
                message: "too many pages",
            });
        }
    }

    Ok(())
}

fn read_array<const N: usize>(buf: &[u8], offset: usize, what: &'static str) -> MtaResult<[u8; N]> {
    bytes::read_array::<N>(buf, offset).ok_or_else(|| MtaError::Truncated {
        what,
        offset,
        need: N,
        have: buf.len().saturating_sub(offset),
    })
}

fn read_u32(buf: &[u8], offset: usize, what: &'static str) -> MtaResult<u32> {
    bytes::read_u32_le(buf, offset).ok_or_else(|| MtaError::Truncated {
        what,
        offset,
        need: 4,
        have: buf.len().saturating_sub(offset),
    })
}

fn read_u64(buf: &[u8], offset: usize, what: &'static str) -> MtaResult<u64> {
    bytes::read_u64_le(buf, offset).ok_or_else(|| MtaError::Truncated {
        what,
        offset,
        need: 8,
        have: buf.len().saturating_sub(offset),
    })
}

fn slice<'a>(buf: &'a [u8], offset: usize, len: usize, what: &'static str) -> MtaResult<&'a [u8]> {
    let end = offset
        .checked_add(len)
        .ok_or(MtaError::InvalidSection {
            message: "slice length overflow",
        })?;
    buf.get(offset..end).ok_or(MtaError::Truncated {
        what,
        offset,
        need: len,
        have: buf.len().saturating_sub(offset),
    })
}

fn decode_utf16le_nul(bytes: &[u8]) -> MtaResult<String> {
    if !bytes.len().is_multiple_of(2) {
        return Err(MtaError::InvalidUtf16);
    }
    let mut end = bytes.len();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == 0 && bytes[i + 1] == 0 {
            end = i;
            break;
        }
        i += 2;
    }
    decode_utf16le_bytes(&bytes[..end]).map_err(|_| MtaError::InvalidUtf16)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_u32(buf: &mut Vec<u8>, value: u32) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn push_u64(buf: &mut Vec<u8>, value: u64) {
        buf.extend_from_slice(&value.to_le_bytes());
    }

    fn encode_utf16le(value: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for unit in value.encode_utf16() {
            out.extend_from_slice(&unit.to_le_bytes());
        }
        out
    }

    fn build_page(records: Vec<(u32, Vec<u8>)>) -> Vec<u8> {
        let mut page = vec![0u8; PAGE_HEADER_SIZE];
        page[0..4].copy_from_slice(&PAGE_SENTINEL.to_le_bytes());
        page[60..64].copy_from_slice(&PAGE_SENTINEL.to_le_bytes());

        let meta_offset = PAGE_PREFIX_SIZE;
        let next_offset = 0u64;
        let self_offset = 0u64;
        page[meta_offset..meta_offset + 8].copy_from_slice(&0u64.to_le_bytes());
        page[meta_offset + 8..meta_offset + 16].copy_from_slice(&next_offset.to_le_bytes());
        page[meta_offset + 16..meta_offset + 24].copy_from_slice(&self_offset.to_le_bytes());

        let mut record_offset = PAGE_HEADER_SIZE;
        for (idx, (entry_index, payload)) in records.into_iter().enumerate() {
            let offset_pos = meta_offset + PAGE_METADATA_SIZE + idx * 8;
            let offset_value = record_offset as u64;
            page[offset_pos..offset_pos + 8].copy_from_slice(&offset_value.to_le_bytes());

            let mut record = Vec::with_capacity(8 + payload.len());
            push_u32(&mut record, entry_index);
            push_u32(&mut record, payload.len() as u32);
            record.extend_from_slice(&payload);

            page.extend_from_slice(&record);
            record_offset += record.len();
        }

        page
    }

    fn build_mta_bytes() -> Vec<u8> {
        let mut msg_payload = Vec::new();
        let msg_bytes = encode_utf16le("hello\0");
        push_u32(&mut msg_payload, msg_bytes.len() as u32);
        msg_payload.extend_from_slice(&msg_bytes);

        let mut evt_payload = vec![0u8; 16];
        evt_payload[0..4].copy_from_slice(&42u32.to_le_bytes());
        evt_payload[8..12].copy_from_slice(&0u32.to_le_bytes());

        let msg_section = build_page(vec![(0, msg_payload)]);
        let evt_section = build_page(vec![(0, evt_payload)]);

        let evt_name = encode_utf16le("EVT");
        let msg_name = encode_utf16le("MSG");

        let evt_entry_size = 20 + evt_name.len();
        let msg_entry_size = 20 + msg_name.len();
        let table_size = evt_entry_size + msg_entry_size;

        let evt_offset = MTA_HEADER_SIZE + table_size;
        let msg_offset = evt_offset + evt_section.len();

        let mut bytes = Vec::new();
        bytes.extend_from_slice(MTA_MAGIC);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, 1);
        push_u32(&mut bytes, table_size as u32);
        push_u32(&mut bytes, 2);

        push_u64(&mut bytes, evt_offset as u64);
        push_u64(&mut bytes, evt_section.len() as u64);
        push_u32(&mut bytes, evt_name.len() as u32);
        bytes.extend_from_slice(&evt_name);

        push_u64(&mut bytes, msg_offset as u64);
        push_u64(&mut bytes, msg_section.len() as u64);
        push_u32(&mut bytes, msg_name.len() as u32);
        bytes.extend_from_slice(&msg_name);

        bytes.extend_from_slice(&evt_section);
        bytes.extend_from_slice(&msg_section);

        bytes
    }

    #[test]
    fn test_mta_minimal_message_lookup() {
        let bytes = build_mta_bytes();
        let mta = MtaFile::from_bytes(&bytes).expect("failed to parse MTA bytes");

        assert_eq!(mta.message_for_event_value(42), Some("hello"));
        assert_eq!(mta.message_for_entry_index(0), Some("hello"));
        assert_eq!(mta.message_for_record_id(42), Some("hello"));
        assert_eq!(mta.message_for_event_value(7), None);
    }

    #[test]
    fn test_decode_utf16le_nul_rejects_odd_length() {
        let err = decode_utf16le_nul(&[0x61]).expect_err("expected invalid utf16");
        assert!(matches!(err, MtaError::InvalidUtf16));
    }
}
