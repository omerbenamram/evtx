use crate::err::{
    ChunkError, DeserializationError, DeserializationResult, EvtxChunkResult, EvtxError,
};

use crate::evtx_record::{
    EVTX_RECORD_HEADER_SIZE, EVTX_RECORD_MAGIC, EvtxRecord, EvtxRecordHeader,
};
use crate::utils::bytes;

use log::{debug, info, trace};

use crate::binxml::ir::{IrTemplateCache, build_record_content};
use crate::string_cache::StringCache;
use crate::{ParserSettings, checksum_ieee};

use bumpalo::Bump;
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
    strings_offsets: [u32; 64],
    template_offsets: [u32; 32],
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
            // Preserve the public error fields and header-first short circuit.
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

    /// Computed CRC32 of the header (bytes 0..120 and 128..512).
    fn header_crc(&self) -> u32 {
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&self.data[..120]);
        hasher.update(&self.data[128..512]);
        hasher.finalize()
    }

    pub fn validate_data_checksum(&self) -> bool {
        self.header.flags.contains(ChunkFlags::NO_CRC32)
            || self
                .data
                .get(EVTX_CHUNK_HEADER_SIZE..self.header.free_space_offset as usize)
                .is_some_and(|data| self.header.events_checksum == checksum_ieee(data))
    }

    pub fn validate_header_checksum(&self) -> bool {
        self.header.flags.contains(ChunkFlags::NO_CRC32)
            || self.header.header_chunk_checksum == self.header_crc()
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
    pub(crate) string_cache: StringCache,
    #[doc(hidden)]
    pub arena: Bump,

    pub(crate) settings: Arc<ParserSettings>,
    /// Lazily-populated per-chunk program caches for `EvtxRecord::into_*`.
    pub(crate) render_caches: std::cell::RefCell<crate::binxml::compiled::RenderCaches>,
    /// Cross-chunk compiled-program store (per parser run; fresh per chunk
    /// when constructed through the public standalone-chunk API).
    pub(crate) program_store: std::sync::Arc<crate::binxml::compiled::ProgramStore>,
}

impl<'chunk> EvtxChunk<'chunk> {
    /// Will fail if the data starts with an invalid evtx chunk header.
    pub fn new(
        data: &'chunk [u8],
        header: &'chunk EvtxChunkHeader,
        settings: Arc<ParserSettings>,
    ) -> EvtxChunkResult<EvtxChunk<'chunk>> {
        EvtxChunk::new_with_arena(data, header, settings, Bump::new(), Default::default())
    }

    /// Will fail if the data starts with an invalid evtx chunk header.
    ///
    /// The provided arena is reset before use, so any previously allocated chunk-scoped
    /// data is cleared while retaining the bump capacity.
    pub(crate) fn new_with_arena(
        data: &'chunk [u8],
        header: &'chunk EvtxChunkHeader,
        settings: Arc<ParserSettings>,
        mut arena: Bump,
        program_store: Arc<crate::binxml::compiled::ProgramStore>,
    ) -> EvtxChunkResult<EvtxChunk<'chunk>> {
        arena.reset();

        info!("Initializing string cache");
        let string_cache = StringCache::populate(data, &header.strings_offsets)
            .map_err(|e| ChunkError::FailedToBuildStringCache { source: e })?;

        Ok(EvtxChunk {
            header,
            data,
            string_cache,
            arena,
            settings,
            render_caches: Default::default(),
            program_store,
        })
    }

    /// Iterate raw record windows (header walk only, no content build).
    /// Used by the compiled-template render path.
    pub(crate) fn iter_raw(&self) -> IterRawRecords<'_> {
        IterRawRecords {
            chunk: self,
            offset_from_chunk_start: EVTX_CHUNK_HEADER_SIZE as u64,
            state: RawState::Walk,
        }
    }

    /// Number of non-empty template-table buckets (cache size hint).
    pub(crate) fn estimated_template_buckets(&self) -> usize {
        self.header
            .template_offsets
            .iter()
            .filter(|&&offset| offset > 0)
            .count()
    }

    /// Return an iterator of records from the chunk.
    /// See `IterChunkRecords` for a more detailed explanation regarding the lifetime scopes of the
    /// resulting records.
    pub fn iter(&mut self) -> IterChunkRecords<'_> {
        let estimated_template_buckets = self.estimated_template_buckets();
        IterChunkRecords {
            raw: self.iter_raw(),
            ir_template_cache: IrTemplateCache::with_capacity(
                estimated_template_buckets,
                &self.arena,
            ),
        }
    }
}

