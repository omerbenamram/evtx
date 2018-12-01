use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};
use time::Duration;

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

fn parse_evtx(evtx: &[u8]) {
    let mut cursor = Cursor::new(evtx);
    let header = EvtxFileHeader::from_reader(&mut cursor).unwrap();

    let mut offset = EVTX_HEADER_SIZE;

    for chunk in 0..header.chunk_count {
        let chunk = EvtxChunk::new(&evtx[offset..offset + EVTX_CHUNK_SIZE]).unwrap();

        println!("Chunk: {:#?}", chunk);

        for record in chunk.into_iter() {
            println!("{:?}", record);
        }

        offset += EVTX_CHUNK_SIZE;
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
        parse_evtx(evtx_file);
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
