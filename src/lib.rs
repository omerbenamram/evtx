//! Public entry points for parsing Windows EVTX files.
//!
//! The main API surface is [`EvtxParser`], which can open an EVTX file or in-memory
//! buffer, iterate chunks, and produce records as structured IR, XML, or JSON.
//! Most consumers only need:
//!
//! - [`EvtxParser`] to open and stream records
//! - [`ParserSettings`] to control formatting and parsing behavior
//! - [`SerializedEvtxRecord`] for XML/JSON output
//! - [`EvtxRecord`] when working with the intermediate representation directly
//!
//! Lower-level modules such as [`binxml`], [`err`], and [`model`] are also exposed
//! for callers that need direct access to BinXML values or the internal record tree.
//!
#![deny(unused_must_use)]
#![forbid(unsafe_code)]
#![allow(clippy::upper_case_acronyms)]
// Don't allow dbg! prints in release.
#![cfg_attr(not(debug_assertions), deny(clippy::dbg_macro))]
#[macro_use]
extern crate bitflags;

pub use evtx_chunk::{EvtxChunk, EvtxChunkData, EvtxChunkHeader, IterChunkRecords};
pub use evtx_file_header::{EvtxFileHeader, HeaderFlags};
pub use evtx_parser::{EvtxParser, IntoIterChunks, IterChunks, ParserSettings};
pub use evtx_record::{EvtxRecord, EvtxRecordHeader, RecordId, SerializedEvtxRecord};
pub use utils::utf16::{Utf16LeDecodeError, Utf16LeSlice};

/// BinXML token/value types and rendering helpers.
pub mod binxml;
/// Error types returned while reading, parsing, and serializing EVTX data.
pub mod err;
/// Intermediate representation (IR) types produced by the BinXML parser.
pub mod model;

// Optional: PE resource parsing to extract WEVT_TEMPLATE blobs (see issue #103).
#[cfg(feature = "wevt_templates")]
pub mod wevt_templates;

mod evtx_chunk;
mod evtx_file_header;
mod evtx_parser;
mod evtx_record;
mod string_cache;
mod utils;

/// Byte offset relative to the start of an EVTX chunk.
pub type ChunkOffset = u32;
/// Byte offset relative to the start of the EVTX file.
pub type FileOffset = u64;

// For tests, we only initialize logging once.
#[cfg(test)]
use std::sync::Once;

#[cfg(test)]
static LOGGER_INIT: Once = Once::new();

use crc32fast::Hasher;

/// Compute the IEEE CRC-32 checksum used by EVTX headers and chunks.
#[inline]
pub fn checksum_ieee(data: &[u8]) -> u32 {
    let mut hasher = Hasher::new();
    hasher.update(data);
    hasher.finalize()
}

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
#[cfg(test)]
pub fn ensure_env_logger_initialized() {
    use std::io::Write;

    LOGGER_INIT.call_once(|| {
        let mut builder = env_logger::Builder::from_default_env();
        builder
            .format(|buf, record| writeln!(buf, "[{}] - {}", record.level(), record.args()))
            .init();
    });
}

// Cannot use `cfg(test)` here since `rustdoc` won't look at it.
#[cfg(debug_assertions)]
mod test_readme {
    macro_rules! calculated_doc {
        ($doc:expr_2021, $id:ident) => {
            #[doc = $doc]
            enum $id {}
        };
    }

    calculated_doc!(include_str!("../README.md"), _DoctestReadme);
}
