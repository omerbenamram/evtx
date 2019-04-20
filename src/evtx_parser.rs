use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::SerializedEvtxRecord;
#[cfg(feature = "multithreading")]
use rayon;
#[cfg(feature = "multithreading")]
use rayon::prelude::*;

use failure::Error;
use log::debug;

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

#[derive(Clone)]
pub struct ParserSettings {
    num_threads: usize,
}

impl Default for ParserSettings {
    fn default() -> Self {
        ParserSettings { num_threads: 0 }
    }
}

impl ParserSettings {
    pub fn new() -> Self {
        ParserSettings::default()
    }

    /// Sets the number of worker threads.
    /// `0` will let rayon decide.
    pub fn num_threads(mut self, num_threads: usize) -> Self {
        self.num_threads = if num_threads == 0 {
            rayon::current_num_threads()
        } else {
            num_threads
        };
        self
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

    pub fn allocate_chunk(data: &mut T, chunk_number: u16) -> Result<EvtxChunkData, Error> {
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        data.seek(SeekFrom::Start(
            (EVTX_FILE_HEADER_SIZE + chunk_number as usize * EVTX_CHUNK_SIZE) as u64,
        ))?;

        data.take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)?;

        EvtxChunkData::new(chunk_data)
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

    /// Return an iterator over all the records.
    /// Records will be serialized using the given `BinXmlOutput`.
    pub fn serialized_records<O: BinXmlOutput<Vec<u8>>>(
        &mut self,
    ) -> impl Iterator<Item = Result<SerializedEvtxRecord, Error>> + '_ {
        let num_threads = max(self.config.num_threads, 1);
        let mut chunks = self.chunks();

        let records_per_chunk = std::iter::from_fn(move || {
            // Allocate some chunks in advances, so they can be parsed in parallel.
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
                            let chunk_records_res = chunk.parse_serialized_records::<O>();

                            match chunk_records_res {
                                Err(err) => vec![Err(err)],
                                Ok(chunk_records) => chunk_records.collect(),
                            }
                        }
                    })
                    .collect();

                Some(iterators.into_iter().flatten().into_iter())
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
        let next = EvtxParser::allocate_chunk(&mut self.parser.data, self.current_chunk_number);

        // We try to read past the `chunk_count` to allow for dirty files.
        // But if we failed, it means we really are at the end of the file.
        if next.is_err() && self.current_chunk_number >= self.parser.header.chunk_count {
            return None;
        }

        self.current_chunk_number += 1;

        Some(next)
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
                    println!("{}", r.data);
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
                    println!("{}", r.data);
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
                    println!("{}", r.data);
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
        )
        .unwrap();

        assert!(chunk.validate_checksum());

        for record in chunk.parse().unwrap().into_iter() {
            if let Err(e) = record {
                println!("{}", e);
                panic!();
            }

            if let Ok(r) = record {
                println!("{}", r.into_xml().unwrap().data);
            }
        }
    }

    #[test]
    // https://github.com/omerbenamram/evtx/issues/10
    fn test_issue_10() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");

        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut count = 0;
        for r in parser.records() {
            r.unwrap();
            count += 1;
        }
        assert_eq!(count, 14621, "Single threaded iteration failed");

        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();
        let mut parser = parser.with_configuration(ParserSettings { num_threads: 8 });

        let mut count = 0;
        for r in parser.records() {
            r.unwrap();
            count += 1;
        }

        assert_eq!(count, 14621, "Parallel iteration failed");
    }

    #[test]
    fn test_parses_sample_with_irregular_boolean_values() {
        ensure_env_logger_initialized();
        // This sample contains boolean values which are not zero or one.
        let evtx_file = include_bytes!("../samples/sample-with-irregular-bool-values.evtx");

        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        for r in parser.records() {
            r.unwrap();
        }
    }
}
