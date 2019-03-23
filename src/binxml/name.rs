use crate::binxml::deserializer::ParsingContext;
use crate::error::Error;
use crate::evtx::ReadSeek;
use crate::utils::read_len_prefixed_utf16_string;
use crate::Offset;
use byteorder::LittleEndian;
use failure::Fail;
use log::trace;
use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};

#[derive(Debug, PartialEq, PartialOrd)]
pub struct BinXmlName<'a>(Cow<'a, str>);

pub type StringHashOffset = (String, u16, Offset);

impl<'a> BinXmlName<'a> {
    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        ctx: &ParsingContext,
    ) -> Result<Self, Error> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = try_read!(cursor, u32);

        // If name is cached, read it and seek ahead if needed.
        if let Some((name, _, n_bytes_read)) = ctx.cached_string_at_offset(name_offset) {
            // Seek if needed
            if name_offset == cursor.position() as u32 {
                cursor
                    .seek(SeekFrom::Current(*n_bytes_read as i64))
                    .map_err(|e| Error::io(e, cursor.position()))?;
            }
            return Ok(BinXmlName(Cow::Borrowed(name)));
        }

        let (name, _, _) = Self::from_stream_at_offset(cursor, name_offset)?;
        Ok(BinXmlName(Cow::Owned(name)))
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream(cursor: &mut Cursor<&[u8]>) -> Result<StringHashOffset, Error> {
        let position_before_read = cursor.position();

        let _ = try_read!(cursor, u32);
        let name_hash = try_read!(cursor, u16);
        let name = read_len_prefixed_utf16_string(cursor, true)
            .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
            .expect("Expected string");

        let position_after_read = cursor.position();

        Ok((name, name_hash, position_after_read - position_before_read))
    }

    /// Reads a `BinXmlName` from a given offset, seeks if needed.
    fn from_stream_at_offset(
        cursor: &mut Cursor<&[u8]>,
        offset: Offset,
    ) -> Result<StringHashOffset, Error> {
        if offset != cursor.position() as u32 {
            trace!(
                "Current offset {}, seeking to {}",
                cursor.position(),
                offset
            );
            let position_before_seek = cursor.position();
            cursor
                .seek(SeekFrom::Start(u64::from(offset)))
                .map_err(|e| Error::io(e, position_before_seek))?;

            let (name, hash, n_bytes_read) = Self::from_stream(cursor)?;

            trace!("Restoring cursor to {}", position_before_seek);
            cursor
                .seek(SeekFrom::Start(position_before_seek as u64))
                .map_err(|e| Error::io(e, position_before_seek))?;

            Ok((name, hash, n_bytes_read))
        } else {
            trace!("Name is at current offset");
            let (name, hash, n_bytes_read) = Self::from_stream(cursor)?;
            Ok((name, hash, n_bytes_read))
        }
    }
}
