//! Carving EVTX chunks out of raw data: a disk image, unallocated space, a memory dump or a
//! damaged file whose header is gone.
//!
//! A chunk is self-contained: 64 KiB that start with `ElfChnk\0` and hold the records, string
//! table and templates they need. [`CarvedEvtx`] scans a source for chunk headers, keeps the
//! ones that look like real chunks, drops exact copies, and presents them to [`EvtxParser`] as
//! a virtual EVTX file: a file header it makes up, then each carved chunk in the order found.
//! So everything the parser does works on carved chunks too, including recovering stale records
//! from their slack (`ParserSettings::recover_records`).
//!
//! Records that are not inside a chunk (the chunk header was overwritten) can't be carved this
//! way: their templates and names are kept in the chunk.
//!
//! [`EvtxParser`]: crate::EvtxParser
//!
//! # Example
//!
//! ```no_run
//! use evtx::EvtxParser;
//! use evtx::carve::CarvedEvtx;
//! use std::fs::File;
//!
//! let carved = CarvedEvtx::new(File::open("unallocated.bin")?)?;
//! for (number, offset) in carved.chunk_offsets().iter().enumerate() {
//!     eprintln!("chunk {number} was carved at offset {offset}");
//! }
//! let mut parser = EvtxParser::from_read_seek(carved)?;
//! for record in parser.records_json() {
//!     println!("{}", record?.data);
//! }
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::collections::HashSet;
use std::io::{self, Read, Seek, SeekFrom};
use std::ops::Range;

use crate::checksum_ieee;
use crate::evtx_chunk::{EVTX_CHUNK_HEADER_SIZE, EvtxChunkHeader, chunk_header_crc};
use crate::evtx_parser::{EVTX_CHUNK_SIZE, EVTX_FILE_HEADER_SIZE};
use crate::evtx_record::{EVTX_RECORD_HEADER_SIZE, EVTX_RECORD_MAGIC};
use crate::utils::bytes;

const CHUNK_MAGIC: &[u8; 8] = b"ElfChnk\0";
/// How much of the source is searched at once.
const SCAN_BLOCK_SIZE: usize = 8 << 20;
/// How much is given up around a byte that can't be read: a page, which is what a read of a
/// bad sector through the page cache loses.
const UNREADABLE_STEP: u64 = 4096;

/// The chunks carved from a source, read as a virtual EVTX file (see the [module docs](self)).
///
/// It reads each chunk from the source when the parser asks for it, so the source is only held
/// open, not copied. A chunk that the source holds only part of, because the source ends or the
/// next chunk carved starts within its 64 KiB, reads as zeros from the first of its records that
/// the source doesn't hold whole: the rest of that record isn't there.
///
/// Where the source can't be read (a bad sector), the scan goes on past it, and
/// [`unreadable_bytes`](Self::unreadable_bytes) says how much was given up. A chunk with such
/// a hole in it ends there, like one that the source holds only part of.
#[derive(Debug)]
pub struct CarvedEvtx<R> {
    source: R,
    offsets: Vec<u64>,
    /// Where in the source the bytes read for each chunk end.
    ends: Vec<u64>,
    header: Vec<u8>,
    position: u64,
    unreadable: u64,
}

impl<R: Read + Seek> CarvedEvtx<R> {
    /// Scans all of `source` for chunks.
    ///
    /// Fails when the source can't be read from its start (nothing in its first 8 MiB): it is
    /// not a file, or it is gone. A read error further in is not an error, see
    /// [`unreadable_bytes`](Self::unreadable_bytes).
    pub fn new(mut source: R) -> io::Result<Self> {
        let source_len = source.seek(SeekFrom::End(0))?;
        let found = find_chunks(&mut source, source_len)?;
        let header = file_header(found.offsets.len(), found.next_record_id);
        Ok(CarvedEvtx {
            source,
            offsets: found.offsets,
            ends: found.ends,
            header,
            position: 0,
            unreadable: found.unreadable,
        })
    }

    /// How many bytes of the source the scan could not read, and so did not search. Zero for a
    /// source that reads without errors.
    pub fn unreadable_bytes(&self) -> u64 {
        self.unreadable
    }

