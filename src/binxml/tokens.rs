use crate::err::{DeserializationError, DeserializationResult as Result};

use winstructs::guid::Guid;

use crate::model::deserialized::*;
use crate::utils::ByteCursor;
use std::io::Cursor;

use crate::binxml::deserializer::BinXmlDeserializer;
use crate::binxml::name::{BinXmlNameEncoding, BinXmlNameRef};
use crate::binxml::value_variant::BinXmlValueType;

use log::trace;

use crate::evtx_chunk::EvtxChunk;
use bumpalo::Bump;
use encoding::EncodingRef;

fn with_cursor<'a, T>(
    cursor: &mut ByteCursor<'a>,
    f: impl FnOnce(&mut Cursor<&'a [u8]>) -> Result<T>,
) -> Result<T> {
    let mut c = Cursor::new(cursor.buf());
    c.set_position(cursor.position());
    let out = f(&mut c)?;
    cursor.set_pos_u64(c.position(), "advance after cursor-backed parse")?;
    Ok(out)
}

pub(crate) fn read_template_cursor<'a>(
    cursor: &mut ByteCursor<'a>,
    _chunk: Option<&'a EvtxChunk<'a>>,
    _arena: &'a Bump,
    _ansi_codec: EncodingRef,
) -> Result<BinXmlTemplateRef> {
    trace!("TemplateInstance at {}", cursor.position());

    let _ = cursor.u8()?;
    let template_id = cursor.u32()?;
    let template_definition_data_offset = cursor.u32()?;
    let mut template_guid: Option<Guid> = None;

    // Need to skip over the template data.
    if (cursor.position() as u32) == template_definition_data_offset {
        let template_header = read_template_definition_header_cursor(cursor)?;
        template_guid = Some(template_header.guid.clone());
        cursor.set_pos_u64(
            cursor.position() + u64::from(template_header.data_size),
            "Skip cached template",
        )?;
    }

    let number_of_substitutions = cursor.u32()?;

    let mut value_descriptors = Vec::with_capacity(number_of_substitutions as usize);

    for _ in 0..number_of_substitutions {
        let size = cursor.u16()?;
        let value_type_token = cursor.u8()?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            }
        })?;

        // Empty
        let _ = cursor.u8()?;

        value_descriptors.push(TemplateValueDescriptor { size, value_type })
    }

    trace!("{:?}", value_descriptors);

    // Keep raw substitution spans (type + size + offset), and skip the value bytes.
    // The expander/serializer will decode on-demand when the substitution is referenced.
    let mut substitutions = Vec::with_capacity(number_of_substitutions as usize);
    for descriptor in value_descriptors {
        let offset_u64 = cursor.position();
        let offset: u32 = u32::try_from(offset_u64).map_err(|_| DeserializationError::Truncated {
            what: "TemplateInstance substitution offset",
            offset: offset_u64,
            need: 0,
            have: 0,
        })?;

        trace!(
            "Offset `0x{offset:08x} ({offset})`: Substitution span: {substitution:?} (size={size})",
            offset = offset,
            substitution = descriptor.value_type,
            size = descriptor.size,
        );

        substitutions.push(TemplateSubstitutionSpan {
            offset,
            size: descriptor.size,
            value_type: descriptor.value_type,
        });

        cursor.advance(usize::from(descriptor.size), "skip TemplateInstance substitution bytes")?;
    }

    Ok(BinXmlTemplateRef {
        template_id,
        template_def_offset: template_definition_data_offset,
        template_guid,
        substitutions,
    })
}

fn read_template_definition_header_cursor(
    cursor: &mut ByteCursor<'_>,
) -> Result<BinXmlTemplateDefinitionHeader> {
    // If any of these fail we cannot reliably report the template information in error.
    let next_template_offset = cursor.u32_named("next_template_offset")?;
    let guid_bytes = cursor.take_bytes(16, "template_guid")?;
    let template_guid = Guid::from_buffer(guid_bytes).map_err(|_| {
        DeserializationError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid GUID",
        ))
    })?;
    // Data size includes the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition (above)
    let data_size = cursor.u32_named("template_data_size")?;

    Ok(BinXmlTemplateDefinitionHeader {
        next_template_offset,
        guid: template_guid,
        data_size,
    })
}