/// An iterator over a chunk, yielding records.
/// This iterator can be created using the `iter` function on `EvtxChunk`.
///
/// The 'a lifetime is (as can be seen in `iter`), smaller than the `chunk lifetime.
/// This is because we can only guarantee that the `EvtxRecord`s we are creating are valid for
/// the duration of the `EvtxChunk` borrow (records reference chunk-scoped data and allocations).
///
/// In practice we have
///
/// | EvtxChunkData ---------------------------------------| Must live the longest, contain the actual data we refer to.
///
/// | EvtxChunk<'chunk>: ---------------------------- | Borrows `EvtxChunkData`.
///     &'chunk EvtxChunkData, StringCache, Bump
///
/// | IterChunkRecords<'a: 'chunk>:  ----- | Borrows `EvtxChunk` for 'a, but will only yield `EvtxRecord<'a>`.
///     &'a EvtxChunkData<'chunk>
///
/// The reason we only keep a single 'a lifetime (and not 'chunk as well) is because we don't
/// care about the larger lifetime, and so it allows us to simplify the definition of the struct.
pub struct IterChunkRecords<'a> {
    raw: IterRawRecords<'a>,
    /// Per-iterator template cache used during streaming tree construction.
    ir_template_cache: IrTemplateCache<'a>,
}

impl<'a> Iterator for IterChunkRecords<'a> {
    type Item = std::result::Result<EvtxRecord<'a>, EvtxError>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let chunk = self.raw.chunk;
        loop {
            let raw = match self.raw.next()? {
                Ok(raw) => raw,
                Err(err) => return Some(Err(err)),
            };

            let content = match build_record_content(raw.bytes, chunk, &mut self.ir_template_cache)
            {
                Ok(content) => content,
                // Carved hits that don't parse are noise, not errors.
                Err(err) if raw.recovered => {
                    debug!("Skipping carved record {}: {err:?}", raw.event_record_id);
                    self.raw.rescan_after(&raw);
                    continue;
                }
                Err(err) => return Some(Err(EvtxError::record(raw.event_record_id, err))),
            };

            return Some(Ok(EvtxRecord {
                chunk,
                event_record_id: raw.event_record_id,
                timestamp: raw.timestamp,
                content,
                binxml_offset: raw.binxml_offset,
                binxml_size: raw.bytes.len() as u32,
                recovered: raw.recovered,
            }));
        }
    }
}

/// One raw record: identifying header fields plus its BinXML byte window.
pub(crate) struct RawRecord<'a> {
    pub(crate) event_record_id: u64,
    pub(crate) timestamp: crate::evtx_record::Timestamp,
    pub(crate) binxml_offset: u64,
    pub(crate) bytes: &'a [u8],
    /// Carved from unallocated chunk space (see `ParserSettings::recover_records`).
    pub(crate) recovered: bool,
}

/// Iterator over raw record windows. Created by [`EvtxChunk::iter_raw`].
///
/// It walks the chunk's live records one after another, from the chunk header to the free
/// space (`free_space_offset`) or the last record (`last_event_record_id`). Past a record it
/// can't read, it reports the error and resyncs on the next live record (see
/// [`IterRawRecords::find_live_record`]). With `recover_records`, it also carves the stale
/// records between and after them.
pub(crate) struct IterRawRecords<'a> {
    chunk: &'a EvtxChunk<'a>,
    offset_from_chunk_start: u64,
    state: RawState,
}

