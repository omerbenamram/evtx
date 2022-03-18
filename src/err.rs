use thiserror::Error;

use crate::evtx_parser::ReadSeek;

use crate::utils::dump_stream;
use crate::FileOffset;
use log::error;

use crate::evtx_record::RecordId;
use std::error::Error as StdError;
use std::io;
use std::path::Path;
use winstructs::guid::Guid;

/// This is the only `Result` type that should be exposed on public interfaces.
pub type Result<T> = std::result::Result<T, EvtxError>;
pub type SerializationResult<T> = std::result::Result<T, crate::err::SerializationError>;
pub(crate) type DeserializationResult<T> = std::result::Result<T, crate::err::DeserializationError>;
pub(crate) type EvtxChunkResult<T> = std::result::Result<T, crate::err::ChunkError>;

/// How many bytes of context we capture on error by default.
const DEFAULT_LOOKBEHIND_LEN: i32 = 100;

/// An IO error which captures additional information about it's context (hexdump).
#[derive(Error, Debug)]
#[error(
    "Offset `0x{offset:08x} ({offset})` - An error has occurred while trying to deserialize binary stream \n\
    {message}

    Original message:
    `{source}`

Hexdump:
    {hexdump}"
)]
pub struct WrappedIoError {
    offset: FileOffset,
    // A hexdump containing information additional information surrounding the token.
    hexdump: String,
    // A message containing extra context.
    message: String,
    // Could be either an I/O error or some other error such as `FromUtf8Error`
    #[source]
    source: Box<dyn StdError + 'static + Send + Sync>,
}

impl WrappedIoError {
    pub fn capture_hexdump<S: ReadSeek>(
        error: Box<(dyn std::error::Error + 'static + Send + Sync)>,
        stream: &mut S,
    ) -> WrappedIoError {
        let offset = stream.tell().unwrap_or_else(|_| {
            error!("while trying to recover error information -> `tell` failed.");
            0
        });

        let hexdump = dump_stream(stream, DEFAULT_LOOKBEHIND_LEN)
            .unwrap_or_else(|_| "<Error while capturing hexdump>".to_string());

        WrappedIoError {
            offset,
            hexdump,
            message: "".to_string(),
            source: error,
        }
    }

    pub fn io_error_with_message<S: ReadSeek, T: AsRef<str>>(
        error: io::Error,
        context: T,
        stream: &mut S,
    ) -> WrappedIoError {
        let offset = stream.tell().unwrap_or_else(|_| {
            error!("while trying to recover error information -> `tell` failed.");
            0
        });

        let hexdump = dump_stream(stream, DEFAULT_LOOKBEHIND_LEN)
            .unwrap_or_else(|_| "<Error while capturing hexdump>".to_string());

        WrappedIoError {
            offset,
            hexdump,
            message: context.as_ref().to_string(),
            source: Box::new(error),
        }
    }
}

#[derive(Debug, Error)]
pub enum DeserializationError {
    /// Represents a general deserialization error.
    /// Includes information about what token was being deserialized, as well an offset and an underlying error.
    #[error("Failed to deserialize `{token_name}` of type `{t}`")]
    FailedToReadToken {
        // Could be anything from a `u32` to an array of strings.
        t: String,
        token_name: &'static str,
        source: WrappedIoError,
    },

    #[error("An expected I/O error has occurred")]
    UnexpectedIoError(#[from] WrappedIoError),

    #[error("An expected I/O error has occurred")]
    RemoveMe(#[from] io::Error),

    /// An extra layer of error indirection to keep template GUID.
    #[error("Failed to deserialize template `{template_id}`")]
    FailedToDeserializeTemplate {
        template_id: Guid,
        source: Box<DeserializationError>,
    },

    /// While decoding ANSI strings, we might get an incorrect decoder, which will yield a special message.
    #[error("Failed to decode ANSI string (encoding used: {encoding_used}) - `{inner_message}`")]
    AnsiDecodeError {
        encoding_used: &'static str,
        inner_message: String,
    },

    #[error(
        "Offset 0x{offset:08x}: Tried to read an invalid byte `0x{value:02x}` as binxml token"
    )]
    InvalidToken { value: u8, offset: u64 },

