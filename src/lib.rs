// This needs to come first!
#[macro_use]
mod macros;

pub mod evtx_parser;
pub use evtx_parser::EvtxParser;
pub use evtx_record::EvtxRecord;

mod binxml;
mod error;
mod evtx_chunk;
mod evtx_file_header;
mod evtx_record;
mod guid;
mod model;
mod ntsid;
mod string_cache;
mod template_cache;
mod utils;

mod xml_output;

pub type Offset = u32;

// For tests, we only initialize logging once.
#[cfg(test)]
use std::sync::{Once, ONCE_INIT};

#[cfg(test)]
static LOGGER_INIT: Once = ONCE_INIT;

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
#[cfg(test)]
pub fn ensure_env_logger_initialized() {
    LOGGER_INIT.call_once(env_logger::init);
}
