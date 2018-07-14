use chrono::prelude::*;
use nom::{le_u16, le_u32, le_u64, le_u8};
use time::Duration;

use crc::crc32;
use std::marker::PhantomData;
use xml::reader::{EventReader, XmlEvent};

#[derive(Debug, PartialEq)]
struct EVTXHeader {
    oldest_chunk: u64,
    current_chunk_num: u64,
    next_record_num: u64,
    header_size: u32,
    minor_version: u16,
    major_version: u16,
    header_block_size: u16,
    chunk_count: u16,
    flags: HeaderFlags,
    // Checksum is of first 120 bytes of header
    checksum: u32,
}

#[derive(Debug, PartialEq)]
enum HeaderFlags {
    Dirty,
    Full,
}

named!(evtx_header<&[u8], EVTXHeader>,
    do_parse!(
       tag!(b"ElfFile\x00")
       >> oldest_chunk: le_u64
       >> current_chunk_num: le_u64
       >> next_record_num: le_u64
       >> header_size: le_u32
       >> minor_version: le_u16
       >> major_version: le_u16
       >> header_block_size: le_u16
       >> chunk_count: le_u16
       >> take!(76) // unused
       >> flags: switch!(le_u32,
            1 => value!(HeaderFlags::Dirty) |
            2 => value!(HeaderFlags::Full)
       )
       >> checksum: le_u32
       >> take!(4096 - 128) // unused
       >> (EVTXHeader {oldest_chunk, current_chunk_num, next_record_num, header_block_size, minor_version,
                       major_version, header_size, chunk_count, flags, checksum})
   )
);

#[derive(Debug, PartialEq)]
struct EVTXChunkHeader {
    first_event_record_number: u64,
    last_event_record_number: u64,
    first_event_record_id: u64,
    last_event_record_id: u64,
    header_size: u32,
    last_event_record_data_offset: u32,
    free_space_offset: u32,
    events_checksum: u32,
    header_chunk_checksum: u32,
    //    For every string a 16 bit hash is calculated. The hash
    //    value is divided by 64, the number of buckets in the string
    //    table. The remainder then indicates what hash bucket to use.
    //    Every bucket contains the 32 bit offset relative to the chunk
    //    where the string can be found. If a hash collision occurs, the
    //    offset of the last string will be stored in the bucket. The string
    //    object will then provide the offset of the preceding string, thus
    //    building a single-linked list.
    string_table: Vec<u32>,
    template_table: Vec<u32>,
}

named!(evtx_chunk_header<&[u8], EVTXChunkHeader>,
       do_parse!(
          tag!(b"ElfChnk\x00")
          >> first_event_record_number: le_u64
          >> last_event_record_number: le_u64
          >> first_event_record_id: le_u64
          >> last_event_record_id: le_u64
          >> header_size: le_u32
          >> last_event_record_data_offset: le_u32
          >> free_space_offset: le_u32
          >> events_checksum: le_u32
          >> take!(64)
          >> take!(4) // flags?
          >> header_chunk_checksum: le_u32
          >> string_table: count!(le_u32, 64) // StringTable
          >> template_table: count!(le_u32,  32) // TemplateTable
          >> (EVTXChunkHeader {first_event_record_number, last_event_record_number, first_event_record_id,
                               last_event_record_id, header_size, last_event_record_data_offset, free_space_offset,
                               events_checksum, header_chunk_checksum, template_table, string_table})
       )
);

#[derive(Debug, PartialEq)]
struct EVTXRecord<'a> {
    event_record_id: u64,
    timestamp: DateTime<Utc>,
    data: &'a [u8],
}

#[derive(Debug, PartialEq)]
struct FileTime {
    year: u32,
    month: u32,
    day_of_week: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    milis: u32,
}

