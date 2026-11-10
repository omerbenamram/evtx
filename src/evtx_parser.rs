use crate::err::{ChunkError, EvtxError, InputError, Result};

use crate::AnsiCodec;
use crate::binxml::compiled::{JsonProgram, StoredProgram, XmlProgram};
use crate::evtx_chunk::{EVTX_CHUNK_HEADER_SIZE, EvtxChunk, EvtxChunkData, EvtxChunkHeader};
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::SerializedEvtxRecord;
use crate::mta::MtaFile;
use bumpalo::Bump;
use encoding_rs::WINDOWS_1252;

use log::{debug, info, trace, warn};
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};

use crate::EvtxRecord;
use std::cmp::max;
use std::fmt;
use std::fmt::Debug;
use std::iter::Iterator;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[cfg(feature = "multithreading")]
use rayon::spawn;

/// Without `multithreading`, `num_threads` is always 1, so `chunk_pipeline`
/// processes inline and never reaches this.
#[cfg(not(feature = "multithreading"))]
fn spawn(f: impl FnOnce()) {
    f()
}

pub(crate) const EVTX_CHUNK_SIZE: usize = 65536;
pub(crate) const EVTX_FILE_HEADER_SIZE: usize = 4096;

/// Where chunk `chunk_number` starts in the file, if that fits a `u64`.
fn chunk_offset(chunk_number: u64) -> Option<u64> {
    chunk_number
        .checked_mul(EVTX_CHUNK_SIZE as u64)?
        .checked_add(EVTX_FILE_HEADER_SIZE as u64)
}