    /// Where in the source each chunk was found, by chunk number: errors about chunk `n` of
    /// the virtual file are about the chunk at `chunk_offsets()[n]`.
    pub fn chunk_offsets(&self) -> &[u64] {
        &self.offsets
    }

    /// The size of the virtual file.
    fn len(&self) -> u64 {
        EVTX_FILE_HEADER_SIZE as u64 + self.offsets.len() as u64 * EVTX_CHUNK_SIZE as u64
    }
}

impl<R: Read + Seek> Read for CarvedEvtx<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let position = self.position;
        let header_size = EVTX_FILE_HEADER_SIZE as u64;
        let read = if position >= self.len() || buf.is_empty() {
            0
        } else if position < header_size {
            let header = &self.header[position as usize..];
            let n = header.len().min(buf.len());
            buf[..n].copy_from_slice(&header[..n]);
            n
        } else {
            let chunk_size = EVTX_CHUNK_SIZE as u64;
            let index = ((position - header_size) / chunk_size) as usize;
            let within = (position - header_size) % chunk_size;
            let n = buf.len().min((chunk_size - within) as usize);
            let at = self.offsets[index] + within;
            // The part of the chunk that the source has, then zeros.
            let available = self.ends[index].saturating_sub(at).min(n as u64) as usize;
            if available > 0 {
                self.source.seek(SeekFrom::Start(at))?;
                self.source.read_exact(&mut buf[..available])?;
            }
            buf[available..n].fill(0);
            n
        };
        self.position += read as u64;
        Ok(read)
    }
}

impl<R: Read + Seek> Seek for CarvedEvtx<R> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let position = match pos {
            SeekFrom::Start(offset) => Some(offset),
            SeekFrom::End(delta) => self.len().checked_add_signed(delta),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
        }
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before the start"))?;
        self.position = position;
        Ok(position)
    }
}

/// The chunks in a source, in the order found, without exact copies.
struct Found {
    offsets: Vec<u64>,
    /// Where in the source the bytes read for each chunk end.
    ends: Vec<u64>,
    /// The record ID after the highest one they hold.
    next_record_id: u64,
    seen: HashSet<(u32, [u8; 32])>,
    /// How many bytes of the source could not be read.
    unreadable: u64,
    /// Where those bytes are, in order.
    holes: Vec<Range<u64>>,
}

impl Found {
    /// Adds the chunk at `offset`, whose first 64 KiB of the source are in `chunk`, of which only
    /// the bytes up to `end` are its own, unless it is a copy of one added already.
    fn add(&mut self, offset: u64, header: &EvtxChunkHeader, chunk: &mut [u8], end: u64) {
        let len = whole_records_len(chunk, header, (end - offset) as usize);
        chunk[len..].fill(0);
        // The same chunk in a copy of the log (a volume shadow copy, a backup) adds nothing.
        if self.seen.insert(chunk_identity(chunk)) {
            self.offsets.push(offset);
            self.ends.push(offset + len as u64);
            self.next_record_id = self
                .next_record_id
                .max(header.last_event_record_id.saturating_add(1));
        }
    }
}

