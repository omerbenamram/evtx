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
    #[derive(Debug)]
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
    pub arena: bumpalo::Bump,
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
            arena: bumpalo::Bump::new(),
        })
    }

    /// Return an iterator of records from the chunk.
    /// See `IterChunkRecords` for a more detailed explanation regarding the lifetime scopes of the
    /// resulting records.
    pub fn iter(&'chunk mut self) -> IterChunkRecords<'chunk> {
        IterChunkRecords {
            settings: Arc::clone(&self.settings),
            chunk_ptr: self as *mut EvtxChunk<'chunk>,
            offset_from_chunk_start: EVTX_CHUNK_HEADER_SIZE as u64,
            exhausted: false,
            _marker: std::marker::PhantomData,
        }
    }
}

/// An iterator over a chunk, yielding records tied to the iterator borrow lifetime.
pub struct IterChunkRecords<'chunk> {
    chunk_ptr: *mut EvtxChunk<'chunk>,
    offset_from_chunk_start: u64,
    exhausted: bool,
    settings: Arc<ParserSettings>,
    _marker: std::marker::PhantomData<&'chunk mut EvtxChunk<'chunk>>,
}

impl<'chunk> Iterator for IterChunkRecords<'chunk> {
    type Item = std::result::Result<EvtxRecord<'chunk>, EvtxError>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        // SAFETY: chunk_ptr was created from a valid mutable reference in iter() and points to a live EvtxChunk for the lifetime 'chunk.
        let chunk: &mut EvtxChunk<'chunk> = unsafe { &mut *self.chunk_ptr };

        if self.exhausted
            || self.offset_from_chunk_start >= u64::from(chunk.header.free_space_offset)
        {
            return None;
        }

        let mut cursor = Cursor::new(&chunk.data[self.offset_from_chunk_start as usize..]);
        let record_header = match EvtxRecordHeader::from_reader(&mut cursor) {
            Ok(record_header) => record_header,
            Err(err) => {
                self.exhausted = true;
                return Some(Err(EvtxError::DeserializationError(err)));
            }
        };

        info!("Record id - {}", record_header.event_record_id);
        debug!("Record header - {:?}", record_header);
        let binxml_data_size = record_header.record_data_size();
        trace!("Need to deserialize {} bytes of binxml", binxml_data_size);

        // Move iterator state forward before borrowing chunk immutably
        let current_offset = self.offset_from_chunk_start;
        let body_start_offset = current_offset + cursor.position();
        self.offset_from_chunk_start = current_offset + u64::from(record_header.data_size);
        if chunk.header.last_event_record_id == record_header.event_record_id {
            self.exhausted = true;
        }

        // Reset arena BEFORE taking immutable borrows
        chunk.arena.reset();

        // Build deserializer borrowing the chunk immutably
        let chunk_immut: &'chunk EvtxChunk<'chunk> = unsafe { &*self.chunk_ptr };
        let deserializer = BinXmlDeserializer::init(
            chunk_immut.data,
            body_start_offset,
            Some(chunk_immut),
            false,
            self.settings.get_ansi_codec(),
        );

        // Heuristic: average token size is small; pre-reserve proportional to record data size to reduce growth
        let token_capacity_hint: usize = {
            // Bias more aggressively to avoid reallocation: assume ~3 bytes/token, add larger headroom
            let approx = (binxml_data_size as usize) / 3;
            let approx = approx.saturating_add(256);
            if approx < 256 { 256 } else if approx > 262144 { 262144 } else { approx }
        };
        let mut tokens_bv: bumpalo::collections::Vec<'chunk, crate::model::deserialized::BinXMLDeserializedTokens<'chunk>> =
            bumpalo::collections::Vec::with_capacity_in(token_capacity_hint, &chunk_immut.arena);

        let iter = match deserializer.iter_tokens(Some(binxml_data_size)).map_err(|e| EvtxError::FailedToParseRecord {
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
                Ok(token) => tokens_bv.push(token),
                Err(err) => {
                    return Some(Err(err));
                }
            }
        }

        let tokens_slice: &'chunk [crate::model::deserialized::BinXMLDeserializedTokens<'chunk>] = tokens_bv.into_bump_slice();

        Some(Ok(EvtxRecord {
            chunk: chunk_immut,
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            tokens: tokens_slice,
            record_data_size: binxml_data_size,
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
            flags: ChunkFlags::empty(),
            template_offsets: vec![0; 32],
            strings_offsets: vec![0; 64],
        };

        assert_eq!(chunk_header.header_size, expected.header_size);
        assert_eq!(chunk_header.free_space_offset, expected.free_space_offset);
    }
}
