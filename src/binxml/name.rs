use crate::err::DeserializationResult as Result;

use crate::ChunkOffset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::utils::read_len_prefixed_utf16_string;

use std::{
    fmt::Formatter,
    io::{Cursor, Seek, SeekFrom},
};

use quick_xml::events::{BytesEnd, BytesStart};
use std::fmt;

#[derive(Debug, PartialEq, Eq, PartialOrd, Clone, Hash)]
pub struct BinXmlName {
    str: String,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Hash)]
pub struct BinXmlNameRef {
    pub offset: ChunkOffset,
}

impl fmt::Display for BinXmlName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str)
    }
}

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub(crate) struct BinXmlNameLink {
    pub next_string: Option<ChunkOffset>,
    pub hash: u16,
}

impl BinXmlNameLink {
    pub fn from_stream(stream: &mut Cursor<&[u8]>) -> Result<Self> {
        let next_string = try_read!(stream, u32)?;
        let name_hash = try_read!(stream, u16, "name_hash")?;

        Ok(BinXmlNameLink {
            next_string: if next_string > 0 {
                Some(next_string)
            } else {
                None
            },
            hash: name_hash,
        })
    }

    pub fn data_size() -> u32 {
        6
    }
}

impl BinXmlNameRef {
    pub fn from_stream(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let name_offset = try_read!(cursor, u32, "name_offset")?;

        let position_before_string = cursor.position();
        let need_to_seek = position_before_string == u64::from(name_offset);

        if need_to_seek {
            let _ = BinXmlNameLink::from_stream(cursor)?;
            let len = cursor.read_u16::<LittleEndian>()?;

            let nul_terminator_len = 4;
            let data_size = BinXmlNameLink::data_size() + u32::from(len * 2) + nul_terminator_len;

            try_seek!(
                cursor,
                position_before_string + u64::from(data_size),
                "Skip string"
            )?;
        }

        Ok(BinXmlNameRef {
            offset: name_offset,
        })
    }
}

impl BinXmlName {
    #[cfg(test)]
    pub(crate) fn from_str(s: &str) -> Self {
        BinXmlName { str: s.to_string() }
    }

    #[cfg(test)]
    pub(crate) fn from_string(s: String) -> Self {
        BinXmlName { str: s }
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let name = try_read!(cursor, len_prefixed_utf_16_str_nul_terminated, "name")?
            .unwrap_or_else(|| "".to_string());

        Ok(BinXmlName { str: name })
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }
}

impl<'a> From<&'a BinXmlName> for quick_xml::events::BytesStart<'a> {
    fn from(name: &'a BinXmlName) -> Self {
        BytesStart::borrowed_name(name.as_str().as_bytes())
    }
}

impl<'a> From<BinXmlName> for quick_xml::events::BytesEnd<'a> {
    fn from(name: BinXmlName) -> Self {
        let inner = name.as_str().as_bytes();
        BytesEnd::owned(inner.to_vec())
    }
}
