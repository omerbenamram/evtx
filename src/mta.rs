//! Parser for MTA (Message Table Archive) files.
//!
//! MTA files are exported by Windows Event Viewer alongside `.evtx` files and
//! contain localized message strings needed to render human-readable event log
//! entries. Filename convention: `{prefix}_eventlog_{LogName}_{LCID}.MTA`.
//!
//! All multi-byte integers are little-endian.
//!
//! ## File header (24 bytes)
//!
//! | Offset | Size | Type     | Description                    |
//! |--------|------|----------|--------------------------------|
//! | 0x00   | 8    | char[]   | Magic: `MTAFile\0`            |
//! | 0x08   | 4    | u32      | Version major (1)              |
//! | 0x0C   | 4    | u32      | Version minor (1)              |
//! | 0x10   | 4    | u32      | Section descriptor table size  |
//! | 0x14   | 4    | u32      | Number of sections             |
//!
//! ## Section descriptors (variable, immediately after header)
//!
//! Each descriptor:
//!
//! | Size | Type     | Description                              |
//! |------|----------|------------------------------------------|
//! | 8    | u64      | Absolute offset to section data          |
//! | 8    | u64      | Section data size in bytes               |
//! | 4    | u32      | Section name byte length                 |
//! | var  | UTF-16LE | Section name (not null-terminated)        |
//!
//! Known sections: **EVT** (event→message mapping), **MSG** (localized
//! strings), **PUB** (publisher/provider metadata).
//!
//! ## Paged array (shared structure for all sections)
//!
//! Each section is a linked list of fixed-capacity pages (100 entries each).
//!
//! Page layout:
//!
//! | Size | Description                                           |
//! |------|-------------------------------------------------------|
//! | 4    | Sentinel (`0xFFFFFFFF`)                               |
//! | 56   | Bucket array (14 × u32, all identical — max index)   |
//! | 4    | Sentinel (`0xFFFFFFFF`)                               |
//! | 24   | Metadata: page_index (u64), next_offset (u64), self_offset (u64) |
//! | 800  | Offset array (100 × u64, 0 = unused)                 |
//! | var  | Records                                               |
//!
//! Last page: `next_offset == self_offset`.
//!
//! Each record: entry_index (u32), payload_size (u32), then payload.
//!
//! ## EVT record payload (16 bytes, fixed)
//!
//! | Offset | Size | Type | Description                           |
//! |--------|------|------|---------------------------------------|
//! | 0      | 4    | u32  | Event record ID (from record payload) |
//! | 4      | 4    | u32  | (padding, 0)                          |
//! | 8      | 4    | u32  | MSG string index                      |
//! | 12     | 4    | u32  | (padding, 0)                          |
//!
//! ## MSG record payload (variable)
//!
//! | Offset | Size | Type     | Description                     |
//! |--------|------|----------|---------------------------------|
//! | 0      | 4    | u32      | String byte length (incl. NUL)  |
//! | 4      | var  | UTF-16LE | Null-terminated message string  |

