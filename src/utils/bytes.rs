//! Byte-slice utilities for bounds-oriented parsing.
//!
//! This module is intentionally tiny and *boring*: it provides a consistent, well-documented way
//! to read little-endian primitives out of `&[u8]` at fixed offsets, with minimal overhead.
//!
//! There are two layers:
//! - **Option layer** (`read_*`): zero-cost helpers that return `Option<T>`.
//!   Use these when you want to map failures to your own error type (e.g. WEVT parsing).
//! - **Result layer** (`*_r`): wrappers that map `None` to `DeserializationError::Truncated`.
//!   Use these for EVTX parsing where `DeserializationError` is the canonical error type.
//!
//! Design notes:
//! - All numeric reads are **little-endian** (EVTX/WEVT data is LE).
//! - Offsets are `usize` and are interpreted relative to the slice you pass in.
//! - Prefer a single up-front bounds check with [`slice_r`] when parsing fixed-size structs.
//!
//! Example (fixed-size header parsing):
//!
//! ```ignore
//! use crate::utils::bytes;
//!
//! // Ensure the struct is present, then read fields by fixed offsets.
//! let _ = bytes::slice_r(buf, 0, 128, "EVTX file header")?;
//! let magic = bytes::read_array_r::<8>(buf, 0, "file header magic")?;
//! let flags = bytes::read_u32_le_r(buf, 120, "file header flags")?;
//! ```

use crate::err::DeserializationError;

/// Read `N` raw bytes at `offset`.
///
/// Returns `None` if the range is out of bounds.
pub(crate) fn read_array<const N: usize>(buf: &[u8], offset: usize) -> Option<[u8; N]> {
    let end = offset.checked_add(N)?;
    let bytes: [u8; N] = buf.get(offset..end)?.try_into().ok()?;
    Some(bytes)
}

/// Read a single byte at `offset`.
pub(crate) fn read_u8(buf: &[u8], offset: usize) -> Option<u8> {
    buf.get(offset).copied()
}

/// Read a 4-byte signature at `offset` (e.g. `b\"ElfChnk\\0\"[..4]` style).
pub(crate) fn read_sig(buf: &[u8], offset: usize) -> Option<[u8; 4]> {
    read_array::<4>(buf, offset)
}

/// Read a `u16` (little-endian) at `offset`.
pub(crate) fn read_u16_le(buf: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(read_array::<2>(buf, offset)?))
}

/// Read a `u32` (little-endian) at `offset`.
pub(crate) fn read_u32_le(buf: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(read_array::<4>(buf, offset)?))
}

/// Read a `u64` (little-endian) at `offset`.
pub(crate) fn read_u64_le(buf: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(read_array::<8>(buf, offset)?))
}

#[inline]
fn truncated(what: &'static str, offset: usize, need: usize, len: usize) -> DeserializationError {
    DeserializationError::Truncated {
        what,
        offset: offset as u64,
        need,
        have: len.saturating_sub(offset),
    }
}

pub(crate) fn slice_r<'a>(
    buf: &'a [u8],
    offset: usize,
    len: usize,
    what: &'static str,
) -> Result<&'a [u8], DeserializationError> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| truncated(what, offset, len, buf.len()))?;
    buf.get(offset..end)
        .ok_or_else(|| truncated(what, offset, len, buf.len()))
}

/// Read `N` raw bytes at `offset`, or return `DeserializationError::Truncated`.
pub(crate) fn read_array_r<const N: usize>(
    buf: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<[u8; N], DeserializationError> {
    read_array::<N>(buf, offset).ok_or_else(|| truncated(what, offset, N, buf.len()))
}

/// Read a `u16` (little-endian) at `offset`, or return `DeserializationError::Truncated`.
pub(crate) fn read_u16_le_r(
    buf: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<u16, DeserializationError> {
    read_u16_le(buf, offset).ok_or_else(|| truncated(what, offset, 2, buf.len()))
}

/// Read a `u32` (little-endian) at `offset`, or return `DeserializationError::Truncated`.
pub(crate) fn read_u32_le_r(
    buf: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<u32, DeserializationError> {
    read_u32_le(buf, offset).ok_or_else(|| truncated(what, offset, 4, buf.len()))
}

/// Read a `u64` (little-endian) at `offset`, or return `DeserializationError::Truncated`.
pub(crate) fn read_u64_le_r(
    buf: &[u8],
    offset: usize,
    what: &'static str,
) -> Result<u64, DeserializationError> {
    read_u64_le(buf, offset).ok_or_else(|| truncated(what, offset, 8, buf.len()))
}

/// Read a `count`-element `u32` (little-endian) table at `offset`.
///
/// This does a single bounds check for the whole table and then reads each element.
pub(crate) fn read_u32_vec_le_r(
    buf: &[u8],
    offset: usize,
    count: usize,
    what: &'static str,
) -> Result<Vec<u32>, DeserializationError> {
    // Fast fail on table bounds, then read each entry without re-checking offset math overflow.
    let bytes = count
        .checked_mul(4)
        .ok_or_else(|| truncated(what, offset, usize::MAX, buf.len()))?;
    let _ = slice_r(buf, offset, bytes, what)?;

    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        out.push(read_u32_le_r(buf, offset + i * 4, what)?);
    }
    Ok(out)
}
