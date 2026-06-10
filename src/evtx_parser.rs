use crate::err::{ChunkError, EvtxError, InputError, Result};

use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::SerializedEvtxRecord;
use bumpalo::Bump;

use log::trace;
#[cfg(not(feature = "multithreading"))]
use log::warn;

use log::{debug, info};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::EvtxRecord;
use encoding::EncodingRef;
use encoding::all::WINDOWS_1252;
#[cfg(feature = "multithreading")]
use std::cmp::max;
use std::fmt;
use std::fmt::Debug;
use std::iter::Iterator;
use std::path::Path;
use std::sync::Arc;

pub const EVTX_CHUNK_SIZE: usize = 65536;
pub const EVTX_FILE_HEADER_SIZE: usize = 4096;

/// One processed chunk's payload plus its (reusable) bump arena.
struct ChunkBatch<P> {
    payload: P,
    arena: Bump,
}

/// Parse one chunk and serialize all of its records with `f`.
fn process_chunk<U>(
    chunk_res: Result<EvtxChunkData>,
    chunk_id: u64,
    settings: Arc<ParserSettings>,
    arena: Bump,
    f: impl FnMut(Result<EvtxRecord<'_>>) -> Result<U>,
) -> ChunkBatch<Vec<Result<U>>> {
    match chunk_res {
        Err(err) => ChunkBatch {
            payload: vec![Err(err)],
            arena,
        },
        Ok(mut chunk) => match chunk.parse_with_arena(settings, arena) {
            Err(err) => ChunkBatch {
                payload: vec![Err(EvtxError::FailedToParseChunk {
                    chunk_id,
                    source: Box::new(err),
                })],
                arena: Bump::new(),
            },
            Ok(mut chunk_records) => {
                let payload = chunk_records.iter().map(f).collect();
                let arena = chunk_records.into_arena();
                ChunkBatch { payload, arena }
            }
        },
    }
}

/// One record inside a [`RenderedChunk`]: either the half-open end offset of
/// its bytes in `RenderedChunk::data`, or the error it produced.
#[doc(hidden)]
#[derive(Debug)]
pub enum RenderedChunkItem {
    Record { event_record_id: u64, end: usize },
    Failed(EvtxError),
}

/// CLI-internal: a whole chunk's records rendered into one output buffer
/// (each record's bytes end with `\n`), with per-record items in record order.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct RenderedChunk {
    pub data: Vec<u8>,
    pub items: Vec<RenderedChunkItem>,
}

#[derive(Clone, Copy)]
enum RenderFormat {
    Xml,
    Json,
}

