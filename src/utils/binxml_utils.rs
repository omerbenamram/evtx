

use crate::evtx_parser::ReadSeek;
use crate::utils::print_hexdump;
use byteorder::{LittleEndian, ReadBytesExt};
use log::{error, trace};
use std::{
    cmp::min,
    io::{self, Cursor, Error, ErrorKind},
};
use std::char::{decode_utf16};

pub fn read_len_prefixed_utf16_string<T: ReadSeek>(
    stream: &mut T,
    is_null_terminated: bool,
) -> io::Result<Option<String>> {
    let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
    let needed_bytes = (expected_number_of_characters * 2) as u64;

    trace!(
        "Going to read a{}string of len {} from stream",
        if is_null_terminated {
            " null terminated "
        } else {
            " "
        },
        expected_number_of_characters
    );

    let s = read_utf16_by_size(stream, needed_bytes)?;

    if is_null_terminated {
        stream.read_u16::<LittleEndian>()?;
    };

    let s_len = s.as_ref().map(|s| s.len()).unwrap_or(0);

    if s_len == expected_number_of_characters as usize {
        Ok(s)

    } else {
        error!(
            "Expected string of length {}, found string of length {} - {:?}",
            expected_number_of_characters,
            s_len,
            s
        );

        Err(Error::from(ErrorKind::InvalidData))
    }
}


/// Reads a utf16 string from the given stream.
/// size is the actual byte representation of the string (not the number of characters).
pub fn read_utf16_by_size<T: ReadSeek>(stream: &mut T, size : u64) -> io::Result<Option<String>> {
    match size {
        0 => Ok(None),
        _ => read_utf16_string(stream, Some(size as usize / 2)).map(|mut s| {
            // Strip nul terminator if needed
            if let Some('\0') = s.chars().last() {
                s.pop();
            }
            Some(s)
        }),
    }
}


pub fn read_null_terminated_utf16_string<T: ReadSeek>(
    stream: &mut T,
) -> io::Result<String> {
    read_utf16_string(stream, None)
}


/// Reads a utf16 string from the given stream.
/// If `len` is given, exactly `len` u16 values are read from the stream.
/// If `len` is None, the string is assumed to be null terminated and the stream will be read to the first null (0).
fn read_utf16_string<T: ReadSeek>(
    stream: &mut T,
    len: Option<usize>,
) -> io::Result<String> {
    let mut buffer = match len {
        Some(len) => Vec::with_capacity(len),
        None => Vec::new(),
    };

    match len {
        Some(len) => {
            for _ in 0..len {
                let next_char = stream.read_u16::<byteorder::LittleEndian>()?;
                buffer.push(next_char);
            }
        }
        None => {
            loop {
                let next_char = stream.read_u16::<byteorder::LittleEndian>()?;

                if next_char == 0 {
                    break;
                }

                buffer.push(next_char);
            }
        }
    }

    decode_utf16(buffer.into_iter())
        .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
        .collect()
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
