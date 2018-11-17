use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::{format_err, Context, Error, Fail};

use crate::binxml::parse_token;
use crate::binxml::BinXmlDeserializer;
use crate::evtx_record::{EvtxRecord, EvtxRecordHeader};
use crate::model::{BinXMLDeserializedTokens, BinXMLTemplateDefinition};
use crate::utils::*;
use crate::xml_builder::Visitor;
use log::{debug, log};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    fmt::{Debug, Formatter},
    io::Cursor,
    io::{Read, Seek, SeekFrom},
    rc::Rc,
};

const EVTX_HEADER_SIZE: usize = 512;

#[derive(Fail, Debug)]
enum ChunkHeaderParseErrorKind {
    #[fail(display = "Expected magic \"ElfChnk\x00\", got {:#?}", magic)]
    WrongHeaderMagic { magic: [u8; 8] },
}

type TemplateID = u32;
type Offset = u32;

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
    strings_offsets: [u32; 64],
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
    visitor: Box<Visitor<'a>>,
    pub data: &'a [u8],
    pub string_table: HashMap<Offset, (u16, String)>,
    pub template_table: HashMap<TemplateID, Rc<BinXMLTemplateDefinition<'a>>>,
}

pub struct IterRecords<'a> {
    chunk: EvtxChunk<'a>,
    offset_from_chunk_start: u64,
}

impl<'a> Iterator for IterRecords<'a> {
    type Item = Result<EvtxRecord<'a>, Error>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let mut cursor = Cursor::new(&self.chunk.data[self.offset_from_chunk_start as usize..]);
        // TODO: remove unwrap
        let record_header = EvtxRecordHeader::from_reader(&mut cursor).unwrap();

        let binxml_data_size = record_header.data_size - 5 - 4 - 4 - 8 - 8;
        debug!("Need to deserialize {} bytes of binxml", binxml_data_size);
        let deserializer = BinXmlDeserializer {
            chunk: &mut self.chunk,
            offset_from_chunk_start: self.offset_from_chunk_start + cursor.position(),
            data_size: binxml_data_size,
            data_read_so_far: 0,
        };

        let tokens: Vec<BinXMLDeserializedTokens> = deserializer
            .into_iter()
            .filter_map(|t| Some(t.expect("invalid token")))
            .collect();

        parse_tokens(&tokens, &mut self.chunk.visitor);
        for token in tokens {
            parse_token(&token, &mut self.chunk.visitor).unwrap();
        }

        Some(Ok(EvtxRecord {
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            data: &[],
        }))
    }
}

impl<'a> IntoIterator for EvtxChunk<'a> {
    type Item = Result<EvtxRecord<'a>, Error>;
    type IntoIter = IterRecords<'a>;

    fn into_iter(self) -> <Self as IntoIterator>::IntoIter {
        IterRecords {
            chunk: self,
            offset_from_chunk_start: EVTX_HEADER_SIZE as u64,
        }
    }
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
    pub fn new(data: &'a [u8], visitor: impl Visitor<'a> + 'static) -> Result<EvtxChunk, Error> {
        let mut cursor = Cursor::new(data);
        let header = EvtxChunkHeader::from_reader(&mut cursor)?;

        Ok(EvtxChunk {
            data,
            visitor: Box::new(visitor),
            header,
            string_table: HashMap::new(),
            template_table: HashMap::new(),
        })
    }
}

impl EvtxChunkHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxChunkHeader, Error> {
        let mut magic = [0_u8; 8];
        input.take(8).read_exact(&mut magic)?;

        if &magic != b"ElfChnk\x00" {
            return Err(format_err!(
                "Wrong chunk header magic {:?}, magic, expected ElfChnk\x00",
                &magic
            ));
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

        let mut strings_offsets = [0_u32; 64];
        input.read_u32_into::<LittleEndian>(&mut strings_offsets)?;

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
            strings_offsets: [0_u32; 64],
            template_offsets: [0_u32; 32],
        };

        assert_eq!(
            chunk_header.first_event_record_number,
            expected.first_event_record_number
        );
        assert_eq!(
            chunk_header.last_event_record_number,
            expected.last_event_record_number
        );
        assert_eq!(
            chunk_header.first_event_record_id,
            expected.first_event_record_id
        );
        assert_eq!(
            chunk_header.last_event_record_id,
            expected.last_event_record_id
        );
        assert_eq!(chunk_header.header_size, expected.header_size);
        assert_eq!(
            chunk_header.last_event_record_data_offset,
            expected.last_event_record_data_offset
        );
        assert_eq!(chunk_header.free_space_offset, expected.free_space_offset);
        assert_eq!(chunk_header.events_checksum, expected.events_checksum);
        assert!(chunk_header.strings_offsets.len() > 0);
        assert!(chunk_header.template_offsets.len() > 0);
    }
}
