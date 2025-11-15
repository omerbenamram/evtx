use crate::binxml::assemble::{parse_tokens, parse_tokens_streaming};
use crate::err::{
    DeserializationError, DeserializationResult, EvtxError, Result, SerializationError,
};
use crate::json_output::JsonOutput;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::{EvtxChunk, ParserSettings};

use byteorder::ReadBytesExt;
use chrono::prelude::*;
use std::io::{Cursor, Read};
use std::sync::Arc;

pub type RecordId = u64;

#[derive(Debug, Clone)]
pub struct EvtxRecord<'a> {
    pub chunk: &'a EvtxChunk<'a>,
    pub event_record_id: RecordId,
    pub timestamp: DateTime<Utc>,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
    pub settings: Arc<ParserSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvtxRecordHeader {
    pub data_size: u32,
    pub event_record_id: RecordId,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerializedEvtxRecord<T> {
    pub event_record_id: RecordId,
    pub timestamp: DateTime<Utc>,
    pub data: T,
}

impl EvtxRecordHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> DeserializationResult<EvtxRecordHeader> {
        let mut magic = [0_u8; 4];
        input.take(4).read_exact(&mut magic)?;

        if &magic != b"\x2a\x2a\x00\x00" {
            return Err(DeserializationError::InvalidEvtxRecordHeaderMagic { magic });
        }

        let size = try_read!(input, u32)?;
        let record_id = try_read!(input, u64)?;
        let timestamp = try_read!(input, filetime)?;

        Ok(EvtxRecordHeader {
            data_size: size,
            event_record_id: record_id,
            timestamp,
        })
    }

    pub fn record_data_size(&self) -> Result<u32> {
        // 24 - record header size
        // 4 - copy of size record size
        let decal = 24 + 4;
        if self.data_size < decal {
            return Err(EvtxError::InvalidDataSize {
                length: self.data_size,
                expected: decal,
            });
        }
        Ok(self.data_size - decal)
    }
}

impl EvtxRecord<'_> {
    /// Consumes the record, processing it using the given `output_builder`.
    pub fn into_output<T: BinXmlOutput>(self, output_builder: &mut T) -> Result<()> {
        let event_record_id = self.event_record_id;
        parse_tokens(self.tokens, self.chunk, output_builder).map_err(|e| {
            EvtxError::FailedToParseRecord {
                record_id: event_record_id,
                source: Box::new(e),
            }
        })?;

        Ok(())
    }

    /// Consumes the record, returning a `EvtxRecordWithJsonValue` with the `serde_json::Value` data.
    pub fn into_json_value(self) -> Result<SerializedEvtxRecord<serde_json::Value>> {
        let mut output_builder = JsonOutput::new(&self.settings);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        self.into_output(&mut output_builder)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data: output_builder.into_value()?,
        })
    }

    /// Consumes the record and parse it, producing a JSON serialized record.
    pub fn into_json(self) -> Result<SerializedEvtxRecord<String>> {
        let indent = self.settings.should_indent();
        let record_with_json_value = self.into_json_value()?;

        let data = if indent {
            serde_json::to_string_pretty(&record_with_json_value.data)
                .map_err(SerializationError::from)?
        } else {
            serde_json::to_string(&record_with_json_value.data).map_err(SerializationError::from)?
        };

        Ok(SerializedEvtxRecord {
            event_record_id: record_with_json_value.event_record_id,
            timestamp: record_with_json_value.timestamp,
            data,
        })
    }

    /// Consumes the record and streams JSON directly into a buffer using the streaming visitor.
    pub fn into_json_stream(self) -> Result<SerializedEvtxRecord<String>> {
        // Estimate buffer size based on token count
        let capacity_hint = self.tokens.len().saturating_mul(64);
        let buf = Vec::with_capacity(capacity_hint);
        let mut output_builder = crate::JsonStreamOutput::with_writer(buf, &self.settings);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        parse_tokens_streaming(&self.tokens, self.chunk, &mut output_builder).map_err(|e| {
            EvtxError::FailedToParseRecord {
                record_id: event_record_id,
                source: Box::new(e),
            }
        })?;

        let data = String::from_utf8(output_builder.into_writer())
            .map_err(crate::err::SerializationError::from)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord<String>> {
        let mut output_builder = XmlOutput::with_writer(Vec::new(), &self.settings);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        self.into_output(&mut output_builder)?;

        let data =
            String::from_utf8(output_builder.into_writer()).map_err(SerializationError::from)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data,
        })
    }
}
