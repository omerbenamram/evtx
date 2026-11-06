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

use crate::checksum_ieee;
use crate::evtx_chunk::{ChunkFlags, EVTX_CHUNK_HEADER_SIZE, EvtxChunkHeader, chunk_header_crc};
use crate::evtx_parser::{EVTX_CHUNK_SIZE, EVTX_FILE_HEADER_SIZE};
use crate::evtx_record::EVTX_RECORD_MAGIC;

const CHUNK_MAGIC: &[u8; 8] = b"ElfChnk\0";
/// How much of the source is searched at once.
const SCAN_BLOCK_SIZE: usize = 8 << 20;

/// The chunks carved from a source, read as a virtual EVTX file (see the [module docs](self)).
///
/// It reads each chunk from the source when the parser asks for it, so the source is only held
/// open, not copied. A chunk that runs past the end of the source reads as zeros there.
#[derive(Debug)]
pub struct CarvedEvtx<R> {
    source: R,
    source_len: u64,
    offsets: Vec<u64>,
    header: Vec<u8>,
    position: u64,
}

impl<R: Read + Seek> CarvedEvtx<R> {
    /// Scans all of `source` for chunks.
    pub fn new(mut source: R) -> io::Result<Self> {
        let source_len = source.seek(SeekFrom::End(0))?;
        let (offsets, next_record_id) = find_chunks(&mut source, source_len)?;
        let header = file_header(offsets.len(), next_record_id);
        Ok(CarvedEvtx {
            source,
            source_len,
            offsets,
            header,
            position: 0,
        })
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
            let available = self.source_len.saturating_sub(at).min(n as u64) as usize;
            self.source.seek(SeekFrom::Start(at))?;
            self.source.read_exact(&mut buf[..available])?;
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

/// The offsets of the chunks in `source`, in the order found, without exact copies, and the
/// record ID after the highest one they hold.
fn find_chunks<R: Read + Seek>(source: &mut R, source_len: u64) -> io::Result<(Vec<u64>, u64)> {
    let finder = memchr::memmem::Finder::new(CHUNK_MAGIC);
    let mut offsets = Vec::new();
    let mut next_record_id = 1u64;
    let mut seen = HashSet::new();
    let mut block = vec![0u8; SCAN_BLOCK_SIZE];
    let mut chunk = vec![0u8; EVTX_CHUNK_SIZE];
    // Where the next block starts, and where the records of the last chunk carved end, when
    // its events CRC32 shows that they are intact.
    let mut block_start = 0u64;
    let mut records_end = 0u64;
    // Enough of a candidate to tell whether it is a chunk.
    let head_size = EVTX_CHUNK_HEADER_SIZE + EVTX_RECORD_MAGIC.len();

    while block_start < source_len {
        let len = (source_len - block_start).min(SCAN_BLOCK_SIZE as u64) as usize;
        source.seek(SeekFrom::Start(block_start))?;
        source.read_exact(&mut block[..len])?;

        for at in finder.find_iter(&block[..len]) {
            let offset = block_start + at as u64;
            // A chunk's records could hold the magic as data. Only intact records are skipped:
            // a chunk cut short can be followed by another chunk at any offset.
            if offset < records_end {
                continue;
            }
            let available = (source_len - offset).min(EVTX_CHUNK_SIZE as u64) as usize;
            // Read the header first: most candidates are the magic by chance.
            let head = available.min(head_size);
            source.seek(SeekFrom::Start(offset))?;
            source.read_exact(&mut chunk[..head])?;
            chunk[head..head_size].fill(0);
            let Some(header) = looks_like_a_chunk(&chunk) else {
                continue;
            };
            source.read_exact(&mut chunk[head..available])?;
            chunk[available..].fill(0);
            let records = &chunk[EVTX_CHUNK_HEADER_SIZE..header.free_space_offset as usize];
            if header.flags.contains(ChunkFlags::NO_CRC32)
                || checksum_ieee(records) == header.events_checksum
            {
                records_end = offset + u64::from(header.free_space_offset);
            }
            // The same chunk in a copy of the log (a volume shadow copy, a backup) adds nothing.
            if seen.insert(chunk_identity(&chunk)) {
                offsets.push(offset);
                next_record_id = next_record_id.max(header.last_event_record_id.saturating_add(1));
            }
        }

        // The next block overlaps this one by the magic's length less one, so a magic that
        // straddles them is found.
        if block_start + (len as u64) >= source_len {
            break;
        }
        block_start += (len - (CHUNK_MAGIC.len() - 1)) as u64;
    }
    Ok((offsets, next_record_id))
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

    #[test]
    fn a_chunk_cut_short_by_another_chunk_does_not_hide_it() {
        let log = include_bytes!("../samples/security.evtx");
        let chunks = &log[EVTX_FILE_HEADER_SIZE..];
        // The first 4 KiB of the first chunk (the rest was overwritten), then the second chunk.
        let mut image = chunks[..4096].to_vec();
        image.extend_from_slice(&chunks[EVTX_CHUNK_SIZE..2 * EVTX_CHUNK_SIZE]);

        let carved = CarvedEvtx::new(Cursor::new(image)).unwrap();
        assert_eq!(carved.chunk_offsets(), [0, 4096]);
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
