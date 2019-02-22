use crate::utils::datetime_from_filetime;
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::prelude::*;
use std::io::{self, Cursor, Read};

#[derive(Debug, Clone, PartialEq)]
pub struct EvtxRecord {
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
    pub data: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvtxRecordHeader {
    pub data_size: u32,
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
}

impl EvtxRecordHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> io::Result<EvtxRecordHeader> {
        let mut magic = [0_u8; 4];
        input.take(4).read_exact(&mut magic)?;

        assert_eq!(&magic, b"\x2a\x2a\x00\x00", "Wrong record header magic");
        let size = input.read_u32::<LittleEndian>()?;
        let record_id = input.read_u64::<LittleEndian>()?;
        let timestamp = datetime_from_filetime(input.read_u64::<LittleEndian>()?);

        Ok(EvtxRecordHeader {
            data_size: size,
            event_record_id: record_id,
            timestamp,
        })
    }

    pub fn record_data_data_size(&self) -> u32 {
        // 24 - record header size
        // 4 - copy of size record size
        self.data_size - 24 - 4
    }
}
