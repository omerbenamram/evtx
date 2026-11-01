use crate::err::{DeserializationError, DeserializationResult as Result, Result as EvtxResult};

use winstructs::guid::Guid;

use crate::AnsiCodec;
use crate::ChunkOffset;
use crate::binxml::name::read_name_ref;
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};
use crate::evtx_chunk::EvtxChunk;
use crate::utils::{ByteCursor, Utf16LeSlice};

use log::{error, trace, warn};

use bumpalo::Bump;

/// Where a BinXML stream lives and how to decode it.
///
/// Offsets inside the stream are relative to `data`: `chunk.data` for EVTX chunk streams, the
/// fragment blob for WEVT_TEMPLATE streams. `chunk` is `None` exactly for WEVT_TEMPLATE
/// streams, which also means names are stored inline rather than in the chunk string table.
#[derive(Clone, Copy)]
pub(crate) struct Stream<'a> {
    pub(crate) data: &'a [u8],
    pub(crate) chunk: Option<&'a EvtxChunk<'a>>,
    pub(crate) ansi_codec: AnsiCodec,
    pub(crate) bump: &'a Bump,
}

impl<'a> Stream<'a> {
    pub(crate) fn chunk(chunk: &'a EvtxChunk<'a>, bump: &'a Bump) -> Self {
        Stream {
            data: chunk.data,
            chunk: Some(chunk),
            ansi_codec: chunk.settings.get_ansi_codec(),
            bump,
        }
    }

    #[cfg(feature = "wevt_templates")]
    pub(crate) fn wevt(data: &'a [u8], ansi_codec: AnsiCodec, bump: &'a Bump) -> Self {
        Stream {
            data,
            chunk: None,
            ansi_codec,
            bump,
        }
    }
}

/// Receives the tokens [`walk_tokens`] parses. Every token defaults to being ignored.
pub(crate) trait TokenSink<'a> {
    fn template_instance(&mut self, _template: BinXmlTemplateValues<'a>) -> EvtxResult<()> {
        Ok(())
    }
    fn open_start(&mut self, _name: ChunkOffset) -> EvtxResult<()> {
        Ok(())
    }
    fn close_start(&mut self) -> EvtxResult<()> {
        Ok(())
    }
    fn close_empty(&mut self) -> EvtxResult<()> {
        Ok(())
    }
    fn close_element(&mut self) -> EvtxResult<()> {
        Ok(())
    }
    fn value(&mut self, _value: BinXmlValue<'a>) -> EvtxResult<()> {
        Ok(())
    }
    fn attribute(&mut self, _name: ChunkOffset) -> EvtxResult<()> {
        Ok(())
    }
    fn entity_ref(&mut self, _name: ChunkOffset) -> EvtxResult<()> {
        Ok(())
    }
    fn pi_target(&mut self, _name: ChunkOffset) -> EvtxResult<()> {
        Ok(())
    }
    fn pi_data(&mut self, _data: Utf16LeSlice<'a>) -> EvtxResult<()> {
        Ok(())
    }
    fn cdata(&mut self, _text: Utf16LeSlice<'a>) -> EvtxResult<()> {
        Ok(())
    }
    fn char_ref(&mut self, _ch: u16) -> EvtxResult<()> {
        Ok(())
    }
    fn substitution(&mut self, _id: u16, _optional: bool) -> EvtxResult<()> {
        Ok(())
    }
}

