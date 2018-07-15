#![feature(try_from)]
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

#[macro_use]
extern crate nom;

extern crate indextree;

#[macro_use]
extern crate log;
extern crate env_logger;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

extern crate chrono;
extern crate crc;
extern crate encoding;
extern crate time;

#[macro_use]
extern crate enum_primitive_derive;
extern crate core;
extern crate num_traits;

mod binxml;
pub mod evtx_parser;
mod hexdump;
