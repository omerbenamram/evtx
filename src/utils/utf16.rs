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

    /// Borrow the code units preceding the first UTF-16 NUL, if present.
    pub(crate) fn until_nul(bytes: &'a [u8]) -> Self {
        let units = bytes.as_chunks::<2>().0;
        let count = units
            .iter()
            .position(|unit| *unit == [0, 0])
            .unwrap_or(units.len());
        Self::new(bytes, count)
    }

    /// Create an empty UTF-16LE slice.
    pub(crate) fn empty() -> Self {
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
        let mut out = String::with_capacity(self.as_bytes().len() / 2);
        decode_utf16le(self.as_bytes(), false, |ch| out.push(ch))?;
        Ok(out)
    }
}

/// The one UTF-16LE decoder: push each char of `bytes` (stopping at the first
/// NUL when `until_nul`). Odd length is `OddLength`, unpaired surrogates are
/// `InvalidData`.
fn decode_utf16le(
    bytes: &[u8],
    until_nul: bool,
    mut push: impl FnMut(char),
) -> Result<(), Utf16LeDecodeError> {
    if !bytes.len().is_multiple_of(2) {
        return Err(Utf16LeDecodeError::OddLength);
    }
    let units = bytes
        .chunks_exact(2)
        .map(|u| u16::from_le_bytes([u[0], u[1]]));
    for ch in char::decode_utf16(units) {
        match ch.map_err(|_| Utf16LeDecodeError::InvalidData)? {
            '\0' if until_nul => break,
            ch => push(ch),
        }
    }
    Ok(())
}

/// Decode up to `num_chars` UTF-16LE code units (stopping at NUL) into a
/// bump-allocated UTF-8 string slice.
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
    let limit = num_chars.min(bytes.len() / 2);
    let mut out = BumpString::with_capacity_in(limit, bump);
    decode_utf16le(&bytes[..limit * 2], true, |ch| out.push(ch))?;
    Ok(out.into_bump_str())
}

/// Decode a UTF-16LE byte slice until the first NUL (0x0000), if present.
#[cfg(feature = "wevt_templates")]
pub(crate) fn decode_utf16le_bytes_z(bytes: &[u8]) -> Result<String, Utf16LeDecodeError> {
    let mut out = String::with_capacity(bytes.len() / 2);
    decode_utf16le(bytes, true, |ch| out.push(ch))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::{Utf16LeDecodeError as E, Utf16LeSlice, decode_utf16le_bytes_to_bump_str};

    #[test]
    fn decode_rejects_unpaired_surrogates_and_stops_at_nul() {
        let bump = bumpalo::Bump::new();
        let le = |u: &[u16]| u.iter().flat_map(|u| u.to_le_bytes()).collect::<Vec<u8>>();
        let bump_str = |u: &[u16]| decode_utf16le_bytes_to_bump_str(&le(u), u.len(), &bump);
        let owned = |u: &[u16]| Utf16LeSlice::new(&le(u), u.len()).to_string();
        assert_eq!(
            bump_str(&[0x61, 0xD83D, 0xDE00, 0, 0xD800]),
            Ok("a\u{1F600}")
        );
        assert_eq!(owned(&[0x61, 0, 0x62]).as_deref(), Ok("a\0b"));
        for bad in [&[0xD800][..], &[0xDC00], &[0xD800, 0], &[0xD800, 0x61]] {
            assert_eq!(bump_str(bad), Err(E::InvalidData));
            assert_eq!(owned(bad), Err(E::InvalidData));
        }
        assert_eq!(
            decode_utf16le_bytes_to_bump_str(&[0x61, 0, 0x62], 1, &bump),
            Err(E::OddLength)
        );
    }

    #[test]
    fn nul_search_preserves_code_units_on_unaligned_input() {
        for fill in [1u16, 0x100, 0x7fff, 0x8000, 0xd800, 0xffff] {
            for len in 0..=33 {
                for nul in 0..=len {
                    let mut bytes = vec![0xaa]; // unaligned slice start
                    for i in 0..len {
                        bytes.extend_from_slice(&if i == nul { 0 } else { fill }.to_le_bytes());
                    }
                    let bytes = &bytes[1..];
                    let actual = Utf16LeSlice::until_nul(bytes);
                    assert_eq!(
                        actual.num_chars(),
                        nul,
                        "fill {fill:#x}, len {len}, NUL {nul}"
                    );
                    assert_eq!(actual.as_bytes(), &bytes[..nul * 2]);
                }
            }
        }
    }
}
