use crate::err::{DeserializationError, DeserializationResult, WrappedIoError};
use crate::utils::bytes;

use std::io::{Read, Seek};

#[derive(Debug, PartialEq, Eq)]
pub struct EvtxFileHeader {
    pub first_chunk_number: u64,
    pub last_chunk_number: u64,
    pub next_record_id: u64,
    pub header_size: u32,
    pub minor_version: u16,
    pub major_version: u16,
    pub header_block_size: u16,
    pub chunk_count: u16,
    pub flags: HeaderFlags,
    // Checksum is of first 120 bytes of header
    pub checksum: u32,
}

bitflags! {
    #[derive(Debug, PartialEq, Eq)]
    pub struct HeaderFlags: u32 {
        const EMPTY = 0x0;
        const DIRTY = 0x1;
        const FULL = 0x2;
        const NO_CRC32 = 0x4;
    }
}

impl EvtxFileHeader {
    pub fn from_bytes(data: &[u8]) -> DeserializationResult<EvtxFileHeader> {
        // We only need the fixed 128-byte header prefix (the full header block is 4096 bytes).
        let _ = bytes::slice_r(data, 0, 128, "EVTX file header")?;

        let magic = bytes::read_array_r::<8>(data, 0, "file header magic")?;
        if &magic != b"ElfFile\x00" {
            return Err(DeserializationError::InvalidEvtxFileHeaderMagic { magic });
        }

        let oldest_chunk = bytes::read_u64_le_r(data, 8, "file_header_oldest_chunk")?;
        let current_chunk_num = bytes::read_u64_le_r(data, 16, "file_header_current_chunk_num")?;
        let next_record_num = bytes::read_u64_le_r(data, 24, "file_header_next_record_num")?;
        let header_size = bytes::read_u32_le_r(data, 32, "file_header_header_size")?;
        let minor_version = bytes::read_u16_le_r(data, 36, "file_header_minor_version")?;
        let major_version = bytes::read_u16_le_r(data, 38, "file_header_major_version")?;
        let header_block_size = bytes::read_u16_le_r(data, 40, "file_header_header_block_size")?;
        let chunk_count = bytes::read_u16_le_r(data, 42, "file_header_chunk_count")?;

        let raw_flags = bytes::read_u32_le_r(data, 120, "file_header_flags")?;
        let flags = HeaderFlags::from_bits_truncate(raw_flags);
        let checksum = bytes::read_u32_le_r(data, 124, "file_header_checksum")?;

        Ok(EvtxFileHeader {
            first_chunk_number: oldest_chunk,
            last_chunk_number: current_chunk_num,
            next_record_id: next_record_num,
            header_size,
            minor_version,
            major_version,
            header_block_size,
            chunk_count,
            flags,
            checksum,
        })
    }

    pub fn from_stream<T: Read + Seek>(stream: &mut T) -> DeserializationResult<EvtxFileHeader> {
        let mut header_block = [0_u8; crate::evtx_parser::EVTX_FILE_HEADER_SIZE];
        stream.read_exact(&mut header_block).map_err(|e| {
            WrappedIoError::io_error_with_message(
                e,
                "failed to read EVTX file header block",
                stream,
            )
        })?;
        Self::from_bytes(&header_block)
    }
}

#[cfg(test)]
mod tests {
    use crate::checksum_ieee;

    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_parses_evtx_file_handler() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut reader = Cursor::new(&evtx_file[..4096]);
        let parsing_result = EvtxFileHeader::from_stream(&mut reader).unwrap();
        assert_eq!(
            parsing_result,
            EvtxFileHeader {
                first_chunk_number: 0,
                last_chunk_number: 25,
                next_record_id: 2226,
                header_size: 128,
                minor_version: 1,
                major_version: 3,
                header_block_size: 4096,
                chunk_count: 26,
                flags: HeaderFlags::DIRTY,
                checksum: checksum_ieee(&evtx_file[..120]),
            }
        );
    }
}
