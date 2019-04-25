use std::fmt::{self, Debug, Display};
use std::io;
use std::fmt::Write;
use crate::evtx_parser::ReadSeek;
use byteorder::{LittleEndian, ReadBytesExt};

#[derive(PartialOrd, PartialEq, Clone)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    pub fn new(data1: u32, data2: u16, data3: u16, data4: &[u8]) -> Guid {
        let mut data4_owned = [0; 8];
        data4_owned.clone_from_slice(&data4[0..8]);
        Guid {
            data1,
            data2,
            data3,
            data4: data4_owned,
        }
    }

    pub fn from_stream<T: ReadSeek>(stream: &mut T) -> io::Result<Guid> {
        let data1 = stream.read_u32::<LittleEndian>()?;
        let data2 = stream.read_u16::<LittleEndian>()?;
        let data3 = stream.read_u16::<LittleEndian>()?;
        let mut data4 = [0; 8];
        stream.read_exact(&mut data4)?;
        Ok(Guid::new(data1, data2, data3, &data4))
    }

    pub fn to_string(&self) -> String {
        // Using `format!` will extend the string multiple time,
        // but we know ahead of time how much space we need.
        let mut s = String::with_capacity(63);

        write!(
            &mut s,
            "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7]
        ).expect("writing to a preallocated buffer cannot fail");

        s
    }
}

impl Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}
