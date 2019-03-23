#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![feature(seek_convenience)]

pub mod evtx;

// This needs to come first!
#[macro_use]
mod macros;

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
use std::sync::{Once, ONCE_INIT};

static LOGGER_INIT: Once = ONCE_INIT;

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
pub fn ensure_env_logger_initialized() {
    LOGGER_INIT.call_once(|| env_logger::init());
}
