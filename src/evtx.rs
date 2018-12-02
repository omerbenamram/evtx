use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
use log::{info, log};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};
use time::Duration;

use failure::Error;

use crate::evtx_chunk::IterChunkRecords;
use crate::evtx_chunk::{EvtxChunk, EvtxChunkHeader};
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::EvtxRecord;
use crate::utils::*;
use crate::xml_builder::{BinXMLOutput, XMLOutput};
use core::borrow::Borrow;
use core::borrow::BorrowMut;
use crc::crc32;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::stdout;
use std::rc::Rc;

const EVTX_CHUNK_SIZE: usize = 65536;
const EVTX_HEADER_SIZE: usize = 4096;

struct IterRecords<'chunk, T: Read + Seek> {
    header: EvtxFileHeader,
    evtx_data: T,
    chunk_number: u16,
    chunk_iter: IterChunkRecords<'chunk>,
}

impl<'a, T: Read + Seek> Iterator for IterRecords<'a, T> {
    type Item = Result<EvtxRecord, Error>;

    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        // Need to load a new chunk.
        if self.chunk_iter.exhausted() {
            self.chunk_number += 1;
            info!("Allocating new chunk {}", self.chunk_number);

            let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
            self.evtx_data
                .seek(SeekFrom::Start(
                    (EVTX_HEADER_SIZE + self.chunk_number as usize * EVTX_CHUNK_SIZE) as u64,
                ))
                .unwrap();

            self.evtx_data
                .borrow_mut()
                .take(EVTX_CHUNK_SIZE as u64)
                .read_to_end(&mut chunk_data)
                .unwrap();

            let cursor = Cursor::new(chunk_data.as_slice());
            let with_header = EvtxChunk::new(chunk_data).unwrap();
            self.chunk_iter = with_header.into_iter();
        }

        info!(
            "Yielding record at offset {}",
            self.chunk_iter.offset_from_chunk_start()
        );
        self.chunk_iter.next()
    }
}

impl<'a> IterRecords<'a, Cursor<&'a [u8]>> {
    pub fn from_bytes(bytes: &'a [u8]) -> Self {
        let mut borrowing_cursor = Cursor::new(bytes);

        let evtx_header = EvtxFileHeader::from_reader(&mut borrowing_cursor)
            .expect("Failed to read EVTX file header");

        // Allocate the first chunk
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);

        borrowing_cursor
            .borrow_mut()
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .unwrap();

        let chunk = EvtxChunk::new(chunk_data).expect("Failed to read EVTX chunk header");

        IterRecords {
            header: evtx_header,
            evtx_data: borrowing_cursor,
            chunk_number: 0,
            chunk_iter: chunk.into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_variables)]
    use super::*;
    use crate::ensure_env_logger_initialized;
    use crate::evtx_file_header::HeaderFlags;
    use encoding::all::UTF_16LE;
    use encoding::DecoderTrap;
    use encoding::Encoding;
    use std::char::decode_utf16;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_parses_record() {
        let _ = ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let records = IterRecords::from_bytes(evtx_file);

        for (i, record) in records.take(10).enumerate() {
            let record = record.unwrap();
            assert_eq!(record.event_record_id, i as u64 + 1)
        }
    }

    #[test]
    fn test_parses_records_from_different_chunks() {
        let _ = ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let records = IterRecords::from_bytes(evtx_file);

        for (i, record) in records.take(100).enumerate() {
            match record {
                Ok(r) => assert_eq!(r.event_record_id, i as u64 + 1),
                Err(e) => println!("Error while reading record {}, {:?}", i, e),
            }
        }
    }

    #[test]
    fn test_parses_chunk2() {
        let _ = ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");

        let chunk = EvtxChunk::new(
            evtx_file[EVTX_HEADER_SIZE + EVTX_CHUNK_SIZE..EVTX_HEADER_SIZE + 2 * EVTX_CHUNK_SIZE]
                .to_vec(),
        )
        .unwrap();

        println!("Chunk: {:#?}", chunk);

        for record in chunk.into_iter() {
            println!("{:?}", record);
        }
    }
}
