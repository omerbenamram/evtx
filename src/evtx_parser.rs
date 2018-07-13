use chrono::prelude::*;
use nom::{le_i16, le_i32, le_i64, le_u8};
use time::Duration;

use crc::crc32;
use xml::reader::{EventReader, XmlEvent};

#[derive(Debug, PartialEq)]
struct EVTXHeader {
    oldest_chunk: i64,
    current_chunk_num: i64,
    next_record_num: i64,
    header_size: i32,
    minor_version: i16,
    major_version: i16,
    header_block_size: i16,
    chunk_count: i16,
    flags: HeaderFlags,
    // Checksum is of first 120 bytes of header
    checksum: i32,
}

#[derive(Debug, PartialEq)]
enum HeaderFlags {
    Dirty,
    Full,
}

named!(evtx_header<&[u8], EVTXHeader>,
    do_parse!(
       tag!(b"ElfFile\x00")
       >> oldest_chunk: le_i64
       >> current_chunk_num: le_i64
       >> next_record_num: le_i64
       >> header_size: le_i32
       >> minor_version: le_i16
       >> major_version: le_i16
       >> header_block_size: le_i16
       >> chunk_count: le_i16
       >> take!(76) // unused
       >> flags: switch!(le_i32,
            1 => value!(HeaderFlags::Dirty) |
            2 => value!(HeaderFlags::Full)
       )
       >> checksum: le_i32
       >> take!(4096 - 128) // unused
       >> (EVTXHeader {oldest_chunk, current_chunk_num, next_record_num, header_block_size, minor_version,
                       major_version, header_size, chunk_count, flags, checksum})
   )
);

#[derive(Debug, PartialEq)]
struct EVTXChunkHeader {
    first_event_record_number: i64,
    last_event_record_number: i64,
    first_event_record_id: i64,
    last_event_record_id: i64,
    header_size: i32,
    last_event_record_data_offset: i32,
    free_space_offset: i32,
    events_checksum: i32,
    header_chunk_checksum: i32,
}

named!(evtx_chunk_header<&[u8], EVTXChunkHeader>,
       do_parse!(
          tag!(b"ElfChnk\x00")
          >> first_event_record_number: le_i64
          >> last_event_record_number: le_i64
          >> first_event_record_id: le_i64
          >> last_event_record_id: le_i64
          >> header_size: le_i32
          >> last_event_record_data_offset: le_i32
          >> free_space_offset: le_i32
          >> events_checksum: le_i32
          >> take!(64)
          >> take!(4) // flags?
          >> header_chunk_checksum: le_i32
          >> take!(4 * 64) // StringTable
          >> take!(4 * 32) // TemplateTable
          >> (EVTXChunkHeader {first_event_record_number, last_event_record_number, first_event_record_id,
                               last_event_record_id, header_size, last_event_record_data_offset, free_space_offset,
                               events_checksum, header_chunk_checksum})
       )
);

#[derive(Debug, PartialEq)]
struct EVTXRecord<'a> {
    event_record_id: i64,
    timestamp: DateTime<Utc>,
    data: &'a [u8],
}

#[derive(Debug, PartialEq)]
struct FileTime {
    year: i32,
    month: i32,
    day_of_week: i32,
    day: i32,
    hour: i32,
    minute: i32,
    second: i32,
    milis: i32,
}

fn datetime_from_filetime(nanos_since_windows_epoch: i64) -> DateTime<Utc> {
    DateTime::from_utc(
        NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0)
            + Duration::microseconds(nanos_since_windows_epoch / 10),
        Utc,
    )
}

named!(filetime<&[u8], DateTime<Utc>>,
       do_parse!(
            filetime: le_i64
            >> (datetime_from_filetime(filetime))
       )
);

named!(evtx_record<&[u8], EVTXRecord>,
       do_parse!(
          tag!(b"\x2a\x2a\x00\x00")
          >> size: le_i32
          >> event_record_id: le_i64
          >> timestamp: filetime
          >> data: take!(size)
          >> take!(4)
          >> (EVTXRecord {event_record_id, timestamp, data})
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
    use hexdump::print_hexdump;

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
                    checksum: crc32::checksum_ieee(&evtx_file[..120]) as i32,
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
                    events_checksum: -42488155,
                    header_chunk_checksum: crc32::checksum_ieee(bytes_for_checksum.as_slice())
                        as i32,
                }
            ))
        );
    }

    #[test]
    fn test_parses_record() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let evtx_records = &evtx_file[4096 + 512..];
        let parsed = evtx_record(evtx_records);

        let res = parsed.unwrap();
        let record = res.1;

        let ts: DateTime<Utc> = DateTime::from_utc(
            NaiveDateTime::new(
                NaiveDate::from_ymd(2016, 07, 08),
                NaiveTime::from_hms(18, 12, 51),
            ) + Duration::microseconds(681640),
            Utc,
        );

        assert_eq!(record.event_record_id, 1);
        assert_eq!(record.timestamp, ts);

        print_hexdump(record.data, 0, 'x', 2);

        let xml_bytes: Vec<u8> = record.data
            .iter()
            .skip(5)
            .map(|b| *b)
            .collect();

        let parser = EventReader::new(xml_bytes.as_slice());
        let mut depth = 0;
        for e in parser {
            match e {
                Ok(XmlEvent::StartElement { name, .. }) => {
                    println!("{}+{}", indent(depth), name);
                    depth += 1;
                }
                Ok(XmlEvent::EndElement { name }) => {
                    depth -= 1;
                    println!("{}-{}", indent(depth), name);
                }
                Err(e) => {
                    println!("Error: {}", e);
                    break;
                }
                _ => {}
            }
        }
    }
}
