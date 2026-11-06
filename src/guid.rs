//! GUIDs, as EVTX stores them for templates and `WEVT_TEMPLATE` manifests.

use std::fmt::{self, Display};
use std::io::{self, Read};

/// A GUID in its Windows layout: three little-endian integers followed by 8 bytes.
///
/// <https://learn.microsoft.com/en-us/windows/win32/api/guiddef/ns-guiddef-guid>
///
/// # Example
///
/// ```
/// # use evtx::Guid;
/// let raw_guid: &[u8] = &[0x25, 0x96, 0x84, 0x54, 0x78, 0x54, 0x94, 0x49,
///                         0xa5, 0xba, 0x3e, 0x3b, 0x3, 0x28, 0xc3, 0xd];
///
/// let guid = Guid::from_buffer(raw_guid).unwrap();
///
/// assert_eq!(guid.to_string(), "54849625-5478-4994-A5BA-3E3B0328C30D");
/// ```
#[derive(PartialOrd, Ord, PartialEq, Eq, Hash, Clone, Copy, Debug)]
pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    /// Creates a GUID from its components.
    pub fn new(data1: u32, data2: u16, data3: u16, data4: [u8; 8]) -> Guid {
        Guid {
            data1,
            data2,
            data3,
            data4,
        }
    }

    /// Creates a GUID from its 16 on-disk bytes.
    pub fn from_bytes(bytes: [u8; 16]) -> Guid {
        let [a0, a1, a2, a3, b0, b1, c0, c1, data4 @ ..] = bytes;
        Guid::new(
            u32::from_le_bytes([a0, a1, a2, a3]),
            u16::from_le_bytes([b0, b1]),
            u16::from_le_bytes([c0, c1]),
            data4,
        )
    }

    /// Reads a GUID from the first 16 bytes of `buffer`.
    pub fn from_buffer(buffer: &[u8]) -> io::Result<Guid> {
        let bytes = buffer
            .first_chunk::<16>()
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
        Ok(Guid::from_bytes(*bytes))
    }

    /// Reads a GUID from `stream`.
    pub fn from_reader<T: Read>(stream: &mut T) -> io::Result<Guid> {
        let mut bytes = [0; 16];
        stream.read_exact(&mut bytes)?;
        Ok(Guid::from_bytes(bytes))
    }
}

impl Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let d = &self.data4;
        write!(
            f,
            "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.data1, self.data2, self.data3, d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::Guid;

    #[test]
    fn short_buffer_is_an_error() {
        assert!(Guid::from_buffer(&[0; 15]).is_err());
    }

    #[test]
    fn reader_matches_buffer() {
        let raw: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            Guid::from_reader(&mut &raw[..]).unwrap(),
            Guid::from_bytes(raw)
        );
        assert_eq!(
            Guid::from_bytes(raw).to_string(),
            "03020100-0504-0706-0809-0A0B0C0D0E0F"
        );
    }
}
