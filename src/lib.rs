#![deny(unused_must_use)]
#![cfg_attr(backtraces, feature(backtrace))]
#![forbid(unsafe_code)]
#![allow(clippy::upper_case_acronyms)]
// Don't allow dbg! prints in release.
#![cfg_attr(not(debug_assertions), deny(clippy::dbg_macro))]
// This needs to come first!
#[macro_use]
mod macros;

#[macro_use]
extern crate bitflags;

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

pub type ChunkOffset = u32;
pub type FileOffset = u64;

// For tests, we only initialize logging once.
#[cfg(test)]
use std::sync::Once;

#[cfg(test)]
static LOGGER_INIT: Once = Once::new();

use crc32fast::Hasher;

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
        ($doc:expr, $id:ident) => {
            #[doc = $doc]
            enum $id {}
        };
    }

    calculated_doc!(include_str!("../README.md"), _DoctestReadme);
}