/// The chunks in `source`.
fn find_chunks<R: Read + Seek>(source: &mut R, source_len: u64) -> io::Result<Found> {
    let finder = memchr::memmem::Finder::new(CHUNK_MAGIC);
    let mut found = Found {
        offsets: Vec::new(),
        ends: Vec::new(),
        next_record_id: 1,
        seen: HashSet::new(),
        unreadable: 0,
        holes: Vec::new(),
    };
    // Each block is searched with the end of the one before it in front, so that a magic that
    // straddles them is found.
    let overlap = CHUNK_MAGIC.len() - 1;
    let mut block = vec![0u8; overlap + SCAN_BLOCK_SIZE];
    let mut carried = 0usize;
    // The candidate looked at, and the last chunk found, which is added once it is known where
    // it ends: the next chunk found can start inside it.
    let mut chunk = vec![0u8; EVTX_CHUNK_SIZE];
    let mut last_chunk = vec![0u8; EVTX_CHUNK_SIZE];
    // With it, where it ends at the latest: after 64 KiB, or at the first byte of it that
    // can't be read.
    let mut last: Option<(u64, EvtxChunkHeader, u64)> = None;
    // How much of the source has been read, and where the records of the last chunk found
    // end, when its events CRC32 shows that they are intact.
    let mut read_to = 0u64;
    let mut records_end = 0u64;
    // Enough of a candidate to tell whether it is a chunk.
    let head_size = EVTX_CHUNK_HEADER_SIZE + EVTX_RECORD_MAGIC.len();

    while read_to < source_len {
        let len = (source_len - read_to).min(SCAN_BLOCK_SIZE as u64) as usize;
        // An unreadable region (a bad sector) is skipped, not the end of the scan: the chunks
        // around it are still worth carving.
        let unread = read_or_zero(source, read_to, &mut block[carried..carried + len]);
        if let Some(Unreadable { holes, error }) = unread {
            let bytes: u64 = holes.iter().map(|hole| hole.end - hole.start).sum();
            found.unreadable += bytes;
            // A source that gives nothing from its start is not one with a bad sector, and
            // could go on failing without end: a directory is as long as a file can be.
            if found.unreadable == read_to + len as u64 {
                return Err(error);
            }
            log::warn!("carving: skipping {bytes} bytes after offset {read_to}: {error}");
            found.holes.extend(holes);
        }
        let block_start = read_to - carried as u64;
        let searched = &block[..carried + len];

        for at in finder.find_iter(searched) {
            let offset = block_start + at as u64;
            // A chunk's records could hold the magic as data. Only intact records are skipped:
            // a chunk cut short can be followed by another chunk at any offset.
            if offset < records_end {
                continue;
            }
            let available = (source_len - offset).min(EVTX_CHUNK_SIZE as u64) as usize;
            // Read the header first: most candidates are the magic by chance.
            let head = available.min(head_size);
            let head_hole = read_at(source, searched, at, offset, &mut chunk[..head]);
            chunk[head..head_size].fill(0);
            let Some(header) = looks_like_a_chunk(&chunk) else {
                continue;
            };
            let rest = &mut chunk[head..available];
            let rest_hole = read_at(source, searched, at + head, offset + head as u64, rest);
            chunk[available..].fill(0);
            // The holes found so far are before the end of this block, and the other two are
            // the first ones after it.
            let after = found.holes.partition_point(|hole| hole.end <= offset);
            let known_hole = found.holes.get(after).map(|hole| hole.start);
            let limit = [known_hole, head_hole, rest_hole]
                .into_iter()
                .flatten()
                .fold(offset + available as u64, u64::min);
            // A chunk without CRC32s (`NO_CRC32`) can't show that its records are intact.
            let records = &chunk[EVTX_CHUNK_HEADER_SIZE..header.free_space_offset as usize];
            if checksum_ieee(records) == header.events_checksum {
                records_end = offset + u64::from(header.free_space_offset);
            }
            // The last chunk found ends where this one starts, if not before.
            if let Some((last_offset, last_header, last_limit)) = last.take() {
                let end = offset.min(last_limit);
                found.add(last_offset, &last_header, &mut last_chunk, end);
            }
            std::mem::swap(&mut chunk, &mut last_chunk);
            last = Some((offset, header, limit));
        }

        let searched_len = searched.len();
        read_to += len as u64;
        carried = overlap.min(searched_len);
        block.copy_within(searched_len - carried..searched_len, 0);
    }
    if let Some((last_offset, last_header, last_limit)) = last {
        found.add(last_offset, &last_header, &mut last_chunk, last_limit);
    }
    Ok(found)
}

/// Fills `buf` with the bytes of the source at `offset`: from `searched`, where they are at
/// `at`, when it holds all of them. Otherwise it reads them, and gives where the first of them
/// that can't be read is. The scan of the block that they are in counts and reports those.
fn read_at<R: Read + Seek>(
    source: &mut R,
    searched: &[u8],
    at: usize,
    offset: u64,
    buf: &mut [u8],
) -> Option<u64> {
    if let Some(bytes) = searched.get(at..at + buf.len()) {
        buf.copy_from_slice(bytes);
        return None;
    }
    let unreadable = read_or_zero(source, offset, buf)?;
    unreadable.holes.first().map(|hole| hole.start)
}