/// Parse one chunk and render all of its records into a single buffer.
fn render_chunk(
    chunk_res: Result<EvtxChunkData>,
    chunk_id: u64,
    settings: Arc<ParserSettings>,
    arena: Bump,
    format: RenderFormat,
    record_numbers: bool,
) -> ChunkBatch<RenderedChunk> {
    use crate::binxml::ir_json::render_json_record_content;
    use crate::binxml::ir_xml::render_xml_record_content;

    match chunk_res {
        Err(err) => ChunkBatch {
            payload: RenderedChunk {
                data: Vec::new(),
                items: vec![RenderedChunkItem::Failed(err)],
            },
            arena,
        },
        Ok(mut chunk) => match chunk.parse_with_arena(settings.clone(), arena) {
            Err(err) => ChunkBatch {
                payload: RenderedChunk {
                    data: Vec::new(),
                    items: vec![RenderedChunkItem::Failed(EvtxError::FailedToParseChunk {
                        chunk_id,
                        source: Box::new(err),
                    })],
                },
                arena: Bump::new(),
            },
            Ok(mut chunk_records) => {
                let mut data = Vec::with_capacity(2 * EVTX_CHUNK_SIZE);
                let mut items = Vec::new();
                for record in chunk_records.iter() {
                    match record {
                        Err(err) => items.push(RenderedChunkItem::Failed(err)),
                        Ok(record) => {
                            let start = data.len();
                            let event_record_id = record.event_record_id;
                            if record_numbers {
                                use std::io::Write;
                                // Matches the CLI's legacy `Record N` banner; baked in
                                // here so a chunk stays a single consumer-side write.
                                let _ = writeln!(&mut data, "Record {}", event_record_id);
                            }
                            let rendered = match format {
                                RenderFormat::Xml => {
                                    render_xml_record_content(&record.content, &settings, &mut data)
                                }
                                RenderFormat::Json => render_json_record_content(
                                    &record.content,
                                    &settings,
                                    &mut data,
                                ),
                            };
                            match rendered {
                                Ok(()) => {
                                    data.push(b'\n');
                                    items.push(RenderedChunkItem::Record {
                                        event_record_id,
                                        end: data.len(),
                                    });
                                }
                                Err(err) => {
                                    data.truncate(start);
                                    // Match the error shape of `into_xml_bytes`/`into_json_bytes`.
                                    items.push(RenderedChunkItem::Failed(
                                        EvtxError::FailedToParseRecord {
                                            record_id: event_record_id,
                                            source: Box::new(err),
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }
                let arena = chunk_records.into_arena();
                ChunkBatch {
                    payload: RenderedChunk { data, items },
                    arena,
                }
            }
        },
    }
}

// Stable shim until https://github.com/rust-lang/rust/issues/59359 is merged.
// Taken from proposed std code.
pub trait ReadSeek: Read + Seek {
    fn tell(&mut self) -> io::Result<u64> {
        self.stream_position()
    }
    fn stream_len(&mut self) -> io::Result<u64> {
        let old_pos = self.tell()?;
        let len = self.seek(SeekFrom::End(0))?;

        // Avoid seeking a third time when we were already at the end of the
        // stream. The branch is usually way cheaper than a seek operation.
        if old_pos != len {
            self.seek(SeekFrom::Start(old_pos))?;
        }

        Ok(len)
    }
}

impl<T: Read + Seek> ReadSeek for T {}

/// Wraps a single `EvtxFileHeader`.
///
///
/// Example usage (single threaded):
///
/// ```rust
/// # use evtx::EvtxParser;
/// # let fp = std::path::PathBuf::from(format!("{}/samples/security.evtx", std::env::var("CARGO_MANIFEST_DIR").unwrap()));
///
///
/// let mut parser = EvtxParser::from_path(fp).unwrap();
///
/// for record in parser.records() {
///     match record {
///         Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
///         Err(e) => eprintln!("{}", e),
///     }
/// }
///
///
/// ```
/// Example usage (multi-threaded):
///
/// ```rust
/// # use evtx::{EvtxParser, ParserSettings};
/// # let fp = std::path::PathBuf::from(format!("{}/samples/security.evtx", std::env::var("CARGO_MANIFEST_DIR").unwrap()));
///
///
/// let settings = ParserSettings::default().num_threads(0);
/// let mut parser = EvtxParser::from_path(fp).unwrap().with_configuration(settings);
///
/// for record in parser.records() {
///     match record {
///         Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
///         Err(e) => eprintln!("{}", e),
///     }
/// }
///
/// ```
///
pub struct EvtxParser<T: ReadSeek> {
    data: T,
    header: EvtxFileHeader,
    config: Arc<ParserSettings>,
    /// The calculated_chunk_count is the: (<file size> - <header size>) / <chunk size>
    /// This is needed because the chunk count of an EVTX file can be larger than the u16
    /// value stored in the file header.
    calculated_chunk_count: u64,
}
impl<T: ReadSeek> Debug for EvtxParser<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> ::std::fmt::Result {
        f.debug_struct("EvtxParser")
            .field("header", &self.header)
            .field("config", &self.config)
            .finish()
    }
}

#[derive(Clone)]
pub struct ParserSettings {
    /// Controls the number of threads used for parsing chunks concurrently.
    num_threads: usize,
    /// If enabled, chunk with bad checksums will be skipped.
    validate_checksums: bool,
    /// If enabled, XML attributes will be separated in JSON
    /// into a separate field. Example:
    /// {
    ///   "EventID": {
    ///     "#attributes": {
    ///       "Qualifiers": 16384
    ///     },
    ///     "#text": 4111
    ///   }
    /// }
    ///
    /// Becomes:
    /// {
    ///   "EventID": 4111,
    ///   "EventID_attributes": {
    ///     "Qualifiers": 16384
    ///   }
    /// }
    separate_json_attributes: bool,
    /// If true, output will be indented.
    indent: bool,
    /// Controls the ansi codec used to deserialize ansi strings inside the xml document.
    ansi_codec: EncodingRef,
    /// Optional offline WEVT template cache used as a fallback when embedded EVTX templates
    /// are missing/corrupt (common in carved/dirty logs).
    #[cfg(feature = "wevt_templates")]
    wevt_cache: Option<Arc<crate::wevt_templates::WevtCache>>,
}

impl Debug for ParserSettings {
    fn fmt(&self, f: &mut fmt::Formatter) -> ::std::fmt::Result {
        let mut ds = f.debug_struct("ParserSettings");
        ds.field("num_threads", &self.num_threads)
            .field("validate_checksums", &self.validate_checksums)
            .field("separate_json_attributes", &self.separate_json_attributes)
            .field("indent", &self.indent)
            .field("ansi_codec", &self.ansi_codec.name());

        #[cfg(feature = "wevt_templates")]
        ds.field("wevt_cache", &self.wevt_cache.is_some());

        ds.finish()
    }
}

impl PartialEq for ParserSettings {
    fn eq(&self, other: &ParserSettings) -> bool {
        self.ansi_codec.name() == other.ansi_codec.name()
            && self.num_threads == other.num_threads
            && self.validate_checksums == other.validate_checksums
            && self.separate_json_attributes == other.separate_json_attributes
            && self.indent == other.indent
    }
}

impl Default for ParserSettings {
    fn default() -> Self {
        ParserSettings {
            num_threads: 0,
            validate_checksums: false,
            separate_json_attributes: false,
            indent: true,
            ansi_codec: WINDOWS_1252,
            #[cfg(feature = "wevt_templates")]
            wevt_cache: None,
        }
    }
}

impl ParserSettings {
    pub fn new() -> Self {
        ParserSettings::default()
    }

    /// Sets the number of worker threads.
    /// `0` will let rayon decide.
    ///
    #[cfg(feature = "multithreading")]
    pub fn num_threads(mut self, num_threads: usize) -> Self {
        self.num_threads = if num_threads == 0 {
            rayon::current_num_threads()
        } else {
            num_threads
        };
        self
    }

    /// Does nothing and emits a warning when complied without multithreading.
    #[cfg(not(feature = "multithreading"))]
    pub fn num_threads(mut self, _num_threads: usize) -> Self {
        warn!("Setting num_threads has no effect when compiling without multithreading support.");

        self.num_threads = 1;
        self
    }

    /// Sets the ansi codec used by the parser.
    pub fn ansi_codec(mut self, ansi_codec: EncodingRef) -> Self {
        self.ansi_codec = ansi_codec;

        self
    }

    /// Attach an offline WEVT template cache used as a fallback during parsing.
    #[cfg(feature = "wevt_templates")]
    pub fn wevt_cache(mut self, cache: Option<Arc<crate::wevt_templates::WevtCache>>) -> Self {
        self.wevt_cache = cache;
        self
    }

    pub fn validate_checksums(mut self, validate_checksums: bool) -> Self {
        self.validate_checksums = validate_checksums;

        self
    }

    pub fn separate_json_attributes(mut self, separate: bool) -> Self {
        self.separate_json_attributes = separate;

        self
    }

    pub fn indent(mut self, pretty: bool) -> Self {
        self.indent = pretty;

        self
    }

    /// Gets the current ansi codec
    pub fn get_ansi_codec(&self) -> EncodingRef {
        self.ansi_codec
    }

    #[cfg(feature = "wevt_templates")]
    pub(crate) fn get_wevt_cache(&self) -> Option<&Arc<crate::wevt_templates::WevtCache>> {
        self.wevt_cache.as_ref()
    }

    pub fn should_separate_json_attributes(&self) -> bool {
        self.separate_json_attributes
    }

    pub fn should_indent(&self) -> bool {
        self.indent
    }

    pub fn should_validate_checksums(&self) -> bool {
        self.validate_checksums
    }

    pub fn get_num_threads(&self) -> &usize {
        &self.num_threads
    }
}

impl EvtxParser<File> {
    /// Attempts to load an evtx file from a given path, will fail if the path does not exist,
    /// or if evtx header is invalid.
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let path = path
            .as_ref()
            .canonicalize()
            .map_err(|e| InputError::failed_to_open_file(e, &path))?;

        let f = File::open(&path).map_err(|e| InputError::failed_to_open_file(e, &path))?;

        let cursor = f;
        Self::from_read_seek(cursor)
    }
}

impl EvtxParser<Cursor<Vec<u8>>> {
    /// Attempts to load an evtx file from a given path, will fail the evtx header is invalid.
    pub fn from_buffer(buffer: Vec<u8>) -> Result<Self> {
        let cursor = Cursor::new(buffer);
        Self::from_read_seek(cursor)
    }
}

impl<T: ReadSeek> EvtxParser<T> {
    pub fn from_read_seek(mut read_seek: T) -> Result<Self> {
        let evtx_header = EvtxFileHeader::from_stream(&mut read_seek)?;

        // Because an event log can be larger than u16 MAX * EVTX_CHUNK_SIZE,
        // We need to calculate the chunk count instead of using the header value
        // this allows us to continue parsing events past the 4294901760 bytes of
        // chunk data
        let stream_size = ReadSeek::stream_len(&mut read_seek)?;
        let chunk_data_size: u64 =
            match stream_size.checked_sub(evtx_header.header_block_size.into()) {
                Some(c) => c,
                None => {
                    return Err(EvtxError::calculation_error(format!(
                        "Could not calculate valid chunk count because stream size is less \
                            than evtx header block size. (stream_size: {}, header_block_size: {})",
                        stream_size, evtx_header.header_block_size
                    )));
                }
            };
        let chunk_count = chunk_data_size / EVTX_CHUNK_SIZE as u64;

        debug!("EVTX Header: {:#?}", evtx_header);
        Ok(EvtxParser {
            data: read_seek,
            header: evtx_header,
            config: Arc::new(ParserSettings::default()),
            calculated_chunk_count: chunk_count,
        })
    }

    pub fn with_configuration(mut self, configuration: ParserSettings) -> Self {
        self.config = Arc::new(configuration);
        self
    }

    /// Allocate a new chunk from the given data, at the offset expected by `chunk_number`.
    /// If the read chunk contains valid data, an `Ok(Some(EvtxChunkData))` will be returned.
    /// If the read chunk contains invalid data (bad magic, bad checksum when `validate_checksum` is set to true),
    /// of if not enough data can be read (e.g. because we reached EOF), an `Err` is returned.
    /// If the read chunk is empty, `Ok(None)` will be returned.
    fn allocate_chunk(
        data: &mut T,
        chunk_number: u64,
        validate_checksum: bool,
    ) -> Result<Option<EvtxChunkData>> {
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        let chunk_offset = EVTX_FILE_HEADER_SIZE + chunk_number as usize * EVTX_CHUNK_SIZE;

        trace!(
            "Offset `0x{:08x} ({})` - Reading chunk number `{}`",
            chunk_offset, chunk_offset, chunk_number
        );

        data.seek(SeekFrom::Start(chunk_offset as u64))
            .map_err(|e| EvtxError::FailedToParseChunk {
                chunk_id: chunk_number,
                source: Box::new(ChunkError::FailedToSeekToChunk(e)),
            })?;

        let amount_read = data
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .map_err(|_| EvtxError::incomplete_chunk(chunk_number))?;

        if amount_read != EVTX_CHUNK_SIZE {
            return Err(EvtxError::incomplete_chunk(chunk_number));
        }

        // There might be empty chunks in the middle of a dirty file.
        if chunk_data.iter().all(|x| *x == 0) {
            return Ok(None);
        }

        EvtxChunkData::new(chunk_data, validate_checksum)
            .map(Some)
            .map_err(|e| EvtxError::FailedToParseChunk {
                chunk_id: chunk_number,
                source: Box::new(e),
            })
    }

    /// Find the next chunk, staring at `chunk_number` (inclusive).
    /// If a chunk is found, returns the data of the chunk or the relevant error,
    /// and the number of that chunk.
    pub fn find_next_chunk(
        &mut self,
        mut chunk_number: u64,
    ) -> Option<(Result<EvtxChunkData>, u64)> {
        loop {
            match EvtxParser::allocate_chunk(
                &mut self.data,
                chunk_number,
                self.config.validate_checksums,
            ) {
                Err(err) => {
                    // We try to read past the `chunk_count` to allow for dirty files.
                    // But if we failed, it means we really are at the end of the file.
                    if chunk_number >= self.calculated_chunk_count {
                        return None;
                    } else {
                        return Some((Err(err), chunk_number));
                    }
                }
                Ok(None) => {
                    // We try to read past the `chunk_count` to allow for dirty files.
                    // But if we get an empty chunk, we need to keep looking.
                    // Increment and try again.
                    chunk_number = chunk_number.checked_add(1)?
                }
                Ok(Some(chunk)) => {
                    return Some((Ok(chunk), chunk_number));
                }
            };
        }
    }

    /// Return an iterator over all the chunks.
    /// Each chunk supports iterating over it's records in their un-serialized state
    /// (before they are converted to XML or JSON).
    pub fn chunks(&mut self) -> IterChunks<'_, T> {
        IterChunks {
            parser: self,
            current_chunk_number: 0,
        }
    }

    /// Consumes the parser, returning an iterator over all the chunks.
    /// Each chunk supports iterating over it's records in their un-serialized state
    /// (before they are converted to XML or JSON).
    pub fn into_chunks(self) -> IntoIterChunks<T> {
        IntoIterChunks {
            parser: self,
            current_chunk_number: 0,
        }
    }
    /// Process chunks through `work` and yield one payload per chunk, in chunk order.
    ///
    /// With the `multithreading` feature, chunks are processed on the rayon pool as a
    /// bounded streaming pipeline: up to `2 * num_threads` chunks are in flight while
    /// already-completed chunks are drained by the caller, so workers never idle behind
    /// the (serial) consumer.
    #[cfg(feature = "multithreading")]
    fn chunk_pipeline<'a, P: Send + 'static>(
        &'a mut self,
        work: impl Fn(Result<EvtxChunkData>, u64, Arc<ParserSettings>, Bump) -> ChunkBatch<P>
        + Send
        + Sync
        + Clone
        + 'static,
    ) -> impl Iterator<Item = P> + 'a {
        // Retrieve parser settings here, while `self` is immutably borrowed.
        let num_threads = max(self.config.num_threads, 1);
        let chunk_settings = Arc::clone(&self.config);

        let max_in_flight = num_threads * 2;

        // `self` is mutably borrowed from here on.
        let mut chunks = self.chunks();
        let mut arena_pool: Vec<Bump> = (0..max_in_flight)
            .map(|_| Bump::with_capacity(EVTX_CHUNK_SIZE))
            .collect();

        // `thread::Result` so a panicking `work` propagates to the consumer thread
        // (matching the old `par_iter().collect()` behavior) instead of hitting
        // rayon's global handler, which aborts the process by default.
        let (tx, rx) = std::sync::mpsc::channel::<(u64, std::thread::Result<ChunkBatch<P>>)>();
        let mut reorder: std::collections::BTreeMap<u64, std::thread::Result<ChunkBatch<P>>> =
            std::collections::BTreeMap::new();
        let mut next_seq = 0u64;
        let mut next_yield = 0u64;
        let mut in_flight = 0usize;
        let mut exhausted = false;

        std::iter::from_fn(move || {
            if num_threads == 1 {
                // Single-threaded: process inline; a cross-thread round-trip per
                // chunk costs ~3% wall time for no benefit.
                let chunk_res = chunks.next()?;
                let arena = arena_pool.pop().unwrap_or_default();
                let batch = work(chunk_res, next_seq, Arc::clone(&chunk_settings), arena);
                next_seq += 1;
                arena_pool.push(batch.arena);
                return Some(batch.payload);
            }
            loop {
                // Keep the pipeline full. Chunk allocation (I/O) stays on the
                // consumer thread; parsing + serialization go to the rayon pool.
                while !exhausted && in_flight < max_in_flight {
                    let Some(chunk_res) = chunks.next() else {
                        exhausted = true;
                        break;
                    };
                    let arena = arena_pool.pop().unwrap_or_default();
                    let seq = next_seq;
                    next_seq += 1;
                    in_flight += 1;
                    let tx = tx.clone();
                    let work = work.clone();
                    let settings = Arc::clone(&chunk_settings);
                    rayon::spawn(move || {
                        let batch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            work(chunk_res, seq, settings, arena)
                        }));
                        // The receiver may be gone if the iterator was dropped.
                        let _ = tx.send((seq, batch));
                    });
                }

                if let Some(batch) = reorder.remove(&next_yield) {
                    next_yield += 1;
                    in_flight -= 1;
                    let batch = match batch {
                        Ok(batch) => batch,
                        Err(panic) => std::panic::resume_unwind(panic),
                    };
                    arena_pool.push(batch.arena);
                    return Some(batch.payload);
                }

                if in_flight == 0 && exhausted {
                    return None;
                }

                match rx.recv() {
                    Ok((seq, batch)) => {
                        reorder.insert(seq, batch);
                    }
                    Err(_) => return None,
                }
            }
        })
    }

    /// Process chunks through `work` and yield one payload per chunk, in chunk order.
    #[cfg(not(feature = "multithreading"))]
    fn chunk_pipeline<'a, P: Send + 'static>(
        &'a mut self,
        work: impl Fn(Result<EvtxChunkData>, u64, Arc<ParserSettings>, Bump) -> ChunkBatch<P>
        + Send
        + Sync
        + Clone
        + 'static,
    ) -> impl Iterator<Item = P> + 'a {
        let chunk_settings = Arc::clone(&self.config);
        let mut chunks = self.chunks();
        let mut arena_pool: Vec<Bump> = vec![Bump::with_capacity(EVTX_CHUNK_SIZE)];
        let mut seq = 0u64;

        std::iter::from_fn(move || {
            let chunk_res = chunks.next()?;
            let arena = arena_pool.pop().unwrap_or_default();
            let batch = work(chunk_res, seq, Arc::clone(&chunk_settings), arena);
            seq += 1;
            arena_pool.push(batch.arena);
            Some(batch.payload)
        })
    }

    /// Return an iterator over all the records.
    /// Records will be mapped `f`, which must produce owned data from the records.
    pub fn serialized_records<'a, U: Send + 'static>(
        &'a mut self,
        f: impl FnMut(Result<EvtxRecord<'_>>) -> Result<U> + Send + Sync + Clone + 'static,
    ) -> impl Iterator<Item = Result<U>> + 'a {
        self.chunk_pipeline(move |chunk_res, chunk_id, settings, arena| {
            process_chunk(chunk_res, chunk_id, settings, arena, f.clone())
        })
        .flatten()
    }

    /// CLI-internal: render every record into one buffer per chunk (single write
    /// per chunk on the consumer side). Not part of the supported public API.
    #[doc(hidden)]
    pub fn chunks_xml_bytes(
        &mut self,
        record_numbers: bool,
    ) -> impl Iterator<Item = RenderedChunk> + '_ {
        self.chunk_pipeline(move |chunk_res, chunk_id, settings, arena| {
            render_chunk(
                chunk_res,
                chunk_id,
                settings,
                arena,
                RenderFormat::Xml,
                record_numbers,
            )
        })
    }

    /// CLI-internal: see [`Self::chunks_xml_bytes`].
    #[doc(hidden)]
    pub fn chunks_json_bytes(
        &mut self,
        record_numbers: bool,
    ) -> impl Iterator<Item = RenderedChunk> + '_ {
        self.chunk_pipeline(move |chunk_res, chunk_id, settings, arena| {
            render_chunk(
                chunk_res,
                chunk_id,
                settings,
                arena,
                RenderFormat::Json,
                record_numbers,
            )
        })
    }

    /// Return an iterator over all the records.
    /// Records will be XML-formatted.
    pub fn records(&mut self) -> impl Iterator<Item = Result<SerializedEvtxRecord<String>>> + '_ {
        // '_ is required in the signature because the iterator is bound to &self.
        self.serialized_records(|record| record.and_then(|record| record.into_xml()))
    }

    /// Return an iterator over all the records as rendered XML bytes (skips UTF-8 validation).
    pub fn records_bytes(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord<Vec<u8>>>> + '_ {
        self.serialized_records(|record| record.and_then(|record| record.into_xml_bytes()))
    }

    /// Return an iterator over all the records as rendered JSON bytes (skips UTF-8 validation).
    pub fn records_json_bytes(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord<Vec<u8>>>> + '_ {
        self.serialized_records(|record| record.and_then(|record| record.into_json_bytes()))
    }

    /// Return an iterator over all the records.
    /// Records will be JSON-formatted.
    pub fn records_json(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord<String>>> + '_ {
        self.serialized_records(|record| record.and_then(|record| record.into_json()))
    }

    /// Return an iterator over all the records.
    /// Records will have a `serde_json::Value` data attribute.
    pub fn records_json_value(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord<serde_json::Value>>> + '_ {
        self.serialized_records(|record| record.and_then(|record| record.into_json_value()))
    }
}

