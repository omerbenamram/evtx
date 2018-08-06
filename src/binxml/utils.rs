use encoding::all::UTF_16LE;
use encoding::DecoderTrap;
use encoding::Encoding;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use hexdump::print_hexdump;
use std::cmp::min;
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

                // We need to seek manually because the UTF-16 reader
                // does not advance the stream.
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

pub fn read_utf16_by_size<'a>(
    stream: &mut Cursor<&'a [u8]>,
    size: u64,
) -> io::Result<Option<String>> {
    let p = stream.position() as usize;
    let ref_to_utf16_bytes = &stream.get_ref()[p..p + size as usize];

    match size {
        0 => Ok(None),
        _ => match UTF_16LE.decode(ref_to_utf16_bytes, DecoderTrap::Strict) {
            Ok(s) => {
                // We need to seek manually because the UTF-16 reader
                // does not advance the stream.
                stream.seek(SeekFrom::Current(size as i64))?;
                return Ok(Some(s));
            }
            Err(s) => Err(Error::from(ErrorKind::InvalidData)),
        },
    }
}

pub fn dump_cursor(cursor: &Cursor<&[u8]>, lookbehind: i32) {
    let offset = cursor.position();
    let data = cursor.get_ref();
    println!("-------------------------------");
    println!("Current Value {:2X}", data[offset as usize]);
    let m = (offset as i32) - lookbehind;
    let start = if m < 0 { 0 } else { m };
    let end_of_buffer_or_default = min(100, data.len() - offset as usize);
    let end = offset + end_of_buffer_or_default as u64;
    print_hexdump(&data[start as usize..end as usize], 0, 'C');
    println!("\n-------------------------------");
}