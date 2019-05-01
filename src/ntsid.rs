use crate::evtx_parser::ReadSeek;

use byteorder::BigEndian;
use byteorder::{LittleEndian, ReadBytesExt};
use std::fmt;
use std::fmt::Debug;
use std::fmt::Display;
use std::fmt::Write;
use std::io;

#[derive(PartialOrd, PartialEq, Clone)]
pub struct Sid {
    version: u8,
    number_of_elements: u8,
    id_high: u32,
    id_low: u16,
    elements: Vec<u32>,
}

impl Sid {
    pub fn from_stream<S: ReadSeek>(stream: &mut S) -> io::Result<Sid> {
        let version = stream.read_u8()?;
        let number_of_elements = stream.read_u8()?;
        // For some reason these values are kept in be order.
        let id_high = stream.read_u32::<BigEndian>()?;
        let id_low = stream.read_u16::<BigEndian>()?;

        let mut elements = Vec::with_capacity(number_of_elements as usize);

        for _ in 0..number_of_elements {
            elements.push(stream.read_u32::<LittleEndian>()?)
        }

        Ok(Sid {
            version,
            number_of_elements,
            id_high,
            id_low,
            elements,
        })
    }

    pub fn to_string(&self) -> String {
        let mut repr = String::new();

        write!(
            repr,
            "S-{}-{}",
            self.version,
            (self.id_high as u16) ^ (self.id_low),
        )
        .expect("Writing to a String cannot fail");

        for element in self.elements.iter() {
            write!(repr, "-{}", element).expect("Writing to a String cannot fail");
        }

        repr
    }
}

impl Display for Sid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Debug for Sid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}