pub struct IterChunks<'c, T: ReadSeek> {
    parser: &'c mut EvtxParser<T>,
    current_chunk_number: u64,
}

impl<T: ReadSeek> Iterator for IterChunks<'_, T> {
    type Item = Result<EvtxChunkData>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        match self.parser.find_next_chunk(self.current_chunk_number) {
            None => None,
            Some((chunk, chunk_number)) => {
                self.current_chunk_number = chunk_number.checked_add(1)?;

                Some(chunk)
            }
        }
    }
}

pub struct IntoIterChunks<T: ReadSeek> {
    parser: EvtxParser<T>,
    current_chunk_number: u64,
}

impl<T: ReadSeek> Iterator for IntoIterChunks<T> {
    type Item = Result<EvtxChunkData>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        info!("Chunk {}", self.current_chunk_number);
        match self.parser.find_next_chunk(self.current_chunk_number) {
            None => None,
            Some((chunk, chunk_number)) => {
                self.current_chunk_number = chunk_number.checked_add(1)?;

                Some(chunk)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_variables)]

    use super::*;
    use crate::ensure_env_logger_initialized;

    fn process_90_records(buffer: &'static [u8]) -> crate::err::Result<()> {
        let mut parser = EvtxParser::from_buffer(buffer.to_vec())?;

        for (i, record) in parser.records().take(90).enumerate() {
            let r = record?;
            assert_eq!(r.event_record_id, i as u64 + 1);
        }

        Ok(())
    }

