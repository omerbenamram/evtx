use crate::err::{DeserializationError, DeserializationResult as Result};

use winstructs::guid::Guid;

use crate::ChunkOffset;
use crate::binxml::name::{BinXmlNameEncoding, BinXmlNameRef};
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};
use crate::utils::{ByteCursor, Utf16LeSlice};

use log::{error, trace, warn};

use crate::evtx_chunk::EvtxChunk;
use bumpalo::Bump;
use encoding::EncodingRef;
use std::fmt::{self, Formatter};

/// Processing instruction target name.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct BinXMLProcessingInstructionTarget {
    pub name: BinXmlNameRef,
}

/// Open-start element token payload (name and data size).
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct BinXMLOpenStartElement {
    pub data_size: u32,
    pub name: BinXmlNameRef,
}

/// Template definition header stored in the chunk template table.
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub(crate) struct BinXmlTemplateDefinitionHeader {
    /// A pointer to the next template in the bucket.
    pub next_template_offset: ChunkOffset,
    pub guid: Guid,
    pub data_size: u32,
}

impl fmt::Display for BinXmlTemplateDefinitionHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<BinXmlTemplateDefinitionHeader - id: {guid}, data_size: {size}>",
            guid = self.guid,
            size = self.data_size
        )
    }
}

/// Entity reference token payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct BinXmlEntityReference {
    pub name: BinXmlNameRef,
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

/// Placeholder descriptor within a template definition.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct TemplateSubstitutionDescriptor {
    /// Zero-based index of the substitution value.
    pub substitution_index: u16,
    pub value_type: BinXmlValueType,
    pub ignore: bool,
    /// True for conditional substitutions; optional values may be omitted when empty.
    pub optional: bool,
}

/// Fragment header at the start of a BinXML stream.
#[repr(C)]
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct BinXMLFragmentHeader {
    pub major_version: u8,
    pub minor_version: u8,
    pub flags: u8,
}

/// Attribute token payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub(crate) struct BinXMLAttribute {
    pub name: BinXmlNameRef,
}

/// Read a `TemplateInstance` and parse its substitution values directly.
///
/// This is used by the direct IR builder path.
pub(crate) fn read_template_values_cursor<'a>(
    cursor: &mut ByteCursor<'a>,
    chunk: Option<&'a EvtxChunk<'a>>,
    ansi_codec: EncodingRef,
    arena: &'a Bump,
) -> Result<BinXmlTemplateValues<'a>> {
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

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or(
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            },
        )?;

        // Empty
        let _ = cursor.u8()?;

        value_descriptors.push(TemplateValueDescriptor { size, value_type })
    }

    trace!("{:?}", value_descriptors);

    let mut values = Vec::with_capacity(number_of_substitutions as usize);

    for descriptor in value_descriptors {
        let position_before_reading_value = cursor.position();
        trace!(
            "Offset `0x{offset:08x} ({offset})`: Substitution: {substitution:?}",
            offset = position_before_reading_value,
            substitution = descriptor.value_type,
        );

        let value = BinXmlValue::deserialize_value_type_cursor_in(
            &descriptor.value_type,
            cursor,
            chunk,
            Some(descriptor.size),
            ansi_codec,
            arena,
        )?;

        trace!("\t {:?}", value);
        // NullType can mean deleted substitution (and data need to be skipped)
        if value == BinXmlValue::NullType {
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

    let value_type = BinXmlValueType::from_u8(value_type_token).ok_or(
        DeserializationError::InvalidValueVariant {
            value: value_type_token,
            offset: cursor.position(),
        },
    )?;

    let ignore = optional && (value_type == BinXmlValueType::NullType);

    Ok(TemplateSubstitutionDescriptor {
        substitution_index,
        value_type,
        ignore,
        optional,
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
