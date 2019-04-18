use crate::utils::datetime_from_filetime;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::prelude::*;
use std::io::{self, Cursor, Read};
use crate::model::deserialized::BinXMLDeserializedTokens;
use failure::Error;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::json_output::JsonOutput;
use crate::binxml::assemble::parse_tokens;

#[derive(Debug, Clone, PartialEq)]
pub struct EvtxRecord<'a> {
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
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
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> io::Result<EvtxRecordHeader> {
        let mut magic = [0_u8; 4];
        input.take(4).read_exact(&mut magic)?;

        debug_assert_eq!(&magic, b"\x2a\x2a\x00\x00", "Wrong record header magic");
        let size = input.read_u32::<LittleEndian>()?;
        let record_id = input.read_u64::<LittleEndian>()?;
        let timestamp = datetime_from_filetime(input.read_u64::<LittleEndian>()?);

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
    /// Consumes the record, returning a SerializedEvtxRecord with the serialized data.
    pub fn into_serialized<T: BinXmlOutput<Vec<u8>>>(self) -> Result<SerializedEvtxRecord, Error> {
        let mut output_builder = T::with_writer(Vec::new());

        parse_tokens(self.tokens, &mut output_builder)?;

        let data = String::from_utf8(output_builder.into_writer()?)?;

        Ok(SerializedEvtxRecord {
            event_record_id: self.event_record_id,
            timestamp: self.timestamp,
            data,
        })
    }

    /// Consumes the record and parse it, producing a JSON serialized record.
    pub fn into_json(self) -> Result<SerializedEvtxRecord, Error> {
        self.into_serialized::<JsonOutput<Vec<u8>>>()
    }

    /// Consumes the record and parse it, producing an XML serialized record.
    pub fn into_xml(self) -> Result<SerializedEvtxRecord, Error> {
        self.into_serialized::<XmlOutput<Vec<u8>>>()
    }
}