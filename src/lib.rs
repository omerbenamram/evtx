#![feature(try_from)]

#[macro_use]
extern crate nom;

#[macro_use]
extern crate hex_literal;

#[cfg(test)]
#[macro_use]
extern crate pretty_assertions;

extern crate chrono;
extern crate crc;
extern crate encoding;
extern crate time;

extern crate html5ever;
extern crate xml;
extern crate xml5ever;

#[macro_use]
extern crate enum_primitive_derive;
extern crate num_traits;

mod binxml;
pub mod evtx_parser;
mod hexdump;