pub(crate) fn read_template_definition_cursor<'a>(
    cursor: &mut ByteCursor<'a>,
    chunk: Option<&'a EvtxChunk<'a>>,
    arena: &'a Bump,
    ansi_codec: EncodingRef,
) -> Result<BinXMLTemplateDefinition<'a>> {
    let header = read_template_definition_header_cursor(cursor)?;

    trace!(
        "Offset `0x{:08x}` - TemplateDefinition {}",
        cursor.position(),
        header
    );

    let template = match with_cursor(cursor, |c| {
        BinXmlDeserializer::read_binxml_fragment(
            c,
            chunk,
            arena,
            Some(header.data_size),
            true,
            ansi_codec,
        )
    }) {
        Ok(tokens) => BinXMLTemplateDefinition { header, tokens },
        Err(e) => {
            return Err(DeserializationError::FailedToDeserializeTemplate {
                template_id: header.guid,
                source: Box::new(e),
            });
        }
    };

    Ok(template)
}

/// Strictly read a `TemplateDefinitionHeader` at a known offset in an EVTX chunk buffer.
///
/// This does **not** scan for signatures or guess offsets. It only succeeds when the bytes at the
/// provided `offset` look like a valid template definition header followed by a BinXML fragment
/// header (`StartOfStream` + version tuple). This is used by higher-level "offline WEVT cache"
/// logic to match a record's `TemplateInstance` to a template GUID without fully deserializing the
/// template.
pub(crate) fn try_read_template_definition_header_at(
    chunk_data: &[u8],
    offset: u32,
) -> Result<BinXmlTemplateDefinitionHeader> {
    let off = offset as usize;
    let mut cursor = ByteCursor::with_pos(chunk_data, off)?;

    // Read the header using the canonical parser.
    let header = read_template_definition_header_cursor(&mut cursor)?;

    // Validate next_template_offset is either:
    // - 0 (end of list)
    // - equal to itself (observed termination sentinel)
    // - a forward in-chunk offset
    if header.next_template_offset != 0 && header.next_template_offset != offset {
        if header.next_template_offset <= offset {
            return Err(DeserializationError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "template next_template_offset is not forward",
            )));
        }
        if (header.next_template_offset as usize) >= chunk_data.len() {
            return Err(DeserializationError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "template next_template_offset out of bounds",
            )));
        }
    }

    // We should now be positioned immediately after the template header.
    let data_size_usize = header.data_size as usize;
    if data_size_usize < 4 {
        return Err(DeserializationError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "template data_size too small",
        )));
    }

    // Ensure the full template fragment range is in-bounds (strict; we do not accept a header that
    // points past the chunk end).
    let data_start = cursor.pos();
    let data_end = data_start.saturating_add(data_size_usize);
    if data_end > chunk_data.len() {
        return Err(DeserializationError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "template data_size out of bounds",
        )));
    }

    // Verify BinXML fragment header: StartOfStream (0x0f) + major/minor/flags.
    let frag = cursor.take_bytes(4, "template fragment header")?;
    if frag[0] != 0x0f || frag[1] != 0x01 || frag[2] != 0x01 {
        return Err(DeserializationError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "template does not start with BinXML fragment header (StartOfStream 1.1)",
        )));
    }

    Ok(header)
}

pub(crate) fn read_entity_ref_cursor(
    cursor: &mut ByteCursor<'_>,
    name_encoding: BinXmlNameEncoding,
) -> Result<BinXmlEntityReference> {
    trace!("Offset `0x{:08x}` - EntityReference", cursor.position());
    let name = BinXmlNameRef::from_cursor_with_encoding(cursor, name_encoding)?;
    trace!("\t name: {:?}", name);
    Ok(BinXmlEntityReference { name })
}

