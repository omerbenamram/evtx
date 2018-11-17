#![feature(nll)]
#![feature(try_from)]
#![feature(box_syntax)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]


extern crate byteorder;
extern crate xml;
#[macro_use]
extern crate failure;

#[macro_use]
extern crate log;
extern crate env_logger;

#[cfg(test)]
#[macro_use]
extern crate maplit;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

#[cfg(test)]
extern crate itertools;

extern crate chrono;
extern crate crc;
extern crate encoding;
extern crate time;

mod binxml;
mod guid;
mod utils;
mod evtx_file_header;
mod model;
mod evtx_chunk;
mod xml_builder;
mod evtx_record;
pub mod evtx;
