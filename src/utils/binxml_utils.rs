use crate::evtx_parser::ReadSeek;
use thiserror::Error;

use crate::err::{DeserializationError, DeserializationResult, WrappedIoError};

use byteorder::{LittleEndian, ReadBytesExt};

use encoding::{DecoderTrap, EncodingRef, decode};
use log::trace;
use std::char::decode_utf16;
use std::error::Error as StdErr;
use std::io::{self, Error, ErrorKind};
use encoding_rs::UTF_16LE;

#[derive(Debug, Error)]
pub enum FailedToReadString {
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
        "Offset `0x{offset:08x} ({offset})` reading a{nul}string of len {len}",
        offset = stream.tell().unwrap_or(0),
        nul = if is_null_terminated {
            " null terminated "
        } else {
            " "
        },
        len = expected_number_of_characters
    );

    let s = read_utf16_by_size(stream, needed_bytes)?;

    if is_null_terminated {
        stream.read_u16::<LittleEndian>()?;
    };

    // It is useless to check for size equality, since u16 characters may be decoded into multiple u8 chars,
    // so we might end up with more characters than originally asked for.
    //
    // Moreover, the code will also not read **less** characters than asked.
    Ok(s)
}

/// Reads a utf16 string from the given stream.
/// size is the actual byte representation of the string (not the number of characters).
pub fn read_utf16_by_size<T: ReadSeek>(stream: &mut T, size: u64) -> io::Result<Option<String>> {
    match size {
        0 => Ok(None),
        _ => {
            let size_usize = size as usize;
            let mut raw = vec![0u8; size_usize];
            stream.read_exact(&mut raw)?;
            #[cfg(feature = "fast-utf16")]
            {
                let (decoded, had_errors) = UTF_16LE.decode_without_bom_handling(&raw);
                if had_errors {
                    // Fall back to the existing scalar path if we encountered errors
                    let mut cursor = io::Cursor::new(raw);
                    let mut buffer = Vec::with_capacity(size_usize / 2);
                    while (cursor.position() as usize) + 1 < size_usize {
                        let next_char = cursor.read_u16::<LittleEndian>()?;
                        if next_char == 0 { break; }
                        buffer.push(next_char);
                    }
                    let s: String = decode_utf16(buffer.into_iter().take_while(|&b| b != 0x00))
                        .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
                        .collect::<io::Result<String>>()?;
                    let trimmed_len = s.trim_end().len();
                    let mut s = s;
                    if trimmed_len < s.len() { s.truncate(trimmed_len); }
                    Ok(Some(s))
                } else {
                    let mut s = decoded.into_owned();
                    let trimmed_len = s.trim_end().len();
                    if trimmed_len < s.len() { s.truncate(trimmed_len); }
                    Ok(Some(s))
                }
            }
            #[cfg(not(feature = "fast-utf16"))]
            {
                // Original scalar path for compatibility
                let mut cursor = io::Cursor::new(raw);
                let mut buffer = Vec::with_capacity(size_usize / 2);
                while (cursor.position() as usize) + 1 < size_usize {
                    let next_char = cursor.read_u16::<LittleEndian>()?;
                    if next_char == 0 { break; }
                    buffer.push(next_char);
                }
                let mut s: String = decode_utf16(buffer.into_iter().take_while(|&b| b != 0x00))
                    .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
                    .collect::<io::Result<String>>()?;
                let trimmed_len = s.trim_end().len();
                if trimmed_len < s.len() { s.truncate(trimmed_len); }
                Ok(Some(s))
            }
        },
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

            // There may be multiple NULs in the string, prune them.
            bytes.retain(|&b| b != 0);

            let s = match decode(&bytes, DecoderTrap::Strict, ansi_codec).0 {
                Ok(s) => s,
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

    // We need to stop if we see a NUL byte, even if asked for more bytes.
    decode_utf16(buffer.into_iter().take_while(|&byte| byte != 0x00))
        .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
        .collect()
}
