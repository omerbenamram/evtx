#![deny(unused_must_use)]
#![cfg_attr(backtraces, feature(backtrace))]
#![forbid(unsafe_code)]
// Don't allow dbg! prints in release.
#![cfg_attr(not(debug_assertions), deny(clippy::dbg_macro))]
// This needs to come first!
#[macro_use]
mod macros;

pub use evtx_chunk::{EvtxChunk, EvtxChunkData, EvtxChunkHeader, IterChunkRecords};
pub use evtx_parser::{EvtxParser, IntoIterChunks, IterChunks, ParserSettings};
pub use evtx_record::{EvtxRecord, EvtxRecordHeader, SerializedEvtxRecord};
pub use json_output::JsonOutput;
pub use xml_output::{BinXmlOutput, XmlOutput};

pub mod binxml;
pub mod err;
pub mod model;

mod evtx_chunk;
mod evtx_file_header;
mod evtx_parser;
mod evtx_record;
mod string_cache;
mod template_cache;
mod utils;

mod json_output;
mod xml_output;

pub type Offset = u32;

// For tests, we only initialize logging once.
#[cfg(test)]
use std::sync::Once;

#[cfg(test)]
static LOGGER_INIT: Once = Once::new();

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
#[cfg(test)]
pub fn ensure_env_logger_initialized() {
    LOGGER_INIT.call_once(env_logger::init);
}
