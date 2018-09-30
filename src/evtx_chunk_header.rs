use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Error;

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};

use binxml::BinXMLDeserializer;
use model::BinXMLTemplateDefinition;
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::Cursor;
use std::fmt::{Debug, Formatter};
use std::rc::Rc;
use utils::*;

#[derive(Fail, Debug)]
enum ChunkHeaderParseError {
    #[fail(display = "Expected magic \"ElfChnk\x00\", got {:#?}", magic)]
    WrongHeaderMagic { magic: [u8; 8] },
}

type TemplateID = u32;

#[derive(PartialEq)]
pub struct EvtxChunkHeader {
    first_event_record_number: u64,
    last_event_record_number: u64,
    first_event_record_id: u64,
    last_event_record_id: u64,
    header_size: u32,
    last_event_record_data_offset: u32,
    free_space_offset: u32,
    events_checksum: u32,
    header_chunk_checksum: u32,
    // Stored as a vector since arrays implement debug only up to a length of 32 elements.
    // There should be 64 elements in this vector.
    strings_offsets: Vec<u32>,
    template_offsets: [u32; 32],
}

impl Debug for EvtxChunkHeader {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), ::std::fmt::Error> {
        fmt.debug_struct("EvtxChunkHeader")
            .field("first_event_record_number", &self.first_event_record_number)
            .field("last_event_record_number", &self.last_event_record_number)
            .field("checksum", &self.header_chunk_checksum)
            .finish()
    }
}

pub struct EvtxChunk<'a> {
    header: EvtxChunkHeader,
    cursor: Cursor<&'a [u8]>,
    pub data: &'a [u8],
    //    For every string a 16 bit hash is calculated. The hash
    //    value is divided by 64, the number of buckets in the string
    //    table. The remainder then indicates what hash bucket to use.
    //    Every bucket contains the 32 bit offset relative to the chunk
    //    where the string can be found. If a hash collision occurs, the
    //    offset of the last string will be stored in the bucket. The string
    //    object will then provide the offset of the preceding string, thus
    //    building a single-linked list.
    pub string_table: HashMap<u16, Cow<'a, str>>,
    pub template_table: HashMap<TemplateID, Rc<BinXMLTemplateDefinition<'a>>>,
}

impl<'a> Debug for EvtxChunk<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), ::std::fmt::Error> {
        writeln!(fmt, "\nEvtxChunk")?;
        writeln!(fmt, "-----------------------")?;
        writeln!(fmt, "{:#?}", &self.header)?;
        writeln!(fmt, "{} common strings", self.string_table.len())?;
        writeln!(fmt, "{} common templates", self.template_table.len())?;
        Ok(())
    }
}

impl<'a> EvtxChunk<'a> {
    pub fn new(data: &'a [u8]) -> Result<EvtxChunk, Error> {
        let mut cursor = Cursor::new(data);
        let header = EvtxChunkHeader::from_reader(&mut cursor)?;

        Ok(EvtxChunk {
            data,
            cursor,
            header,
            string_table: HashMap::new(),
            template_table: HashMap::new(),
        })
    }

    pub fn populate_cache_tables(&mut self) -> Result<(), Error> {
        let mut cursor = Cursor::new(self.data);

        for offset in self.header.strings_offsets.iter() {
            if *offset > 0 {
                cursor.seek(SeekFrom::Start(*offset as u64))?;
                let _ = cursor.read_u32::<LittleEndian>()?;
                let name_hash = cursor.read_u16::<LittleEndian>()?;

                self.string_table.insert(
                    name_hash,
                    Cow::Owned(
                        read_len_prefixed_utf16_string(&mut cursor, false)
                            .expect("Invalid UTF-16 String")
                            .expect("String cannot be empty"),
                    ),
                );
            }
        }

        Ok(())
    }
}

impl EvtxChunkHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxChunkHeader, Error> {
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

        let mut strings_offsets = Vec::with_capacity(64);
        for _ in 0..64 {
            let offset =  input.read_u32::<LittleEndian>()?;
            strings_offsets.push(offset);
        }

        let mut template_offsets = [0_u32; 32];
        input.read_u32_into::<LittleEndian>(&mut template_offsets)?;

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
            template_offsets,
            strings_offsets,
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
            strings_offsets: vec![],
            template_offsets: [0_u32; 32]
        };

        assert_eq!(chunk_header.first_event_record_number, expected.first_event_record_number);
        assert_eq!(chunk_header.last_event_record_number, expected.last_event_record_number);
        assert_eq!(chunk_header.first_event_record_id, expected.first_event_record_id);
        assert_eq!(chunk_header.last_event_record_id, expected.last_event_record_id);
        assert_eq!(chunk_header.header_size, expected.header_size);
        assert_eq!(chunk_header.last_event_record_data_offset, expected.last_event_record_data_offset);
        assert_eq!(chunk_header.free_space_offset, expected.free_space_offset);
        assert_eq!(chunk_header.events_checksum, expected.events_checksum);
        assert!(chunk_header.strings_offsets.len() > 0);
        assert!(chunk_header.template_offsets.len() > 0);
    }
}
