use crate::binxml::ir_json::render_json_record;
use crate::binxml::ir_xml::render_xml_record;
use crate::err::{DeserializationError, DeserializationResult, EvtxError, Result};
use crate::model::ir::IrTree;
use crate::utils::ByteCursor;
use crate::utils::bytes;
use crate::utils::windows::filetime_to_timestamp;
use crate::{EvtxChunk, ParserSettings};

use jiff::Timestamp;
use std::io::Cursor;
use std::sync::Arc;

pub type RecordId = u64;

pub(crate) const EVTX_RECORD_HEADER_SIZE: usize = 24;

#[derive(Debug, Clone)]
pub struct EvtxRecord<'a> {
    pub chunk: &'a EvtxChunk<'a>,
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
    pub tree: IrTree<'a>,
    pub binxml_offset: u64,
    pub binxml_size: u32,
    pub settings: Arc<ParserSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvtxRecordHeader {
    pub data_size: u32,
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedEvtxRecord<T> {
    pub event_record_id: RecordId,
    pub timestamp: Timestamp,
    pub data: T,
}

impl EvtxRecordHeader {
    pub fn from_bytes_at(buf: &[u8], offset: usize) -> DeserializationResult<EvtxRecordHeader> {
        let _ = bytes::slice_r(buf, offset, EVTX_RECORD_HEADER_SIZE, "EVTX record header")?;

        let magic = bytes::read_array_r::<4>(buf, offset, "record header magic")?;
        if &magic != b"\x2a\x2a\x00\x00" {
            return Err(DeserializationError::InvalidEvtxRecordHeaderMagic { magic });
        }

        let size = bytes::read_u32_le_r(buf, offset + 4, "record.data_size")?;
        let record_id = bytes::read_u64_le_r(buf, offset + 8, "record.event_record_id")?;
        let filetime = bytes::read_u64_le_r(buf, offset + 16, "record.filetime")?;

        let timestamp = filetime_to_timestamp(filetime)?;

        Ok(EvtxRecordHeader {
            data_size: size,
            event_record_id: record_id,
            timestamp,
        })
    }

    pub fn from_bytes(buf: &[u8]) -> DeserializationResult<EvtxRecordHeader> {
        Self::from_bytes_at(buf, 0)
    }

    pub fn from_reader(input: &mut Cursor<&[u8]>) -> DeserializationResult<EvtxRecordHeader> {
        let start = input.position() as usize;
        let buf = input.get_ref();
        let header = Self::from_bytes_at(buf, start)?;
        input.set_position((start + EVTX_RECORD_HEADER_SIZE) as u64);
        Ok(header)
    }

    pub fn record_data_size(&self) -> Result<u32> {
        // 24 - record header size
        // 4 - copy of size record size
        let decal = EVTX_RECORD_HEADER_SIZE as u32 + 4;
        if self.data_size < decal {
            return Err(EvtxError::InvalidDataSize {
                length: self.data_size,
                expected: decal,
            });
        }
        Ok(self.data_size - decal)
    }
}

impl<'a> EvtxRecord<'a> {
    /// Consumes the record and returns the rendered JSON as a `serde_json::Value`.
    pub fn into_json_value(self) -> Result<SerializedEvtxRecord<serde_json::Value>> {
        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        let record_with_json = self.into_json()?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data: serde_json::from_str(&record_with_json.data)
                .map_err(crate::err::SerializationError::from)?,
        })
    }

    /// Consumes the record and renders it as compact JSON (streaming IR renderer).
    pub fn into_json(self) -> Result<SerializedEvtxRecord<String>> {
        // Estimate buffer size based on BinXML size
        let capacity_hint = self.binxml_size as usize * 2;
        let buf = Vec::with_capacity(capacity_hint);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;

        let mut writer = buf;
        render_json_record(&self.tree, &self.settings, &mut writer).map_err(|e| {
            EvtxError::FailedToParseRecord {
                record_id: event_record_id,
                source: Box::new(e),
            }
        })?;
        let data = String::from_utf8(writer).map_err(crate::err::SerializationError::from)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord<String>> {
        let capacity_hint = self.binxml_size as usize * 2;
        let buf = Vec::with_capacity(capacity_hint);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;

        let mut writer = buf;
        render_xml_record(&self.tree, &self.settings, &mut writer).map_err(|e| {
            EvtxError::FailedToParseRecord {
                record_id: event_record_id,
                source: Box::new(e),
            }
        })?;

        let data = String::from_utf8(writer).map_err(crate::err::SerializationError::from)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data,
        })
    }

    /// Parse all `TemplateInstance` substitution arrays from this record.
    ///
    /// This is a lightweight scan over the record's BinXML stream that extracts typed substitution
    /// values without building a legacy token vector.
    pub fn template_instances(&self) -> Result<Vec<crate::binxml::BinXmlTemplateValues<'a>>> {
        use crate::binxml::name::BinXmlNameEncoding;
        use crate::binxml::tokens::{
            read_attribute_cursor, read_entity_ref_cursor, read_fragment_header_cursor,
            read_open_start_element_cursor, read_processing_instruction_data_cursor,
            read_processing_instruction_target_cursor, read_substitution_descriptor_cursor,
            read_template_values_cursor,
        };

        let ansi_codec = self.settings.get_ansi_codec();
        let mut out: Vec<crate::binxml::BinXmlTemplateValues<'a>> = Vec::new();

        let mut cursor = ByteCursor::with_pos(self.chunk.data, self.binxml_offset as usize)?;
        let mut data_read: u32 = 0;
        let data_size = self.binxml_size;
        let mut eof = false;

        while !eof && data_read < data_size {
            let start = cursor.position();
            let token_byte = cursor.u8()?;

            match token_byte {
                0x00 => {
                    eof = true;
                }
                0x0c => {
                    let template = read_template_values_cursor(
                        &mut cursor,
                        Some(self.chunk),
                        ansi_codec,
                        &self.chunk.arena,
                    )?;
                    out.push(template);
                }
                0x01 => {
                    let _ = read_open_start_element_cursor(
                        &mut cursor,
                        false,
                        false,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x41 => {
                    let _ = read_open_start_element_cursor(
                        &mut cursor,
                        true,
                        false,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x02..=0x04 => {
                    // Structural tokens; no payload.
                }
                0x05 | 0x45 => {
                    let _ = crate::binxml::value_variant::BinXmlValue::from_binxml_cursor_in(
                        &mut cursor,
                        Some(self.chunk),
                        None,
                        ansi_codec,
                        &self.chunk.arena,
                    )?;
                }
                0x06 | 0x46 => {
                    let _ = read_attribute_cursor(&mut cursor, BinXmlNameEncoding::Offset)?;
                }
                0x09 | 0x49 => {
                    let _ = read_entity_ref_cursor(&mut cursor, BinXmlNameEncoding::Offset)?;
                }
                0x0a => {
                    let _ = read_processing_instruction_target_cursor(
                        &mut cursor,
                        BinXmlNameEncoding::Offset,
                    )?;
                }
                0x0b => {
                    let _ = read_processing_instruction_data_cursor(&mut cursor)?;
                }
                0x0d => {
                    let _ = read_substitution_descriptor_cursor(&mut cursor, false)?;
                }
                0x0e => {
                    let _ = read_substitution_descriptor_cursor(&mut cursor, true)?;
                }
                0x0f => {
                    let _ = read_fragment_header_cursor(&mut cursor)?;
                }
                0x07 | 0x47 => {
                    return Err(DeserializationError::UnimplementedToken {
                        name: "CDataSection",
                        offset: cursor.position(),
                    }
                    .into());
                }
                0x08 | 0x48 => {
                    return Err(DeserializationError::UnimplementedToken {
                        name: "CharReference",
                        offset: cursor.position(),
                    }
                    .into());
                }
                _ => {
                    return Err(DeserializationError::InvalidToken {
                        value: token_byte,
                        offset: cursor.position(),
                    }
                    .into());
                }
            }

            let total_read = cursor.position() - start;
            data_read = data_read.saturating_add(total_read as u32);
        }

        Ok(out)
    }
}
