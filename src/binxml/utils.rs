use encoding::all::UTF_16LE;
use encoding::DecoderTrap;
use encoding::Encoding;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Cursor, Error, ErrorKind, Seek, SeekFrom};

pub fn read_len_prefixed_utf16_string<'a>(
    stream: &mut Cursor<&'a [u8]>,
    is_null_terminated: bool,
) -> io::Result<Option<String>> {
    let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
    let needed_bytes = (expected_number_of_characters * 2) as usize;

    let p = stream.position() as usize;
    let ref_to_utf16_bytes = &stream.get_ref()[p..p + needed_bytes];

    match expected_number_of_characters {
        0 => Ok(None),
        _ => match UTF_16LE.decode(ref_to_utf16_bytes, DecoderTrap::Strict) {
            Ok(s) => {
                let mut bytes_to_seek = needed_bytes as i64;
                if is_null_terminated {
                    bytes_to_seek += 2;
                }

                stream.seek(SeekFrom::Current(bytes_to_seek))?;
                if expected_number_of_characters as usize != s.len() {
                    return Err(Error::from(ErrorKind::InvalidData));
                }

                return Ok(Some(s));
            }
            Err(s) => Err(Error::from(ErrorKind::InvalidData)),
        },
    }
}
