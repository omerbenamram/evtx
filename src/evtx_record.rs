use crate::binxml::assemble::parse_tokens;
use crate::err::{self, Result};
use crate::evtx_parser::ReadSeek;
use crate::json_output::JsonOutput;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::ParserSettings;

use std::io::{Cursor, Read};

use byteorder::ReadBytesExt;
use chrono::prelude::*;
use snafu::{ensure, ResultExt};

#[derive(Debug, Clone, PartialEq)]
pub struct EvtxRecord<'a> {
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
    pub settings: &'a ParserSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvtxRecordHeader {
    pub data_size: u32,
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SerializedEvtxRecord<T> {
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
    pub data: T,
}

impl EvtxRecordHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxRecordHeader> {
        let mut magic = [0_u8; 4];
        input.take(4).read_exact(&mut magic)?;

        ensure!(
            &magic == b"\x2a\x2a\x00\x00",
            err::InvalidEvtxRecordHeaderMagic { magic }
        );

        let size = try_read!(input, u32);
        let record_id = try_read!(input, u64);
        let timestamp = try_read!(input, filetime);

        Ok(EvtxRecordHeader {
            data_size: size,
            event_record_id: record_id,
            timestamp,
        })
    }

    pub fn record_data_size(&self) -> u32 {
        // 24 - record header size
        // 4 - copy of size record size
        self.data_size - 24 - 4
    }
}

impl<'a> EvtxRecord<'a> {
    /// Consumes the record, processing it using the given `output_builder`.
    pub fn into_output<T: BinXmlOutput>(self, output_builder: &mut T) -> Result<()> {
        parse_tokens(self.tokens, output_builder)?;

        Ok(())
    }

    /// Consumes the record, returning a `EvtxRecordWithJsonValue` with the `serde_json::Value` data.
    pub fn into_json_value(self) -> Result<SerializedEvtxRecord<serde_json::Value>> {
        let mut output_builder = JsonOutput::new(self.settings);

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
            serde_json::to_string_pretty(&record_with_json_value.data).context(err::JsonError)?
        } else {
            serde_json::to_string(&record_with_json_value.data).context(err::JsonError)?
        };

        Ok(SerializedEvtxRecord {
            event_record_id: record_with_json_value.event_record_id,
            timestamp: record_with_json_value.timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord<String>> {
        let mut output_builder = XmlOutput::with_writer(Vec::new(), &self.settings);

        let event_record_id = self.event_record_id;
        let timestamp = self.timestamp;
        self.into_output(&mut output_builder)?;

        let data = String::from_utf8(output_builder.into_writer()?)
            .context(err::RecordContainsInvalidUTF8)?;

        Ok(SerializedEvtxRecord {
            event_record_id,
            timestamp,
            data,
        })
    }
}
