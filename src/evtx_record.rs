use crate::binxml::deserializer::BinXmlDeserializer;
use crate::binxml::ir_json::render_json_record;
use crate::binxml::ir_xml::render_xml_record;
use crate::err::{
    DeserializationError, DeserializationResult, EvtxError, Result, SerializationError,
};
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::model::ir::IrTree;
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
    /// Consumes the record, returning a `EvtxRecordWithJsonValue` with the `serde_json::Value` data.
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

    /// Consumes the record and parse it, producing a JSON serialized record.
    pub fn into_json(self) -> Result<SerializedEvtxRecord<String>> {
        let indent = self.settings.should_indent();
        let record_with_json = self.into_json_stream()?;

        if !indent {
            return Ok(record_with_json);
        }

        let value: serde_json::Value =
            serde_json::from_str(&record_with_json.data).map_err(SerializationError::from)?;
        let data = serde_json::to_string_pretty(&value).map_err(SerializationError::from)?;

        Ok(SerializedEvtxRecord {
            event_record_id: record_with_json.event_record_id,
            timestamp: record_with_json.timestamp,
            data,
        })
    }

    /// Consumes the record and streams JSON directly into a buffer using the IR tree renderer.
    pub fn into_json_stream(self) -> Result<SerializedEvtxRecord<String>> {
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

    /// Rebuild the flattened token stream on demand (legacy tooling only).
    pub fn tokens(&self) -> Result<Vec<BinXMLDeserializedTokens<'a>>> {
        let deserializer = BinXmlDeserializer::init(
            self.chunk.data,
            self.binxml_offset,
            Some(self.chunk),
            false,
            self.settings.get_ansi_codec(),
        );

        let iter = deserializer
            .iter_tokens(Some(self.binxml_size))
            .map_err(|e| EvtxError::FailedToParseRecord {
                record_id: self.event_record_id,
                source: Box::new(EvtxError::DeserializationError(e)),
            })?;

        let mut tokens = Vec::new();
        for token in iter {
            let token = token.map_err(|e| EvtxError::FailedToParseRecord {
                source: Box::new(EvtxError::DeserializationError(e)),
                record_id: self.event_record_id,
            })?;
            tokens.push(token);
        }

        Ok(tokens)
    }
}
