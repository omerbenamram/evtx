use quick_xml;
#[cfg(backtraces)]
use std::backtrace::Backtrace;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, EvtxError>;

#[derive(Debug, Error)]
pub enum EvtxError {
    #[error("Offset {offset}: An I/O error has occurred while trying to read {t}")]
    FailedToRead {
        offset: u64,
        t: &'static str,
        source: std::io::Error,
        #[cfg(backtraces)]
        backtrace: Backtrace,
    },

    #[error("An I/O error has occurred")]
    IO {
        #[from]
        source: std::io::Error,
        #[cfg(backtraces)]
        backtrace: Backtrace,
    },

    #[error("Failed to access path: {}", path)]
    InvalidInputPath {
        source: std::io::Error,
        // Not a path because it is invalid
        path: String,
    },

    #[error("Failed to open file {}", path.display())]
    FailedToOpenFile {
        source: std::io::Error,
        path: std::path::PathBuf,
    },

    /// Errors related to Deserialization
    #[error("Reached EOF while trying to allocate chunk {chunk_number}")]
    IncompleteChunk { chunk_number: u16 },

    #[error("Invalid EVTX record header magic, expected `2a2a0000`, found `{magic:2X?}`")]
    InvalidEvtxRecordHeaderMagic { magic: [u8; 4] },

    #[error("Invalid EVTX chunk header magic, expected `ElfChnk0`, found `{magic:2X?}`")]
    InvalidEvtxChunkMagic { magic: [u8; 8] },

    #[error("Invalid EVTX file header magic, expected `ElfFile0`, found `{magic:2X?}`")]
    InvalidEvtxFileHeaderMagic { magic: [u8; 8] },

    #[error("Unknown EVTX record header flags value: {value}")]
    UnknownEvtxHeaderFlagValue { value: u32 },

    #[error("chunk data CRC32 invalid")]
    InvalidChunkChecksum {},

    #[error("Failed to deserialize record {record_id}")]
    FailedToDeserializeRecord {
        record_id: u64,
        source: Box<EvtxError>,
    },

    #[error("Offset {offset}: Tried to read an invalid byte `{value:x}` as binxml token")]
    InvalidToken { value: u8, offset: u64 },

    #[error("Offset {offset}: Tried to read an invalid byte `{value:x}` as binxml value variant")]
    InvalidValueVariant { value: u8, offset: u64 },

    #[error("Offset {offset}: Value variant `{name}` (size {size:?}) is unimplemented")]
    UnimplementedValueVariant {
        name: String,
        size: Option<u16>,
        offset: u64,
    },

    #[error("Offset {offset}: Token `{name}` is unimplemented")]
    UnimplementedToken { name: &'static str, offset: u64 },

    #[error("Offset {offset}: Failed to decode UTF-16 string")]
    FailedToDecodeUTF16String { source: std::io::Error, offset: u64 },

    #[error("Offset {offset}: Failed to decode UTF-8 string")]
    FailedToDecodeUTF8String {
        source: std::string::FromUtf8Error,
        offset: u64,
    },

    #[error("Offset {offset}: Failed to decode ansi string (used encoding scheme {encoding}), failed with: {message}")]
    FailedToDecodeANSIString {
        encoding: &'static str,
        message: String,
        offset: u64,
    },

    #[error("Offset {offset}: Failed to read windows time")]
    FailedToReadWindowsTime {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[error("Offset {offset}: Failed to decode GUID")]
    FailedToReadGUID {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[error("Offset {offset}: Failed to decode NTSID")]
    FailedToReadNTSID {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[error("Failed to create record model, reason: {message}")]
    FailedToCreateRecordModel { message: &'static str },

    /// Errors related to Serialization
    // Since `quick-xml` maintains the stack for us, structural errors with the XML
    // Will be included in this generic error alongside IO errors.
    #[error("Writing to XML failed")]
    XmlOutputError {
        #[from]
        source: quick_xml::Error,
    },

    #[error("Building a JSON document failed with message: {message}")]
    JsonStructureError { message: &'static str },

    #[error("`serde_json` failed")]
    JsonError {
        #[from]
        source: serde_json::error::Error,
    },

    #[error("Record data contains invalid UTF-8")]
    RecordContainsInvalidUTF8 {
        #[from]
        source: std::string::FromUtf8Error,
    },

    /// Misc Errors
    #[error("Unimplemented: {name}")]
    Unimplemented { name: String },
    #[error("An unexpected error has occurred: {detail}")]
    Any { detail: String },
}

/// Generic error handler for quick prototyping, inspired by failure's `format_err!` macro.
#[macro_export]
macro_rules! format_err {
   ($($arg:tt)*) => { $crate::err::EvtxError::Any { detail: format!($($arg)*) } }
}

/// Errors on unimplemented functions instead on panicking.
#[macro_export]
macro_rules! unimplemented_fn {
   ($($arg:tt)*) => { Err($crate::err::EvtxError::Unimplemented { name: format!($($arg)*) }) }
}
