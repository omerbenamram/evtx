use winstructs::guid::Guid;

use super::error::{Result, WevtManifestError};
use crate::utils::bytes;

pub(super) fn read_sized_utf16_string(
    buf: &[u8],
    offset: u32,
    what: &'static str,
) -> Result<String> {
    let off_usize = u32_to_usize(offset, what, buf.len())?;
    require_len(buf, off_usize, 4, what)?;
    let size = read_u32_named(buf, off_usize, what)?;
    if size < 4 {
        return Err(size_err(what, offset, size));
    }
    let size_usize = size as usize;
    require_len(buf, off_usize, size_usize, what)?;
    let bytes = &buf[off_usize + 4..off_usize + size_usize];
    decode_utf16_z(bytes, what, offset)
}

pub(super) fn decode_utf16_z(bytes: &[u8], what: &'static str, offset: u32) -> Result<String> {
    crate::utils::decode_utf16le_bytes_z(bytes)
        .map_err(|_| WevtManifestError::InvalidUtf16String { what, offset })
}

pub(super) fn size_err(what: &'static str, offset: u32, size: u32) -> WevtManifestError {
    WevtManifestError::SizeOutOfBounds { what, offset, size }
}

pub(super) fn off_err(what: &'static str, offset: u32, len: usize) -> WevtManifestError {
    WevtManifestError::OffsetOutOfBounds { what, offset, len }
}

pub(super) fn trunc_err(
    what: &'static str,
    offset: u32,
    need: usize,
    have: usize,
) -> WevtManifestError {
    WevtManifestError::Truncated {
        what,
        offset,
        need,
        have,
    }
}

fn truncated(buf: &[u8], offset: usize, need: usize, what: &'static str) -> WevtManifestError {
    trunc_err(
        what,
        usize_to_u32(offset),
        need,
        buf.len().saturating_sub(offset),
    )
}

pub(super) fn read_sig_named(buf: &[u8], offset: usize, what: &'static str) -> Result<[u8; 4]> {
    bytes::read_sig(buf, offset).ok_or_else(|| truncated(buf, offset, 4, what))
}

pub(super) fn read_u8_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u8> {
    buf.get(offset)
        .copied()
        .ok_or_else(|| truncated(buf, offset, 1, what))
}

pub(super) fn read_u16_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u16> {
    bytes::read_u16_le(buf, offset).ok_or_else(|| truncated(buf, offset, 2, what))
}

pub(super) fn read_u32_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u32> {
    bytes::read_u32_le(buf, offset).ok_or_else(|| truncated(buf, offset, 4, what))
}

pub(super) fn read_u64_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u64> {
    bytes::read_u64_le(buf, offset).ok_or_else(|| truncated(buf, offset, 8, what))
}

pub(super) fn read_guid_named(buf: &[u8], offset: usize, what: &'static str) -> Result<Guid> {
    let bytes =
        bytes::read_array::<16>(buf, offset).ok_or_else(|| truncated(buf, offset, 16, what))?;
    Guid::from_buffer(&bytes).map_err(|_| WevtManifestError::InvalidGuid {
        what,
        offset: usize_to_u32(offset),
    })
}

pub(super) fn u32_to_usize(offset: u32, what: &'static str, len: usize) -> Result<usize> {
    let off = offset as usize;
    if off > len {
        return Err(off_err(what, offset, len));
    }
    Ok(off)
}

pub(super) fn usize_to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

pub(super) fn require_len(buf: &[u8], off: usize, need: usize, what: &'static str) -> Result<()> {
    if off > buf.len() || buf.len().saturating_sub(off) < need {
        return Err(truncated(buf, off, need, what));
    }
    Ok(())
}

pub(super) fn checked_end(len: usize, off: u32, size: u32, what: &'static str) -> Result<usize> {
    let start = u32_to_usize(off, what, len)?;
    // `start <= len`, so this comparison cannot overflow on any target.
    if size as usize > len - start {
        return Err(size_err(what, off, size));
    }
    Ok(start + size as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oversized_region_is_a_size_error() {
        let err = checked_end(64, 40, u32::MAX, "TEMP.size").unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::SizeOutOfBounds {
                what: "TEMP.size",
                offset: 40,
                size: u32::MAX,
            }
        ));
        assert_eq!(checked_end(64, 40, 24, "TEMP.size").unwrap(), 64);
    }
}