#[derive(Debug, Clone, Copy)]
enum RawState {
    /// The next live record starts at the offset.
    Walk,
    /// Carving stale records that end by `until`, then walking from `until` if `walk_after`.
    Carve {
        until: u64,
        walk_after: bool,
    },
    Done,
}

impl<'a> Iterator for IterRawRecords<'a> {
    type Item = std::result::Result<RawRecord<'a>, EvtxError>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        loop {
            match self.state {
                RawState::Walk => {
                    if self.offset_from_chunk_start >= self.live_end() {
                        self.end_live_area();
                        continue;
                    }
                    match self.record_at(self.offset_from_chunk_start, false) {
                        Ok((raw, end)) => {
                            self.offset_from_chunk_start = end;
                            if raw.event_record_id == self.chunk.header.last_event_record_id {
                                self.end_live_area();
                            }
                            return Some(Ok(raw));
                        }
                        // Zeroes where a record should start are where the free space really
                        // starts when the header's `free_space_offset` is too large (issue
                        // #197), unless live records follow them.
                        Err(err) => {
                            if self.resync() || !is_blank(&err) {
                                return Some(Err(err));
                            }
                        }
                    }
                }
                RawState::Carve { until, walk_after } => {
                    if let Some(raw) = self.carve_record(until) {
                        return Some(Ok(raw));
                    }
                    if walk_after {
                        self.offset_from_chunk_start = until;
                        self.state = RawState::Walk;
                    } else {
                        self.state = RawState::Done;
                    }
                }
                RawState::Done => return None,
            }
        }
    }
}

/// A zeroed or truncated record header.
fn is_blank(err: &EvtxError) -> bool {
    matches!(
        err,
        EvtxError::DeserializationError(
            DeserializationError::InvalidEvtxRecordHeaderMagic {
                magic: [0, 0, 0, 0]
            } | DeserializationError::Truncated { .. }
        )
    )
}

impl<'a> IterRawRecords<'a> {
    /// End of the live area: the free space, or the chunk's end when it says more.
    fn live_end(&self) -> u64 {
        u64::from(self.chunk.header.free_space_offset).min(self.chunk.data.len() as u64)
    }

    /// No more live records: carve the rest of the chunk with `recover_records`.
    fn end_live_area(&mut self) {
        self.state = if self.chunk.settings.should_recover_records() {
            RawState::Carve {
                until: self.chunk.data.len() as u64,
                walk_after: false,
            }
        } else {
            RawState::Done
        };
    }

    /// Past the unreadable record at the offset, walk on from the next live record; with
    /// `recover_records`, carve the stale records before it first. False when no live
    /// record follows.
    fn resync(&mut self) -> bool {
        let Some(next) = self.find_live_record(self.offset_from_chunk_start + 1) else {
            self.end_live_area();
            return false;
        };
        if self.chunk.settings.should_recover_records() {
            self.state = RawState::Carve {
                until: next,
                walk_after: true,
            };
        } else {
            self.offset_from_chunk_start = next;
        }
        true
    }

    /// The offset of the next live record at or after `from`: a record magic whose header
    /// and trailing size copy check out, whose record starts by the chunk's last record and
    /// ends in the live area, and whose ID is in the chunk header's range. A chunk caught
    /// while it was being reused (a volume shadow copy's, say) also holds records of the
    /// log's other pass over it, whose IDs are outside that range.
    fn find_live_record(&self, mut from: u64) -> Option<u64> {
        let header = &self.chunk.header;
        let ids = header.first_event_record_id..=header.last_event_record_id;
        let live_end = self.live_end();
        while let Some(pos) = self.find_magic(from, live_end) {
            if let Ok((raw, end)) = self.record_at(pos, true)
                && end <= live_end
                && pos <= u64::from(header.last_event_record_data_offset)
                && ids.contains(&raw.event_record_id)
            {
                return Some(pos);
            }
            from = pos + 1;
        }
        None
    }

