use byteorder::{LittleEndian, ReadBytesExt};
use failure::{self, bail, format_err};

use crate::evtx_record::{EvtxRecord, EvtxRecordHeader};
use crate::utils::*;
use crate::xml_output::XmlOutput;
use crate::json_output::JsonOutput;
use crc::crc32;
use log::{debug, error, info, trace};
use std::{
    fmt::{Debug, Formatter},
    io::Cursor,
    io::{Read, Seek, SeekFrom},
};
use crate::xml_output::BinXmlOutput;

use crate::binxml::assemble::parse_tokens;
use crate::binxml::deserializer::BinXmlDeserializer;
use crate::string_cache::StringCache;
use crate::template_cache::TemplateCache;
use log::Level;
use std::io::{BufReader, BufWriter};

const EVTX_CHUNK_HEADER_SIZE: usize = 512;

pub struct EvtxChunkHeader {
    pub first_event_record_number: u64,
    pub last_event_record_number: u64,
    pub first_event_record_id: u64,
    pub last_event_record_id: u64,
    pub header_size: u32,
    pub last_event_record_data_offset: u32,
    pub free_space_offset: u32,
    pub events_checksum: u32,
    pub header_chunk_checksum: u32,
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
            .field("free_space_offset", &self.free_space_offset)
            .finish()
    }
}

pub struct EvtxChunkData {
    pub header: EvtxChunkHeader,
    pub data: Vec<u8>,
    pub string_cache: StringCache,
}

impl EvtxChunkData {
    pub fn new(data: Vec<u8>) -> Result<Self, failure::Error> {
        let mut cursor = Cursor::new(data.as_slice());
        let header = EvtxChunkHeader::from_reader(&mut cursor)?;

        let chunk = EvtxChunkData { header, data, string_cache: StringCache::new() };
        if !chunk.validate_checksum() {
            bail!("Invalid header checksum");
        }

        Ok(chunk)
    }

    pub fn into_records<'a>(&'a mut self) -> Result<Vec<Result<EvtxRecord<'a>, failure::Error>>, failure::Error> {
        Ok(self.parse()?.into_iter().collect())
    }

    pub fn parse(&mut self) -> Result<EvtxChunk, failure::Error> {
        EvtxChunk::new(&self.data, &self.header, &mut self.string_cache)
    }

    pub fn validate_data_checksum(&self) -> bool {
        debug!("Validating data checksum");

        let expected_checksum = self.header.events_checksum;

        let checksum = crc32::checksum_ieee(
            &self.data[EVTX_CHUNK_HEADER_SIZE..self.header.free_space_offset as usize],
        );

        debug!(
            "Expected checksum: {:?}, found: {:?}",
            expected_checksum, checksum
        );

        checksum == expected_checksum
    }

    pub fn validate_header_checksum(&self) -> bool {
        debug!("Validating header checksum");

        let expected_checksum = self.header.header_chunk_checksum;

        let header_bytes_1 = &self.data[..120];
        let header_bytes_2 = &self.data[128..512];

        let bytes_for_checksum: Vec<u8> = header_bytes_1
            .iter()
            .chain(header_bytes_2)
            .cloned()
            .collect();

        let checksum = crc32::checksum_ieee(bytes_for_checksum.as_slice());

        debug!(
            "Expected checksum: {:?}, found: {:?}",
            expected_checksum, checksum
        );

        checksum == expected_checksum
    }

    pub fn validate_checksum(&self) -> bool {
        self.validate_header_checksum() && self.validate_data_checksum()
    }
}

pub struct EvtxChunk<'a> {
    pub data: &'a [u8],
    pub header: &'a EvtxChunkHeader,
    pub string_cache: &'a StringCache,
    pub template_table: TemplateCache<'a>,
}

impl<'a> EvtxChunk<'a> {
    /// Will fail if the data starts with an invalid evtx chunk header.
    pub fn new(
        data: &'a [u8],
        header: &'a EvtxChunkHeader,
        string_cache: &'a mut StringCache,
    ) -> Result<EvtxChunk<'a>, failure::Error> {
        let _cursor = Cursor::new(data);

        let mut template_table = TemplateCache::new();

        info!("Initializing string cache");
        string_cache.populate(&data, &header.strings_offsets)?;
        info!("Initializing template cache");
        template_table.populate(&data, &header.template_offsets)?;

