use crate::checksum_ieee;
use crate::err::{DeserializationError, DeserializationResult};
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
    /// The CRC32 of the header's first 120 bytes, as stored. A header with `NO_CRC32` stores 0.
    pub checksum: u32,
    /// The CRC32 of the header's first 120 bytes as read, which an intact header stores (see
    /// [`EvtxFileHeader::validate_checksum`]).
    pub computed_checksum: u32,
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
    /// Whether the header is intact: its stored CRC32 is that of its first 120 bytes, or
    /// `NO_CRC32` says that it stores none.
    ///
    /// [`EvtxParser`](crate::EvtxParser) reads a file whose header fails this anyway, and logs a
    /// warning: it finds chunks at their fixed offsets, and checks them with their own CRC32s.
    pub fn validate_checksum(&self) -> bool {
        self.flags.contains(HeaderFlags::NO_CRC32) || self.checksum == self.computed_checksum
    }

    pub fn from_bytes(data: &[u8]) -> DeserializationResult<EvtxFileHeader> {
        // We only need the fixed 128-byte header prefix (the full header block is 4096 bytes).
        let prefix = bytes::slice_r(data, 0, 128, "EVTX file header")?;

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
        // Keep the bits evtx doesn't know, so that they show.
        let flags = HeaderFlags::from_bits_retain(raw_flags);
        let checksum = bytes::read_u32_le_r(data, 124, "file_header_checksum")?;
        let computed_checksum = checksum_ieee(&prefix[..120]);

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
            computed_checksum,
        })
    }

    pub fn from_stream<T: Read + Seek>(stream: &mut T) -> DeserializationResult<EvtxFileHeader> {
        let mut header_block = [0_u8; crate::evtx_parser::EVTX_FILE_HEADER_SIZE];
        stream.read_exact(&mut header_block).map_err(|e| {
            std::io::Error::new(
                e.kind(),
                format!("failed to read EVTX file header block: {e}"),
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
                computed_checksum: checksum_ieee(&evtx_file[..120]),
            }
        );
        assert!(parsing_result.validate_checksum());
    }

    #[test]
    fn header_checksum_is_checked() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut header = evtx_file[..4096].to_vec();
        // The next record ID, which the CRC32 covers.
        header[24] ^= 1;
        let parsed = EvtxFileHeader::from_bytes(&header).unwrap();
        assert_eq!(parsed.next_record_id, 2227);
        assert_eq!(parsed.checksum, 0x3f6e_33d5);
        assert_eq!(parsed.computed_checksum, checksum_ieee(&header[..120]));
        assert!(!parsed.validate_checksum());

        // The flags come after the 120 bytes the CRC32 covers, and NO_CRC32 says it's unused.
        header[120] |= HeaderFlags::NO_CRC32.bits() as u8;
        assert!(
            EvtxFileHeader::from_bytes(&header)
                .unwrap()
                .validate_checksum()
        );
    }

    #[test]
    fn header_without_crc32_is_valid() {
        // Its header has NO_CRC32 and stores 0.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/samples/Application_no_crc32.evtx"
        );
        let mut file = std::fs::File::open(path).unwrap();
        let header = EvtxFileHeader::from_stream(&mut file).unwrap();
        assert_eq!(header.flags, HeaderFlags::NO_CRC32);
        assert_eq!(header.checksum, 0);
        assert_ne!(header.computed_checksum, 0);
        assert!(header.validate_checksum());
    }

    #[test]
    fn unknown_header_flags_are_kept() {
        // None of the flags evtx knows, only bits it doesn't.
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/samples/post-Security.evtx");
        let mut file = std::fs::File::open(path).unwrap();
        let header = EvtxFileHeader::from_stream(&mut file).unwrap();
        assert_eq!(header.flags.bits(), 0x1b97_0000);
        assert!(!header.flags.intersects(HeaderFlags::all()));
    }
}