    // For clion profiler
    #[test]
    fn test_process_single_chunk() -> crate::err::Result<()> {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        process_90_records(evtx_file)?;

        Ok(())
    }

    #[test]
    fn test_sample_2() {
        let evtx_file = include_bytes!("../samples/system.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let records: Vec<_> = parser.records().take(10).collect();

        for (i, record) in records.iter().enumerate() {
            match record {
                Ok(r) => {
                    assert_eq!(
                        r.event_record_id,
                        i as u64 + 1,
                        "Parser is skipping records!"
                    );
                }
                Err(e) => panic!("Error while reading record {}, {:?}", i, e),
            }
        }

        // It should be empty, and not a [].
        assert!(
            records[0]
                .as_ref()
                .unwrap()
                .data
                .contains("<Binary></Binary>")
        );
        assert!(
            records[1]
                .as_ref()
                .unwrap()
                .data
                .contains("<Binary>E107070003000C00110010001C00D6000000000000000000</Binary>")
        );
    }

    #[test]
    fn test_parses_first_10_records() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        for (i, record) in parser.records().take(10).enumerate() {
            match record {
                Ok(r) => {
                    assert_eq!(
                        r.event_record_id,
                        i as u64 + 1,
                        "Parser is skipping records!"
                    );
                }
                Err(e) => panic!("Error while reading record {}, {:?}", i, e),
            }
        }
    }

