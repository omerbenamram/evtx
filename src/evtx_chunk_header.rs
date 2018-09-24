use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Error;

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use model::BinXMLTemplateDefinition;
use std::borrow::Cow;
use std::rc::Rc;
use std::io::Cursor;
use utils::*;
use std::cell::RefCell;

#[derive(Fail, Debug)]
enum ChunkHeaderParseError {
    #[fail(display = "Expected magic \"ElfChnk\x00\", got {:#?}", magic)]
    WrongHeaderMagic { magic: [u8; 8] },
}

type TemplateID = u32;

#[derive(Debug, PartialEq)]
pub struct EvtxChunkHeader<'a> {
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
    pub string_table: HashMap<u16, Cow<'a, str>>,
    pub template_table: RefCell<HashMap<TemplateID, Rc<BinXMLTemplateDefinition<'a>>>>,
}

impl<'a> EvtxChunkHeader<'a> {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxChunkHeader<'a>, Error> {
        let mut magic = [0_u8; 8];
        input.take(8).read_exact(&mut magic)?;

        if &magic != b"ElfChnk\x00" {
            return Err(Error::from(ChunkHeaderParseError::WrongHeaderMagic {
                magic,
            }));
        }

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

        let original_curosr_position = input.position();
        // Eagerly load Common strings table.
        for offset in common_string_offsets.iter() {
            if *offset > 0 {
                input.seek(SeekFrom::Start(*offset as u64))?;
                let _ = input.read_u32::<LittleEndian>()?;
                let name_hash = input.read_u16::<LittleEndian>()?;

                string_table.insert(
                    name_hash,
                    Cow::Owned(
                        read_len_prefixed_utf16_string(input, false)
                            .expect("Invalid UTF-16 String")
                            .expect("String cannot be empty"),
                    ),
                );
            }
        }

        input.seek(SeekFrom::Start(original_curosr_position))?;

        // Skip template pointers
        input.seek(SeekFrom::Current(32 * 4))?;

        // Templates will be evaluated and inserted lazily by the deserializer.
        let template_table = RefCell::new(HashMap::new());

        Ok(EvtxChunkHeader {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crc::crc32;
    use itertools::assert_equal;
    use itertools::Itertools;
    use std::hash::Hash;
    use std::io::Cursor;

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

        let chunk_header = EvtxChunkHeader::from_reader(&mut cursor).unwrap();
        let expected_string_table = hashmap! {
            21615 => Cow::Borrowed("System"),
            31548 => Cow::Borrowed("SystemTime"),
            10155 => Cow::Borrowed("ServiceShutdown"),
            31729 => Cow::Borrowed("Provider"),
            11936 => Cow::Borrowed("Security"),
            62114 => Cow::Borrowed("Correlation"),
            53098 => Cow::Borrowed("Keywords"),
            14725 => Cow::Borrowed("ThreadID"),
            52836 => Cow::Borrowed("Level"),
            28554 => Cow::Borrowed("Data"),
            30542 => Cow::Borrowed("xmlns:auto-ns3"),
            17461 => Cow::Borrowed("UserData"),
            38219 => Cow::Borrowed("Name"),
            3258 => Cow::Borrowed("Event"),
            7854 => Cow::Borrowed("Opcode"),
            55849 => Cow::Borrowed("Qualifiers"),
            33348 => Cow::Borrowed("EventData"),
            19558 => Cow::Borrowed("UserID"),
            28219 => Cow::Borrowed("Computer"),
            46520 => Cow::Borrowed("Execution"),
            838 => Cow::Borrowed("EventRecordID"),
            24963 => Cow::Borrowed("Channel"),
            2328 => Cow::Borrowed("Version")
        };

        let expected = EvtxChunkHeader {
            first_event_record_number: 1,
            last_event_record_number: 91,
            first_event_record_id: 1,
            last_event_record_id: 91,
            header_size: 128,
            last_event_record_data_offset: 64928,
            free_space_offset: 65376,
            events_checksum: 4252479141,
            header_chunk_checksum: crc32::checksum_ieee(bytes_for_checksum.as_slice()),
            string_table: expected_string_table.clone(),
            template_table: RefCell::new(HashMap::new()),
        };

        assert_equal(
            expected_string_table.iter().sorted(),
            chunk_header.string_table.iter().sorted(),
        );

        assert_eq!(chunk_header, expected);
    }
}
