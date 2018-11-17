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