/// What a read had to give up.
struct Unreadable {
    /// Where, in order. Never empty.
    holes: Vec<Range<u64>>,
    /// The first error.
    error: io::Error,
}

/// Fills `buf` with the bytes of the source at `offset`. If that fails, it reads them a page
/// at a time, leaves zeros for each page that can't be read, and says where those are.
fn read_or_zero<R: Read + Seek>(source: &mut R, offset: u64, buf: &mut [u8]) -> Option<Unreadable> {
    let mut read = |offset: u64, buf: &mut [u8]| {
        source
            .seek(SeekFrom::Start(offset))
            .and_then(|_| source.read_exact(buf))
    };
    let error = read(offset, buf).err()?;
    let mut holes: Vec<Range<u64>> = Vec::new();
    let mut done = 0usize;
    while done < buf.len() {
        let at = offset + done as u64;
        let to_page_end = UNREADABLE_STEP - at % UNREADABLE_STEP;
        let step = (buf.len() - done).min(to_page_end as usize);
        let page = &mut buf[done..done + step];
        if read(at, page).is_err() {
            page.fill(0);
            match holes.last_mut() {
                Some(hole) if hole.end == at => hole.end += step as u64,
                _ => holes.push(at..at + step as u64),
            }
        }
        done += step;
    }
    // Nothing given up: the error did not come back.
    (!holes.is_empty()).then_some(Unreadable { holes, error })
}

/// How much of a chunk of which the source holds only the first `held` bytes to read: up to
/// the first of its live records that the source doesn't hold whole. The parser would read the
/// rest of that record from what follows (zeros, or another chunk) as if it were its own.
fn whole_records_len(chunk: &[u8], header: &EvtxChunkHeader, held: usize) -> usize {
    let live_end = header.free_space_offset as usize;
    if held >= live_end {
        return held;
    }
    let mut at = EVTX_CHUNK_HEADER_SIZE;
    // Walk the live records as the parser does, from one to the next.
    while at < live_end {
        if at + EVTX_RECORD_HEADER_SIZE > held {
            return at.min(held);
        }
        let (Some(size), Some(id)) = (
            bytes::read_u32_le(chunk, at + 4),
            bytes::read_u64_le(chunk, at + 8),
        ) else {
            break;
        };
        let end = at.saturating_add(size as usize);
        // Past a damaged record, the parser looks for the next live record itself.
        if !chunk[at..].starts_with(EVTX_RECORD_MAGIC)
            || (size as usize) < EVTX_RECORD_HEADER_SIZE
            || end > live_end
        {
            break;
        }
        if end > held {
            return at;
        }
        if id == header.last_event_record_id {
            break;
        }
        at = end;
    }
    held
}

/// Whether a chunk header is that of a chunk rather than the magic by chance: its fields are
/// in range, and its header CRC32 matches or a record starts right after it. Gives the chunk's
/// header if so. Reads only the header and the 4 bytes after it.
fn looks_like_a_chunk(chunk: &[u8]) -> Option<EvtxChunkHeader> {
    let header = EvtxChunkHeader::from_bytes(chunk).ok()?;
    let in_range = header.header_size == 128
        && (EVTX_CHUNK_HEADER_SIZE as u32..=EVTX_CHUNK_SIZE as u32)
            .contains(&header.free_space_offset)
        && header.last_event_record_data_offset < header.free_space_offset
        && header.first_event_record_number <= header.last_event_record_number
        && header.first_event_record_id <= header.last_event_record_id;
    if !in_range {
        return None;
    }
    (chunk_header_crc(chunk) == header.header_chunk_checksum
        || chunk[EVTX_CHUNK_HEADER_SIZE..].starts_with(EVTX_RECORD_MAGIC))
    .then_some(header)
}

/// Tells chunks apart: two chunks with the same CRC32 and record range are taken to be the
/// same chunk.
fn chunk_identity(chunk: &[u8]) -> (u32, [u8; 32]) {
    let mut range = [0u8; 32];
    range.copy_from_slice(&chunk[8..40]);
    (checksum_ieee(chunk), range)
}

