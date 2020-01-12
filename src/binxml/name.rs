use crate::err::DeserializationResult as Result;

use crate::ChunkOffset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::utils::read_len_prefixed_utf16_string;

use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};

use crate::evtx_chunk::EvtxChunk;
use log::trace;
use quick_xml::events::{BytesEnd, BytesStart};
use serde::export::Formatter;
use std::fmt;

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct BinXmlName<'a> {
    str: Cow<'a, str>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlNameRef {
    pub offset: ChunkOffset,
}

impl<'a> fmt::Display for BinXmlName<'a> {
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

impl<'a> BinXmlName<'a> {
    #[cfg(test)]
    pub(crate) fn from_str(s: &'a str) -> Self {
        BinXmlName {
            str: Cow::Borrowed(s),
        }
    }

    #[cfg(test)]
    pub(crate) fn from_string(s: String) -> Self {
        BinXmlName { str: Cow::Owned(s) }
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream(cursor: &mut Cursor<&'a [u8]>) -> Result<Self> {
        let name = try_read!(cursor, len_prefixed_utf_16_str_nul_terminated, "name")?
            .unwrap_or(Cow::Borrowed(""));

        Ok(BinXmlName { str: name })
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }
}

impl<'a> Into<quick_xml::events::BytesStart<'a>> for &'a BinXmlName<'a> {
    fn into(self) -> BytesStart<'a> {
        BytesStart::borrowed_name(self.as_str().as_bytes())
    }
}

impl<'a> Into<quick_xml::events::BytesEnd<'a>> for BinXmlName<'a> {
    fn into(self) -> BytesEnd<'a> {
        let inner = self.as_str().as_bytes();
        BytesEnd::owned(inner.to_vec())
    }
}