    /// The next record magic in `[from, until)`.
    fn find_magic(&self, from: u64, until: u64) -> Option<u64> {
        let data = self.chunk.data;
        let until = (until as usize).min(data.len());
        let from = (from as usize).min(until);
        memchr::memmem::find(&data[from..until], EVTX_RECORD_MAGIC).map(|hit| (from + hit) as u64)
    }

    /// Carve the next stale record from the offset that ends by `until`, and move past it.
    fn carve_record(&mut self, until: u64) -> Option<RawRecord<'a>> {
        while let Some(pos) = self.find_magic(self.offset_from_chunk_start, until) {
            match self.record_at(pos, true) {
                Ok((raw, end)) if end <= until => {
                    self.offset_from_chunk_start = end;
                    return Some(raw);
                }
                _ => self.offset_from_chunk_start = pos + 1,
            }
        }
        self.offset_from_chunk_start = until;
        None
    }

    /// Resume carving just after `raw`'s magic, for a hit that turned out to be noise
    /// (real records may sit inside the span it claimed).
    pub(crate) fn rescan_after(&mut self, raw: &RawRecord<'_>) {
        self.offset_from_chunk_start = raw.binxml_offset - EVTX_RECORD_HEADER_SIZE as u64 + 1;
    }

    /// The record at `record_start` and the offset it ends at. A carved one must also end
    /// with the copy of its size.
    fn record_at(
        &self,
        record_start: u64,
        carving: bool,
    ) -> std::result::Result<(RawRecord<'a>, u64), EvtxError> {
        let chunk = self.chunk;
        let record_header = EvtxRecordHeader::from_bytes_at(chunk.data, record_start as usize)?;

        trace!(
            "Record id - {} header - {:?}",
            record_header.event_record_id, record_header
        );

        let binxml_data_size = record_header.record_data_size()?;

        trace!("Need to deserialize {} bytes of binxml", binxml_data_size);

        let binxml_start = record_start + EVTX_RECORD_HEADER_SIZE as u64;
        let binxml_end = binxml_start.saturating_add(binxml_data_size as u64);
        if binxml_end > chunk.data.len() as u64 {
            return Err(EvtxError::record(
                record_header.event_record_id,
                EvtxError::FailedToCreateRecordModel("record BinXML slice is out of bounds"),
            ));
        }

        if carving
            && bytes::read_u32_le(chunk.data, binxml_end as usize) != Some(record_header.data_size)
        {
            return Err(EvtxError::FailedToCreateRecordModel(
                "carved record size copy mismatch",
            ));
        }

        let raw = RawRecord {
            event_record_id: record_header.event_record_id,
            timestamp: record_header.timestamp,
            binxml_offset: binxml_start,
            bytes: &chunk.data[binxml_start as usize..binxml_end as usize],
            recovered: carving,
        };
        Ok((raw, record_start + u64::from(record_header.data_size)))
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
        let mut strings_offsets = [0_u32; 64];
        for (i, offset) in strings_offsets.iter_mut().enumerate() {
            *offset = bytes::read_u32_le_r(data, 128 + i * 4, "chunk.strings_offsets")?;
        }
        let mut template_offsets = [0_u32; 32];
        for (i, offset) in template_offsets.iter_mut().enumerate() {
            *offset = bytes::read_u32_le_r(data, 384 + i * 4, "chunk.template_offsets")?;
        }

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

    #[test]
    fn test_parses_evtx_chunk_header() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk_header =
            &evtx_file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_HEADER_SIZE];

        let chunk_header = EvtxChunkHeader::from_bytes(chunk_header).unwrap();

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
        assert_eq!(chunk_header.strings_offsets.len(), 64);
        assert_eq!(chunk_header.template_offsets.len(), 32);
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
    fn checksum_errors_preserve_fields_and_reject_invalid_data_bounds() {
        let file = include_bytes!("../samples/security.evtx");
        let original = &file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
        for offset in [0, 511, EVTX_CHUNK_SIZE as u32 + 1, u32::MAX] {
            let mut data = original.to_vec();
            data[48..52].copy_from_slice(&offset.to_le_bytes());
            // An invalid header must fail before attempting to slice the record data.
            assert!(matches!(
                EvtxChunkData::new(data.clone(), true),
                Err(ChunkError::InvalidChunkChecksum {
                    expected: 0,
                    found: 0
                })
            ));

            // A valid header checksum must not make an invalid data range panic either.
            let chunk = EvtxChunkData::new(data.clone(), false).unwrap();
            let checksum = chunk.header_crc();
            data[124..128].copy_from_slice(&checksum.to_le_bytes());
            assert!(matches!(
                EvtxChunkData::new(data, true),
                Err(ChunkError::InvalidChunkChecksum {
                    expected: 0,
                    found: 0
                })
            ));
        }
        let mut data = original.to_vec();
        data[EVTX_CHUNK_HEADER_SIZE] ^= 1;
        assert!(matches!(
            EvtxChunkData::new(data, true),
            Err(ChunkError::InvalidChunkChecksum {
                expected: 0,
                found: 0
            })
        ));
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

    /// Chunk 0 of security.evtx (records 1 to 91), and the offset of each record by ID.
    fn security_chunk() -> (Vec<u8>, Vec<usize>) {
        let file = include_bytes!("../samples/security.evtx");
        let data = file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE].to_vec();
        let mut offsets = vec![0, EVTX_CHUNK_HEADER_SIZE];
        for id in 1..91 {
            let size = bytes::read_u32_le(&data, offsets[id] + 4).unwrap();
            offsets.push(offsets[id] + size as usize);
        }
        (data, offsets)
    }

    /// The raw records of a chunk: `(ID, recovered)` for each, `None` for an error.
    fn raw_records(data: Vec<u8>, recover: bool) -> Vec<Option<(u64, bool)>> {
        let mut chunk_data = EvtxChunkData::new(data, false).unwrap();
        let settings = Arc::new(ParserSettings::new().recover_records(recover));
        let chunk = chunk_data.parse(settings).unwrap();
        chunk
            .iter_raw()
            .map(|raw| raw.ok().map(|raw| (raw.event_record_id, raw.recovered)))
            .collect()
    }

    fn live(ids: std::ops::RangeInclusive<u64>) -> impl Iterator<Item = Option<(u64, bool)>> {
        ids.map(|id| Some((id, false)))
    }

    #[test]
    fn walk_resyncs_past_a_corrupt_record() {
        let (mut data, at) = security_chunk();
        data[at[30]] = b'X';
        let expected: Vec<_> = live(1..=29).chain([None]).chain(live(31..=91)).collect();
        assert_eq!(raw_records(data.clone(), false), expected);
        assert_eq!(raw_records(data, true), expected);
    }

    #[test]
    fn walk_resyncs_past_zeroed_records() {
        let (mut data, at) = security_chunk();
        data[at[30]..at[41]].fill(0);
        let expected: Vec<_> = live(1..=29).chain([None]).chain(live(41..=91)).collect();
        assert_eq!(raw_records(data.clone(), false), expected);
        assert_eq!(raw_records(data, true), expected);
    }

    #[test]
    fn walk_resyncs_on_records_in_the_chunk_header_range_only() {
        // Record 31 now has an ID from another pass of the log over the chunk.
        let (mut data, at) = security_chunk();
        data[at[30]] = b'X';
        data[at[31] + 8..at[31] + 16].copy_from_slice(&1000_u64.to_le_bytes());
        let before = || live(1..=29).chain([None]);
        assert_eq!(
            raw_records(data.clone(), false),
            before().chain(live(32..=91)).collect::<Vec<_>>()
        );
        assert_eq!(
            raw_records(data, true),
            before()
                .chain([Some((1000, true))])
                .chain(live(32..=91))
                .collect::<Vec<_>>()
        );
    }
}