pub(crate) fn read_attribute_cursor(
    cursor: &mut ByteCursor<'_>,
    name_encoding: BinXmlNameEncoding,
) -> Result<BinXMLAttribute> {
    trace!("Offset `0x{:08x}` - Attribute", cursor.position());
    let name = BinXmlNameRef::from_cursor_with_encoding(cursor, name_encoding)?;
    Ok(BinXMLAttribute { name })
}

pub(crate) fn read_fragment_header_cursor(
    cursor: &mut ByteCursor<'_>,
) -> Result<BinXMLFragmentHeader> {
    trace!("Offset `0x{:08x}` - FragmentHeader", cursor.position());
    let major_version = cursor.u8_named("fragment_header_major_version")?;
    let minor_version = cursor.u8_named("fragment_header_minor_version")?;
    let flags = cursor.u8_named("fragment_header_flags")?;
    Ok(BinXMLFragmentHeader {
        major_version,
        minor_version,
        flags,
    })
}

pub(crate) fn read_processing_instruction_target_cursor(
    cursor: &mut ByteCursor<'_>,
    name_encoding: BinXmlNameEncoding,
) -> Result<BinXMLProcessingInstructionTarget> {
    trace!(
        "Offset `0x{:08x}` - ProcessingInstructionTarget",
        cursor.position(),
    );

    let name = BinXmlNameRef::from_cursor_with_encoding(cursor, name_encoding)?;
    trace!("\tPITarget Name - {:?}", name);
    Ok(BinXMLProcessingInstructionTarget { name })
}

pub(crate) fn read_processing_instruction_data_cursor(
    cursor: &mut ByteCursor<'_>,
) -> Result<String> {
    trace!(
        "Offset `0x{:08x}` - ProcessingInstructionTarget",
        cursor.position(),
    );

    let data = cursor
        .len_prefixed_utf16_string(false, "pi_data")?
        .unwrap_or_default();
    trace!("PIData - {}", data,);
    Ok(data)
}

pub(crate) fn read_substitution_descriptor_cursor(
    cursor: &mut ByteCursor<'_>,
    optional: bool,
) -> Result<TemplateSubstitutionDescriptor> {
    trace!(
        "Offset `0x{:08x}` - SubstitutionDescriptor<optional={}>",
        cursor.position(),
        optional
    );
    let substitution_index = cursor.u16()?;
    let value_type_token = cursor.u8()?;

    let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
        DeserializationError::InvalidValueVariant {
            value: value_type_token,
            offset: cursor.position(),
        }
    })?;

    let ignore = optional && (value_type == BinXmlValueType::NullType);

    Ok(TemplateSubstitutionDescriptor {
        substitution_index,
        value_type,
        ignore,
    })
}

pub(crate) fn read_open_start_element_cursor(
    cursor: &mut ByteCursor<'_>,
    has_attributes: bool,
    has_dependency_identifier: bool,
    name_encoding: BinXmlNameEncoding,
) -> Result<BinXMLOpenStartElement> {
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
        let dependency_identifier = cursor.u16_named("open_start_element_dependency_identifier")?;

        trace!(
            "\t Dependency Identifier - `0x{:04x} ({})`",
            dependency_identifier, dependency_identifier
        );
    }

    let data_size = cursor.u32_named("open_start_element_data_size")?;

    trace!("\t Data Size - {}", data_size);
    let name = BinXmlNameRef::from_cursor_with_encoding(cursor, name_encoding)?;
    trace!("\t Name - {:?}", name);

    let _attribute_list_data_size = if has_attributes {
        cursor.u32_named("open_start_element_attribute_list_data_size")?
    } else {
        0
    };

    Ok(BinXMLOpenStartElement { data_size, name })
}
