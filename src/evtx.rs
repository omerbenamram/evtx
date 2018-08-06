use binxml::utils::read_len_prefixed_utf16_string;
use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
use nom::{le_u16, le_u32, le_u64, IResult};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};
use time::Duration;

use binxml::model::BinXMLTemplateDefinition;
use crc::crc32;
use hexdump::print_hexdump;
use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use binxml::utils::dump_cursor;

#[derive(Debug, PartialEq)]
pub struct EVTXHeader {
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

pub fn evtx_header(input: &[u8]) -> IResult<&[u8], EVTXHeader> {
    return do_parse!(
        input,
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
    );
}

#[derive(Debug, PartialEq)]
pub struct EVTXChunkHeader<'a> {
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
    string_table: HashMap<u16, Rc<String>>,
    template_table: HashMap<u32, BinXMLTemplateDefinition<'a>>,
}

#[derive(Debug, PartialEq)]
pub struct EVTXRecord<'a> {
    event_record_id: u64,
    timestamp: DateTime<Utc>,
    data: &'a [u8],
}

fn evtx_record(input: &[u8]) -> IResult<&[u8], EVTXRecord> {
    return do_parse!(
        input,
        tag!(b"\x2a\x2a\x00\x00")
          >> size: le_u32
          >> event_record_id: le_u64
          >> timestamp: filetime
          >> data: take!(size)
          // Size is repeated
          >> take!(4) >> (EVTXRecord {
            event_record_id,
            timestamp,
            data,
        })
    );
}

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

pub fn evtx_chunk_header<'a>(input: &mut Cursor<&'a [u8]>) -> io::Result<EVTXChunkHeader<'a>> {
    let mut magic = [0_u8; 8];
    input.take(8).read_exact(&mut magic)?;

    assert_eq!(&magic, b"ElfChnk\x00", "Wrong chunk header magic");
    let first_event_record_number = input.read_u64::<LittleEndian>()?;
    let last_event_record_number = input.read_u64::<LittleEndian>()?;
    let first_event_record_id = input.read_u64::<LittleEndian>()?;
    let last_event_record_id = input.read_u64::<LittleEndian>()?;

    let header_size = input.read_u32::<LittleEndian>()?;
    let last_event_record_data_offset = input.read_u32::<LittleEndian>()?;
    let free_space_offset = input.read_u32::<LittleEndian>()?;
    let events_checksum = input.read_u32::<LittleEndian>()?;

    // Reserved
    input.seek(SeekFrom::Current(64))?;
    // Flags
    input.seek(SeekFrom::Current(4))?;

    let header_chunk_checksum = input.read_u32::<LittleEndian>()?;

    let mut common_string_offsets = [0_u32; 64];
    input.read_u32_into::<LittleEndian>(&mut common_string_offsets)?;

    let mut string_table = HashMap::with_capacity(64);

    for offset in common_string_offsets.iter() {
        if offset > &0 {
            input.seek(SeekFrom::Start(*offset as u64))?;
            let _ = input.read_u32::<LittleEndian>()?;
            let name_hash = input.read_u16::<LittleEndian>()?;

            string_table.insert(
                name_hash,
                Rc::new(
                    read_len_prefixed_utf16_string(input, false)
                        .expect("Invalid UTF-16 String")
                        .expect("String cannot be empty"),
                ),
            );
        }
    }

    let template_table = HashMap::new();

    Ok(EVTXChunkHeader {
        first_event_record_number,
        last_event_record_number,
        first_event_record_id,
        last_event_record_id,
        header_size,
        last_event_record_data_offset,
        free_space_offset,
        events_checksum,
        header_chunk_checksum,
        template_table,
        string_table,
    })
}

fn filetime(input: &[u8]) -> IResult<&[u8], DateTime<Utc>> {
    return do_parse!(
        input,
        filetime: le_u64 >> (datetime_from_filetime(filetime))
    );
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
        let chunk_header = &evtx_file[4096..];
        let header_bytes_1 = &chunk_header[..120];
        let header_bytes_2 = &chunk_header[128..512];

        let bytes_for_checksum: Vec<u8> = header_bytes_1
            .iter()
            .chain(header_bytes_2)
            .map(|b| *b)
            .collect();

        let mut cursor = Cursor::new(chunk_header);

        assert_eq!(
            evtx_chunk_header(&mut cursor).unwrap(),
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
                string_table: hashmap! {
                    21615 => Rc::new("System".to_owned()),
                    31548 => Rc::new("SystemTime".to_owned()),
                    10155 => Rc::new("ServiceShutdown".to_owned()),
                    31729 => Rc::new("Provider".to_owned()),
                    11936 => Rc::new("Security".to_owned()),
                    62114 => Rc::new("Correlation".to_owned()),
                    53098 => Rc::new("Keywords".to_owned()),
                    14725 => Rc::new("ThreadID".to_owned()),
                    52836 => Rc::new("Level".to_owned()),
                    28554 => Rc::new("Data".to_owned()),
                    30542 => Rc::new("xmlns:auto-ns3".to_owned()),
                    17461 => Rc::new("UserData".to_owned()),
                    38219 => Rc::new("Name".to_owned()),
                    3258 => Rc::new("Event".to_owned()),
                    7854 => Rc::new("Opcode".to_owned()),
                    55849 => Rc::new("Qualifiers".to_owned()),
                    33348 => Rc::new("EventData".to_owned()),
                    19558 => Rc::new("UserID".to_owned()),
                    28219 => Rc::new("Computer".to_owned()),
                    46520 => Rc::new("Execution".to_owned()),
                    838 => Rc::new("EventRecordID".to_owned()),
                    24963 => Rc::new("Channel".to_owned()),
                    2328 => Rc::new("Version".to_owned())
                },
                template_table: hashmap!{},
            }
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
