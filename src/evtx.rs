use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};
use time::Duration;

use model::BinXMLTemplateDefinition;
use crc::crc32;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use utils::*;



#[derive(Debug, PartialEq)]
pub struct EVTXRecord<'a> {
    event_record_id: u64,
    timestamp: DateTime<Utc>,
    data: &'a [u8],
}

//fn evtx_record(input: &[u8]) -> IResult<&[u8], EVTXRecord> {
//    return do_parse!(
//        input,
//        tag!(b"\x2a\x2a\x00\x00")
//          >> size: le_u32
//          >> event_record_id: le_u64
//          >> timestamp: filetime
//          >> data: take!(size)
//          // Size is repeated
//          >> take!(4) >> (EVTXRecord {
//            event_record_id,
//            timestamp,
//            data,
//        })
//    );
//}

#[derive(Debug, PartialEq)]
pub struct EVTXRecordHeader {
    pub data_size: u32,
    pub event_record_id: u64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct FileTime {
    pub year: u32,
    pub month: u32,
    pub day_of_week: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub milis: u32,
}

pub fn evtx_record_header(input: &mut Cursor<&[u8]>) -> io::Result<EVTXRecordHeader> {
    let mut magic = [0_u8; 4];
    input.take(4).read_exact(&mut magic)?;

    assert_eq!(&magic, b"\x2a\x2a\x00\x00", "Wrong record header magic");
    let size = input.read_u32::<LittleEndian>()?;
    let record_id = input.read_u64::<LittleEndian>()?;
    let timestamp = datetime_from_filetime(input.read_u64::<LittleEndian>()?);

    Ok(EVTXRecordHeader {
        data_size: size,
        event_record_id: record_id,
        timestamp,
    })
}

pub fn datetime_from_filetime(nanos_since_windows_epoch: u64) -> DateTime<Utc> {
    DateTime::from_utc(
        NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0)
            + Duration::microseconds((nanos_since_windows_epoch / 10) as i64),
        Utc,
    )
}


// TODO: remove unused internals
struct EvtxParser<'a> {
    current: EVTXRecord<'a>,
    total_number_of_records: i32,
    current_record_number: i32,
    binary_stream: &'a mut Cursor<&'a [u8]>
}

//impl<'a> EvtxParser<'a> {
//    pub fn parse_evtx_stream<S: Read>(stream: S) -> impl Iterator<Item=EVTXRecord> + 'a {
//        unimplemented!();
//    }
//}


#[cfg(test)]
mod tests {
    #[allow(unused_variables)]
    use super::*;
    use encoding::all::UTF_16LE;
    use encoding::DecoderTrap;
    use encoding::Encoding;
    use std::char::decode_utf16;
    use std::fs::File;
    use std::io::Write;
    use evtx_file_header::HeaderFlags;


//    #[test]
//    fn test_parses_record() {
//        let evtx_file = include_bytes!("../samples/security.evtx");
//        let evtx_records = &evtx_file[4096 + 512..];
//        let parsed = evtx_record(evtx_records);
//
//        let (_, record) = parsed.unwrap();
//
//        let ts: DateTime<Utc> = DateTime::from_utc(
//            NaiveDateTime::new(
//                NaiveDate::from_ymd(2016, 07, 08),
//                NaiveTime::from_hms(18, 12, 51),
//            ) + Duration::microseconds(681640),
//            Utc,
//        );
//
//        assert_eq!(record.event_record_id, 1);
//        assert_eq!(record.timestamp, ts);
//
//        print_hexdump(record.data, 0, 'x');
//
//        let xml_bytes: Vec<u8> = record.data.iter().map(|b| *b).collect();
//        let mut f = File::create("binxml.dat").unwrap();
//        f.write_all(&xml_bytes).unwrap();
//    }
}
