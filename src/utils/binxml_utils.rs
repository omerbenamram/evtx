use crate::evtx_parser::ReadSeek;
use thiserror::Error;

use crate::err::{DeserializationError, DeserializationResult, WrappedIoError};

use byteorder::{LittleEndian, ReadBytesExt};

use encoding::{decode, DecoderTrap, EncodingRef};
use log::trace;
use std::char::decode_utf16;
use std::error::Error as StdErr;
use std::io::{self, Error, ErrorKind};

#[derive(Debug, Error)]
pub enum FailedToReadString {
    #[error(
        "Expected string of length {}, found string of length {} - \
         `{}`",
        expected_len,
        found_len,
        data
    )]
    UnexpectedLength {
        expected_len: u16,
        found_len: usize,
        data: String,
    },

    #[error("An I/O error has occurred")]
    IoError(#[from] io::Error),
}

pub fn read_len_prefixed_utf16_string<T: ReadSeek>(
    stream: &mut T,
    is_null_terminated: bool,
) -> Result<Option<String>, FailedToReadString> {
    let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
    let needed_bytes = u64::from(expected_number_of_characters * 2);

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

    let s_len = s.as_ref().map(String::len).unwrap_or(0);

    if s_len == expected_number_of_characters as usize {
        Ok(s)
    } else {
        let string_if_successful = s.unwrap_or_else(|| "".to_string());
        let truncated = if string_if_successful.len() > 25 {
            let temp = &string_if_successful[0..25];
            temp.to_owned() + "..."
        } else {
            string_if_successful
        };

        Err(FailedToReadString::UnexpectedLength {
            expected_len: expected_number_of_characters,
            found_len: s_len,
            data: truncated,
        })
    }
}

/// Reads a utf16 string from the given stream.
/// size is the actual byte representation of the string (not the number of characters).
pub fn read_utf16_by_size<T: ReadSeek>(stream: &mut T, size: u64) -> io::Result<Option<String>> {
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

/// Reads an ansi encoded string from the given stream using `ansi_codec`.
pub fn read_ansi_encoded_string<T: ReadSeek>(
    stream: &mut T,
    size: u64,
    ansi_codec: EncodingRef,
) -> DeserializationResult<Option<String>> {
    match size {
        0 => Ok(None),
        _ => {
            let mut bytes = vec![0; size as usize];
            stream.read_exact(&mut bytes)?;

            let s = match decode(&bytes, DecoderTrap::Strict, ansi_codec).0 {
                Ok(mut s) => {
                    if let Some('\0') = s.chars().last() {
                        s.pop();
                    }
                    s
                }
                Err(message) => {
                    let as_boxed_err = Box::<dyn StdErr + Send + Sync>::from(message.to_string());
                    let wrapped_io_err = WrappedIoError::capture_hexdump(as_boxed_err, stream);
                    return Err(DeserializationError::FailedToReadToken {
                        t: format!("ansi_string {}", ansi_codec.name()),
                        token_name: "",
                        source: wrapped_io_err,
                    });
                }
            };

            Ok(Some(s))
        }
    }
}

pub fn read_null_terminated_utf16_string<T: ReadSeek>(stream: &mut T) -> io::Result<String> {
    read_utf16_string(stream, None)
}

/// Reads a utf16 string from the given stream.
/// If `len` is given, exactly `len` u16 values are read from the stream.
/// If `len` is None, the string is assumed to be null terminated and the stream will be read to the first null (0).
fn read_utf16_string<T: ReadSeek>(stream: &mut T, len: Option<usize>) -> io::Result<String> {
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
        None => loop {
            let next_char = stream.read_u16::<byteorder::LittleEndian>()?;

            if next_char == 0 {
                break;
            }

            buffer.push(next_char);
        },
    }

    decode_utf16(buffer.into_iter())
        .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
        .collect()
}
