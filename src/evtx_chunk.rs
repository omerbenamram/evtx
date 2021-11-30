use crate::err::{
    ChunkError, DeserializationError, DeserializationResult, EvtxChunkResult, EvtxError,
};

use crate::evtx_record::{EvtxRecord, EvtxRecordHeader};

use log::{debug, info, trace};
use std::{
    io::Cursor,
    io::{Read, Seek, SeekFrom},
};

use crate::binxml::deserializer::BinXmlDeserializer;
use crate::string_cache::StringCache;
use crate::template_cache::TemplateCache;
use crate::{ParserSettings, checksum_ieee};

use byteorder::{LittleEndian, ReadBytesExt};
use std::sync::Arc;

const EVTX_CHUNK_HEADER_SIZE: usize = 512;

bitflags! {
    pub struct ChunkFlags: u32 {
        const EMPTY = 0x0;
        const DIRTY = 0x1;
        const NO_CRC32 = 0x4;
    }
}

#[derive(Debug)]
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
    pub flags: ChunkFlags,
    // A list of buckets containing the offsets of all strings in the chunk.
    // Each bucket contains an initial offset for a `BinXmlNameLink`, which in turn contains
    // the offset for the next strings.
    // Empty buckets are given the value 0.
    //  ----------       ------------------
    // |          |     |                  |
    // |  offset  | --> |  BinXmlNameLink  | ---> 0
    // |          |     |                  |
    //  ----------       ------------------
    strings_offsets: Vec<u32>,
    template_offsets: Vec<u32>,
}

/// A struct which owns all the data associated with a chunk.
/// See EvtxChunk for more.
pub struct EvtxChunkData {
    pub header: EvtxChunkHeader,
    pub data: Vec<u8>,
}

impl EvtxChunkData {
    /// Construct a new chunk from the given data.
    /// Note that even when validate_checksum is set to false, the header magic is still checked.
    pub fn new(data: Vec<u8>, validate_checksum: bool) -> EvtxChunkResult<Self> {
        let mut cursor = Cursor::new(data.as_slice());
        let header = EvtxChunkHeader::from_reader(&mut cursor)?;

        let chunk = EvtxChunkData { header, data };
        if validate_checksum && !chunk.validate_checksum() {
            // TODO: return checksum here.
            return Err(ChunkError::InvalidChunkChecksum {
                expected: 0,
                found: 0,
            });
        }

        Ok(chunk)
    }

    /// Require that the settings live at least as long as &self.
    pub fn parse(&mut self, settings: Arc<ParserSettings>) -> EvtxChunkResult<EvtxChunk> {
        EvtxChunk::new(&self.data, &self.header, Arc::clone(&settings))
    }

    pub fn validate_data_checksum(&self) -> bool {
        debug!("Validating data checksum");

        let checksum_disabled = self.header.flags.contains(ChunkFlags::NO_CRC32);

        let expected_checksum = if !checksum_disabled {
            self.header.events_checksum
        } else {
            0
        };

        let computed_checksum = if !checksum_disabled {
            checksum_ieee(
                &self.data[EVTX_CHUNK_HEADER_SIZE..self.header.free_space_offset as usize],
            )
        } else {
            0
        };

        debug!(
            "Expected checksum: {:?}, found: {:?}",
            expected_checksum, computed_checksum
        );

        computed_checksum == expected_checksum
    }

    pub fn validate_header_checksum(&self) -> bool {
        debug!("Validating header checksum");

        let checksum_disabled = self.header.flags.contains(ChunkFlags::NO_CRC32);

        let expected_checksum = if !checksum_disabled {
            self.header.header_chunk_checksum
        } else {
            0
        };

        let header_bytes_1 = &self.data[..120];
        let header_bytes_2 = &self.data[128..512];

        let bytes_for_checksum: Vec<u8> = header_bytes_1
            .iter()
            .chain(header_bytes_2)
            .cloned()
            .collect();

        let computed_checksum = if !checksum_disabled {
            checksum_ieee(bytes_for_checksum.as_slice())
        } else {
            0
        };

        debug!(
            "Expected checksum: {:?}, found: {:?}",
            expected_checksum, computed_checksum
        );

        computed_checksum == expected_checksum
    }

