use crate::err::{DeserializationError, DeserializationResult as Result, WrappedIoError};

pub use byteorder::{LittleEndian, ReadBytesExt};
use winstructs::guid::Guid;

use crate::model::deserialized::*;
use std::io::Cursor;

use crate::binxml::deserializer::BinXmlDeserializer;
use crate::binxml::name::BinXmlNameRef;
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};
use crate::utils::read_len_prefixed_utf16_string;

use log::{trace, warn};

use std::io::Seek;
use std::io::SeekFrom;

use crate::evtx_chunk::EvtxChunk;
use encoding::EncodingRef;

pub fn read_template<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    chunk: Option<&'a EvtxChunk<'a>>,
    ansi_codec: EncodingRef,
) -> Result<BinXmlTemplateRef<'a>> {
    trace!("TemplateInstance at {}", cursor.position());

    let _ = try_read!(cursor, u8)?;
    let _template_id = try_read!(cursor, u32)?;
    let template_definition_data_offset = try_read!(cursor, u32)?;

    // Need to skip over the template data.
    if (cursor.position() as u32) == template_definition_data_offset {
        let template_header = read_template_definition_header(cursor)?;
        try_seek!(
            cursor,
            cursor.position() + u64::from(template_header.data_size),
            "Skip cached template"
        )?;
    }

    let number_of_substitutions = try_read!(cursor, u32)?;

    let mut value_descriptors = Vec::with_capacity(number_of_substitutions as usize);

    for _ in 0..number_of_substitutions {
        let size = try_read!(cursor, u16)?;
        let value_type_token = try_read!(cursor, u8)?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or(
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            },
        )?;

        // Empty
        let _ = try_read!(cursor, u8)?;

        value_descriptors.push(TemplateValueDescriptor { size, value_type })
    }

    trace!("{:?}", value_descriptors);

    let mut substitution_array = Vec::with_capacity(number_of_substitutions as usize);

    for descriptor in value_descriptors {
        let position_before_reading_value = cursor.position();
        trace!(
            "Offset `0x{offset:08x} ({offset})`: Substitution: {substitution:?}",
            offset = position_before_reading_value,
            substitution = descriptor.value_type,
        );
        let value = BinXmlValue::deserialize_value_type(
            &descriptor.value_type,
            cursor,
            chunk,
            Some(descriptor.size),
            ansi_codec,
        )?;

        trace!("\t {:?}", value);
        // NullType can mean deleted substitution (and data need to be skipped)
        if value == BinXmlValue::NullType {
            trace!("\t Skipping `NullType` descriptor");
            try_seek!(
                cursor,
                cursor.position() + u64::from(descriptor.size),
                "NullType Descriptor"
            )?;
        }

        let current_position = cursor.position();
        let expected_position = position_before_reading_value + u64::from(descriptor.size);

        if expected_position != current_position {
            let diff = expected_position - current_position;
            // This sometimes occurs with dirty samples, but it's usually still possible to recover the rest of the record.
            // Sometimes however the log will contain a lot of zero fields.
            warn!("Read incorrect amount of data, cursor position is at {}, but should have ended up at {}, last descriptor was {:?}.",
                  current_position,
                  expected_position,
                  &descriptor);

            try_seek!(cursor, current_position + diff, "Broken record")?;
        }
        substitution_array.push(BinXMLDeserializedTokens::Value(value));
    }

    Ok(BinXmlTemplateRef {
        template_def_offset: template_definition_data_offset,
        substitution_array,
    })
}

pub fn read_template_definition_header(
    cursor: &mut Cursor<&[u8]>,
) -> Result<BinXmlTemplateDefinitionHeader> {
    // If any of these fail we cannot reliably report the template information in error.
    let next_template_offset = try_read!(cursor, u32, "next_template_offset")?;
    let template_guid = try_read!(cursor, guid, "template_guid")?;
    // Data size includes the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition (above)
    let data_size = try_read!(cursor, u32, "template_data_size")?;

    Ok(BinXmlTemplateDefinitionHeader {
        next_template_offset,
        guid: template_guid,
        data_size,
    })
}

pub fn read_template_definition<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    chunk: Option<&'a EvtxChunk<'a>>,
    ansi_codec: EncodingRef,
) -> Result<BinXMLTemplateDefinition<'a>> {
    let header = read_template_definition_header(cursor)?;

    trace!(
        "Offset `0x{:08x}` - TemplateDefinition {}",
        cursor.position(),
        header
    );

    let template = match BinXmlDeserializer::read_binxml_fragment(
        cursor,
        chunk,
        Some(header.data_size),
        false,
        ansi_codec,
    ) {
        Ok(tokens) => BinXMLTemplateDefinition { header, tokens },
        Err(e) => {
            return Err(DeserializationError::FailedToDeserializeTemplate {
                template_id: header.guid,
                source: Box::new(e),
            })
        }
    };

    Ok(template)
}

