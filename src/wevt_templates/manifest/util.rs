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
        return Err(WevtManifestError::SizeOutOfBounds { what, offset, size });
    }
    let size_usize = usize::try_from(size).map_err(|_| WevtManifestError::SizeOutOfBounds {
        what,
        offset,
        size,
    })?;
    require_len(buf, off_usize, size_usize, what)?;
    let bytes = &buf[off_usize + 4..off_usize + size_usize];
    decode_utf16_z(bytes, what, offset)
}

pub(super) fn decode_utf16_z(bytes: &[u8], what: &'static str, offset: u32) -> Result<String> {
    crate::utils::decode_utf16le_bytes_z(bytes)
        .map_err(|_| WevtManifestError::InvalidUtf16String { what, offset })
}

pub(super) fn read_sig_named(buf: &[u8], offset: usize, what: &'static str) -> Result<[u8; 4]> {
    bytes::read_sig(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 4,
        have: buf.len().saturating_sub(offset),
    })
}

pub(super) fn read_u8_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u8> {
    bytes::read_u8(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 1,
        have: buf.len().saturating_sub(offset),
    })
}

pub(super) fn read_u16_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u16> {
    bytes::read_u16_le(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 2,
        have: buf.len().saturating_sub(offset),
    })
}

pub(super) fn read_u32_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u32> {
    bytes::read_u32_le(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 4,
        have: buf.len().saturating_sub(offset),
    })
}

pub(super) fn read_u64_named(buf: &[u8], offset: usize, what: &'static str) -> Result<u64> {
    bytes::read_u64_le(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 8,
        have: buf.len().saturating_sub(offset),
    })
}

pub(super) fn read_guid_named(buf: &[u8], offset: usize, what: &'static str) -> Result<Guid> {
    let bytes = bytes::read_array::<16>(buf, offset).ok_or(WevtManifestError::Truncated {
        what,
        offset: usize_to_u32(offset),
        need: 16,
        have: buf.len().saturating_sub(offset),
    })?;
    Guid::from_buffer(&bytes).map_err(|_| WevtManifestError::InvalidGuid {
        what,
        offset: usize_to_u32(offset),
    })
}

pub(super) fn u32_to_usize(offset: u32, what: &'static str, len: usize) -> Result<usize> {
    let off = usize::try_from(offset).map_err(|_| WevtManifestError::OffsetOutOfBounds {
        what,
        offset,
        len,
    })?;
    if off > len {
        return Err(WevtManifestError::OffsetOutOfBounds { what, offset, len });
    }
    Ok(off)
}

pub(super) fn usize_to_u32(v: usize) -> u32 {
    u32::try_from(v).unwrap_or(u32::MAX)
}

pub(super) fn require_len(buf: &[u8], off: usize, need: usize, what: &'static str) -> Result<()> {
    if off > buf.len() || buf.len().saturating_sub(off) < need {
        return Err(WevtManifestError::Truncated {
            what,
            offset: usize_to_u32(off),
            need,
            have: buf.len().saturating_sub(off),
        });
    }
    Ok(())
}

pub(super) fn checked_end(len: usize, off: u32, size: u32, what: &'static str) -> Result<usize> {
    let off_usize = u32_to_usize(off, what, len)?;
    let size_usize = usize::try_from(size).map_err(|_| WevtManifestError::SizeOutOfBounds {
        what,
        offset: off,
        size,
    })?;
    let end = off_usize
        .checked_add(size_usize)
        .ok_or(WevtManifestError::SizeOutOfBounds {
            what,
            offset: off,
            size,
        })?;
    if end > len {
        return Err(WevtManifestError::SizeOutOfBounds {
            what,
            offset: off,
            size,
        });
    }
    Ok(end)
}
