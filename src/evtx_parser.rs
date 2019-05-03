use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::SerializedEvtxRecord;
#[cfg(feature = "multithreading")]
use rayon;
#[cfg(feature = "multithreading")]
use rayon::prelude::*;

use failure::{format_err, Error};
use log::{debug, info};

use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};

use crate::json_output::JsonOutput;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use std::cmp::max;
use std::path::Path;

pub const EVTX_CHUNK_SIZE: usize = 65536;
pub const EVTX_FILE_HEADER_SIZE: usize = 4096;

pub trait ReadSeek: Read + Seek {
    fn tell(&mut self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
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
    config: ParserSettings,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParserSettings {
    /// Controls the number of threads used for parsing chunks concurrently.
    num_threads: usize,
    /// If enabled, chunk with bad checksums will be skipped.
    validate_checksums: bool,
    /// If true, output will be indented.
    indent: bool,
}

impl Default for ParserSettings {
    fn default() -> Self {
        ParserSettings {
            num_threads: 0,
            validate_checksums: false,
            indent: true,
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

    pub fn validate_checksums(mut self, validate_checksums: bool) -> Self {
        self.validate_checksums = validate_checksums;

        self
    }

    pub fn indent(mut self, pretty: bool) -> Self {
        self.indent = pretty;

        self
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
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().canonicalize()?;
        let f = File::open(&path)?;

        let cursor = f;
        Self::from_read_seek(cursor)
    }
}

impl EvtxParser<Cursor<Vec<u8>>> {
    /// Attempts to load an evtx file from a given path, will fail the evtx header is invalid.
    pub fn from_buffer(buffer: Vec<u8>) -> Result<Self, Error> {
        let cursor = Cursor::new(buffer);
        Self::from_read_seek(cursor)
    }
}

impl<T: ReadSeek> EvtxParser<T> {
    fn from_read_seek(mut read_seek: T) -> Result<Self, Error> {
        let evtx_header = EvtxFileHeader::from_reader(&mut read_seek)?;

        debug!("EVTX Header: {:#?}", evtx_header);
        Ok(EvtxParser {
            data: read_seek,
            header: evtx_header,
            config: ParserSettings::default(),
        })
    }

    pub fn with_configuration(mut self, configuration: ParserSettings) -> Self {
        self.config = configuration;
        self
    }

    /// Allocate a new chunk from the given data, at the offset expected by `chunk_number`.
    /// If the read chunk contains valid data, an `Ok(Some(EvtxChunkData))` will be returned.
    /// If the read chunk contains invalid data (bad magic, bad checksum when `validate_checksum` is set to true),
    /// of if not enough data can be read (e.g. because we reached EOF), an `Err` is returned.
    /// If the read chunk is empty, `Ok(None)` will be returned.
    fn allocate_chunk(
        data: &mut T,
        chunk_number: u16,
        validate_checksum: bool,
    ) -> Result<Option<EvtxChunkData>, Error> {
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        let chunk_offset = EVTX_FILE_HEADER_SIZE + chunk_number as usize * EVTX_CHUNK_SIZE;

        data.seek(SeekFrom::Start(chunk_offset as u64))?;

        let amount_read = data
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)?;

        if amount_read != EVTX_CHUNK_SIZE {
            return Err(format_err!("Reached EOF while trying to read a chunk"));
        }

        // There might be empty chunks in the middle of a dirty file.
        if chunk_data.iter().all(|x| *x == 0) {
            return Ok(None);
        }

        EvtxChunkData::new(chunk_data, validate_checksum).map(Some)
    }

    /// Find the next chunk, staring at `chunk_number` (inclusive).
    /// If a chunk is found, returns the data of the chunk or the relevant error,
    /// and the number of that chunk.
    pub fn find_next_chunk(
        &mut self,
        mut chunk_number: u16,
    ) -> Option<(Result<EvtxChunkData, Error>, u16)> {
        loop {
            match EvtxParser::allocate_chunk(
                &mut self.data,
                chunk_number,
                self.config.validate_checksums,
            ) {
                Err(err) => {
                    // We try to read past the `chunk_count` to allow for dirty files.
                    // But if we failed, it means we really are at the end of the file.
                    if chunk_number >= self.header.chunk_count {
                        return None;
                    } else {
                        return Some((Err(err), chunk_number));
                    }
                }
                Ok(None) => {
                    // We try to read past the `chunk_count` to allow for dirty files.
                    // But if we get an empty chunk, we need to keep looking.
                    // Increment and try again.
                    chunk_number += 1;
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
    pub fn chunks(&mut self) -> IterChunks<T> {
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

    /// Return an iterator over all the records.
    /// Records will be serialized using the given `BinXmlOutput`.
    pub fn serialized_records<O: BinXmlOutput<Vec<u8>>>(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord, Error>> + '_ {
        let num_threads = max(self.config.num_threads, 1);
        let chunk_settings = self.config.clone();

        let mut chunks = self.chunks();

        let records_per_chunk = std::iter::from_fn(move || {
            // Allocate some chunks in advance, so they can be parsed in parallel.
            let mut chunk_of_chunks = Vec::with_capacity(num_threads);

            for _ in 0..num_threads {
                if let Some(chunk) = chunks.next() {
                    chunk_of_chunks.push(chunk);
                };
            }

            // We only stop once no chunks can be allocated.
            if chunk_of_chunks.is_empty() {
                None
            } else {
                #[cfg(feature = "multithreading")]
                let chunk_iter = chunk_of_chunks.into_par_iter();

                #[cfg(not(feature = "multithreading"))]
                let chunk_iter = chunk_of_chunks.into_iter();

                // Serialize the records in each chunk.
                let iterators: Vec<Vec<Result<SerializedEvtxRecord, Error>>> = chunk_iter
                    .map(|chunk_res| match chunk_res {
                        Err(err) => vec![Err(err)],
                        Ok(mut chunk) => {
                            let chunk_records_res = chunk.parse(&chunk_settings);

                            match chunk_records_res {
                                Err(err) => vec![Err(err)],
                                Ok(mut chunk_records) => {
                                    chunk_records.iter_serialized_records::<O>().collect()
                                }
                            }
                        }
                    })
                    .collect();

                Some(iterators.into_iter().flatten())
            }
        });

        records_per_chunk.flatten()
    }

    /// Return an iterator over all the records.
    /// Records will be XML-formatted.
    pub fn records(&mut self) -> impl Iterator<Item = Result<SerializedEvtxRecord, Error>> + '_ {
        // '_ is required in the signature because the iterator is bound to &self.
        self.serialized_records::<XmlOutput<Vec<u8>>>()
    }

    /// Return an iterator over all the records.
    /// Records will be JSON-formatted.
    pub fn records_json(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord, Error>> + '_ {
        self.serialized_records::<JsonOutput<Vec<u8>>>()
    }
}

pub struct IterChunks<'c, T: ReadSeek> {
    parser: &'c mut EvtxParser<T>,
    current_chunk_number: u16,
}

impl<'c, T: ReadSeek> Iterator for IterChunks<'c, T> {
    type Item = Result<EvtxChunkData, Error>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        match self.parser.find_next_chunk(self.current_chunk_number) {
            None => None,
            Some((chunk, chunk_number)) => {
                self.current_chunk_number = chunk_number + 1;

                Some(chunk)
            }
        }
    }
}

pub struct IntoIterChunks<T: ReadSeek> {
    parser: EvtxParser<T>,
    current_chunk_number: u16,
}

impl<T: ReadSeek> Iterator for IntoIterChunks<T> {
    type Item = Result<EvtxChunkData, Error>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        info!("Chunk {}", self.current_chunk_number);
        match self.parser.find_next_chunk(self.current_chunk_number) {
            None => None,
            Some((chunk, chunk_number)) => {
                self.current_chunk_number = chunk_number + 1;

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

    fn process_90_records(buffer: &'static [u8]) {
        let mut parser = EvtxParser::from_buffer(buffer.to_vec()).unwrap();

        for (i, record) in parser.records().take(90).enumerate() {
            match record {
                Ok(r) => {
                    assert_eq!(r.event_record_id, i as u64 + 1);
                }
                Err(e) => println!("Error while reading record {}, {:?}", i, e),
            }
        }
    }

    // For clion profiler
    #[test]
    fn test_process_single_chunk() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        process_90_records(evtx_file);
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
        assert!(records[0]
            .as_ref()
            .unwrap()
            .data
            .contains("<Binary></Binary>"));
        assert!(records[1]
            .as_ref()
            .unwrap()
            .data
            .contains("<Binary>E107070003000C00110010001C00D6000000000000000000</Binary>"));
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

        let records: Vec<_> = parser.records().collect();
        assert_eq!(records.len(), 4);
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

        for record in chunk.parse(&ParserSettings::default()).unwrap().iter() {
            record.unwrap();
        }
    }

    #[test]
    fn test_into_chunsk() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/new-user-security.evtx");
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let records: Vec<_> = parser.into_chunks().collect();
        assert_eq!(records.len(), 1);
    }

}
