pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::ParsingContext;
use crate::error::Error;
use crate::evtx::ReadSeek;
use crate::utils::read_len_prefixed_utf16_string;
use crate::Offset;
use core::borrow::Borrow;
use failure::Fail;
use log::trace;
use std::borrow::Cow;
use std::io::{Cursor, Seek, SeekFrom};
use std::rc::Rc;
use xml::name::Name;

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct BinXmlName<'a>(Cow<'a, str>);

pub type StringHashOffset = (String, u16, Offset);

impl<'a, 'b: 'a> BinXmlName<'a> {
    pub fn from_binxml_stream<T: ReadSeek + 'b>(
        cursor: &'a mut T,
        ctx: Rc<ParsingContext<'a, 'b>>,
    ) -> Result<Self, Error> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = try_read!(cursor, u32);

        // If name is cached, read it and seek ahead if needed.
        if let Some((name, _, n_bytes_read)) = ctx.cached_string_at_offset(name_offset) {
            // Seek if needed
            if name_offset == cursor.stream_position()? as u32 {
                cursor.seek(SeekFrom::Current(*n_bytes_read as i64))?;
            }
            return Ok(BinXmlName(Cow::Borrowed(name)));
        }

        let (name, _, _) = Self::from_stream_at_offset(cursor, name_offset)?;
        Ok(BinXmlName(Cow::Owned(name)))
    }

    /// Reads a tuple of (String, Hash, Offset) from a stream.
    pub fn from_stream<T: ReadSeek + 'b>(cursor: &mut T) -> Result<StringHashOffset, Error> {
        let position_before_read = cursor.stream_position()?;

        let _ = try_read!(cursor, u32);
        let name_hash = try_read!(cursor, u16);
        let name = read_len_prefixed_utf16_string(cursor, true)
            .map_err(|e| {
                Error::utf16_decode_error(e, cursor.stream_position().expect("Failed to tell"))
            })?
            .expect("Expected string");

        let position_after_read = cursor.stream_position()?;

        Ok((
            name,
            name_hash,
            (position_after_read - position_before_read) as Offset,
        ))
    }

    /// Reads a `BinXmlName` from a given offset, seeks if needed.
    fn from_stream_at_offset<T: ReadSeek + 'b>(
        cursor: &mut T,
        offset: Offset,
    ) -> Result<StringHashOffset, Error> {
        if offset != cursor.stream_position()? as u32 {
            trace!(
                "Current offset {}, seeking to {}",
                cursor.stream_position()?,
                offset
            );
            let position_before_seek = cursor.stream_position()?;
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
}

impl<'a> Into<xml::name::Name<'a>> for &'a BinXmlName<'a> {
    fn into(self) -> Name<'a> {
        Name::from(self.0.borrow())
    }
}
