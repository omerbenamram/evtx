use crate::err::{
    ChunkError, DeserializationError, DeserializationResult, EvtxChunkResult, EvtxError,
};

use crate::evtx_record::{EVTX_RECORD_HEADER_SIZE, EvtxRecord, EvtxRecordHeader};
use crate::utils::bytes;

use log::{debug, info, trace};
use std::io::Cursor;

use crate::binxml::deserializer::BinXmlDeserializer;
use crate::binxml::ir::{build_tree_from_iter, IrTemplateCache};
use crate::string_cache::StringCache;
use crate::template_cache::TemplateCache;
use crate::{ParserSettings, checksum_ieee};

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
        let header = EvtxChunkHeader::from_bytes(&data)?;

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
    pub fn parse(&mut self, settings: Arc<ParserSettings>) -> EvtxChunkResult<EvtxChunk<'_>> {
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
            TemplateCache::populate(data, &header.template_offsets, settings.get_ansi_codec())
                .map_err(|e| ChunkError::FailedToBuildTemplateCache {
                    message: e.to_string(),
                    source: Box::new(e),
                })?;

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
    pub fn iter(&mut self) -> IterChunkRecords<'_> {
        IterChunkRecords {
            settings: Arc::clone(&self.settings),
            chunk: self,
            offset_from_chunk_start: EVTX_CHUNK_HEADER_SIZE as u64,
            exhausted: false,
            ir_template_cache: IrTemplateCache::new(),
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
    /// Per-iterator template cache used during streaming tree construction.
    ir_template_cache: IrTemplateCache<'a>,
}

impl<'a> Iterator for IterChunkRecords<'a> {
    type Item = std::result::Result<EvtxRecord<'a>, EvtxError>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        // Be resilient to corrupted chunk headers: `free_space_offset` is user-controlled data
        // coming from the EVTX stream, and may point past the end of the chunk.
        let effective_free_space_offset = u64::from(self.chunk.header.free_space_offset)
            .min(self.chunk.data.len().try_into().unwrap_or(u64::MAX));

        if self.exhausted || self.offset_from_chunk_start >= effective_free_space_offset {
            return None;
        }

        let record_start = self.offset_from_chunk_start;
        let record_start_usize = record_start as usize;

        if record_start_usize >= self.chunk.data.len() {
            // Avoid panicking on an out-of-bounds slice if the header is corrupted.
            self.exhausted = true;
            return None;
        }

        if self.chunk.data.len() - record_start_usize < 4 {
            // Not enough bytes for the record header magic, treat as end-of-chunk.
            self.exhausted = true;
            return None;
        }

        let record_header =
            match EvtxRecordHeader::from_bytes_at(self.chunk.data, record_start_usize) {
                Ok(record_header) => record_header,
                Err(DeserializationError::InvalidEvtxRecordHeaderMagic { magic }) => {
                    // Some producers write incorrect `free_space_offset` / `last_event_record_id`.
                    // In such cases we may attempt to parse the chunk slack area, which is typically
                    // zero-padded. Treat an all-zero "magic" as a clean end-of-chunk instead of
                    // emitting an error (see issue #197).
                    if magic == [0, 0, 0, 0] {
                        self.exhausted = true;
                        return None;
                    }

                    self.exhausted = true;
                    return Some(Err(EvtxError::DeserializationError(
                        DeserializationError::InvalidEvtxRecordHeaderMagic { magic },
                    )));
                }
                Err(DeserializationError::Truncated { .. }) => {
                    // Truncated record header near the end-of-chunk: treat as clean end-of-chunk.
                    self.exhausted = true;
                    return None;
                }
                Err(err) => {
                    // We currently do not try to recover after an invalid record.
                    self.exhausted = true;
                    return Some(Err(EvtxError::DeserializationError(err)));
                }
            };

        info!("Record id - {}", record_header.event_record_id);
        debug!("Record header - {:?}", record_header);

        let binxml_data_size = match record_header.record_data_size() {
            Ok(size) => size,
            Err(err) => {
                //The evtx record is corrupted, skip the rest of the chunk
                //It could be interesting to carve the rest of the chunk to find the next EVTX record header magic `2a2a0000`
                self.exhausted = true;
                return Some(Err(err));
            }
        };

        trace!("Need to deserialize {} bytes of binxml", binxml_data_size);

        // `EvtxChunk` only owns `template_table`, which we want to loan to the Deserializer.
        // `data` and `string_cache` are both references and are `Copy`ed when passed to init.
        // We avoid creating new references so that `BinXmlDeserializer` can still generate 'a data.
        let deserializer = BinXmlDeserializer::init(
            self.chunk.data,
            record_start + EVTX_RECORD_HEADER_SIZE as u64,
            Some(self.chunk),
            false,
            self.settings.get_ansi_codec(),
        );

