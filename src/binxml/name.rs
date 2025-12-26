use crate::err::DeserializationError;
use crate::err::DeserializationResult as Result;

use crate::ChunkOffset;
use crate::utils::ByteCursor;

use std::{fmt::Formatter, io::Cursor};

use quick_xml::events::{BytesEnd, BytesStart};
use std::fmt;

const WEVT_INLINE_NAME_HASH_MULTIPLIER: u32 = 65599;

/// MS-EVEN6 NameHash (low 16 bits of: hash=0; for each UTF-16 code unit: hash = hash*65599 + code_unit).
#[cfg(test)]
pub(crate) fn compute_wevt_inline_name_hash_utf16(
    code_units: impl IntoIterator<Item = u16>,
) -> u16 {
    let mut hash: u32 = 0;
    for cu in code_units {
        hash = hash
            .wrapping_mul(WEVT_INLINE_NAME_HASH_MULTIPLIER)
            .wrapping_add(u32::from(cu));
    }
    (hash & 0xffff) as u16
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Clone, Hash)]
pub struct BinXmlName {
    str: String,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BinXmlNameEncoding {
    /// Standard EVTX encoding where names are referenced by offsets into the chunk string table.
    Offset,
    /// WEVT_TEMPLATE / CRIM 5.x encoding where names are stored inline as:
    /// `u16 name_hash` + `u16 char_count` + `UTF-16LE chars` + `u16 NUL`.
    ///
    /// Primary reference: MS-EVEN6 (`Name = NameHash NameNumChars NullTerminatedUnicodeString`).
    WevtInline,
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
    pub(crate) fn from_cursor(cursor: &mut ByteCursor<'_>) -> Result<Self> {
        let next_string = cursor.u32()?;
        let name_hash = cursor.u16_named("name_hash")?;

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
    pub(crate) fn from_cursor(cursor: &mut ByteCursor<'_>) -> Result<Self> {
        let name_offset = cursor.u32_named("name_offset")?;

        let position_before_string = cursor.position();
        let need_to_seek = position_before_string == u64::from(name_offset);

        if need_to_seek {
            let _ = BinXmlNameLink::from_cursor(cursor)?;
            let len = cursor.u16_named("string_table_name_len")?;

            let nul_terminator_len = 4;
            let data_size = BinXmlNameLink::data_size() + u32::from(len * 2) + nul_terminator_len;

            cursor.set_pos_u64(position_before_string + u64::from(data_size), "Skip string")?;
        }

        Ok(BinXmlNameRef {
            offset: name_offset,
        })
    }

    pub fn from_stream(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_cursor(&mut c)?;
        cursor.set_position(c.position());
        Ok(v)
    }

    pub fn from_stream_with_encoding(
        cursor: &mut Cursor<&[u8]>,
        encoding: BinXmlNameEncoding,
    ) -> Result<Self> {
        match encoding {
            BinXmlNameEncoding::Offset => Self::from_stream(cursor),
            BinXmlNameEncoding::WevtInline => Self::from_stream_wevt_inline(cursor),
        }
    }

    pub(crate) fn from_cursor_with_encoding(
        cursor: &mut ByteCursor<'_>,
        encoding: BinXmlNameEncoding,
    ) -> Result<Self> {
        match encoding {
            BinXmlNameEncoding::Offset => Self::from_cursor(cursor),
            BinXmlNameEncoding::WevtInline => Self::from_cursor_wevt_inline(cursor),
        }
    }

    fn from_cursor_wevt_inline(cursor: &mut ByteCursor<'_>) -> Result<Self> {
        let name_offset = cursor.position() as ChunkOffset;
        let stored_hash = cursor.u16_named("wevt_inline_name_hash")?;
        // character count
        let char_count = cursor.u16_named("wevt_inline_name_character_count")?;

        let mut hash: u32 = 0;
        for _ in 0..char_count {
            let code_unit = cursor.u16_named("wevt_inline_name_code_unit")?;
            hash = hash
                .wrapping_mul(WEVT_INLINE_NAME_HASH_MULTIPLIER)
                .wrapping_add(u32::from(code_unit));
        }

        let nul = cursor.u16_named("wevt_inline_name_nul")?;
        if nul != 0 {
            return Err(DeserializationError::WevtInlineNameMissingNulTerminator {
                found: nul,
                offset: u64::from(name_offset),
            });
        }

        let expected_hash = (hash & 0xffff) as u16;
        if stored_hash != expected_hash {
            return Err(DeserializationError::WevtInlineNameHashMismatch {
                expected: expected_hash,
                found: stored_hash,
                offset: u64::from(name_offset),
            });
        }

        Ok(BinXmlNameRef {
            offset: name_offset,
        })
    }

    fn from_stream_wevt_inline(cursor: &mut Cursor<&[u8]>) -> Result<Self> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_cursor_wevt_inline(&mut c)?;
        cursor.set_position(c.position());
        Ok(v)
    }
}

/// Resolve a WEVT inline name at the given offset.
///
/// The offset should point to the start of the inline name structure, i.e. the `name_hash` field.
#[cfg(any(test, feature = "wevt_templates"))]
pub(crate) fn read_wevt_inline_name_at(data: &[u8], offset: ChunkOffset) -> Result<BinXmlName> {
    let mut cursor = ByteCursor::with_pos(data, offset as usize)?;
    let _ = cursor.u16_named("wevt_inline_name_hash")?;
    let name = cursor
        .len_prefixed_utf16_string(true, "wevt_inline_name")?
        .unwrap_or_default();
    Ok(BinXmlName { str: name })
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
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_cursor(&mut c)?;
        cursor.set_position(c.position());
        Ok(v)
    }

    pub(crate) fn from_cursor(cursor: &mut ByteCursor<'_>) -> Result<Self> {
        let name = cursor
            .len_prefixed_utf16_string(true, "name")?
            .unwrap_or_default();
        Ok(BinXmlName { str: name })
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }
}

impl<'a> From<&'a BinXmlName> for quick_xml::events::BytesStart<'a> {
    fn from(name: &'a BinXmlName) -> Self {
        BytesStart::new(name.as_str())
    }
}

impl<'a> From<&'a BinXmlName> for quick_xml::events::BytesEnd<'a> {
    fn from(name: &'a BinXmlName) -> Self {
        BytesEnd::new(name.as_str())
    }
}
