use crate::err::DeserializationResult as Result;

use crate::ChunkOffset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::utils::read_len_prefixed_utf16_string;

use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};

use crate::evtx_chunk::EvtxChunk;
use quick_xml::events::{BytesEnd, BytesStart};

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct BinXmlName<'a> {
    str: Cow<'a, str>,
    data_size: u32,
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

    pub fn data_size() -> ChunkOffset {
        12
    }
}

impl<'a> BinXmlName<'a> {
    #[cfg(test)]
    pub(crate) fn from_str(s: &'a str) -> Self {
        BinXmlName {
            str: Cow::Borrowed(s),
            data_size: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn from_string(s: String) -> Self {
        BinXmlName {
            str: Cow::Owned(s),
            data_size: 0,
        }
    }

    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
    ) -> Result<Self> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = try_read!(cursor, u32, "name_offset")?;

        // If name is cached, read it and seek ahead if needed.
        if let Some(name) =
            chunk.and_then(|chunk| chunk.string_cache.get_cached_string(name_offset))
        {
            // Seek if needed
            let position_after_string = cursor.position() + u64::from(name.data_size);
            try_seek!(cursor, position_after_string, "Skip cached string")?;

            // This doesn't actually clone the string, just the reference and the `data_size`.
            Ok(name.clone())
        } else {
            Self::from_stream(cursor)
        }
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream(cursor: &mut Cursor<&'a [u8]>) -> Result<Self> {
        let position_before_read = cursor.position();

        let name = try_read!(cursor, len_prefixed_utf_16_str_nul_terminated, "name")?
            .unwrap_or(Cow::Borrowed(""));

        let position_after_read = cursor.position();
        let data_size = (position_after_read - position_before_read) as ChunkOffset
            + BinXmlNameLink::data_size();

        Ok(BinXmlName {
            str: name,
            data_size,
        })
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