        Ok(EvtxChunk {
            header,
            data,
            string_cache,
            template_table,
        })
    }
}

pub struct IterChunkRecords<'a> {
    chunk: EvtxChunk<'a>,
    offset_from_chunk_start: u64,
    exhausted: bool,
}

impl<'a> Iterator for IterChunkRecords<'a> {
    type Item = Result<EvtxRecord<'a>, failure::Error>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.exhausted
            || self.offset_from_chunk_start >= self.chunk.header.free_space_offset as u64
        {
            return None;
        }

        let mut cursor = Cursor::new(&self.chunk.data[self.offset_from_chunk_start as usize..]);

        let record_header = EvtxRecordHeader::from_reader(&mut cursor).unwrap();
        info!("Record id - {}", record_header.event_record_id);
        debug!("Record header - {:?}", record_header);

        let binxml_data_size = record_header.record_data_size();

        trace!("Need to deserialize {} bytes of binxml", binxml_data_size);
        let deserializer = BinXmlDeserializer::init(
            self.chunk.data,
            self.offset_from_chunk_start + cursor.position(),
            self.chunk.string_cache,
            &self.chunk.template_table,
        );


        let mut tokens = vec![];
        let iter = match deserializer.iter_tokens(Some(binxml_data_size)) {
            Ok(iter) => iter,
            Err(e) => return Some(Err(format_err!("{}", e))),
        };

        for token in iter {
            match token {
                Ok(token) => {
                    trace!("successfully read {:?}", token);
                    tokens.push(token)
                }
                Err(e) => {
                    error!("Tried to read an invalid token!, {}", e);

                    if log::log_enabled!(Level::Debug) {
                        let mut cursor = Cursor::new(self.chunk.data);
                        cursor
                            .seek(SeekFrom::Start(
                                e.offset().expect("Err to have offset information"),
                            ))
                            .unwrap();
                        dump_cursor(&mut cursor, 10);
                    }

                    self.offset_from_chunk_start += u64::from(record_header.data_size);
                    return Some(Err(e.into()));
                }
            }
        }

        self.offset_from_chunk_start += u64::from(record_header.data_size);


        if self.chunk.header.last_event_record_id == record_header.event_record_id {
            self.exhausted = true;
        }

        Some(Ok(EvtxRecord {
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            tokens,
        }))
    }
}

impl<'a> IntoIterator for EvtxChunk<'a> {
    type Item = Result<EvtxRecord<'a>, failure::Error>;
    type IntoIter = IterChunkRecords<'a>;

    fn into_iter(self) -> <Self as IntoIterator>::IntoIter {
        IterChunkRecords {
            chunk: self,
            offset_from_chunk_start: EVTX_CHUNK_HEADER_SIZE as u64,
            exhausted: false,
        }
    }
}

impl<'a> Debug for EvtxChunk<'a> {
    fn fmt(&self, fmt: &mut Formatter) -> Result<(), ::std::fmt::Error> {
        writeln!(fmt, "\nEvtxChunk")?;
        writeln!(fmt, "-----------------------")?;
        writeln!(fmt, "{:#?}", &self.header)?;
        writeln!(fmt, "{} common strings", self.string_cache.len())?;
        writeln!(fmt, "{} common templates", self.template_table.len())?;
        Ok(())
    }
}

impl EvtxChunkHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> Result<EvtxChunkHeader, failure::Error> {
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
    use crate::ensure_env_logger_initialized;
    use crate::evtx_parser::EVTX_CHUNK_SIZE;
    use crate::evtx_parser::EVTX_FILE_HEADER_SIZE;

    use std::io::Cursor;

    #[test]
    fn test_parses_evtx_chunk_header() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk_header =
            &evtx_file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_HEADER_SIZE];

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
            events_checksum: 4_252_479_141,
            header_chunk_checksum: 978805790,
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
        assert_eq!(
            chunk_header.header_chunk_checksum,
            expected.header_chunk_checksum
        );
        assert!(!chunk_header.strings_offsets.is_empty());
        assert!(!chunk_header.template_offsets.is_empty());
    }

    #[test]
    fn test_validate_checksum() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk_data =
            evtx_file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE].to_vec();

        let chunk = EvtxChunkData::new(chunk_data).unwrap();
        assert!(chunk.validate_checksum());
    }
}
