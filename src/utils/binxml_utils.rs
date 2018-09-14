use encoding::all::UTF_16LE;
use encoding::DecoderTrap;
use encoding::Encoding;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use std::cmp::min;
use std::io::{self, Cursor, Error, ErrorKind, Seek, SeekFrom, Read};
use utils::print_hexdump;

pub fn read_len_prefixed_utf16_string(
    stream: &mut Cursor<&[u8]>,
    is_null_terminated: bool,
) -> io::Result<Option<String>> {
    let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
    let needed_bytes = (expected_number_of_characters * 2) as usize;

    read_utf16_by_size(stream, needed_bytes as u64)
        .and_then(|s| {
            if let Some(string) = s {
                if string.len() == expected_number_of_characters as usize {
                    return Ok(Some(string));
                } else {
                    error!(
                        "Expected string of length {}, found string of length {}",
                        string.len(),
                        expected_number_of_characters
                    );
                    return Err(Error::from(ErrorKind::InvalidData));
                }
            }
            return Err(Error::from(ErrorKind::InvalidData));
        }).and_then(|s| {
        // Seek null terminator if needed (we can't feed it to the decoder)
        if is_null_terminated {
            stream.read_u16::<LittleEndian>()?;
        };
        Ok(s)
    })
}

pub fn read_utf16_by_size(
    stream: &mut Cursor<&[u8]>,
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