/// Parse the BinXML token stream `src.data[start..start + len]` (up to EOF), handing each
/// token to `sink`. Template definitions (`has_dep_id`) carry dependency IDs on element starts.
pub(crate) fn walk_tokens<'a>(
    src: Stream<'a>,
    start: usize,
    len: usize,
    has_dep_id: bool,
    sink: &mut impl TokenSink<'a>,
) -> EvtxResult<()> {
    let mut cursor = ByteCursor::with_pos(src.data, start)?;
    // Public record offsets/sizes may be edited by callers; oversized windows still
    // report the cursor's normal truncation error, including on 32-bit targets.
    let end = start.saturating_add(len);
    let inline_names = src.chunk.is_none();

    while cursor.pos() < end {
        let token = cursor.u8()?;
        match token {
            token::EOF => break,
            token::TEMPLATE_INSTANCE => sink.template_instance(read_template_values_cursor(
                &mut cursor,
                src.ansi_codec,
                src.bump,
            )?)?,
            0x01 | 0x41 => sink.open_start(read_open_start_element_cursor(
                &mut cursor,
                token & 0x40 != 0,
                has_dep_id,
                inline_names,
            )?)?,
            0x02 => sink.close_start()?,
            0x03 => sink.close_empty()?,
            0x04 => sink.close_element()?,
            0x05 | 0x45 => sink.value(BinXmlValue::from_binxml_cursor_in(
                &mut cursor,
                None,
                src.ansi_codec,
                src.bump,
            )?)?,
            0x06 | 0x46 => sink.attribute(read_name_ref(&mut cursor, inline_names)?)?,
            0x09 | 0x49 => sink.entity_ref(read_name_ref(&mut cursor, inline_names)?)?,
            0x0a => sink.pi_target(read_name_ref(&mut cursor, inline_names)?)?,
            0x0b => sink.pi_data(read_processing_instruction_data_cursor(&mut cursor)?)?,
            0x0d | 0x0e => sink.substitution(
                read_substitution_descriptor_cursor(&mut cursor)?,
                token == 0x0e,
            )?,
            token::FRAGMENT_HEADER => {
                cursor.u8_named("fragment_header_major_version")?;
                cursor.u8_named("fragment_header_minor_version")?;
                cursor.u8_named("fragment_header_flags")?;
            }
            0x07 | 0x47 => sink.cdata(
                cursor
                    .len_prefixed_utf16_string(false, "cdata")?
                    .unwrap_or_else(Utf16LeSlice::empty),
            )?,
            0x08 | 0x48 => sink.char_ref(cursor.u16_named("char_ref")?)?,
            _ => {
                return Err(DeserializationError::InvalidToken {
                    value: token,
                    offset: cursor.position(),
                }
                .into());
            }
        }
    }
    Ok(())
}

/// Template definition header stored in the chunk template table.
#[derive(Debug)]
pub(crate) struct TemplateHeader {
    pub(crate) guid: [u8; 16],
    /// Includes the fragment header, element and end of file token; excludes
    /// this header.
    pub(crate) data_size: u32,
}

pub(crate) fn read_template_header(cursor: &mut ByteCursor<'_>) -> Result<TemplateHeader> {
    let _next_template_offset = cursor.u32_named("next_template_offset")?;
    let guid = cursor.array::<16>("template_guid")?;
    let data_size = cursor.u32_named("template_data_size")?;
    Ok(TemplateHeader { guid, data_size })
}

/// Template instance payload parsed into substitution values.
///
/// This avoids allocating legacy token wrappers for every substitution. The values are parsed
/// directly and consumed by the IR builder.
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplateValues<'a> {
    pub template_id: u32,
    pub template_def_offset: ChunkOffset,
    pub template_guid: Option<Guid>,
    pub values: Vec<BinXmlValue<'a>>,
}

/// Descriptor for a template substitution value payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
struct TemplateValueDescriptor {
    size: u16,
    value_type: BinXmlValueType,
}

/// BinXML stream token bytes (MS-EVEN6 §2.2.3.1).
pub(crate) mod token {
    pub(crate) const EOF: u8 = 0x00;
    pub(crate) const TEMPLATE_INSTANCE: u8 = 0x0c;
    pub(crate) const FRAGMENT_HEADER: u8 = 0x0f;
}

/// Offset of the `TemplateInstance` token's body when `bytes` is a (possibly
/// fragment-wrapped) single-instance stream; `None` for any other shape.
///
/// The fragment-header form requires at least one byte after the instance
/// token, matching the legacy shape check.
pub(crate) const fn single_instance_offset(bytes: &[u8]) -> Option<usize> {
    match bytes {
        [
            token::FRAGMENT_HEADER,
            _,
            _,
            _,
            token::TEMPLATE_INSTANCE,
            _,
            ..,
        ] => Some(5),
        [token::TEMPLATE_INSTANCE, ..] => Some(1),
        _ => None,
    }
}