named!(evtx_record<&[u8], EVTXRecord>,
       do_parse!(
          tag!(b"\x2a\x2a\x00\x00")
          >> size: le_u32
          >> event_record_id: le_u64
          >> timestamp: filetime
          >> data: take!(size)
          // Size is repeated
          >> take!(4)
          >> (EVTXRecord {event_record_id, timestamp, data})
       )
);

fn datetime_from_filetime(nanos_since_windows_epoch: u64) -> DateTime<Utc> {
    DateTime::from_utc(
        NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0)
            + Duration::microseconds((nanos_since_windows_epoch / 10) as i64),
        Utc,
    )
}

named!(filetime<&[u8], DateTime<Utc>>,
       do_parse!(
            filetime: le_u64
            >> (datetime_from_filetime(filetime))
       )
);

fn indent(size: usize) -> String {
    const INDENT: &'static str = "    ";
    (0..size)
        .map(|_| INDENT)
        .fold(String::with_capacity(size * INDENT.len()), |r, s| r + s)
}

#[cfg(test)]
mod tests {
    #[allow(unused_variables)]
    use super::*;
    use encoding::all::UTF_16LE;
    use encoding::DecoderTrap;
    use encoding::Encoding;
    use hexdump::print_hexdump;
    use std::char::decode_utf16;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_parses_evtx_file_handler() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let parsing_result = evtx_header(&evtx_file[..4096]);
        assert_eq!(
            parsing_result,
            Ok((
                &b""[..],
                EVTXHeader {
                    oldest_chunk: 0,
                    current_chunk_num: 25,
                    next_record_num: 2226,
                    header_size: 128,
                    minor_version: 1,
                    major_version: 3,
                    header_block_size: 4096,
                    chunk_count: 26,
                    flags: HeaderFlags::Dirty,
                    checksum: crc32::checksum_ieee(&evtx_file[..120]),
                }
            ))
        );
    }

    #[test]
    fn test_parses_evtx_chunk_header() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk_header = &evtx_file[4096..(4096 + 512)];
        let header_bytes_1 = &chunk_header[..120];
        let header_bytes_2 = &chunk_header[128..512];

        let bytes_for_checksum: Vec<u8> = header_bytes_1
            .iter()
            .chain(header_bytes_2)
            .map(|b| *b)
            .collect();

        let utf_16_string: String = UTF_16LE
            .decode(&evtx_file[4096 + 1565..4096 + 2029], DecoderTrap::Strict)
            .unwrap();

        println!("{}", utf_16_string);
        print_hexdump(&evtx_file[4096 + 1565..4096 + 2029], 0, 'C');

        assert_eq!(
            evtx_chunk_header(chunk_header),
            Ok((
                &b""[..],
                EVTXChunkHeader {
                    first_event_record_number: 1,
                    last_event_record_number: 91,
                    first_event_record_id: 1,
                    last_event_record_id: 91,
                    header_size: 128,
                    last_event_record_data_offset: 64928,
                    free_space_offset: 65376,
                    events_checksum: 4252479141,
                    header_chunk_checksum: crc32::checksum_ieee(bytes_for_checksum.as_slice()),
                    string_table: vec![],
                    template_table: vec![],
                }
            ))
        );
    }

    #[test]
    fn test_parses_record() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let evtx_records = &evtx_file[4096 + 512..];
        let parsed = evtx_record(evtx_records);

        let (_, record) = parsed.unwrap();

        let ts: DateTime<Utc> = DateTime::from_utc(
            NaiveDateTime::new(
                NaiveDate::from_ymd(2016, 07, 08),
                NaiveTime::from_hms(18, 12, 51),
            ) + Duration::microseconds(681640),
            Utc,
        );

        assert_eq!(record.event_record_id, 1);
        assert_eq!(record.timestamp, ts);

        print_hexdump(record.data, 0, 'x');

        let xml_bytes: Vec<u8> = record.data.iter().map(|b| *b).collect();
        let mut f = File::create("binxml.dat").unwrap();
        f.write_all(&xml_bytes).unwrap();
    }
}