use crate::binxml::value_variant::BinXmlValue;
use crate::evtx_record::{EvtxRecord, SerializedEvtxRecord};
use crate::model::ir::{Node, Text};
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
    #[error(
        "MTA file truncated while reading {what} at offset {offset} (need {need}, have {have})"
    )]
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
    event_record_id_to_msg: HashMap<u32, String>,
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

        let table_end =
            MTA_HEADER_SIZE
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
            // Strip trailing UTF-16 null if present, then decode by known length.
            let trimmed = raw
                .strip_suffix(&[0, 0])
                .unwrap_or(raw);
            let string =
                decode_utf16le_bytes(trimmed).map_err(|_| MtaError::InvalidUtf16)?;
            let index = entry_index as usize;
            if index >= messages.len() {
                messages.resize(index + 1, None);
            }
            messages[index] = Some(string);
            Ok(())
        })?;

        let mut event_record_id_to_msg: HashMap<u32, String> = HashMap::new();
        parse_paged_records(evt, |entry_index, payload| {
            if payload.len() < 16 {
                return Err(MtaError::InvalidSection {
                    message: "evt payload too small",
                });
            }

            // EventRecordId field in event
            let event_record_id = read_u32(payload, 0, "evt.event_record_id")?;

            // Index of the message in the MSG section that corresponds to this event
            let msg_index = read_u32(payload, 8, "evt.msg_index")?;

            let message = messages
                .get(msg_index as usize)
                .and_then(|opt| opt.as_ref())
                .ok_or(MtaError::InvalidSection {
                    message: "evt references non-existent msg index",
                })?;

            event_record_id_to_msg.insert(event_record_id, message.clone());
            Ok(())
        })?;

        Ok(MtaFile {
            event_record_id_to_msg,
        })
    }

    pub fn message_for_record_id(&self, record_id: u32) -> Option<&str> {
        self.event_record_id_to_msg
            .get(&record_id)
            .map(|s| s.as_str())
    }

    /// Look up a localized message for a serialized EVTX record by extracting
    /// `EventRecordID` from the serialized payload (JSON or XML string).
    pub fn message_for_record(&self, record: &SerializedEvtxRecord<String>) -> Option<&str> {
        let id = extract_event_record_id_from_str(&record.data)?;
        self.message_for_record_id(id)
    }

    /// Look up a localized message for an EVTX record by extracting
    /// `EventRecordID` from the IR tree (Event > System > EventRecordID).
    pub fn message_for_evtx_record(&self, record: &EvtxRecord<'_>) -> Option<&str> {
        let id = extract_event_record_id_from_tree(&record.tree)? as u32;
        self.message_for_record_id(id)
    }
}

/// Extract `EventRecordID` from the IR tree (Event > System > EventRecordID).
fn extract_event_record_id_from_tree(tree: &crate::model::ir::IrTree<'_>) -> Option<u64> {
    let root = tree.root_element();
    let arena = tree.arena();

    let system_id = root.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "System").then_some(*id)
        }
        _ => None,
    })?;

    let system = arena.get(system_id)?;

    let erid_el_id = system.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "EventRecordID").then_some(*id)
        }
        _ => None,
    })?;

    let erid_el = arena.get(erid_el_id)?;

    erid_el.children.iter().find_map(|node| match node {
        Node::Text(Text::Utf8(s)) => s.parse::<u64>().ok(),
        Node::Text(Text::Utf16(s)) => s.to_string().ok()?.parse::<u64>().ok(),
        Node::Value(BinXmlValue::UInt64Type(v)) => Some(*v),
        Node::Value(BinXmlValue::UInt32Type(v)) => Some(*v as u64),
        Node::Value(BinXmlValue::Int64Type(v)) => u64::try_from(*v).ok(),
        Node::Value(BinXmlValue::Int32Type(v)) => u64::try_from(*v).ok(),
        _ => None,
    })
}

/// Extract `EventRecordID` from a serialized record string (JSON or XML).
fn extract_event_record_id_from_str(data: &str) -> Option<u32> {
    // Try JSON first: deserialize and walk Event.System.EventRecordID
    if data.starts_with('{') {
        let v: serde_json::Value = serde_json::from_str(data).ok()?;
        let id = v.get("Event")?.get("System")?.get("EventRecordID")?;
        return match id {
            serde_json::Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
            serde_json::Value::String(s) => s.parse::<u32>().ok(),
            _ => None,
        };
    }
    // XML: <EventRecordID>123</EventRecordID>
    if let Some(pos) = data.find("<EventRecordID") {
        let rest = &data[pos..];
        let gt = rest.find('>')?;
        let after = &rest[gt + 1..];
        let end = after.find('<')?;
        return after[..end].trim().parse::<u32>().ok();
    }
    None
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
        bytes.get(self.offset..end).ok_or(MtaError::InvalidSection {
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
            let off = read_u64(section, offsets_start + idx * 8, "mta.page.offset")? as usize;
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
            let payload_end =
                payload_start
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
    let end = offset.checked_add(len).ok_or(MtaError::InvalidSection {
        message: "slice length overflow",
    })?;
    buf.get(offset..end).ok_or(MtaError::Truncated {
        what,
        offset,
        need: len,
        have: buf.len().saturating_sub(offset),
    })
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

        assert_eq!(mta.message_for_record_id(42), Some("hello"));
    }

}