/// Whether a single-instance `stream` whose instance parsed `consumed` bytes
/// ends there: the rest is empty or starts with EOF. `None` when the instance
/// read past the end of `stream`.
pub(crate) fn ends_stream(stream: &[u8], consumed: usize) -> Option<bool> {
    stream
        .get(consumed..)
        .map(|rest| matches!(rest, [] | [token::EOF, ..]))
}

/// Read a `TemplateInstance` and parse its substitution values directly.
///
/// This is used by the direct IR builder path.
pub(crate) fn read_template_values_cursor<'a>(
    cursor: &mut ByteCursor<'a>,
    ansi_codec: AnsiCodec,
    arena: &'a Bump,
) -> Result<BinXmlTemplateValues<'a>> {
    trace!("TemplateInstance at {}", cursor.position());

    let _ = cursor.u8()?;
    let template_id = cursor.u32()?;
    let template_definition_data_offset = cursor.u32()?;
    let mut template_guid: Option<Guid> = None;

    // Need to skip over the template data.
    if (cursor.position() as u32) == template_definition_data_offset {
        let template_header = read_template_header(cursor)?;
        template_guid = Some(Guid::from_buffer(&template_header.guid).expect("16-byte GUID"));
        cursor.set_pos_u64(
            cursor.position() + u64::from(template_header.data_size),
            "Skip cached template",
        )?;
    }

    let number_of_substitutions = cursor.u32()?;
    let descriptor_start = cursor.pos();
    // Validate the entire table before reading values, preserving both error
    // precedence and cursor positions for truncated/unknown descriptors.
    for _ in 0..number_of_substitutions {
        let _ = cursor.u16()?;
        BinXmlValueType::from_cursor(cursor)?;
        let _ = cursor.u8()?;
    }
    let descriptors = &cursor.buf()[descriptor_start..cursor.pos()];
    let mut values = Vec::with_capacity(descriptors.len() / 4);
    for &[lo, hi, ty, _] in descriptors.as_chunks::<4>().0 {
        let descriptor = TemplateValueDescriptor {
            size: u16::from_le_bytes([lo, hi]),
            value_type: BinXmlValueType::from_u8(ty).expect("validated descriptor type"),
        };
        let position_before_reading_value = cursor.position();
        trace!(
            "Offset `0x{offset:08x} ({offset})`: Substitution: {substitution:?}",
            offset = position_before_reading_value,
            substitution = descriptor.value_type,
        );

        let value = BinXmlValue::deserialize_value_type_cursor_in(
            &descriptor.value_type,
            cursor,
            Some(descriptor.size),
            ansi_codec,
            arena,
        )?;

        trace!("\t {:?}", value);
        // NullType can mean deleted substitution (and data need to be skipped)
        if matches!(value, BinXmlValue::NullType) {
            trace!("\t Skipping `NullType` descriptor");
            cursor.set_pos_u64(
                cursor.position() + u64::from(descriptor.size),
                "NullType Descriptor",
            )?;
        }

        let current_position = cursor.position();
        let expected_position = position_before_reading_value + u64::from(descriptor.size);

        if expected_position != current_position {
            let diff = expected_position as i128 - current_position as i128;
            // This sometimes occurs with dirty samples, but it's usually still possible to recover the rest of the record.
            // Sometimes however the log will contain a lot of zero fields.
            warn!(
                "Read incorrect amount of data, cursor position is at {}, but should have ended up at {}, last descriptor was {:?}.",
                current_position, expected_position, &descriptor
            );

            match u64::try_from(diff) {
                Ok(u64_diff) => {
                    cursor.set_pos_u64(current_position + u64_diff, "Broken record")?;
                }
                Err(_) => error!("Broken record"),
            }
        }
        values.push(value);
    }

    Ok(BinXmlTemplateValues {
        template_id,
        template_def_offset: template_definition_data_offset,
        template_guid,
        values,
    })
}

pub(crate) fn read_processing_instruction_data_cursor<'a>(
    cursor: &mut ByteCursor<'a>,
) -> Result<Utf16LeSlice<'a>> {
    trace!(
        "Offset `0x{:08x}` - ProcessingInstructionTarget",
        cursor.position(),
    );

    let data = cursor
        .len_prefixed_utf16_string(false, "pi_data")?
        .unwrap_or_else(Utf16LeSlice::empty);
    trace!("PIData - {} chars", data.num_chars());
    Ok(data)
}

