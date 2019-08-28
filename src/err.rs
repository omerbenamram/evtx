use quick_xml;
use snafu::{Backtrace, ErrorCompat, Snafu};

use std::io;
use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Snafu)]
#[snafu(visibility(pub(crate)))]
pub enum Error {
    #[snafu(display(
        "Offset {}: An I/O error has occurred while trying to read {}: {}",
        offset,
        t,
        source
    ))]
    FailedToRead {
        offset: u64,
        t: String,
        source: std::io::Error,
        backtrace: Backtrace,
    },

    #[snafu(display("An I/O error has occurred: {}", source))]
    IO {
        source: std::io::Error,
        backtrace: Backtrace,
    },

    #[snafu(display("Invalid input path, cannot canonicalize: {}: {}", path, source))]
    InvalidInputPath {
        source: std::io::Error,
        // Not a path because it is invalid
        path: String,
    },
    #[snafu(display("Failed to open file {}: {}", path.display(), source))]
    FailedToOpenFile {
        source: std::io::Error,
        path: PathBuf,
    },

    /// Errors related to Deserialization

    #[snafu(display("Reached EOF while trying to allocate chunk {}", chunk_number))]
    IncompleteChunk { chunk_number: u16 },

    #[snafu(display(
        "Invalid EVTX record header magic, expected `2a2a0000`, found `{:2X?}`",
        magic
    ))]
    InvalidEvtxRecordHeaderMagic { magic: [u8; 4] },

    #[snafu(display(
        "Invalid EVTX chunk header magic, expected `ElfChnk0`, found `{:2X?}`",
        magic
    ))]
    InvalidEvtxChunkMagic { magic: [u8; 8] },

    #[snafu(display(
        "Invalid EVTX file header magic, expected `ElfFile0`, found `{:2X?}`",
        magic
    ))]
    InvalidEvtxFileHeaderMagic { magic: [u8; 8] },
    #[snafu(display("Unknown EVTX record header flags value: {}", value))]
    UnknownEvtxHeaderFlagValue { value: u32 },

    #[snafu(display("chunk data CRC32 invalid"))]
    InvalidChunkChecksum {},

    #[snafu(display(
        "Failed to deserialize record {}, caused by:\n\t {}",
        record_id,
        source
    ))]
    FailedToDeserializeRecord {
        record_id: u64,
        #[snafu(backtrace)]
        #[snafu(source(from(Error, Box::new)))]
        source: Box<Error>,
    },
    #[snafu(display(
        "Offset {}: Tried to read an invalid byte `{:x}` as binxml token",
        offset,
        value
    ))]
    InvalidToken { value: u8, offset: u64 },

    #[snafu(display(
        "Offset {}: Tried to read an invalid byte `{:x}` as binxml value variant",
        offset,
        value
    ))]
    InvalidValueVariant { value: u8, offset: u64 },

    #[snafu(display(
        "Offset {}: Value variant `{}` (size {:?}) is unimplemented",
        offset,
        name,
        size
    ))]
    UnimplementedValueVariant {
        name: String,
        size: Option<u16>,
        offset: u64,
    },

    #[snafu(display("Offset {}: Token `{}` is unimplemented", offset, name))]
    UnimplementedToken { name: String, offset: u64 },

    #[snafu(display(
        "Offset {}: Failed to decode UTF-16 string, caused by: {}",
        offset,
        source
    ))]
    FailedToDecodeUTF16String { source: std::io::Error, offset: u64 },

    #[snafu(display(
        "Offset {}: Failed to decode UTF-8 string, caused by: {}",
        offset,
        source
    ))]
    FailedToDecodeUTF8String {
        source: std::string::FromUtf8Error,
        offset: u64,
    },

    #[snafu(display(
        "Offset {}: Failed to decode ansi string (used encoding scheme {}), caused by: {}",
        offset,
        encoding,
        message
    ))]
    FailedToDecodeANSIString {
        encoding: &'static str,
        message: String,
        offset: u64,
    },

    #[snafu(display(
        "Offset {}: Failed to read windows time, caused by: {}",
        offset,
        source
    ))]
    FailedToReadWindowsTime {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[snafu(display("Offset {}: Failed to decode GUID, caused by: {}", offset, source))]
    FailedToReadGUID {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[snafu(display("Offset {}: Failed to decode NTSID, caused by: {}", offset, source))]
    FailedToReadNTSID {
        source: winstructs::err::Error,
        offset: u64,
    },

    #[snafu(display("Failed to create record model, reason: {}", message))]
    FailedToCreateRecordModel { message: String },

    /// Errors related to Serialization
    // Since `quick-xml` maintains the stack for us, structural errors with the XML
    // Will be included in this generic error alongside IO errors.
    #[snafu(display("Writing to XML failed with: {}", message))]
    XmlOutputError { message: String },

    #[snafu(display("Building a JSON document failed with message: {}", message,))]
    JsonStructureError { message: String },

    #[snafu(display("`serde_json` failed with error: {}", source))]
    JsonError { source: serde_json::error::Error },

    #[snafu(display("Record data contains invalid UTF-8: {}", source))]
    RecordContainsInvalidUTF8 { source: std::string::FromUtf8Error },

    /// Misc Errors
    #[snafu(display("Unimplemented: {}", name))]
    Unimplemented { name: String },
    #[snafu(display("An unexpected error has occurred: {}", detail))]
    Any { detail: String },
}

/// Generic error handler for quick prototyping, inspired by failure's `format_err!` macro.
#[macro_export]
macro_rules! format_err {
   ($($arg:tt)*) => { $crate::err::Any { detail: format!($($arg)*) }.fail() }
}

/// Errors on unimplemented functions instead on panicking.
#[macro_export]
macro_rules! unimplemented_fn {
   ($($arg:tt)*) => { $crate::err::Unimplemented { name: format!($($arg)*) }.fail() }
}

/// Adapter for `quick-xml` error type, which is implemented internally in `failure`,
/// and provides no easy way of producing std compatible `Error`
#[derive(Debug)]
pub struct QuickXmlError {
    message: String,
}

impl From<quick_xml::Error> for Error {
    fn from(err: quick_xml::Error) -> Self {
        Error::XmlOutputError {
            message: format!("{}", err),
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::IO {
            source: err,
            backtrace: Backtrace::new(),
        }
    }
}

pub fn dump_err_with_backtrace(err: &Error) {
    eprintln!("{}", err);

    if let Some(bt) = err.backtrace() {
        eprintln!("{}", bt);
    }
}
