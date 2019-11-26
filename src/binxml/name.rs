use crate::err::{DeserializationResult as Result, WrappedIoError};

use crate::ChunkOffset;
pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::utils::read_len_prefixed_utf16_string;

use log::trace;
use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};

use crate::evtx_chunk::EvtxChunk;
use quick_xml::events::{BytesEnd, BytesStart};

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct BinXmlName<'a>(pub Cow<'a, str>);

pub type StringHashOffset = (String, u16, ChunkOffset);

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
        let name_offset = try_read!(cursor, u32, "name_offset")?;

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

        let _ = try_read!(cursor, u32)?;
        let name_hash = try_read!(cursor, u16, "name_hash")?;
        let name = try_read!(cursor, len_prefixed_utf_16_str_nul_terminated, "name")?
            .unwrap_or(Cow::Borrowed(""));

        let position_after_read = cursor.position();

        Ok((
            name.to_string(),
            name_hash,
            (position_after_read - position_before_read) as ChunkOffset,
        ))
    }

    /// Reads a `BinXmlName` from a given offset, seeks if needed.
    fn from_stream_at_offset(
        cursor: &mut Cursor<&'a [u8]>,
        offset: ChunkOffset,
    ) -> Result<StringHashOffset> {
        trace!(
            "Offset {} - Reading name at offset {}.",
            cursor.position(),
            offset
        );

        if offset != cursor.position() as u32 {
            trace!("Seeking to {}", offset);

            let position_before_seek = cursor.position();
            // TODO: Seeking would usually fail here, so we need to dump the context at the original offset.
            cursor
                .seek(SeekFrom::Start(u64::from(offset)))
                .map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        format!("failed to seek when reading name"),
                        cursor,
                    )
                })?;

            let (name, hash, n_bytes_read) = Self::from_stream(cursor)?;

            trace!("Restoring cursor to {}", position_before_seek);
            cursor
                .seek(SeekFrom::Start(u64::from(position_before_seek)))
                .map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        format!("failed to seek when reading name"),
                        cursor,
                    )
                })?;

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
