use crate::err::{EvtxError, Result};
use crate::evtx_parser::ReadSeek;

pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::utils::read_len_prefixed_utf16_string;
use crate::Offset;

use log::trace;
use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};

use crate::evtx_chunk::EvtxChunk;
use quick_xml::events::{BytesEnd, BytesStart};

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct BinXmlName<'a>(pub Cow<'a, str>);

pub type StringHashOffset = (String, u16, Offset);

impl<'a> BinXmlName<'a> {
    pub fn from_static_string(s: &'static str) -> Self {
        BinXmlName(Cow::Borrowed(s))
    }

    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
    ) -> Result<BinXmlName<'a>> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = try_read!(cursor, u32);

        // If name is cached, read it and seek ahead if needed.
        if let Some((name, _, n_bytes_read)) =
            chunk.and_then(|chunk| chunk.string_cache.get_string_and_hash(name_offset))
        {
            // Seek if needed
            if name_offset == cursor.position() as u32 {
                cursor.seek(SeekFrom::Current(i64::from(*n_bytes_read)))?;
            }
            return Ok(BinXmlName(Cow::Borrowed(name)));
        }

        let (name, _, _) = Self::from_stream_at_offset(cursor, name_offset)?;
        Ok(BinXmlName(Cow::Owned(name)))
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream(cursor: &mut Cursor<&'a [u8]>) -> Result<StringHashOffset> {
        let position_before_read = cursor.position();

        let _ = try_read!(cursor, u32);
        let name_hash = try_read!(cursor, u16);
        let name = read_len_prefixed_utf16_string(cursor, true)
            .map_err(|e| EvtxError::FailedToDecodeUTF16String {
                source: e,
                offset: cursor.position(),
            })?
            // If string is None, just fill in a new string
            .unwrap_or_else(String::new);

        let position_after_read = cursor.position();

        Ok((
            name,
            name_hash,
            (position_after_read - position_before_read) as Offset,
        ))
    }

    /// Reads a `BinXmlName` from a given offset, seeks if needed.
    fn from_stream_at_offset(
        cursor: &mut Cursor<&'a [u8]>,
        offset: Offset,
    ) -> Result<StringHashOffset> {
        if offset != cursor.position() as u32 {
            trace!(
                "Current offset {}, seeking to {}",
                cursor.position(),
                offset
            );

            let position_before_seek = cursor.position();
            cursor.seek(SeekFrom::Start(u64::from(offset)))?;

            let (name, hash, n_bytes_read) = Self::from_stream(cursor)?;

            trace!("Restoring cursor to {}", position_before_seek);
            cursor.seek(SeekFrom::Start(position_before_seek as u64))?;

            Ok((name, hash, n_bytes_read))
        } else {
            trace!("Name is at current offset");
            let (name, hash, n_bytes_read) = Self::from_stream(cursor)?;
            Ok((name, hash, n_bytes_read))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'a> Into<quick_xml::events::BytesStart<'a>> for &'a BinXmlName<'a> {
    fn into(self) -> BytesStart<'a> {
        BytesStart::borrowed_name(self.0.as_bytes())
    }
}

impl<'a> Into<quick_xml::events::BytesEnd<'a>> for BinXmlName<'a> {
    fn into(self) -> BytesEnd<'a> {
        let inner = self.0.as_bytes();
        BytesEnd::owned(inner.to_vec())
    }
}