        let iter = match deserializer
            .iter_tokens(Some(binxml_data_size))
            .map_err(|e| EvtxError::FailedToParseRecord {
                record_id: record_header.event_record_id,
                source: Box::new(EvtxError::DeserializationError(e)),
            }) {
            Ok(iter) => iter,
            Err(err) => return Some(Err(err)),
        };

        let tree_result =
            build_tree_from_iter(iter, self.chunk, &mut self.ir_template_cache).map_err(|err| {
                EvtxError::FailedToParseRecord {
                    record_id: record_header.event_record_id,
                    source: Box::new(err),
                }
            });

        self.offset_from_chunk_start += u64::from(record_header.data_size);

        if self.chunk.header.last_event_record_id == record_header.event_record_id {
            self.exhausted = true;
        }

        let tree = match tree_result {
            Ok(tree) => tree,
            Err(err) => return Some(Err(err)),
        };

        Some(Ok(EvtxRecord {
            chunk: self.chunk,
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            tree,
            binxml_offset: record_start + EVTX_RECORD_HEADER_SIZE as u64,
            binxml_size: binxml_data_size,
            settings: Arc::clone(&self.settings),
        }))
    }
}

impl EvtxChunkHeader {
    pub fn from_bytes(data: &[u8]) -> DeserializationResult<EvtxChunkHeader> {
        // We only parse the fixed header prefix; the rest of the chunk may be shorter in some
        // corrupted cases, but the header itself must be present.
        let _ = bytes::slice_r(data, 0, EVTX_CHUNK_HEADER_SIZE, "EVTX chunk header")?;

        let magic = bytes::read_array_r::<8>(data, 0, "chunk header magic")?;

        if &magic != b"ElfChnk\x00" {
            return Err(DeserializationError::InvalidEvtxChunkMagic { magic });
        }

        let first_event_record_number =
            bytes::read_u64_le_r(data, 8, "chunk.first_event_record_number")?;
        let last_event_record_number =
            bytes::read_u64_le_r(data, 16, "chunk.last_event_record_number")?;
        let first_event_record_id = bytes::read_u64_le_r(data, 24, "chunk.first_event_record_id")?;
        let last_event_record_id = bytes::read_u64_le_r(data, 32, "chunk.last_event_record_id")?;

        let header_size = bytes::read_u32_le_r(data, 40, "chunk.header_size")?;
        let last_event_record_data_offset =
            bytes::read_u32_le_r(data, 44, "chunk.last_event_record_data_offset")?;
        let free_space_offset = bytes::read_u32_le_r(data, 48, "chunk.free_space_offset")?;
        let events_checksum = bytes::read_u32_le_r(data, 52, "chunk.events_checksum")?;

        let raw_flags = bytes::read_u32_le_r(data, 120, "chunk.flags")?;
        let flags = ChunkFlags::from_bits_truncate(raw_flags);

        let header_chunk_checksum = bytes::read_u32_le_r(data, 124, "chunk.header_chunk_checksum")?;

        // Offsets arrays: fixed sizes (64 + 32 u32s).
        let strings_offsets = bytes::read_u32_vec_le_r(data, 128, 64, "chunk.strings_offsets")?;
        let template_offsets = bytes::read_u32_vec_le_r(data, 384, 32, "chunk.template_offsets")?;

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

    pub fn from_reader(input: &mut Cursor<&[u8]>) -> DeserializationResult<EvtxChunkHeader> {
        let start = input.position() as usize;
        let buf = input.get_ref();
        let slice = bytes::slice_r(buf, start, EVTX_CHUNK_HEADER_SIZE, "EVTX chunk header")?;

        let header = Self::from_bytes(slice)?;
        input.set_position((start + EVTX_CHUNK_HEADER_SIZE) as u64);
        Ok(header)
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

    #[test]
    fn test_iter_ends_cleanly_when_chunk_header_offsets_are_too_large() {
        ensure_env_logger_initialized();

        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk_data =
            evtx_file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE].to_vec();

        // Parse once to get a baseline count.
        let mut baseline = EvtxChunkData::new(chunk_data.clone(), false).unwrap();
        let settings = Arc::new(ParserSettings::new());
        let baseline_count = {
            let mut chunk = baseline.parse(Arc::clone(&settings)).unwrap();
            chunk
                .iter()
                .try_fold(0usize, |acc, record| record.map(|_| acc + 1))
                .unwrap()
        };

        // Now simulate a broken chunk header like in issue #197: `last_event_record_id` and
        // `free_space_offset` are larger than the actual number of records/data.
        let mut corrupted = EvtxChunkData::new(chunk_data, false).unwrap();
        corrupted.header.last_event_record_id =
            corrupted.header.last_event_record_id.saturating_add(100);
        corrupted.header.free_space_offset = EVTX_CHUNK_SIZE as u32;

        let corrupted_count = {
            let mut chunk = corrupted.parse(settings).unwrap();
            chunk
                .iter()
                .try_fold(0usize, |acc, record| record.map(|_| acc + 1))
                .unwrap()
        };

        assert_eq!(corrupted_count, baseline_count);
    }
}
