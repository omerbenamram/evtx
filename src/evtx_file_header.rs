use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::{Error, Fail};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

#[derive(Fail, Debug)]
enum HeaderParseError {
    #[fail(display = "Expected magic \"ElfFile\x00\", got {:#?}", magic)]
    WrongHeaderMagic { magic: [u8; 8] },
    #[fail(display = "Unknown flag value: {:#?}", flag)]
    UnknownFlagValue { flag: u32 },
}

#[derive(Debug, PartialEq)]
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

#[derive(Debug, PartialEq)]
pub enum HeaderFlags {
    Empty,
    Dirty,
    Full,
}

impl EvtxFileHeader {
    pub fn from_reader<T: Read + Seek>(stream: &mut T) -> Result<EvtxFileHeader, Error> {
        let mut magic = [0_u8; 8];
        stream.take(8).read_exact(&mut magic)?;

        if &magic != b"ElfFile\x00" {
            return Err(Error::from(HeaderParseError::WrongHeaderMagic { magic }));
        }

        let oldest_chunk = stream.read_u64::<LittleEndian>()?;
        let current_chunk_num = stream.read_u64::<LittleEndian>()?;
        let next_record_num = stream.read_u64::<LittleEndian>()?;
        let header_size = stream.read_u32::<LittleEndian>()?;
        let minor_version = stream.read_u16::<LittleEndian>()?;
        let major_version = stream.read_u16::<LittleEndian>()?;
        let header_block_size = stream.read_u16::<LittleEndian>()?;
        let chunk_count = stream.read_u16::<LittleEndian>()?;

        // unused
        stream.seek(SeekFrom::Current(76))?;
        let flags = match stream.read_u32::<LittleEndian>()? {
            0_u32 => HeaderFlags::Empty,
            1_u32 => HeaderFlags::Dirty,
            2_u32 => HeaderFlags::Full,
            other => {
                return Err(Error::from(HeaderParseError::UnknownFlagValue {
                    flag: other,
                }))
            }
        };

        let checksum = stream.read_u32::<LittleEndian>()?;
        // unused
        stream.seek(SeekFrom::Current(4096 - 128))?;
        Ok(EvtxFileHeader {
            first_chunk_number: oldest_chunk,
            last_chunk_number: current_chunk_num,
            next_record_id: next_record_num,
            header_block_size,
            minor_version,
            major_version,
            header_size,
            chunk_count,
            flags,
            checksum,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crc::crc32;
    use std::io::Cursor;

    #[test]
    fn test_parses_evtx_file_handler() {
        let evtx_file = include_bytes!("../samples/security.evtx");
        let mut reader = Cursor::new(&evtx_file[..4096]);
        let parsing_result = EvtxFileHeader::from_reader(&mut reader).unwrap();
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
                flags: HeaderFlags::Dirty,
                checksum: crc32::checksum_ieee(&evtx_file[..120]),
            }
        );
    }
}
