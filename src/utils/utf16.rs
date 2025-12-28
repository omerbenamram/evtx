//! UTF-16LE utilities used by BinXML parsing and IR rendering.
//!
//! The EVTX BinXML stream stores most text as UTF-16LE. Decoding those strings
//! eagerly into UTF-8 creates allocator churn and extra copies. This module
//! keeps UTF-16LE slices borrowed from the underlying chunk and provides:
//! - validation and trimming helpers for UTF-16LE payloads,
//! - on-demand UTF-8 decoding for legacy paths,
//! - streaming JSON/XML escaping directly from UTF-16LE bytes.
//!
//! The streaming escape functions are designed for hot paths: they avoid
//! intermediate allocations and include an ASCII fast path to copy common
//! ASCII data directly.

use std::io;
use std::io::Write;

/// Errors that can occur while validating or decoding UTF-16LE data.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Utf16LeDecodeError {
    /// The input byte slice has an odd length.
    OddLength,
    /// The UTF-16LE data contains invalid surrogate pairs.
    InvalidData,
}

/// A borrowed UTF-16LE slice plus a logical code-unit length.
///
/// `num_chars` counts UTF-16 code units (not Unicode scalar values) and
/// limits how much of `bytes` should be considered during rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Utf16LeSlice<'a> {
    bytes: &'a [u8],
    num_chars: usize,
}

impl<'a> Utf16LeSlice<'a> {
    /// Create a new UTF-16LE slice view.
    pub fn new(bytes: &'a [u8], num_chars: usize) -> Self {
        Utf16LeSlice { bytes, num_chars }
    }

    /// Returns an empty UTF-16LE slice.
    pub fn empty() -> Self {
        Utf16LeSlice {
            bytes: &[],
            num_chars: 0,
        }
    }

    /// Returns the backing byte slice (may include unused trailing bytes).
    pub fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// Returns the number of UTF-16 code units to consider.
    pub fn num_chars(&self) -> usize {
        self.num_chars
    }

    /// Returns the UTF-16LE byte slice truncated to `num_chars`.
    pub fn as_bytes(&self) -> &'a [u8] {
        let max = self.num_chars.saturating_mul(2).min(self.bytes.len());
        &self.bytes[..max]
    }

    /// Returns true if the slice contains no code units.
    pub fn is_empty(&self) -> bool {
        self.num_chars == 0
    }

    /// Decode the UTF-16LE slice into a UTF-8 string.
    pub fn to_string(&self) -> Result<String, Utf16LeDecodeError> {
        decode_utf16le_bytes(self.as_bytes())
    }
}

/// Decode a UTF-16LE byte slice into a UTF-8 string.
pub(crate) fn decode_utf16le_bytes(bytes: &[u8]) -> Result<String, Utf16LeDecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }

    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    String::from_utf16(&units).map_err(|_| Utf16LeDecodeError::InvalidData)
}

/// Decode a UTF-16LE byte slice until the first NUL (0x0000), if present.
pub(crate) fn decode_utf16le_bytes_z(bytes: &[u8]) -> Result<String, Utf16LeDecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }

    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }

    decode_utf16_units_z(&units)
}

/// Decode UTF-16 code units until the first NUL (0x0000), if present.
pub(crate) fn decode_utf16_units_z(units: &[u16]) -> Result<String, Utf16LeDecodeError> {
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    String::from_utf16(&units[..end]).map_err(|_| Utf16LeDecodeError::InvalidData)
}

/// Validate UTF-16LE input and trim trailing Unicode whitespace.
///
/// Returns the number of UTF-16 code units to keep after trimming.
pub(crate) fn trim_utf16le_whitespace(
    bytes: &[u8],
    num_chars: usize,
) -> Result<usize, Utf16LeDecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }

    let max_chars = bytes.len() / 2;
    let limit = num_chars.min(max_chars);
    if limit == 0 {
        return Ok(0);
    }

    let mut unit_index = 0usize;
    let mut last_non_ws = 0usize;
    let mut saw_non_ws = false;

    while unit_index < limit {
        let (ch, consumed) = decode_utf16le_at_unit(bytes, unit_index, limit)?;
        if ch == '\u{0}' {
            break;
        }
        let next_index = unit_index + consumed;
        if !ch.is_whitespace() {
            last_non_ws = next_index;
            saw_non_ws = true;
        }
        unit_index = next_index;
    }

    Ok(if saw_non_ws { last_non_ws } else { 0 })
}

