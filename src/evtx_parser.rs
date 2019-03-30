use crate::evtx_chunk::EvtxChunkData;
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::EvtxRecord;
use core::borrow::BorrowMut;
use failure::Error;
use log::{debug, info};
use memmap::{self, Mmap};

use owning_ref::OwningRef;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};

use std::ops::Deref;
use std::path::Path;
use std::vec::IntoIter;

pub const EVTX_CHUNK_SIZE: usize = 65536;
pub const EVTX_FILE_HEADER_SIZE: usize = 4096;

// Inspired by https://github.com/mitsuhiko/unbox/src/formats/cab.rs
// Armin Ronacher is a genius.
pub trait ReadSeek: Read + Seek {
    fn tell(&mut self) -> io::Result<u64> {
        self.seek(SeekFrom::Current(0))
    }
}

impl<T: Read + Seek> ReadSeek for T {}

struct StableDerefMmap(Mmap);

impl Deref for StableDerefMmap {
    type Target = [u8];

    #[inline]
    fn deref(&self) -> &[u8] {
        self.0.deref()
    }
}

unsafe impl stable_deref_trait::StableDeref for StableDerefMmap {}

pub struct EvtxParser {
    data: Box<dyn ReadSeek>,
}

impl EvtxParser {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref().canonicalize()?;
        let f = File::open(&path)?;
        let mmap = unsafe { StableDerefMmap(Mmap::map(&f)?) };
        let owning_mmap = OwningRef::new(mmap);

        let cursor = Box::new(Cursor::new(owning_mmap)) as Box<dyn ReadSeek>;

        Ok(EvtxParser { data: cursor })
    }

    pub fn from_buffer(buffer: &'static [u8]) -> Self {
        let cursor = Box::new(Cursor::new(buffer)) as Box<dyn ReadSeek>;
        EvtxParser { data: cursor }
    }

    pub fn records(self) -> IterRecords<Box<dyn ReadSeek>> {
        IterRecords::from(self.data)
    }
}

impl<T: ReadSeek> IterRecords<T> {
    pub fn from(mut read_seek: T) -> Self {
        let evtx_header =
            EvtxFileHeader::from_reader(&mut read_seek).expect("Failed to read EVTX file header");

        debug!("EVTX Header: {:#?}", evtx_header);
        // Allocate the first chunk
        info!("Allocating initial chunk");
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);

        read_seek
            .borrow_mut()
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .unwrap();

        let chunk = EvtxChunkData::new(chunk_data).expect("Failed to read EVTX chunk header");
        debug!("EVTX Chunk 0 Header: {:#?}", chunk.header);
        assert!(chunk.validate_checksum(), "Invalid checksum");

        let allocated_records: Vec<Result<EvtxRecord, failure::Error>> =
            chunk.parse().into_iter().collect();
        let records = allocated_records.into_iter();

        IterRecords {
            header: evtx_header,
            evtx_data: read_seek,
            chunk_number: 0,
            chunk_records: records,
        }
    }
}

pub struct IterRecords<T: ReadSeek> {
    header: EvtxFileHeader,
    evtx_data: T,
    chunk_number: u16,
    chunk_records: IntoIter<Result<EvtxRecord, failure::Error>>,
}

impl<T: ReadSeek> Iterator for IterRecords<T> {
    type Item = Result<EvtxRecord, Error>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let mut next = self.chunk_records.next();

        // Need to load a new chunk.
        if next.is_none() {
            // If the next chunk is going to be more than the chunk count (which is 1 based)
            if self.chunk_number + 1 == self.header.chunk_count {
                return None;
            }

            self.chunk_number += 1;
            info!("Allocating new chunk {}", self.chunk_number);

            let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
            self.evtx_data
                .seek(SeekFrom::Start(
                    (EVTX_FILE_HEADER_SIZE + self.chunk_number as usize * EVTX_CHUNK_SIZE) as u64,
                ))
                .unwrap();

            self.evtx_data
                .borrow_mut()
                .take(EVTX_CHUNK_SIZE as u64)
                .read_to_end(&mut chunk_data)
                .unwrap();

            let chunk_data = EvtxChunkData::new(chunk_data).unwrap();
            let allocated_records: Vec<Result<EvtxRecord, failure::Error>> =
                chunk_data.parse().into_iter().collect();
            let records = allocated_records.into_iter();
            self.chunk_records = records;
            next = self.chunk_records.next()
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
        let parser = EvtxParser::from_buffer(buffer);

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
        let parser = EvtxParser::from_buffer(evtx_file);

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
        let parser = EvtxParser::from_buffer(evtx_file);

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
        let parser = EvtxParser::from_buffer(evtx_file);

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

        for record in chunk.parse().into_iter() {
            if let Err(e) = record {
                println!("{}", e);
                panic!();
            }

            if let Ok(r) = record {
                println!("{}", r.data);
            }
        }
    }
}
