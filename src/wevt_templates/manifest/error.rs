use thiserror::Error;

#[derive(Debug, Error)]
pub enum WevtManifestError {
    #[error("invalid signature at offset {offset}: expected {expected:?}, got {found:?}")]
    InvalidSignature {
        offset: u32,
        expected: [u8; 4],
        found: [u8; 4],
    },

    #[error("buffer too small for {what} at offset {offset} (need {need} bytes, have {have})")]
    Truncated {
        what: &'static str,
        offset: u32,
        need: usize,
        have: usize,
    },

    #[error("offset {offset} out of bounds for {what} (len={len})")]
    OffsetOutOfBounds {
        what: &'static str,
        offset: u32,
        len: usize,
    },

    #[error("size {size} out of bounds for {what} at offset {offset}")]
    SizeOutOfBounds {
        what: &'static str,
        offset: u32,
        size: u32,
    },

    #[error("invalid count {count} for {what} at offset {offset}")]
    CountOutOfBounds {
        what: &'static str,
        offset: u32,
        count: u32,
    },

    #[error("invalid utf-16 string for {what} at offset {offset}")]
    InvalidUtf16String { what: &'static str, offset: u32 },

    #[error("invalid GUID for {what} at offset {offset}")]
    InvalidGuid { what: &'static str, offset: u32 },
}

pub(super) type Result<T> = std::result::Result<T, WevtManifestError>;