/// Write UTF-16LE input as JSON-escaped UTF-8.
pub(crate) fn write_utf16le_json_escaped<W: Write>(
    writer: &mut W,
    text: Utf16LeSlice<'_>,
) -> io::Result<()> {
    write_utf16le_escaped(writer, text, EscapeMode::Json)
}

/// Write UTF-16LE input as XML-escaped UTF-8.
pub(crate) fn write_utf16le_xml_escaped<W: Write>(
    writer: &mut W,
    text: Utf16LeSlice<'_>,
    in_attribute: bool,
) -> io::Result<()> {
    write_utf16le_escaped(writer, text, EscapeMode::Xml { in_attribute })
}

/// Write UTF-16LE input as raw UTF-8 (no escaping).
pub(crate) fn write_utf16le_raw<W: Write>(
    writer: &mut W,
    text: Utf16LeSlice<'_>,
) -> io::Result<()> {
    write_utf16le_escaped(writer, text, EscapeMode::None)
}

enum EscapeMode {
    None,
    Json,
    Xml { in_attribute: bool },
}

fn write_utf16le_escaped<W: Write>(
    writer: &mut W,
    text: Utf16LeSlice<'_>,
    mode: EscapeMode,
) -> io::Result<()> {
    let bytes = text.as_bytes();
    let max_bytes = bytes.len();
    if max_bytes < 2 {
        return Ok(());
    }

    let mut byte_pos = 0usize;
    let mut out_buf = [0u8; 256];
    let mut out_len = 0usize;

    while byte_pos + 1 < max_bytes {
        let lo = bytes[byte_pos];
        let hi = bytes[byte_pos + 1];

        if hi == 0 && lo <= 0x7F && !ascii_needs_escape(lo, &mode) {
            out_buf[out_len] = lo;
            out_len += 1;
            byte_pos += 2;
            if out_len == out_buf.len() {
                writer.write_all(&out_buf)?;
                out_len = 0;
            }
            continue;
        }

        if out_len != 0 {
            writer.write_all(&out_buf[..out_len])?;
            out_len = 0;
        }

        let (ch, consumed) = decode_utf16le_at_byte(bytes, byte_pos, max_bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid UTF-16LE"))?;
        byte_pos += consumed;
        write_char_escaped(writer, ch, &mode)?;
    }

    if out_len != 0 {
        writer.write_all(&out_buf[..out_len])?;
    }

    Ok(())
}

#[inline]
fn ascii_needs_escape(byte: u8, mode: &EscapeMode) -> bool {
    match mode {
        EscapeMode::None => false,
        EscapeMode::Json => match byte {
            b'"' | b'\\' => true,
            0x00..=0x1F => true,
            _ => false,
        },
        EscapeMode::Xml { in_attribute } => match byte {
            b'&' | b'<' | b'>' => true,
            b'"' | b'\'' if *in_attribute => true,
            _ => false,
        },
    }
}

fn write_char_escaped<W: Write>(writer: &mut W, ch: char, mode: &EscapeMode) -> io::Result<()> {
    match mode {
        EscapeMode::None => write_char_raw(writer, ch),
        EscapeMode::Json => write_char_json(writer, ch),
        EscapeMode::Xml { in_attribute } => write_char_xml(writer, ch, *in_attribute),
    }
}

fn write_char_raw<W: Write>(writer: &mut W, ch: char) -> io::Result<()> {
    let mut buf = [0u8; 4];
    let s = ch.encode_utf8(&mut buf);
    writer.write_all(s.as_bytes())
}

fn write_char_json<W: Write>(writer: &mut W, ch: char) -> io::Result<()> {
    match ch {
        '"' => writer.write_all(br#"\""#),
        '\\' => writer.write_all(br#"\\"#),
        '\u{08}' => writer.write_all(br#"\b"#),
        '\u{0C}' => writer.write_all(br#"\f"#),
        '\n' => writer.write_all(br#"\n"#),
        '\r' => writer.write_all(br#"\r"#),
        '\t' => writer.write_all(br#"\t"#),
        c if c <= '\u{1F}' => {
            let value = c as u32;
            let hi = ((value >> 4) & 0xF) as u8;
            let lo = (value & 0xF) as u8;
            let mut buf = [0u8; 6];
            buf[0] = b'\\';
            buf[1] = b'u';
            buf[2] = b'0';
            buf[3] = b'0';
            buf[4] = hex_digit(hi);
            buf[5] = hex_digit(lo);
            writer.write_all(&buf)
        }
        _ => write_char_raw(writer, ch),
    }
}

fn write_char_xml<W: Write>(writer: &mut W, ch: char, in_attribute: bool) -> io::Result<()> {
    match ch {
        '&' => writer.write_all(b"&amp;"),
        '<' => writer.write_all(b"&lt;"),
        '>' => writer.write_all(b"&gt;"),
        '"' if in_attribute => writer.write_all(b"&quot;"),
        '\'' if in_attribute => writer.write_all(b"&apos;"),
        _ => write_char_raw(writer, ch),
    }
}

#[inline]
fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'A' + (value - 10),
    }
}

fn decode_utf16le_at_unit(
    bytes: &[u8],
    unit_index: usize,
    max_units: usize,
) -> Result<(char, usize), Utf16LeDecodeError> {
    if unit_index >= max_units {
        return Err(Utf16LeDecodeError::InvalidData);
    }
    let lo = bytes
        .get(unit_index * 2)
        .copied()
        .ok_or(Utf16LeDecodeError::InvalidData)?;
    let hi = bytes
        .get(unit_index * 2 + 1)
        .copied()
        .ok_or(Utf16LeDecodeError::InvalidData)?;
    let cu = u16::from_le_bytes([lo, hi]);
    decode_utf16le_unit_value(bytes, cu, unit_index, max_units)
}

fn decode_utf16le_at_byte(
    bytes: &[u8],
    byte_pos: usize,
    max_bytes: usize,
) -> Result<(char, usize), Utf16LeDecodeError> {
    if byte_pos + 1 >= max_bytes {
        return Err(Utf16LeDecodeError::InvalidData);
    }
    let cu = u16::from_le_bytes([bytes[byte_pos], bytes[byte_pos + 1]]);
    let unit_index = byte_pos / 2;
    let max_units = max_bytes / 2;
    let (ch, consumed_units) = decode_utf16le_unit_value(bytes, cu, unit_index, max_units)?;
    Ok((ch, consumed_units * 2))
}

fn decode_utf16le_unit_value(
    bytes: &[u8],
    cu: u16,
    unit_index: usize,
    max_units: usize,
) -> Result<(char, usize), Utf16LeDecodeError> {
    match cu {
        0xD800..=0xDBFF => {
            let next_index = unit_index + 1;
            if next_index >= max_units {
                return Err(Utf16LeDecodeError::InvalidData);
            }
            let lo = bytes
                .get(next_index * 2)
                .copied()
                .ok_or(Utf16LeDecodeError::InvalidData)?;
            let hi = bytes
                .get(next_index * 2 + 1)
                .copied()
                .ok_or(Utf16LeDecodeError::InvalidData)?;
            let cu2 = u16::from_le_bytes([lo, hi]);
            if !(0xDC00..=0xDFFF).contains(&cu2) {
                return Err(Utf16LeDecodeError::InvalidData);
            }
            let high = (cu as u32) - 0xD800;
            let low = (cu2 as u32) - 0xDC00;
            let codepoint = 0x10000 + ((high << 10) | low);
            let ch = char::from_u32(codepoint).ok_or(Utf16LeDecodeError::InvalidData)?;
            Ok((ch, 2))
        }
        0xDC00..=0xDFFF => Err(Utf16LeDecodeError::InvalidData),
        _ => {
            let ch = char::from_u32(u32::from(cu)).ok_or(Utf16LeDecodeError::InvalidData)?;
            Ok((ch, 1))
        }
    }
}
