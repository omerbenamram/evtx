use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use chrono::prelude::*;
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::iter::{IntoIterator, Iterator};
use time::Duration;

use crate::evtx_chunk::{EvtxChunk, EvtxChunkHeader};
use crate::evtx_file_header::EvtxFileHeader;
use crate::evtx_record::EvtxRecord;
use crate::utils::*;
use crate::xml_builder::{BinXMLTreeBuilder, Visitor};
use crc::crc32;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::stdout;
use std::rc::Rc;

const EVTX_CHUNK_SIZE: usize = 65536;
const EVTX_HEADER_SIZE: usize = 4096;

fn parse_evtx<'a, V: Visitor<'a> + 'static>(evtx: &'a [u8], visitor: V) {
    let mut cursor = Cursor::new(evtx);
    let header = EvtxFileHeader::from_reader(&mut cursor);

    let chunk = EvtxChunk::new(
        &evtx[EVTX_HEADER_SIZE..EVTX_HEADER_SIZE + EVTX_CHUNK_SIZE],
        visitor,
    ).unwrap();

    for record in chunk.into_iter().take(10) {
        println!("{:?}", record);
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
        let evtx_file = include_bytes!("../samples/security.evtx");
        let visitor = BinXMLTreeBuilder::with_writer(stdout());
        parse_evtx(evtx_file, visitor);
    }
}