/// Read a substitution descriptor (index + validated value type), returning the index.
pub(crate) fn read_substitution_descriptor_cursor(cursor: &mut ByteCursor<'_>) -> Result<u16> {
    trace!(
        "Offset `0x{:08x}` - SubstitutionDescriptor",
        cursor.position()
    );
    let substitution_index = cursor.u16()?;
    BinXmlValueType::from_cursor(cursor)?;
    Ok(substitution_index)
}

/// Read an element start header and return its name offset.
pub(crate) fn read_open_start_element_cursor(
    cursor: &mut ByteCursor<'_>,
    has_attributes: bool,
    has_dependency_identifier: bool,
    inline_names: bool,
) -> Result<ChunkOffset> {
    trace!(
        "Offset `0x{:08x}` - OpenStartElement<has_attributes={}, has_dependency_identifier={}>",
        cursor.position(),
        has_attributes,
        has_dependency_identifier
    );

    // Element start headers come in (at least) two variants:
    // - Template definitions: include a dependency identifier (u16)
    // - Direct record elements / nested BinXML (substitution value type 0x21): omit it
    if has_dependency_identifier {
        let _dependency_identifier =
            cursor.u16_named("open_start_element_dependency_identifier")?;
    }

    let _data_size = cursor.u32_named("open_start_element_data_size")?;
    let name = read_name_ref(cursor, inline_names)?;

    if has_attributes {
        let _attribute_list_data_size =
            cursor.u32_named("open_start_element_attribute_list_data_size")?;
    }

    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::WINDOWS_1252;

    #[test]
    fn oversized_token_window_reports_truncation() {
        struct Ignore;
        impl TokenSink<'_> for Ignore {}
        let bump = Bump::new();
        let src = Stream {
            data: &[0, 2],
            chunk: None,
            ansi_codec: WINDOWS_1252,
            bump: &bump,
        };
        let err = walk_tokens(src, 1, usize::MAX, false, &mut Ignore).unwrap_err();
        assert!(matches!(
            err,
            crate::err::EvtxError::DeserializationError(DeserializationError::Truncated {
                what: "u8",
                offset: 2,
                need: 1,
                have: 0
            })
        ));
    }

    #[test]
    fn truncated_fragment_headers_keep_field_errors() {
        struct Ignore;
        impl TokenSink<'_> for Ignore {}
        let bump = Bump::new();
        let bytes = [0x0f, 1, 1];
        for (i, what) in [
            "fragment_header_major_version",
            "fragment_header_minor_version",
            "fragment_header_flags",
        ]
        .into_iter()
        .enumerate()
        {
            let data = &bytes[..i + 1];
            let src = Stream {
                data,
                chunk: None,
                ansi_codec: WINDOWS_1252,
                bump: &bump,
            };
            let err = walk_tokens(src, 0, data.len(), false, &mut Ignore).unwrap_err();
            let crate::err::EvtxError::DeserializationError(DeserializationError::Truncated {
                what: actual,
                offset,
                need: 1,
                have: 0,
            }) = err
            else {
                panic!("unexpected error: {err:?}")
            };
            assert_eq!(actual, what);
            assert_eq!(offset, (i + 1) as u64);
        }
    }

    #[test]
    fn descriptor_errors_precede_value_errors() {
        for ty in [0xff, 0x04] {
            let mut bytes = vec![0; 9];
            bytes.extend_from_slice(&2u32.to_le_bytes());
            bytes.extend_from_slice(&[8, 0, 0x11, 0]); // first value would be truncated
            bytes.extend_from_slice(&[1, 0, ty]); // second descriptor lacks padding
            let bump = Bump::new();
            let mut cursor = ByteCursor::with_pos(&bytes, 0).unwrap();
            let error = read_template_values_cursor(&mut cursor, WINDOWS_1252, &bump).unwrap_err();
            assert_eq!(cursor.pos(), 20);
            if ty == 0xff {
                assert!(matches!(
                    error,
                    DeserializationError::InvalidValueVariant {
                        value: 0xff,
                        offset: 20
                    }
                ));
            } else {
                assert!(matches!(
                    error,
                    DeserializationError::Truncated {
                        offset: 20,
                        need: 1,
                        have: 0,
                        ..
                    }
                ));
            }
        }
    }
}
