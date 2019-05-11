use crate::err::{self, Result};
use crate::evtx_parser::ReadSeek;
use snafu::{ensure, ResultExt};

use crate::binxml::assemble::parse_tokens;
use crate::json_output::JsonOutput;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::utils::datetime_from_filetime;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::ParserSettings;
use byteorder::ReadBytesExt;
use chrono::prelude::*;
use std::io::{Cursor, Read};

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
pub struct SerializedEvtxRecord {
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
    pub data: String,
}

impl EvtxRecordHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxRecordHeader> {
        let mut magic = [0_u8; 4];
        input.take(4).read_exact(&mut magic).context(err::IO)?;

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
    /// Consumes the record, returning a `SerializedEvtxRecord` with the serialized data.
    pub fn into_serialized<T: BinXmlOutput<Vec<u8>>>(self) -> Result<SerializedEvtxRecord> {
        let mut output_builder = T::with_writer(Vec::new(), &self.settings);

        parse_tokens(self.tokens, &mut output_builder)?;

        let data = String::from_utf8(output_builder.into_writer()?)
            .context(err::RecordContainsInvalidUTF8)?;

        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing a JSON serialized record.
    pub fn into_json(self) -> Result<SerializedEvtxRecord> {
        self.into_serialized::<JsonOutput<Vec<u8>>>()
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord> {
        self.into_serialized::<XmlOutput<Vec<u8>>>()
    }
}
