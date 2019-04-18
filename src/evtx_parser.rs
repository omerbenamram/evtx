use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::EvtxRecord;
#[cfg(feature = "multithreading")]
use rayon::{current_num_threads, prelude::*};

use failure::Error;
use log::{debug, info};

use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{Flatten, IntoIterator, Iterator};

use std::path::Path;
use std::vec::IntoIter;

pub const EVTX_CHUNK_SIZE: usize = 65536;
pub const EVTX_FILE_HEADER_SIZE: usize = 4096;

pub trait ReadSeek: Read + Seek {
    fn tell(&mut self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
}

impl<T: Read + Seek> ReadSeek for T {}

pub struct EvtxParser<T: ReadSeek> {
    data: T,
    header: EvtxFileHeader,
}

impl EvtxParser<File> {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().canonicalize()?;
        let f = File::open(&path)?;

        let cursor = f;
        Self::from_read_seek(cursor)
    }
}

impl EvtxParser<Cursor<Vec<u8>>> {
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
        })
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

    #[cfg(feature = "multithreading")]
    pub fn parallel_records(self) -> IterRecords<T> {
        IterRecords {
            header: self.header,
            data: self.data,
            current_chunk_number: 0,
            chunk_records: None,
            num_threads: current_num_threads(),
            exhausted: false,
        }
    }

    pub fn records(self) -> IterRecords<T> {
        IterRecords {
            header: self.header,
            data: self.data,
            current_chunk_number: 0,
            chunk_records: None,
            num_threads: 1,
            exhausted: false,
        }
    }
}

impl<T: ReadSeek> IterRecords<T> {
    fn allocate_chunks(&mut self) -> Result<(), Error> {
        let mut chunks = vec![];

        // Some dirty samples contains additional chunks (further than the marked number).
        // So instead of trusting the file header, we opportunistically try to read a bit further.
        // If we fail, we terminate iteration.
        for _ in 0..self.num_threads {
            info!("Allocating new chunk {}", self.current_chunk_number);
            match EvtxParser::allocate_chunk(&mut self.data, self.current_chunk_number) {
                Ok(chunk) => {
                    chunks.push(chunk);
                    self.current_chunk_number += 1;
                }
                Err(_) => {
                    info!("Next block does not contain a valid chunk. terminating iteration.");
                    self.exhausted = true;
                    break;
                }
            }
        }

        #[cfg(feature = "multithreading")]
        let iterators: Result<
            Vec<Vec<Result<EvtxRecord, failure::Error>>>,
            failure::Error,
        > = {
            if self.num_threads > 1 {
                chunks
                    .into_par_iter()
                    .map(EvtxChunkData::into_records)
                    .collect()
            } else {
                chunks
                    .into_iter()
                    .map(EvtxChunkData::into_records)
                    .collect()
            }
        };

        #[cfg(not(feature = "multithreading"))]
        let mut iterators: Result<
            Vec<Vec<Result<EvtxRecord, failure::Error>>>,
            failure::Error,
        > = chunks
            .into_iter()
            .map(EvtxChunkData::into_records)
            .collect();

        match iterators {
            Ok(inner) => {
                self.chunk_records = Some(inner.into_iter().flatten());
                Ok(())
            }
            Err(e) => Err(e),
        }
    }
}

type FlatIterator<T> = Flatten<IntoIter<Vec<T>>>;

pub struct IterRecords<T: ReadSeek> {
    header: EvtxFileHeader,
    data: T,
    current_chunk_number: u16,
    chunk_records: Option<FlatIterator<Result<EvtxRecord, failure::Error>>>,
    num_threads: usize,
    // This is turned on when the next chunk is invalid data.
    exhausted: bool,
}

impl<T: ReadSeek> Iterator for IterRecords<T> {
    type Item = Result<EvtxRecord, Error>;
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        // Load first chunk
        if self.chunk_records.is_none() {
            if let Err(e) = self.allocate_chunks() {
                return Some(Err(e));
            }
        }

        let mut next = self.chunk_records.as_mut().unwrap().next();

        // Need to load a new chunk.
        if next.is_none() {
            // If the next chunk is going to be more than the chunk count (which is 1 based)
            if let Err(e) = self.allocate_chunks() {
                return Some(Err(e));
            }

            // When using multiple threads, we may reach exhaustion but still have some data left.
            if self.exhausted && self.chunk_records.is_none() {
                return None;
            }

            next = self.chunk_records.as_mut().unwrap().next()
        }

        next
    }
}

#[cfg(test)]
mod tests {
    #![allow(unused_variables)]

    use super::*;
    use crate::ensure_env_logger_initialized;

    fn process_90_records(buffer: &'static [u8]) {
        let parser = EvtxParser::from_buffer(buffer.to_vec()).unwrap();

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
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

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
        assert!(records[0].as_ref().unwrap().data.contains("<Binary></Binary>"));
        assert!(records[1].as_ref().unwrap().data.contains("<Binary>e177030c01101001c0d6000000000</Binary>"));
    }

    #[test]
    fn test_parses_first_10_records() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

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
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

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
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut record_ids = HashSet::new();
        for record in parser.parallel_records().take(1000) {
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
        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        assert_eq!(parser.records().count(), 4);
    }

    #[test]
    fn test_parses_chunk2() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");

        let chunk = EvtxChunkData::new(
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
                println!("{}", r.data);
            }
        }
    }

    #[test]
    // https://github.com/omerbenamram/evtx/issues/10
    fn test_issue_10() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");

        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut count = 0;
        for r in parser.records() {
            r.unwrap();
            count += 1;
        }
        assert_eq!(count, 14621, "Single threaded iteration failed");

        let parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut count = 0;
        for r in parser.parallel_records() {
            r.unwrap();
            count += 1;
        }

        assert_eq!(count, 14621, "Parallel iteration failed");
    }
}