/// One processed chunk's payload plus its (reusable) bump arena.
struct ChunkBatch<P> {
    payload: P,
    arena: Bump,
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

impl RenderedChunk {
    /// Each rendered record's ID and bytes, without the trailing `\n`.
    pub fn records(&self) -> impl Iterator<Item = (u64, &[u8])> {
        let mut start = 0;
        self.items.iter().filter_map(move |item| match *item {
            RenderedChunkItem::Record {
                event_record_id,
                end,
            } => {
                let record = &self.data[start..end - 1];
                start = end;
                Some((event_record_id, record))
            }
            RenderedChunkItem::Failed(_) => None,
        })
    }
}

/// Render all of a parsed chunk's records into a single buffer, in `P`'s format.
///
/// The buffer starts as big as `largest_output`, the largest output a chunk of
/// this log has rendered so far, because growing it copies it unless the
/// allocator happens to have free space right after it.
fn render_chunk<P: StoredProgram>(
    chunk: &mut EvtxChunk<'_>,
    record_numbers: bool,
    largest_output: &AtomicUsize,
) -> RenderedChunk {
    use crate::binxml::compiled::{
        ChunkBudget, MAX_RENDER_NODES, Preflight, ProgramCache, try_render_compiled,
    };
    use crate::binxml::ir::{IrTemplateCache, build_tree_from_binxml_bytes_direct};
    use crate::binxml::value_render::ValueRenderer;

    let chunk_ref: &EvtxChunk = chunk;
    let settings = &chunk_ref.settings;
    let mta = settings.get_mta();
    // At least 128 KiB, and at most 1 MiB so that one unusual chunk doesn't
    // make every later buffer that big.
    let capacity = largest_output
        .load(Ordering::Relaxed)
        .clamp(2 * EVTX_CHUNK_SIZE, 16 * EVTX_CHUNK_SIZE);
    let mut data = Vec::with_capacity(capacity);
    let mut items = Vec::new();
    let mut ir_cache =
        IrTemplateCache::with_capacity(chunk_ref.estimated_template_buckets(), &chunk_ref.arena);
    let mut progs = ProgramCache::<P>::default();
    let mut preflight = Preflight::default();
    let mut value_renderer = ValueRenderer::new();
    let budget = ChunkBudget::default();

    let mut raw_records = chunk_ref.iter_raw();
    while let Some(raw) = raw_records.next() {
        let raw = match raw {
            Ok(raw) => raw,
            Err(err) => {
                items.push(RenderedChunkItem::Failed(err));
                continue;
            }
        };
        let record_start = data.len();
        let event_record_id = raw.event_record_id;
        if record_numbers {
            use std::io::Write;
            // Matches the CLI's legacy `Record N` banner; baked in
            // here so a chunk stays a single consumer-side write.
            let suffix = if raw.recovered { " (recovered)" } else { "" };
            let _ = writeln!(&mut data, "Record {}{}", event_record_id, suffix);
        }
        let body_start = data.len();
        let rendered = budget
            .render(MAX_RENDER_NODES, &mut data, |budget, data| {
                let compiled = try_render_compiled(
                    raw.bytes,
                    event_record_id,
                    chunk_ref,
                    &mut ir_cache,
                    &mut progs,
                    &mut preflight,
                    &mut value_renderer,
                    budget,
                    data,
                )?;
                if compiled {
                    return Ok(());
                }
                // Slow lane: fully materialize, then run the same walker the
                // compiler uses — one output semantics for every record shape.
                let mut tree = build_tree_from_binxml_bytes_direct(
                    raw.bytes,
                    chunk_ref,
                    &mut ir_cache,
                    budget,
                )?;
                if let Some(mta) = mta {
                    crate::mta::add_rendering_info(
                        &mut tree,
                        event_record_id,
                        mta,
                        &chunk_ref.arena,
                    );
                }
                P::render_tree(&tree, settings, budget, data)
            })
            // Errors match the shape of `into_xml_bytes`/`into_json_bytes`.
            .map_err(|err| EvtxError::record(event_record_id, err));
        match rendered {
            Ok(()) => {
                if raw.recovered {
                    P::mark_recovered(&mut data, body_start);
                }
                data.push(b'\n');
                items.push(RenderedChunkItem::Record {
                    event_record_id,
                    end: data.len(),
                });
            }
            Err(err) => {
                data.truncate(record_start);
                if raw.recovered {
                    debug!("Skipping carved record {event_record_id}: {err:?}");
                    raw_records.rescan_after(&raw);
                } else {
                    items.push(RenderedChunkItem::Failed(err));
                }
            }
        }
    }
    largest_output.fetch_max(data.len(), Ordering::Relaxed);
    RenderedChunk { data, items }
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
    /// Controls the ANSI code page used to deserialize ANSI strings inside the xml document.
    ansi_codec: AnsiCodec,
    /// If enabled, also carve stale records from unallocated chunk space
    /// (cleared chunks and the slack past the last record).
    recover_records: bool,
    /// Optional offline WEVT template cache used as a fallback when embedded EVTX templates
    /// are missing/corrupt (common in carved/dirty logs).
    #[cfg(feature = "wevt_templates")]
    wevt_cache: Option<Arc<crate::wevt_templates::WevtCache>>,
    /// Optional `.MTA` file whose localized strings are added to records as `RenderingInfo`.
    mta: Option<Arc<MtaFile>>,
}

impl Debug for ParserSettings {
    fn fmt(&self, f: &mut fmt::Formatter) -> ::std::fmt::Result {
        let mut ds = f.debug_struct("ParserSettings");
        ds.field("num_threads", &self.num_threads)
            .field("validate_checksums", &self.validate_checksums)
            .field("separate_json_attributes", &self.separate_json_attributes)
            .field("indent", &self.indent)
            .field("ansi_codec", &self.ansi_codec.name())
            .field("recover_records", &self.recover_records);

        #[cfg(feature = "wevt_templates")]
        ds.field("wevt_cache", &self.wevt_cache.is_some());

        ds.field("mta", &self.mta.is_some());

        ds.finish()
    }
}

impl PartialEq for ParserSettings {
    fn eq(&self, other: &ParserSettings) -> bool {
        // Caches and MTA files compare by identity.
        #[cfg(feature = "wevt_templates")]
        if self.wevt_cache.as_ref().map(Arc::as_ptr) != other.wevt_cache.as_ref().map(Arc::as_ptr) {
            return false;
        }
        self.ansi_codec.name() == other.ansi_codec.name()
            && self.num_threads == other.num_threads
            && self.validate_checksums == other.validate_checksums
            && self.separate_json_attributes == other.separate_json_attributes
            && self.indent == other.indent
            && self.recover_records == other.recover_records
            && self.mta.as_ref().map(Arc::as_ptr) == other.mta.as_ref().map(Arc::as_ptr)
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
            recover_records: false,
            #[cfg(feature = "wevt_templates")]
            wevt_cache: None,
            mta: None,
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

    /// Sets the [`AnsiCodec`] used to decode ANSI strings.
    pub fn ansi_codec(mut self, ansi_codec: AnsiCodec) -> Self {
        self.ansi_codec = ansi_codec;

        self
    }

    /// Attach an offline WEVT template cache used as a fallback during parsing.
    #[cfg(feature = "wevt_templates")]
    pub fn wevt_cache(mut self, cache: Option<Arc<crate::wevt_templates::WevtCache>>) -> Self {
        self.wevt_cache = cache;
        self
    }

    /// Add the localized strings of an `.MTA` file (from the `LocaleMetaData` folder next to a
    /// log saved with display information) to every record as a `RenderingInfo` element. See
    /// [`crate::mta`].
    ///
    /// It must be the file saved with the log. An event's message is found by its record's
    /// position and `EventRecordID`, so a record that is not an event of that log (another
    /// log's, a carved one, one recovered from slack) can get another event's message.
    ///
    /// This slows rendering down, most of all through the record API ([`EvtxParser::records`]
    /// and the like), which then builds each record's tree to add the element.
    pub fn mta(mut self, mta: Option<Arc<MtaFile>>) -> Self {
        self.mta = mta;
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

    /// Also yield records carved from unallocated chunk space (cleared chunks, chunk slack).
    /// Such records have `recovered` set; the record iterators skip ones that fail to parse
    /// or render. `evtx_dump --recover-records` marks them in its output.
    ///
    /// A chunk whose header can't be read is carved too, after its error. Its records come
    /// out when the templates they use are still in the chunk.
    ///
    /// ```rust
    /// # use evtx::{EvtxParser, ParserSettings};
    /// # let fp = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("samples/security.evtx");
    /// let settings = ParserSettings::new().recover_records(true);
    /// let mut parser = EvtxParser::from_path(fp).unwrap().with_configuration(settings);
    /// for record in parser.records_json().filter_map(Result::ok) {
    ///     if record.recovered {
    ///         println!("carved record {}", record.event_record_id);
    ///     }
    /// }
    /// ```
    pub fn recover_records(mut self, recover: bool) -> Self {
        self.recover_records = recover;

        self
    }

    pub(crate) fn get_ansi_codec(&self) -> AnsiCodec {
        self.ansi_codec
    }

    #[cfg(feature = "wevt_templates")]
    pub(crate) fn get_wevt_cache(&self) -> Option<&Arc<crate::wevt_templates::WevtCache>> {
        self.wevt_cache.as_ref()
    }

    pub(crate) fn get_mta(&self) -> Option<&MtaFile> {
        self.mta.as_deref()
    }

    pub(crate) fn should_separate_json_attributes(&self) -> bool {
        self.separate_json_attributes
    }

    pub(crate) fn should_indent(&self) -> bool {
        self.indent
    }

    pub(crate) fn should_recover_records(&self) -> bool {
        self.recover_records
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
    /// Loads an evtx file from a buffer that holds all of its bytes. Fails if the buffer doesn't
    /// start with an EVTX file header.
    pub fn from_buffer(buffer: Vec<u8>) -> Result<Self> {
        let cursor = Cursor::new(buffer);
        Self::from_read_seek(cursor)
    }
}

impl<T: ReadSeek> EvtxParser<T> {
    /// Loads an evtx file from any seekable reader, such as a `File` or a `Cursor`. Fails if it
    /// doesn't start with an EVTX file header.
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
        if !evtx_header.validate_checksum() {
            warn!(
                "EVTX file header CRC32 invalid (stored {:#010x}, computed {:#010x}); reading the file anyway",
                evtx_header.checksum, evtx_header.computed_checksum
            );
        }
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

    /// The file header, as read when the parser was created.
    ///
    /// Check [`EvtxFileHeader::validate_checksum`] to see whether it is intact: the parser reads
    /// a file whose header isn't, since it doesn't need the header to find chunks.
    pub fn header(&self) -> &EvtxFileHeader {
        &self.header
    }

    /// Read the chunk at the offset expected by `chunk_number`.
    /// If it holds data, an `Ok(Some(EvtxChunkData))` is returned. That goes for a last chunk
    /// that the end of the file cuts off too, as long as its header is whole: its data is then
    /// shorter than a chunk, and the records that fit are read.
    /// If it is invalid (bad magic, bad checksum when `validate_checksum` is set to true), if
    /// it can't be read, or if the file ends before the end of its header, an `Err` is returned.
    /// With `recover_headerless`, a whole chunk with a bad magic is `Ok` instead, to be carved
    /// for records (see `EvtxChunkData::read`); its error then comes from its records iterator.
    /// If the chunk is empty, `Ok(None)` is returned.
    fn allocate_chunk(
        data: &mut T,
        chunk_number: u64,
        validate_checksum: bool,
        recover_headerless: bool,
    ) -> Result<Option<EvtxChunkData>> {
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        // No file reaches an offset that doesn't fit.
        let chunk_offset = chunk_offset(chunk_number)
            .ok_or_else(|| EvtxError::chunk(chunk_number, ChunkError::IncompleteChunk))?;

        trace!(
            "Offset `0x{:08x} ({})` - Reading chunk number `{}`",
            chunk_offset, chunk_offset, chunk_number
        );

        data.seek(SeekFrom::Start(chunk_offset))
            .map_err(|e| EvtxError::chunk(chunk_number, ChunkError::FailedToSeekToChunk(e)))?;

        // The end of the file is a short read, not an error: an error here is the reader's
        // (a disk that can't be read, an exception from a Python file object).
        let amount_read = data
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .map_err(|e| EvtxError::chunk(chunk_number, ChunkError::FailedToReadChunk(e)))?;

        if amount_read < EVTX_CHUNK_HEADER_SIZE {
            return Err(EvtxError::chunk(chunk_number, ChunkError::IncompleteChunk));
        }

        // There might be empty chunks in the middle of a dirty file.
        if chunk_data.iter().all(|x| *x == 0) {
            return Ok(None);
        }

        // Bytes that end before a chunk does aren't a chunk without a header to say so.
        let keep_headerless = recover_headerless && amount_read == EVTX_CHUNK_SIZE;
        let mut chunk = EvtxChunkData::read(chunk_data, validate_checksum, keep_headerless)
            .map_err(|e| EvtxError::chunk(chunk_number, e))?;
        chunk.number = Some(chunk_number);
        if amount_read < EVTX_CHUNK_SIZE {
            warn!(
                "Chunk {chunk_number} is cut off by the end of the file after {amount_read} of \
                 its {EVTX_CHUNK_SIZE} bytes; reading the records that fit"
            );
        }
        Ok(Some(chunk))
    }

    /// Find the next chunk, starting at `chunk_number` (inclusive) and before `end`.
    /// If a chunk is found, returns the data of the chunk or the relevant error,
    /// and the number of that chunk.
    pub(crate) fn find_next_chunk(
        &mut self,
        mut chunk_number: u64,
        end: u64,
    ) -> Option<(Result<EvtxChunkData>, u64)> {
        loop {
            if chunk_number >= end {
                return None;
            }
            match EvtxParser::allocate_chunk(
                &mut self.data,
                chunk_number,
                self.config.validate_checksums,
                // Past the last whole chunk, bytes without a chunk header aren't a chunk.
                self.config.should_recover_records() && chunk_number < self.calculated_chunk_count,
            ) {
                Err(err) => {
                    // Past the last whole chunk there is a cut-off chunk, which is read like
                    // the others, or the end of the file, or bytes that aren't a chunk.
                    let not_a_chunk = matches!(
                        &err,
                        EvtxError::FailedToParseChunk { source, .. } if matches!(
                            **source,
                            ChunkError::IncompleteChunk | ChunkError::FailedToParseChunkHeader(_)
                        )
                    );
                    let whole_chunks = self.calculated_chunk_count;
                    if chunk_number > whole_chunks || (chunk_number == whole_chunks && not_a_chunk)
                    {
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

    /// The next chunk in record order and its number, advancing `cursor` past it.
    fn next_chunk(&mut self, cursor: &mut ChunkCursor) -> Option<(Result<EvtxChunkData>, u64)> {
        loop {
            // Once it has wrapped around, the walk ends where it started.
            let end = if cursor.wrapped {
                cursor.oldest
            } else {
                u64::MAX
            };
            if let Some((chunk, chunk_number)) = self.find_next_chunk(cursor.next, end) {
                cursor.next = chunk_number.checked_add(1)?;
                return Some((chunk, chunk_number));
            }
            if cursor.wrapped || cursor.oldest == 0 {
                return None;
            }
            cursor.wrapped = true;
            cursor.next = 0;
        }
    }

    /// The chunk that holds the oldest records, where Windows starts reading a log: the one
    /// whose header names the lowest record ID. A full log writes each new chunk over its oldest
    /// one, so once it has wrapped around, its oldest chunk can be anywhere in the file. The file
    /// header names that chunk too, but a log that wasn't closed cleanly can be ahead of its
    /// header: the LiveId sample's header names the chunk after its oldest.
    fn oldest_chunk(&mut self) -> u64 {
        let mut oldest = (u64::MAX, 0);
        let mut header = [0; EVTX_CHUNK_HEADER_SIZE];
        // One more than the whole chunks, for a last chunk that the end of the file cuts off.
        for chunk_number in 0..=self.calculated_chunk_count {
            let Some(offset) = chunk_offset(chunk_number) else {
                break;
            };
            if self.data.seek(SeekFrom::Start(offset)).is_err()
                || self.data.read_exact(&mut header).is_err()
            {
                break;
            }
            // A header whose range is reversed says nothing about where its records belong.
            if let Ok(chunk) = EvtxChunkHeader::from_bytes(&header)
                && chunk.first_event_record_id <= chunk.last_event_record_id
                && chunk.first_event_record_id < oldest.0
            {
                oldest = (chunk.first_event_record_id, chunk_number);
            }
        }
        oldest.1
    }

    /// Return an iterator over all the chunks, in record order: from the chunk that holds the
    /// oldest records to the end of the file, then from the start of the file, as Windows reads
    /// a log that has wrapped around.
    /// Each chunk supports iterating over it's records in their un-serialized state
    /// (before they are converted to XML or JSON).
    pub fn chunks(&mut self) -> IterChunks<'_, T> {
        let cursor = ChunkCursor::at(self.oldest_chunk());
        IterChunks {
            parser: self,
            cursor,
        }
    }

    /// Consumes the parser, returning an iterator over all the chunks, in record order (see
    /// [`Self::chunks`]).
    /// Each chunk supports iterating over it's records in their un-serialized state
    /// (before they are converted to XML or JSON).
    pub fn into_chunks(mut self) -> IntoIterChunks<T> {
        let cursor = ChunkCursor::at(self.oldest_chunk());
        IntoIterChunks {
            parser: self,
            cursor,
        }
    }
    /// Parse each chunk and turn it into a payload with `f` (a chunk that fails to
    /// read or parse becomes `on_err(err)`); yields one payload per chunk, in chunk order.
    ///
    /// With the `multithreading` feature, chunks are processed on the rayon pool as a
    /// bounded streaming pipeline: up to `2 * num_threads` chunks are in flight while
    /// already-completed chunks are drained by the caller, so workers never idle behind
    /// the (serial) consumer.
    fn chunk_pipeline<'a, P: Send + 'static>(
        &'a mut self,
        on_err: impl Fn(EvtxError) -> P + Send + Sync + Clone + 'static,
        f: impl Fn(&mut EvtxChunk<'_>) -> P + Send + Sync + Clone + 'static,
    ) -> impl Iterator<Item = P> + 'a {
        // Retrieve parser settings here, while `self` is immutably borrowed.
        let num_threads = max(self.config.num_threads, 1);
        let settings = Arc::clone(&self.config);
        let program_store = Arc::new(crate::binxml::compiled::ProgramStore::default());
        // Parse chunk `chunk_number` into a reused `arena` and build its payload.
        let work = move |chunk_res: Result<EvtxChunkData>, chunk_number: u64, arena: Bump| {
            let chunk_data = match chunk_res {
                Ok(chunk_data) => chunk_data,
                Err(err) => {
                    return ChunkBatch {
                        payload: on_err(err),
                        arena,
                    };
                }
            };
            let mut chunk = EvtxChunk::new_with_arena(
                &chunk_data.data,
                &chunk_data.header,
                Arc::clone(&settings),
                arena,
                Arc::clone(&program_store),
                Some(chunk_number),
            );
            ChunkBatch {
                payload: f(&mut chunk),
                arena: chunk.arena,
            }
        };

        let max_in_flight = num_threads * 2;

        // `self` is mutably borrowed from here on.
        let mut chunks = self.chunks();
        // Arenas are created on first use and recycled, so the pool grows to
        // exactly what is in flight (one when single-threaded).
        let mut arena_pool: Vec<Bump> = Vec::new();
        let new_arena = || Bump::with_capacity(EVTX_CHUNK_SIZE);

        // `thread::Result` so a panicking `work` propagates to the consumer thread
        // (matching the old `par_iter().collect()` behavior) instead of hitting
        // rayon's global handler, which aborts the process by default.
        // Created on first use: the single-threaded path never needs it.
        let mut channel = None;
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
                let (chunk_res, chunk_number) = chunks.next_numbered()?;
                let arena = arena_pool.pop().unwrap_or_else(new_arena);
                let batch = work(chunk_res, chunk_number, arena);
                arena_pool.push(batch.arena);
                return Some(batch.payload);
            }
            let (tx, rx) = channel.get_or_insert_with(
                std::sync::mpsc::channel::<(u64, std::thread::Result<ChunkBatch<P>>)>,
            );
            loop {
                // Keep the pipeline full. Chunk allocation (I/O) stays on the
                // consumer thread; parsing + serialization go to the rayon pool.
                while !exhausted && in_flight < max_in_flight {
                    let Some((chunk_res, chunk_number)) = chunks.next_numbered() else {
                        exhausted = true;
                        break;
                    };
                    let arena = arena_pool.pop().unwrap_or_else(new_arena);
                    let seq = next_seq;
                    next_seq += 1;
                    in_flight += 1;
                    let tx = tx.clone();
                    let work = work.clone();
                    spawn(move || {
                        let batch = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            work(chunk_res, chunk_number, arena)
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

    /// Return an iterator over all the records.
    /// Records will be mapped `f`, which must produce owned data from the records.
    pub fn serialized_records<'a, U: Send + 'static>(
        &'a mut self,
        f: impl FnMut(Result<EvtxRecord<'_>>) -> Result<U> + Send + Sync + Clone + 'static,
    ) -> impl Iterator<Item = Result<U>> + 'a {
        self.chunk_pipeline(
            |err| vec![Err(err)],
            move |chunk| {
                let mut f = f.clone();
                chunk
                    .iter()
                    .filter_map(|record| {
                        let recovered = record.as_ref().is_ok_and(|r| r.recovered);
                        let out = f(record);
                        // Carved hits that fail to render are noise, like ones that fail to build.
                        (!recovered || out.is_ok()).then_some(out)
                    })
                    .collect()
            },
        )
        .flatten()
    }

    /// CLI-internal: render every record into one buffer per chunk (single write
    /// per chunk on the consumer side). Not part of the supported public API.
    #[doc(hidden)]
    pub fn chunks_xml_bytes(
        &mut self,
        record_numbers: bool,
    ) -> impl Iterator<Item = RenderedChunk> + '_ {
        self.rendered_chunks(false, record_numbers)
    }

    /// CLI-internal: see [`Self::chunks_xml_bytes`].
    #[doc(hidden)]
    pub fn chunks_json_bytes(
        &mut self,
        record_numbers: bool,
    ) -> impl Iterator<Item = RenderedChunk> + '_ {
        self.rendered_chunks(true, record_numbers)
    }

    fn rendered_chunks(
        &mut self,
        json: bool,
        record_numbers: bool,
    ) -> impl Iterator<Item = RenderedChunk> + '_ {
        let failed = |err| RenderedChunk {
            data: Vec::new(),
            items: vec![RenderedChunkItem::Failed(err)],
        };
        // Shared by the workers, to size each chunk's buffer (see `render_chunk`).
        let largest_output = Arc::new(AtomicUsize::new(0));
        self.chunk_pipeline(failed, move |chunk| {
            if json {
                render_chunk::<JsonProgram>(chunk, record_numbers, &largest_output)
            } else {
                render_chunk::<XmlProgram>(chunk, record_numbers, &largest_output)
            }
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

/// Where a walk over the chunks in record order is (see [`EvtxParser::chunks`]).
#[derive(Debug, Clone, Copy)]
struct ChunkCursor {
    /// The next chunk number to look at.
    next: u64,
    /// The chunk that holds the oldest records, where the walk starts.
    oldest: u64,
    /// Whether the walk has gone past the end of the file, on to its first chunk.
    wrapped: bool,
}

impl ChunkCursor {
    fn at(oldest: u64) -> Self {
        ChunkCursor {
            next: oldest,
            oldest,
            wrapped: false,
        }
    }
}

pub struct IterChunks<'c, T: ReadSeek> {
    parser: &'c mut EvtxParser<T>,
    cursor: ChunkCursor,
}

impl<T: ReadSeek> IterChunks<'_, T> {
    /// The next chunk and its number in the file.
    fn next_numbered(&mut self) -> Option<(Result<EvtxChunkData>, u64)> {
        self.parser.next_chunk(&mut self.cursor)
    }
}

impl<T: ReadSeek> Iterator for IterChunks<'_, T> {
    type Item = Result<EvtxChunkData>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        self.next_numbered().map(|(chunk, _)| chunk)
    }
}

pub struct IntoIterChunks<T: ReadSeek> {
    parser: EvtxParser<T>,
    cursor: ChunkCursor,
}

impl<T: ReadSeek> Iterator for IntoIterChunks<T> {
    type Item = Result<EvtxChunkData>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        info!("Chunk {}", self.cursor.next);
        self.parser
            .next_chunk(&mut self.cursor)
            .map(|(chunk, _)| chunk)
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

        // Its Binary depends on a NULL value, so it is left out, as Windows does.
        assert!(!records[0].as_ref().unwrap().data.contains("<Binary"));
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
    fn file_with_a_corrupt_header_is_read() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut corrupt = evtx_file.to_vec();
        // The last chunk number, which the header's CRC32 covers.
        corrupt[16] ^= 0xff;

        let mut parser = EvtxParser::from_buffer(corrupt).unwrap();
        let header = parser.header();
        assert_eq!(header.checksum, 0x3f6e_33d5);
        assert!(!header.validate_checksum());
        let records = parser.records().filter(Result::is_ok).count();

        let mut intact = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();
        assert!(intact.header().validate_checksum());
        assert_eq!(records, intact.records().filter(Result::is_ok).count());
    }

    /// `security.evtx` cut `cut` bytes short of the end of its last chunk with records (its
    /// 26th, which holds records 2202 to 2261): the IDs of the records read, and the errors.
    fn records_of_cut_off_log(cut: usize, settings: ParserSettings) -> (Vec<u64>, Vec<EvtxError>) {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let end = EVTX_FILE_HEADER_SIZE + 26 * EVTX_CHUNK_SIZE;
        let mut parser = EvtxParser::from_buffer(evtx_file[..end - cut].to_vec())
            .unwrap()
            .with_configuration(settings);
        let (mut ids, mut errors) = (Vec::new(), Vec::new());
        for record in parser.records() {
            match record {
                Ok(record) => ids.push(record.event_record_id),
                Err(error) => errors.push(error),
            }
        }
        (ids, errors)
    }

    #[test]
    fn last_chunk_cut_off_in_its_free_space_is_read() {
        ensure_env_logger_initialized();
        for settings in [
            ParserSettings::new(),
            // What is left holds all that the chunk's CRC32s cover.
            ParserSettings::new().validate_checksums(true),
            ParserSettings::new().recover_records(true),
        ] {
            let (ids, errors) = records_of_cut_off_log(1000, settings);
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(ids, (1..=2261).collect::<Vec<_>>());
        }
    }

    #[test]
    fn last_chunk_cut_off_in_its_records_is_read_up_to_the_cut() {
        ensure_env_logger_initialized();
        let (ids, errors) = records_of_cut_off_log(40_000, ParserSettings::new());
        let last = *ids.last().unwrap();
        assert!((2202..2261).contains(&last), "{last}");
        assert_eq!(ids, (1..=last).collect::<Vec<_>>());
        // The record that the end of the file cuts off.
        let [error] = &errors[..] else {
            panic!("{errors:?}")
        };
        assert!(
            matches!(
                error,
                EvtxError::FailedToReadRecordAt {
                    chunk: Some(25),
                    ..
                }
            ),
            "{error:?}"
        );

        // The records' CRC32 covers bytes that are gone.
        let settings = ParserSettings::new().validate_checksums(true);
        let (ids, errors) = records_of_cut_off_log(40_000, settings);
        assert_eq!(ids, (1..=2201).collect::<Vec<_>>());
        assert!(
            matches!(
                &errors[..],
                [EvtxError::FailedToParseChunk { chunk_id: 25, source }]
                    if matches!(**source, ChunkError::InvalidFreeSpaceOffset { .. })
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn last_chunk_cut_off_between_two_records_reports_the_loss() {
        ensure_env_logger_initialized();
        // Record 2232 starts 25,656 bytes into the chunk.
        let (ids, errors) = records_of_cut_off_log(EVTX_CHUNK_SIZE - 25_656, ParserSettings::new());
        assert_eq!(ids, (1..=2231).collect::<Vec<_>>());
        assert!(
            matches!(
                &errors[..],
                [EvtxError::FailedToReadRecordAt {
                    chunk: Some(25),
                    offset: 25_656,
                    ..
                }]
            ),
            "{errors:?}"
        );
    }

    #[test]
    fn last_chunk_cut_off_in_its_header_is_left_out() {
        ensure_env_logger_initialized();
        for cut in [EVTX_CHUNK_SIZE - 511, EVTX_CHUNK_SIZE - 1] {
            let (ids, errors) = records_of_cut_off_log(cut, ParserSettings::new());
            assert!(errors.is_empty(), "{errors:?}");
            assert_eq!(ids, (1..=2201).collect::<Vec<_>>());
        }
    }

    #[test]
    fn wrapped_log_with_a_cut_off_last_chunk_starts_at_its_oldest_chunk() {
        ensure_env_logger_initialized();
        // The first two chunks of security.evtx in reverse order, the second cut off in its
        // free space: the oldest records are in the chunk that isn't whole.
        let evtx_file = include_bytes!("../samples/security.evtx");
        let chunk = |n: usize| {
            let start = EVTX_FILE_HEADER_SIZE + n * EVTX_CHUNK_SIZE;
            &evtx_file[start..start + EVTX_CHUNK_SIZE]
        };
        let mut wrapped = evtx_file[..EVTX_FILE_HEADER_SIZE].to_vec();
        wrapped.extend_from_slice(chunk(1));
        wrapped.extend_from_slice(&chunk(0)[..EVTX_CHUNK_SIZE - 100]);

        let mut parser = EvtxParser::from_buffer(wrapped).unwrap();
        let ids: Vec<_> = parser
            .records()
            .map(|record| record.unwrap().event_record_id)
            .collect();
        assert_eq!(ids.len(), 177);
        assert!(ids.is_sorted(), "{ids:?}");
        assert_eq!(ids[0], 1);
    }

    #[test]
    fn read_errors_are_reported_as_they_are() {
        /// Fails the reads of the second chunk's records.
        struct FailingReader(Cursor<Vec<u8>>);
        impl Read for FailingReader {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                // After the chunk's header, which finding the oldest chunk reads.
                let records = EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE + EVTX_CHUNK_HEADER_SIZE;
                let end = EVTX_FILE_HEADER_SIZE + 2 * EVTX_CHUNK_SIZE;
                let position = self.0.position() as usize;
                if (records..end).contains(&position) {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, "no entry"));
                }
                // A read never runs into those bytes from before them.
                let room = records.checked_sub(position).unwrap_or(buf.len());
                let len = buf.len().min(room);
                self.0.read(&mut buf[..len])
            }
        }
        impl Seek for FailingReader {
            fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
                self.0.seek(pos)
            }
        }

        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let reader = FailingReader(Cursor::new(evtx_file.to_vec()));
        let mut parser = EvtxParser::from_read_seek(reader).unwrap();
        let (mut records, mut errors) = (0, Vec::new());
        for record in parser.records() {
            match record {
                Ok(_) => records += 1,
                Err(error) => errors.push(error),
            }
        }
        // Every record but the second chunk's (92 to 177).
        assert_eq!(records, 2261 - 86);
        let [
            EvtxError::FailedToParseChunk {
                chunk_id: 1,
                source,
            },
        ] = &errors[..]
        else {
            panic!("{errors:?}")
        };
        let ChunkError::FailedToReadChunk(source) = &**source else {
            panic!("{source:?}")
        };
        assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(source.to_string(), "no entry");
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
    #[cfg(feature = "wevt_templates")]
    fn settings_compare_wevt_caches_by_identity() {
        use crate::wevt_templates::WevtCache;

        let cache = Arc::new(WevtCache::new());
        let with = |cache| ParserSettings::new().wevt_cache(cache);
        assert_eq!(
            with(Some(Arc::clone(&cache))),
            with(Some(Arc::clone(&cache)))
        );
        assert_ne!(
            with(Some(Arc::clone(&cache))),
            with(Some(Arc::new(WevtCache::new())))
        );
        assert_ne!(with(Some(cache)), with(None));
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

    #[test]
    fn test_chunk_buffers_start_with_room_for_the_largest_output_so_far() {
        ensure_env_logger_initialized();
        // 18 of its 53 chunks render more than 2 * EVTX_CHUNK_SIZE of XML.
        let evtx_file = include_bytes!("../samples/Application.evtx");
        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1));

        let mut largest = 0;
        for chunk in parser.chunks_xml_bytes(false) {
            let room = largest.clamp(2 * EVTX_CHUNK_SIZE, 16 * EVTX_CHUNK_SIZE);
            assert!(chunk.data.capacity() >= room);
            largest = largest.max(chunk.data.len());
        }
        assert!(largest > 2 * EVTX_CHUNK_SIZE);
    }
}
