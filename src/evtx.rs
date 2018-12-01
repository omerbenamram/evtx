use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
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
            let mut chunk = Vec::with_capacity(EVTX_CHUNK_SIZE);
            self.evtx_data.seek(SeekFrom::Start(
                (EVTX_HEADER_SIZE + self.chunk_number as usize * EVTX_CHUNK_SIZE) as u64,
            ));

            self.evtx_data.read_exact(&mut chunk);

            let mut cursor = Cursor::new(chunk.as_slice());
            let with_header = EvtxChunk::new(&chunk).unwrap();
            self.chunk_iter = with_header.into_iter();
        }

        self.chunk_iter.next()
    }
}

impl<'a> IterRecords<'a, Cursor<Vec<u8>>> {
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let mut borrowing_cursor = Cursor::new(bytes.as_slice());

        let evtx_header = EvtxFileHeader::from_reader(&mut borrowing_cursor)
            .expect("Failed to read EVTX file header");

        // Allocate the first chunk
        let mut chunk = Vec::with_capacity(EVTX_CHUNK_SIZE);
        borrowing_cursor.seek(SeekFrom::Start((EVTX_HEADER_SIZE) as u64));
        borrowing_cursor.read_exact(&mut chunk);

        let chunk = EvtxChunk::new(&chunk).expect("Failed to read EVTX chunk header");

        let owning_cursor = Cursor::new(bytes);

        IterRecords {
            header: evtx_header,
            evtx_data: owning_cursor,
            chunk_number: 0,
            chunk_iter: chunk.into_iter(),
        }
    }
}

#[cfg(test)]
mod tests {
    #[allow(unused_variables)]
    use super::*;
    use crate::evtx_file_header::HeaderFlags;
    use encoding::all::UTF_16LE;
    use encoding::DecoderTrap;
    use encoding::Encoding;
    use std::char::decode_utf16;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_parses_record() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");
        let records = IterRecords::from_bytes(evtx_file.to_vec());

        for record in records.take(1) {
            println!("{:?}", record);
        }
    }

    #[test]
    fn test_parses_chunk2() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");

        let chunk = EvtxChunk::new(
            &evtx_file[EVTX_HEADER_SIZE + EVTX_CHUNK_SIZE..EVTX_HEADER_SIZE + 2 * EVTX_CHUNK_SIZE],
        )
        .unwrap();

        println!("Chunk: {:#?}", chunk);

        for record in chunk.into_iter() {
            println!("{:?}", record);
        }
    }
}
