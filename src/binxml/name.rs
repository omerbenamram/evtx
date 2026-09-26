use crate::err::DeserializationError;
use crate::err::DeserializationResult as Result;

use crate::ChunkOffset;
use crate::utils::ByteCursor;

use std::fmt::Formatter;

use std::fmt;

const WEVT_INLINE_NAME_HASH_MULTIPLIER: u32 = 65599;

/// Size of the `next_string` (u32) + `hash` (u16) link before a string-table name.
pub(crate) const NAME_LINK_SIZE: u32 = 6;

#[derive(Debug, PartialEq, Eq, PartialOrd, Clone, Hash)]
pub struct BinXmlName {
    str: String,
}

impl fmt::Display for BinXmlName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str)
    }
}

/// Read a string-table link, returning the next string's offset (if any).
pub(crate) fn read_name_link(cursor: &mut ByteCursor<'_>) -> Result<Option<ChunkOffset>> {
    let next_string = cursor.u32()?;
    let _hash = cursor.u16_named("name_hash")?;
    Ok((next_string > 0).then_some(next_string))
}

/// Read a name reference and return the name's offset in the cursor's buffer.
///
/// EVTX chunk streams reference the chunk string table by offset. WEVT_TEMPLATE / CRIM 5.x
/// streams (`inline`) store names in place as `u16 name_hash` + `u16 char_count` +
/// `UTF-16LE chars` + `u16 NUL` (MS-EVEN6: `Name = NameHash NameNumChars NullTerminatedUnicodeString`).
pub(crate) fn read_name_ref(cursor: &mut ByteCursor<'_>, inline: bool) -> Result<ChunkOffset> {
    if inline {
        read_wevt_inline_name(cursor)
    } else {
        read_offset_name_ref(cursor)
    }
}

fn read_offset_name_ref(cursor: &mut ByteCursor<'_>) -> Result<ChunkOffset> {
    let name_offset = cursor.u32_named("name_offset")?;

    let position_before_string = cursor.position();
    // The string may be stored inline right after the reference; skip it.
    if position_before_string == u64::from(name_offset) {
        let _ = read_name_link(cursor)?;
        let len = cursor.u16_named("string_table_name_len")?;

        let nul_terminator_len = 4;
        // `len * 2` in u16 overflows for len > 32767: a panic under overflow checks, a silent
        // wrap otherwise, which would leave the cursor misplaced. Widen before multiplying.
        let data_size = NAME_LINK_SIZE + u32::from(len) * 2 + nul_terminator_len;

        cursor.set_pos_u64(position_before_string + u64::from(data_size), "Skip string")?;
    }

    Ok(name_offset)
}

fn read_wevt_inline_name(cursor: &mut ByteCursor<'_>) -> Result<ChunkOffset> {
    let name_offset = cursor.position() as ChunkOffset;
    let stored_hash = cursor.u16_named("wevt_inline_name_hash")?;
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

    Ok(name_offset)
}

impl BinXmlName {
    pub(crate) fn from_cursor(cursor: &mut ByteCursor<'_>) -> Result<Self> {
        let name = cursor
            .len_prefixed_utf16_string_utf8(true, "name")?
            .unwrap_or_default();
        Ok(BinXmlName { str: name })
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }
}