    pub fn validate_checksum(&self) -> bool {
        self.validate_header_checksum() && self.validate_data_checksum()
    }
}

/// A struct which can hold references to chunk data (`EvtxChunkData`).
/// All references are created together,
/// and can be assume to live for the entire duration of the parsing phase.
/// See more info about lifetimes in `IterChunkRecords`.
#[derive(Debug)]
pub struct EvtxChunk<'chunk> {
    pub data: &'chunk [u8],
    pub header: &'chunk EvtxChunkHeader,
    pub string_cache: StringCache,
    pub template_table: TemplateCache<'chunk>,

    pub settings: Arc<ParserSettings>,
}

impl<'chunk> EvtxChunk<'chunk> {
    /// Will fail if the data starts with an invalid evtx chunk header.
    pub fn new(
        data: &'chunk [u8],
        header: &'chunk EvtxChunkHeader,
        settings: Arc<ParserSettings>,
    ) -> EvtxChunkResult<EvtxChunk<'chunk>> {
        let _cursor = Cursor::new(data);

        info!("Initializing string cache");
        let string_cache = StringCache::populate(data, &header.strings_offsets)
            .map_err(|e| ChunkError::FailedToBuildStringCache { source: e })?;

        info!("Initializing template cache");
        let template_table =
            TemplateCache::populate(data, &header.template_offsets, settings.get_ansi_codec())?;

        Ok(EvtxChunk {
            header,
            data,
            string_cache,
            template_table,
            settings,
        })
    }

    /// Return an iterator of records from the chunk.
    /// See `IterChunkRecords` for a more detailed explanation regarding the lifetime scopes of the
    /// resulting records.
    pub fn iter(&mut self) -> IterChunkRecords {
        IterChunkRecords {
            settings: Arc::clone(&self.settings),
            chunk: self,
            offset_from_chunk_start: EVTX_CHUNK_HEADER_SIZE as u64,
            exhausted: false,
        }
    }
}

/// An iterator over a chunk, yielding records.
/// This iterator can be created using the `iter` function on `EvtxChunk`.
///
/// The 'a lifetime is (as can be seen in `iter`), smaller than the `chunk lifetime.
/// This is because we can only guarantee that the `EvtxRecord`s we are creating are valid for
/// the duration of the `EvtxChunk` borrow (because we reference the `TemplateCache` which is
/// owned by it).
///
/// In practice we have
///
/// | EvtxChunkData ---------------------------------------| Must live the longest, contain the actual data we refer to.
///
/// | EvtxChunk<'chunk>: ---------------------------- | Borrows `EvtxChunkData`.
///     &'chunk EvtxChunkData, TemplateCache<'chunk>
///
/// | IterChunkRecords<'a: 'chunk>:  ----- | Borrows `EvtxChunk` for 'a, but will only yield `EvtxRecord<'a>`.
///     &'a EvtxChunkData<'chunk>
///
/// The reason we only keep a single 'a lifetime (and not 'chunk as well) is because we don't
/// care about the larger lifetime, and so it allows us to simplify the definition of the struct.
pub struct IterChunkRecords<'a> {
    chunk: &'a EvtxChunk<'a>,
    offset_from_chunk_start: u64,
    exhausted: bool,
    settings: Arc<ParserSettings>,
}

impl<'a> Iterator for IterChunkRecords<'a> {
    type Item = std::result::Result<EvtxRecord<'a>, EvtxError>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        if self.exhausted
            || self.offset_from_chunk_start >= u64::from(self.chunk.header.free_space_offset)
        {
            return None;
        }

