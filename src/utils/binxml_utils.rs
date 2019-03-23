use encoding::{all::UTF_16LE, DecoderTrap, Encoding};

use crate::evtx::ReadSeek;
use crate::utils::print_hexdump;
use byteorder::{ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use log::{debug, error, log, trace};
use std::{
    cmp::min,
    io::{self, Cursor, Error, ErrorKind, Read, Seek, SeekFrom},
};

pub fn read_len_prefixed_utf16_string<T: ReadSeek>(
    stream: &mut T,
    is_null_terminated: bool,
) -> io::Result<Option<String>> {
    let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
    let needed_bytes = (expected_number_of_characters * 2) as usize;
    trace!(
        "Going to read a string of len {} from stream",
        expected_number_of_characters
    );

    read_utf16_by_size(stream, needed_bytes as u64)
        .and_then(|s| {
            if let Some(string) = s {
                if string.len() == expected_number_of_characters as usize {
                    return Ok(Some(string));
                } else {
                    error!(
                        "Expected string of length {}, found string of length {} - {}",
                        string.len(),
                        expected_number_of_characters,
                        string
                    );
                    return Err(Error::from(ErrorKind::InvalidData));
                }
            }
            Ok(Some("".to_string()))
        })
        .and_then(|s| {
            // Seek null terminator if needed (we can't feed it to the decoder)
            if is_null_terminated {
                stream.read_u16::<LittleEndian>()?;
            };
            Ok(s)
        })
}

pub fn read_utf16_by_size<T: ReadSeek>(stream: &mut T, size: u64) -> io::Result<Option<String>> {
    let p = stream.stream_position()? as usize;

    let mut buffer = vec![0; size as usize];
    let ref_to_utf16_bytes = stream.read_exact(&mut buffer);

    match size {
        0 => Ok(None),
        _ => match UTF_16LE.decode(&mut buffer, DecoderTrap::Strict) {
            Ok(s) => Ok(Some(s)),
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