    #[error(
        "Offset 0x{offset:08x}: Tried to read an invalid byte `0x{value:2x}` as binxml value variant"
    )]
    InvalidValueVariant { value: u8, offset: u64 },

    #[error("An out-of-range date, invalid month and/or day")]
    InvalidDateTimeError,

    /// Assertion errors.
    #[error("Invalid EVTX record header magic, expected `2a2a0000`, found `{magic:2X?}`")]
    InvalidEvtxRecordHeaderMagic { magic: [u8; 4] },

    #[error("Invalid EVTX chunk header magic, expected `ElfChnk0`, found `{magic:2X?}`")]
    InvalidEvtxChunkMagic { magic: [u8; 8] },

    #[error("Invalid EVTX file header magic, expected `ElfFile0`, found `{magic:2X?}`")]
    InvalidEvtxFileHeaderMagic { magic: [u8; 8] },

    #[error("Unknown EVTX record header flags value: {value}")]
    UnknownEvtxHeaderFlagValue { value: u32 },

    /// Unimplemented Tokens/Variants.
    #[error("Offset {offset}: Token `{name}` is unimplemented")]
    UnimplementedToken { name: &'static str, offset: u64 },

    #[error("Offset {offset}: Value variant `{name}` (size {size:?}) is unimplemented")]
    UnimplementedValueVariant {
        name: String,
        size: Option<u16>,
        offset: u64,
    },
}

// TODO: this should be pub(crate), but we need to make `BinXmlOutput` private to do that.
/// Errors related to Serialization of Binxml token trees to XML/JSON.
#[derive(Debug, Error)]
pub enum SerializationError {
    // Since `quick-xml` maintains the stack for us, structural errors with the XML
    // Will be included in this generic error alongside IO errors.
    #[error("Writing to XML failed")]
    XmlOutputError {
        #[from]
        source: quick_xml::Error,
    },

    #[error("Building a JSON document failed with message: {message}")]
    JsonStructureError { message: String },

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

    #[error("Unimplemented: {message}")]
    Unimplemented { message: String },
}

#[derive(Debug, Error)]
pub enum InputError {
    #[error("Failed to open file {}", path.display())]
    FailedToOpenFile {
        source: std::io::Error,
        path: std::path::PathBuf,
    },
}

impl InputError {
    /// Context Convenience for `InputError`
    pub fn failed_to_open_file<P: AsRef<Path>>(source: io::Error, path: P) -> Self {
        InputError::FailedToOpenFile {
            source,
            path: path.as_ref().to_path_buf(),
        }
    }
}

/// Raised on Invalid/Incomplete data
/// May also be raised if common chunk resources are not read succesfully.
#[derive(Debug, Error)]
pub enum ChunkError {
    #[error("Reached EOF while trying to allocate chunk")]
    IncompleteChunk,

    #[error("Failed to seek to start of chunk.")]
    FailedToSeekToChunk(io::Error),

    #[error("Failed to parse chunk header")]
    FailedToParseChunkHeader(#[from] DeserializationError),

    #[error("chunk data CRC32 invalid")]
    InvalidChunkChecksum { expected: u32, found: u32 },

    #[error("Failed to build string cache")]
    FailedToBuildStringCache { source: DeserializationError },

    #[error("Failed to build template cache")]
    FailedToBuildTemplateCache {
        message: String,
        source: DeserializationError,
    },
}

/// Public result API.
/// Inner errors are considered implementation details and are opaque.
#[derive(Debug, Error)]
pub enum EvtxError {
    #[error("An error occurred while trying to read input.")]
    InputError(#[from] InputError),

    #[error("An error occurred while trying to serialize binary xml to output.")]
    SerializationError(#[from] SerializationError),

    // TODO: Should this be split to `ChunkError` vs `RecordError`?
    #[error("An error occurred while trying to deserialize evtx stream.")]
    DeserializationError(#[from] DeserializationError),

    #[error("Failed to parse chunk number {chunk_id}")]
    FailedToParseChunk { chunk_id: u64, source: ChunkError },

    #[error("Failed to parse record number {record_id}")]
    FailedToParseRecord {
        record_id: RecordId,
        source: Box<EvtxError>,
    },

    #[error("Calculation Error, reason: {}", .0)]
    CalculationError(String),

    #[error("An IO error occured.")]
    IoError(#[from] std::io::Error),

    // TODO: move this error.
    #[error("Failed to create record model, reason: {}", .0)]
    FailedToCreateRecordModel(&'static str),

    // TODO: should we keep an `Unimplemented` variant at public API?
    #[error("Unimplemented: {name}")]
    Unimplemented { name: String },
}

impl EvtxError {
    pub fn calculation_error(msg: String) -> EvtxError {
        EvtxError::CalculationError(msg)
    }

    pub fn incomplete_chunk(chunk_id: u64) -> EvtxError {
        EvtxError::FailedToParseChunk {
            chunk_id,
            source: ChunkError::IncompleteChunk,
        }
    }
}


/// Errors on unimplemented functions instead on panicking.
#[macro_export]
macro_rules! unimplemented_fn {
   ($($arg:tt)*) => { Err($crate::err::EvtxError::Unimplemented { name: format!($($arg)*) }) }
}