pub fn read_entity_ref(cursor: &mut Cursor<&[u8]>) -> Result<BinXmlEntityReference> {
    trace!("Offset `0x{:08x}` - EntityReference", cursor.position());
    let name = BinXmlNameRef::from_stream(cursor)?;
    trace!("\t name: {:?}", name);

    Ok(BinXmlEntityReference { name })
}

pub fn read_attribute(cursor: &mut Cursor<&[u8]>) -> Result<BinXMLAttribute> {
    trace!("Offset `0x{:08x}` - Attribute", cursor.position());
    let name = BinXmlNameRef::from_stream(cursor)?;

    Ok(BinXMLAttribute { name })
}

pub fn read_fragment_header(cursor: &mut Cursor<&[u8]>) -> Result<BinXMLFragmentHeader> {
    trace!("Offset `0x{:08x}` - FragmentHeader", cursor.position());
    let major_version = try_read!(cursor, u8, "fragment_header_major_version")?;
    let minor_version = try_read!(cursor, u8, "fragment_header_minor_version")?;
    let flags = try_read!(cursor, u8, "fragment_header_flags")?;
    Ok(BinXMLFragmentHeader {
        major_version,
        minor_version,
        flags,
    })
}

pub fn read_processing_instruction_target(
    cursor: &mut Cursor<&[u8]>,
) -> Result<BinXMLProcessingInstructionTarget> {
    trace!(
        "Offset `0x{:08x}` - ProcessingInstructionTarget",
        cursor.position(),
    );

    let name = BinXmlNameRef::from_stream(cursor)?;
    trace!("\tPITarget Name - {:?}", name);
    Ok(BinXMLProcessingInstructionTarget { name })
}

pub fn read_processing_instruction_data(cursor: &mut Cursor<&[u8]>) -> Result<String> {
    trace!(
        "Offset `0x{:08x}` - ProcessingInstructionTarget",
        cursor.position(),
    );

    let data =
        try_read!(cursor, len_prefixed_utf_16_str, "pi_data")?.unwrap_or_else(|| "".to_string());
    trace!("PIData - {}", data,);
    Ok(data)
}

pub fn read_substitution_descriptor(
    cursor: &mut Cursor<&[u8]>,
    optional: bool,
) -> Result<TemplateSubstitutionDescriptor> {
    trace!(
        "Offset `0x{:08x}` - SubstitutionDescriptor<optional={}>",
        cursor.position(),
        optional
    );
    let substitution_index = try_read!(cursor, u16)?;
    let value_type_token = try_read!(cursor, u8)?;

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
    })
}

pub fn read_open_start_element(
    cursor: &mut Cursor<&[u8]>,
    chunk: Option<&EvtxChunk>,
    has_attributes: bool,
    is_substitution: bool,
) -> Result<BinXMLOpenStartElement> {
    trace!(
        "Offset `0x{:08x}` - OpenStartElement<has_attributes={}, is_substitution={}>",
        cursor.position(),
        has_attributes,
        is_substitution
    );

    // According to https://github.com/libyal/libevtx/blob/master/documentation/Windows%20XML%20Event%20Log%20(EVTX).asciidoc
    // The dependency identifier is not present when the element start is used in a substitution token.
    if !is_substitution {
        let _dependency_identifier =
            try_read!(cursor, u16, "open_start_element_dependency_identifier")?;

        trace!(
            "\t Dependency Identifier - `0x{:04x} ({})`",
            _dependency_identifier,
            _dependency_identifier
        );
    }

    let data_size = try_read!(cursor, u32, "open_start_element_data_size")?;

    // This is a heuristic, sometimes `dependency_identifier` is not present even though it should have been.
    // This will result in interpreting garbage bytes as the data size.
    // We try to recover from this situation by rolling back the cursor and trying again, without reading the `dependency_identifier`.
    if let Some(c) = chunk {
        if data_size >= c.data.len() as u32 {
            warn!(
                "Detected a case where `dependency_identifier` should not have been read. \
                 Trying to read again without it."
            );
            cursor.seek(SeekFrom::Current(-6)).map_err(|e| {
                WrappedIoError::io_error_with_message(
                    e,
                    "failed to skip when recovering from `dependency_identifier` hueristic",
                    cursor,
                )
            })?;
            return read_open_start_element(cursor, chunk, has_attributes, true);
        }
    }

    trace!("\t Data Size - {}", data_size);
    let name = BinXmlNameRef::from_stream(cursor)?;
    trace!("\t Name - {:?}", name);

    let _attribute_list_data_size = if has_attributes {
        try_read!(cursor, u32, "open_start_element_attribute_list_data_size")?
    } else {
        0
    };

    Ok(BinXMLOpenStartElement { data_size, name })
}