    #[test]
    fn test_parses_records_from_different_chunks() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        for (i, record) in parser.records().take(1000).enumerate() {
            match record {
                Ok(r) => {
                    assert_eq!(r.event_record_id, i as u64 + 1);
                }
                Err(e) => println!("Error while reading record {}, {:?}", i, e),
            }
        }
    }

    #[test]
    #[cfg(feature = "multithreading")]
    fn test_multithreading() {
        use std::collections::HashSet;

        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut record_ids = HashSet::new();
        for record in parser.records().take(1000) {
            match record {
                Ok(r) => {
                    record_ids.insert(r.event_record_id);
                }
                Err(e) => panic!("Error while reading record {:?}", e),
            }
        }

        assert_eq!(record_ids.len(), 1000);
    }

    #[test]
    fn test_file_with_only_a_single_chunk() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/new-user-security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        assert_eq!(parser.records().count(), 4);
    }

    #[test]
    fn test_parses_chunk2() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");

        let mut chunk = EvtxChunkData::new(
            evtx_file[EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE
                ..EVTX_FILE_HEADER_SIZE + 2 * EVTX_CHUNK_SIZE]
                .to_vec(),
            false,
        )
        .unwrap();

        assert!(chunk.validate_checksum());

        for record in chunk
            .parse(Arc::new(ParserSettings::default()))
            .unwrap()
            .iter()
        {
            record.unwrap();
        }
    }

    #[test]
    fn test_into_chunks() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/new-user-security.evtx");
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        assert_eq!(parser.into_chunks().count(), 1);
    }

    #[test]
    fn test_into_json_value_records() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/new-user-security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let records: Vec<_> = parser.records_json_value().collect();

        for record in records {
            let record = record.unwrap();

            assert!(record.data.is_object());
            assert!(record.data.as_object().unwrap().contains_key("Event"));
        }
    }

    #[test]
    fn test_parse_event_with_zero_() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/new-user-security.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let records: Vec<_> = parser.records_json_value().collect();

        for record in records {
            let record = record.unwrap();

            assert!(record.data.is_object());
            assert!(record.data.as_object().unwrap().contains_key("Event"));
        }
    }
}