        let mut cursor = Cursor::new(&self.chunk.data[self.offset_from_chunk_start as usize..]);

        let record_header = match EvtxRecordHeader::from_reader(&mut cursor) {
            Ok(record_header) => record_header,
            Err(err) => {
                // We currently do not try to recover after an invalid record.
                self.exhausted = true;

                return Some(Err(EvtxError::DeserializationError(err)));
            }
        };

        info!("Record id - {}", record_header.event_record_id);
        debug!("Record header - {:?}", record_header);

        let binxml_data_size = record_header.record_data_size();

        trace!("Need to deserialize {} bytes of binxml", binxml_data_size);

        // `EvtxChunk` only owns `template_table`, which we want to loan to the Deserializer.
        // `data` and `string_cache` are both references and are `Copy`ed when passed to init.
        // We avoid creating new references so that `BinXmlDeserializer` can still generate 'a data.
        let deserializer = BinXmlDeserializer::init(
            self.chunk.data,
            self.offset_from_chunk_start + cursor.position(),
            Some(self.chunk),
            false,
            self.settings.get_ansi_codec(),
        );

        let mut tokens = vec![];
        let iter = match deserializer
            .iter_tokens(Some(binxml_data_size))
            .map_err(|e| EvtxError::FailedToParseRecord {
                record_id: record_header.event_record_id,
                source: Box::new(EvtxError::DeserializationError(e)),
            }) {
            Ok(iter) => iter,
            Err(err) => return Some(Err(err)),
        };

        for token in iter {
            match token.map_err(|e| EvtxError::FailedToParseRecord {
                source: Box::new(EvtxError::DeserializationError(e)),
                record_id: record_header.event_record_id,
            }) {
                Ok(token) => tokens.push(token),
                Err(err) => {
                    self.offset_from_chunk_start += u64::from(record_header.data_size);
                    return Some(Err(err));
                }
            }
        }

        self.offset_from_chunk_start += u64::from(record_header.data_size);

        if self.chunk.header.last_event_record_id == record_header.event_record_id {
            self.exhausted = true;
        }

        Some(Ok(EvtxRecord {
            chunk: self.chunk,
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            tokens,
            settings: Arc::clone(&self.settings),
        }))
    }
}

impl EvtxChunkHeader {
    pub fn from_reader(input: &mut Cursor<&[u8]>) -> DeserializationResult<EvtxChunkHeader> {
        let mut magic = [0_u8; 8];
        input.take(8).read_exact(&mut magic)?;

        if &magic != b"ElfChnk\x00" {
            return Err(DeserializationError::InvalidEvtxChunkMagic { magic });
        }

        let first_event_record_number = try_read!(input, u64)?;
        let last_event_record_number = try_read!(input, u64)?;
        let first_event_record_id = try_read!(input, u64)?;
        let last_event_record_id = try_read!(input, u64)?;

        let header_size = try_read!(input, u32)?;
        let last_event_record_data_offset = try_read!(input, u32)?;
        let free_space_offset = try_read!(input, u32)?;
        let events_checksum = try_read!(input, u32)?;

        // Reserved
        input.seek(SeekFrom::Current(64))?;

        let raw_flags = try_read!(input, u32)?;
        let flags = ChunkFlags::from_bits_truncate(raw_flags);

        let header_chunk_checksum = try_read!(input, u32)?;

        let mut strings_offsets = vec![0_u32; 64];
        input.read_u32_into::<LittleEndian>(&mut strings_offsets)?;

        let mut template_offsets = vec![0_u32; 32];
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
            flags,
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
            header_chunk_checksum: 978_805_790,
            flags: ChunkFlags::EMPTY,
            strings_offsets: vec![0_u32; 64],
            template_offsets: vec![0_u32; 32],
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

        let chunk = EvtxChunkData::new(chunk_data, false).unwrap();
        assert!(chunk.validate_checksum());
    }
}