/// A file header for `chunk_count` carved chunks: the oldest chunk is the first, and the log is
/// marked dirty, since it wasn't closed.
fn file_header(chunk_count: usize, next_record_id: u64) -> Vec<u8> {
    let mut file_header = vec![0u8; EVTX_FILE_HEADER_SIZE];
    file_header[..8].copy_from_slice(b"ElfFile\0");
    let last_chunk = chunk_count.saturating_sub(1) as u64;
    file_header[16..24].copy_from_slice(&last_chunk.to_le_bytes());
    file_header[24..32].copy_from_slice(&next_record_id.to_le_bytes());
    file_header[32..36].copy_from_slice(&128u32.to_le_bytes());
    file_header[36..38].copy_from_slice(&1u16.to_le_bytes());
    file_header[38..40].copy_from_slice(&3u16.to_le_bytes());
    file_header[40..42].copy_from_slice(&(EVTX_FILE_HEADER_SIZE as u16).to_le_bytes());
    // The parser counts the chunks from the file size, past what fits here.
    let chunk_count = u16::try_from(chunk_count).unwrap_or(u16::MAX);
    file_header[42..44].copy_from_slice(&chunk_count.to_le_bytes());
    // DIRTY.
    file_header[120..124].copy_from_slice(&1u32.to_le_bytes());
    let checksum = checksum_ieee(&file_header[..120]);
    file_header[124..128].copy_from_slice(&checksum.to_le_bytes());
    file_header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EvtxParser, ParserSettings};
    use std::collections::BTreeSet;
    use std::io::Cursor;

    fn record_ids(parser: &mut EvtxParser<impl crate::evtx_parser::ReadSeek>) -> BTreeSet<u64> {
        parser
            .records()
            .map(|record| record.unwrap().event_record_id)
            .collect()
    }

    /// A source whose bytes in `bad` can't be read, like a disk with bad sectors.
    struct BadSectors {
        inner: Cursor<Vec<u8>>,
        bad: std::ops::Range<u64>,
    }

    impl Read for BadSectors {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let start = self.inner.position();
            let end = start + buf.len() as u64;
            if start < self.bad.end && self.bad.start < end {
                return Err(io::Error::other("bad sector"));
            }
            self.inner.read(buf)
        }
    }

    impl Seek for BadSectors {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.inner.seek(pos)
        }
    }

    #[test]
    fn an_unreadable_region_does_not_end_the_scan() {
        let log = include_bytes!("../samples/security.evtx");
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        // A bad region, then the log's chunks past the first scan block.
        let mut image = vec![0u8; SCAN_BLOCK_SIZE + 4096];
        image.extend_from_slice(chunks);
        let source = BadSectors {
            inner: Cursor::new(image),
            bad: 512..1024,
        };
        let carved = CarvedEvtx::new(source).unwrap();
        assert!(!carved.chunk_offsets().is_empty());
        assert_eq!(carved.chunk_offsets()[0], (SCAN_BLOCK_SIZE + 4096) as u64);
        assert_eq!(carved.unreadable_bytes(), 4096);
    }

    #[test]
    fn a_bad_sector_costs_its_page_and_not_the_chunks_near_it() {
        let log = include_bytes!("../samples/security.evtx");
        let expected = record_ids(&mut EvtxParser::from_buffer(log.to_vec()).unwrap());
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        // A bad sector, and the log's chunks 1 MiB after it, in the same scan block.
        let mut image = vec![0u8; 1 << 20];
        image.extend_from_slice(chunks);
        let source = BadSectors {
            inner: Cursor::new(image),
            bad: 512..1024,
        };
        let carved = CarvedEvtx::new(source).unwrap();
        assert_eq!(carved.chunk_offsets().len(), 26);
        assert_eq!(carved.unreadable_bytes(), 4096);
        let mut parser = EvtxParser::from_read_seek(carved).unwrap();
        assert_eq!(record_ids(&mut parser), expected);
    }

    #[test]
    fn a_chunk_with_a_bad_sector_ends_there() {
        let log = include_bytes!("../samples/security.evtx");
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        // Records 1 to 9 are whole in the first chunk's first 8 KiB, and the page after that,
        // where record 10 goes on, is bad: zeros there would read as its values. The second
        // chunk starts at record 92. The chunk is in the first scan block, and then astride
        // the first two.
        for chunks_at in [0, SCAN_BLOCK_SIZE - 4096] {
            let mut image = vec![0u8; chunks_at];
            image.extend_from_slice(chunks);
            let bad = chunks_at as u64 + 8192;
            let source = BadSectors {
                inner: Cursor::new(image),
                bad: bad..bad + 8,
            };
            let carved = CarvedEvtx::new(source).unwrap();
            assert_eq!(carved.chunk_offsets().len(), 26);
            assert_eq!(carved.unreadable_bytes(), 4096);
            let mut parser = EvtxParser::from_read_seek(carved).unwrap();
            let ids = record_ids(&mut parser);
            assert_eq!(ids.len(), 9 + (2261 - 91));
            assert!(ids.range(..92).eq(&(1..=9).collect::<BTreeSet<u64>>()));
        }
    }

    #[test]
    fn a_magic_that_straddles_two_scan_blocks_is_found() {
        let log = include_bytes!("../samples/security.evtx");
        let chunk = &log[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
        for before_block_end in 1..CHUNK_MAGIC.len() {
            let mut image = vec![0u8; SCAN_BLOCK_SIZE - before_block_end];
            image.extend_from_slice(chunk);
            let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
            let at = (SCAN_BLOCK_SIZE - before_block_end) as u64;
            assert_eq!(carved.chunk_offsets(), [at]);
        }
    }

    #[test]
    fn a_source_that_cannot_be_read_is_an_error() {
        // It ends at once, where a directory would go on failing: it is as long as a file can
        // be.
        let source = BadSectors {
            inner: Cursor::new(vec![0u8; 3 * SCAN_BLOCK_SIZE]),
            bad: 0..u64::MAX,
        };
        let error = CarvedEvtx::new(source).err().unwrap();
        assert_eq!(error.to_string(), "bad sector");
    }

    #[test]
    fn carves_the_chunks_of_a_log_buried_in_noise() {
        let log = include_bytes!("../samples/security.evtx");
        let expected = record_ids(&mut EvtxParser::from_buffer(log.to_vec()).unwrap());

        // Noise, then the log's chunks at an odd offset without its file header, then the
        // first two chunks again (a copy), then a chunk magic by chance.
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        let mut image: Vec<u8> = (0..100_003u32).map(|i| (i * 7 + 3) as u8).collect();
        let chunks_at = image.len() as u64;
        image.extend_from_slice(chunks);
        image.extend_from_slice(&[0xAB; 1234]);
        image.extend_from_slice(&chunks[..2 * EVTX_CHUNK_SIZE]);
        image.extend_from_slice(b"ElfChnk\0 but not a chunk");
        image.extend_from_slice(&[0; 4096]);

        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        // The file has room for more chunks than it has used, and those are zeros.
        let offsets: Vec<u64> = (0..chunks.len() / EVTX_CHUNK_SIZE)
            .filter(|i| chunks[i * EVTX_CHUNK_SIZE..].starts_with(CHUNK_MAGIC))
            .map(|i| chunks_at + (i * EVTX_CHUNK_SIZE) as u64)
            .collect();
        assert_eq!(offsets.len(), 26);
        assert_eq!(carved.chunk_offsets(), offsets);

        let mut parser = EvtxParser::from_read_seek(carved)
            .unwrap()
            .with_configuration(ParserSettings::new().validate_checksums(true));
        assert_eq!(record_ids(&mut parser), expected);
    }

    #[test]
    fn a_chunk_cut_short_by_the_end_of_the_source_still_parses() {
        let log = include_bytes!("../samples/security.evtx");
        let first = &log[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE];
        let chunk = EvtxChunkHeader::from_bytes(first).unwrap();
        // Cut right after its last record.
        let image = first[..chunk.free_space_offset as usize].to_vec();

        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        assert_eq!(carved.chunk_offsets(), [0]);
        let mut parser = EvtxParser::from_read_seek(carved).unwrap();
        let ids = record_ids(&mut parser);
        assert_eq!(
            (ids.first().copied(), ids.last().copied()),
            (
                Some(chunk.first_event_record_id),
                Some(chunk.last_event_record_id)
            )
        );
    }

    /// The IDs of the records of each chunk of `carved`, which must all parse.
    fn ids_by_chunk(carved: CarvedEvtx<Cursor<Vec<u8>>>, recover: bool) -> Vec<Vec<u64>> {
        let settings = ParserSettings::new().recover_records(recover);
        let mut parser = EvtxParser::from_read_seek(carved)
            .unwrap()
            .with_configuration(settings.clone());
        let settings = std::sync::Arc::new(settings);
        parser
            .chunks()
            .map(|chunk| {
                let mut chunk = chunk.unwrap();
                let mut chunk = chunk.parse(settings.clone()).unwrap();
                let ids = chunk.iter().map(|record| record.unwrap().event_record_id);
                ids.collect()
            })
            .collect()
    }

    #[test]
    fn a_chunk_cut_short_by_another_chunk_does_not_hide_it() {
        let log = include_bytes!("../samples/security.evtx");
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        // The first 4 KiB of the first chunk (the rest was overwritten), then the second chunk.
        let mut image = chunks[..4096].to_vec();
        image.extend_from_slice(&chunks[EVTX_CHUNK_SIZE..2 * EVTX_CHUNK_SIZE]);

        let carved = CarvedEvtx::new(Cursor::new(image.clone())).unwrap();
        assert_eq!(carved.chunk_offsets(), [0, 4096]);
        // Records 1 and 2 are whole in the first 4 KiB. The first chunk doesn't read the
        // second one's bytes as its own: its string table would point into them.
        for recover in [false, true] {
            let carved = CarvedEvtx::new(Cursor::new(image.clone())).unwrap();
            let ids = ids_by_chunk(carved, recover);
            assert_eq!(ids, [vec![1, 2], (92..=177).collect()], "{recover}");
        }
    }

    #[test]
    fn a_chunk_cut_short_by_a_chunk_of_another_log_keeps_to_its_records() {
        let security = include_bytes!("../samples/security.evtx");
        let system = include_bytes!("../samples/system.evtx");
        // Half of security's first chunk (records 1 to 91), then system's (records 1 to 108),
        // whose IDs the first chunk's walk would take for its own.
        let mut image = security[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 32768].to_vec();
        image.extend_from_slice(&system[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 65536]);

        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        assert_eq!(carved.chunk_offsets(), [0, 32768]);
        // Records 1 to 42 are whole in the first 32 KiB.
        let ids = ids_by_chunk(carved, true);
        assert_eq!(ids, [(1..=42).collect::<Vec<_>>(), (1..=108).collect()]);
    }

    #[test]
    fn a_record_cut_short_by_the_end_of_the_source_is_left_out() {
        let log = include_bytes!("../samples/security.evtx");
        // Records 1 to 9 are whole in the first 8 KiB, and record 10 starts there: the zeros
        // after the cut would read as its values.
        let image = log[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 8192].to_vec();
        for recover in [false, true] {
            let carved = CarvedEvtx::new(Cursor::new(image.clone())).unwrap();
            assert_eq!(ids_by_chunk(carved, recover), [(1..=9).collect::<Vec<_>>()]);
        }
    }

    #[test]
    fn a_chunk_without_crc32s_does_not_hide_the_chunk_that_cut_it_short() {
        // Its header says that its records run to 22400, and it has no CRC32 to show that they
        // are intact.
        let no_crc32 = include_bytes!("../samples/Application_no_crc32.evtx");
        let security = include_bytes!("../samples/security.evtx");
        let mut image = no_crc32[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 8192].to_vec();
        image.extend_from_slice(&security[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 65536]);

        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        assert_eq!(carved.chunk_offsets(), [0, 8192]);
    }

    #[test]
    fn a_chunk_header_cut_short_is_not_an_error() {
        let log = include_bytes!("../samples/security.evtx");
        let image = log[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 300].to_vec();
        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        assert!(carved.chunk_offsets().is_empty());
    }

    #[test]
    fn a_source_without_chunks_is_an_empty_log() {
        let carved = CarvedEvtx::new(Cursor::new(vec![0u8; 10_000])).unwrap();
        assert!(carved.chunk_offsets().is_empty());
        let mut parser = EvtxParser::from_read_seek(carved).unwrap();
        assert!(record_ids(&mut parser).is_empty());
    }
}
