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
    /// Whether `str` is an XML name, checked once here: a chunk's records use each of its
    /// names many times over.
    is_xml_name: bool,
}

impl fmt::Display for BinXmlName {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.str)
    }
}

/// Whether `name` is an XML `Name`: what XML and JSON output need of an element or attribute
/// name, an entity reference, a processing instruction target or a JSON key, since they write
/// it as it is (XML has no way to escape a name). A name with a quote, a space or an angle
/// bracket in it would end its tag or its key early and have the rest read as markup.
/// Windows writes names that came from a manifest, which are XML names.
pub(crate) fn is_xml_name(name: &str) -> bool {
    // XML 1.0 (fifth edition), productions [4] NameStartChar and [4a] NameChar.
    fn starts_name(c: char) -> bool {
        matches!(c,
            ':' | 'A'..='Z' | '_' | 'a'..='z'
            | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
            | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
            | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
            | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
    }
    fn continues_name(c: char) -> bool {
        starts_name(c)
            || matches!(c,
                '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
    }
    // Nearly every name is ASCII, which a walk over its bytes settles.
    if name.is_ascii() {
        let starts = |b: u8| b.is_ascii_alphabetic() || b == b'_' || b == b':';
        let continues = |b: u8| starts(b) || b.is_ascii_digit() || b == b'-' || b == b'.';
        return match name.as_bytes() {
            [first, rest @ ..] => starts(*first) && rest.iter().all(|b| continues(*b)),
            [] => false,
        };
    }
    let mut chars = name.chars();
    chars.next().is_some_and(starts_name) && chars.all(continues_name)
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
        let is_xml_name = is_xml_name(&name);
        Ok(BinXmlName {
            str: name,
            is_xml_name,
        })
    }

    /// Whether the name is an XML name, which is what the renderers can write.
    pub(crate) fn is_xml_name(&self) -> bool {
        self.is_xml_name
    }

    pub fn as_str(&self) -> &str {
        &self.str
    }
}

#[cfg(test)]
mod tests {
    use super::is_xml_name;

    #[test]
    fn ascii_names_are_judged_as_any_other() {
        // The ASCII walk and the walk over characters, which a non-ASCII character after the
        // name under test selects, agree on every ASCII character first and second.
        for first in 0..128u8 {
            for second in 0..128u8 {
                let name = String::from_utf8(vec![first, second]).unwrap();
                assert_eq!(
                    is_xml_name(&name),
                    is_xml_name(&format!("{name}é")),
                    "{name:?}"
                );
            }
            let name = char::from(first).to_string();
            assert_eq!(
                is_xml_name(&name),
                is_xml_name(&format!("{name}é")),
                "{name:?}"
            );
        }
        assert!(!is_xml_name(""));
    }
}
