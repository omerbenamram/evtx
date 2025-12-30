//! UTF-16LE utilities used by BinXML parsing and IR rendering.
//!
//! The EVTX BinXML stream stores most text as UTF-16LE. Decoding those strings
//! eagerly into UTF-8 creates allocator churn and extra copies. This module
//! keeps UTF-16LE slices borrowed from the underlying chunk and provides:
//! - validation and trimming helpers for UTF-16LE payloads,
//! - on-demand UTF-8 decoding for legacy paths.

use bumpalo::Bump;
use bumpalo::collections::String as BumpString;

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

    /// Create an empty UTF-16LE slice.
    pub fn empty() -> Self {
        Utf16LeSlice {
            bytes: &[],
            num_chars: 0,
        }
    }

    /// Returns the UTF-16LE byte slice truncated to `num_chars`.
    pub fn as_bytes(&self) -> &'a [u8] {
        let max = self.num_chars.saturating_mul(2).min(self.bytes.len());
        &self.bytes[..max]
    }

    /// Returns the number of UTF-16 code units in this slice.
    pub fn num_chars(&self) -> usize {
        self.num_chars
    }

    /// Returns true if the slice contains no UTF-16 code units.
    pub fn is_empty(&self) -> bool {
        self.num_chars == 0 || self.bytes.is_empty()
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

/// Decode UTF-16LE bytes into a bump-allocated UTF-8 string slice.
#[allow(dead_code)]
pub(crate) fn decode_utf16le_bytes_to_bump_str<'a>(
    bytes: &[u8],
    num_chars: usize,
    bump: &'a Bump,
) -> Result<&'a str, Utf16LeDecodeError> {
    if bytes.is_empty() || num_chars == 0 {
        return Ok("");
    }

    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }

    let max_units = bytes.len() / 2;
    let limit = num_chars.min(max_units);
    if limit == 0 {
        return Ok("");
    }

    let mut out = BumpString::with_capacity_in(limit, bump);
    let mut unit_index = 0usize;
    while unit_index < limit {
        let (ch, consumed) = decode_utf16le_at_unit(bytes, unit_index, limit)?;
        if ch == '\u{0}' {
            break;
        }
        out.push(ch);
        unit_index += consumed;
    }

    Ok(out.into_bump_str())
}

/// Decode a UTF-16LE byte slice until the first NUL (0x0000), if present.
#[cfg(feature = "wevt_templates")]
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
#[cfg(feature = "wevt_templates")]
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
