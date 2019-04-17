use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::{EvtxRecord, SerializedEvtxRecord};
#[cfg(feature = "multithreading")]
use rayon;
#[cfg(feature = "multithreading")]
use rayon::prelude::*;

use failure::Error;
use log::{debug, info};

use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{Flatten, IntoIterator, Iterator, Peekable};

use std::path::Path;
use std::vec::IntoIter;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::json_output::JsonOutput;
use std::marker::PhantomData;

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
///
///
/// let parser = EvtxParser::from_path(fp).unwrap();
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
///
///
/// let settings = ParserSettings::default().num_threads(0);
/// let parser = EvtxParser::from_path(fp).unwrap().with_configuration(settings);
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

#[derive(Copy, Clone, PartialOrd, PartialEq)]
pub enum EvtxOutputFormat {
    JSON,
    XML,
}

pub struct ParserSettings {
    output_format: EvtxOutputFormat,
    num_threads: usize,
}

impl Default for ParserSettings {
    fn default() -> Self {
        ParserSettings {
            output_format: EvtxOutputFormat::XML,
            num_threads: 0,
        }
    }
}

impl ParserSettings {
    pub fn new() -> Self {
        ParserSettings::default()
    }

    /// Sets the output format of the evtx records.
    pub fn output_format(mut self, format: EvtxOutputFormat) -> Self {
        self.output_format = format;
        self
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

    pub fn chunks(&mut self) -> IterChunks<T> {
        IterChunks {
            parser: self,
            current_chunk_number: 0,
        }
    }

    // TODO: This doesn't work.
    #[cfg(not(feature = "multithreading"))]
    pub fn serialized_records<O: BinXmlOutput<Vec<u8>>>(&mut self) -> impl Iterator<Item=Result<SerializedEvtxRecord, Error>> + '_ {
        IterSerializedRecords {
            chunks: self.chunks(),
            current_chunk_records: None,
        }
    }

    #[cfg(feature = "multithreading")]
    pub fn serialized_records<'a, 'c, O: BinXmlOutput<Vec<u8>>>(&'a mut self) -> impl Iterator<Item=Result<SerializedEvtxRecord, Error>>
    {
        let chunks: Vec<Result<EvtxChunkData, Error>> = self.chunks().collect();

        let iterators: Vec<Vec<Result<SerializedEvtxRecord, Error>>> = chunks.into_par_iter().map(
            |chunk_res| {
                match chunk_res {
                    Err(err) => vec![Err(err)],
                    Ok(mut chunk) => {
                        let chunk_records_res = chunk.into_serialized_records::<O>();

                        match chunk_records_res {
                            Err(err) => vec![Err(err)],
                            Ok(chunk_records) => chunk_records,
                        }
                    }
                }
            })
            .collect();

        iterators.into_iter().flatten().into_iter()
    }

    pub fn records(&mut self) -> impl Iterator<Item=Result<SerializedEvtxRecord, Error>> + '_ {
        self.serialized_records::<XmlOutput<Vec<u8>>>()
    }


    pub fn records_json(&mut self) -> impl Iterator<Item=Result<SerializedEvtxRecord, Error>> + '_ {
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
        if self.current_chunk_number == self.parser.header.chunk_count {
            return None;
        }

        let next = EvtxParser::allocate_chunk(&mut self.parser.data, self.current_chunk_number);

        self.current_chunk_number += 1;

        Some(next)
    }
}

pub struct IterSerializedRecords<'c, T: ReadSeek, O: BinXmlOutput<Vec<u8>>> {
    chunks: IterChunks<'c, T>,
    current_chunk_records: Option<std::vec::IntoIter<Result<SerializedEvtxRecord, Error>>>,
    _phantom: PhantomData<O>,
}

impl<'c, T: ReadSeek, O: BinXmlOutput<Vec<u8>>> Iterator for IterSerializedRecords<'c, T, O> {
    type Item = Result<SerializedEvtxRecord, Error>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let next_record_item: Option<Result<SerializedEvtxRecord, Error>> = self.current_chunk_records.as_mut().and_then(|records_iter| records_iter.next());

        if next_record_item.is_some() {
            return next_record_item;
        }

        let next_chunk_item = self.chunks.next();

        let mut next_chunk = match next_chunk_item {
            None => {
                return None;
            }
            Some(Err(e)) => {
                return Some(Err(e));
            }
            Some(Ok(chunk)) => chunk
        };

        let records = match next_chunk.into_records() {
            Err(e) => {
                return Some(Err(e));
            }
            Ok(records) => records,
        };

        let mut serialized_records: Vec<Result<SerializedEvtxRecord, Error>> = records.into_iter().map(
            |record_res| record_res.and_then(
                |record| record.into_serialized::<O>())).collect();
        let mut records_iter = serialized_records.into_iter();

        // We assume a chunk always has at least a single record. Is that true?
        let next = records_iter.next();

        self.current_chunk_records = Some(records_iter);

        next
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
}
