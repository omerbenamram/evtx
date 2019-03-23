pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::{error::Error, guid::Guid, model::deserialized::*};
use std::io::Cursor;

use crate::binxml::deserializer::{BinXmlDeserializer, ParsingContext};
use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::{BinXMLValue, BinXMLValueType};
use crate::utils::{read_len_prefixed_utf16_string, read_utf16_by_size};
use byteorder::LittleEndian;
use log::{debug, log, trace};
use std::borrow::Cow;
use std::io::Seek;
use std::io::SeekFrom;
use std::rc::Rc;

pub fn read_template_definition<'a>(
    cursor: &mut Cursor<&'a [u8]>,
) -> Result<BinXMLTemplateDefinition<'a>, Error> {
    let next_template_offset = try_read!(cursor, u32);

    let template_guid = Guid::from_stream(cursor)
        .map_err(|e| Error::other("Failed to read GUID from stream", cursor.position()))?;

    let data_size = try_read!(cursor, u32);

    // Data size includes the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition (above)
    let start_position = cursor.position();
    let de = BinXmlDeserializer::init_without_cache(cursor.get_ref(), start_position);

    // TODO: this is supposed to error if we have'nt read enough data.
    let element = de.iter_tokens(data_size).collect();

    Ok(BinXMLTemplateDefinition {
        next_template_offset,
        template_guid,
        data_size,
        tokens: element,
    })
}

pub fn read_entity_ref<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    ctx: &ParsingContext,
) -> Result<BinXmlEntityReference<'a>, Error> {
    debug!("EntityReference at {}", cursor.position());
    let name = BinXmlName::from_binxml_stream(cursor, ctx)?;
    debug!("\t name: {:?}", name);

    Ok(BinXmlEntityReference { name })
}

pub fn read_attribute<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    ctx: &ParsingContext,
) -> Result<BinXMLAttribute<'a>, Error> {
    let name = BinXmlName::from_binxml_stream(cursor, ctx)?;

    Ok(BinXMLAttribute { name })
}

pub fn read_fragment_header(cursor: &mut Cursor<&[u8]>) -> Result<BinXMLFragmentHeader, Error> {
    debug!("FragmentHeader at {}", cursor.position());
    let major_version = try_read!(cursor, u8);
    let minor_version = try_read!(cursor, u8);
    let flags = try_read!(cursor, u8);
    Ok(BinXMLFragmentHeader {
        major_version,
        minor_version,
        flags,
    })
}

pub fn read_substitution(
    cursor: &mut Cursor<&[u8]>,
    optional: bool,
) -> Result<TemplateSubstitutionDescriptor, Error> {
    let substitution_index = try_read!(cursor, u16);
    let value_type_token = try_read!(cursor, u8);

    let value_type = BinXMLValueType::from_u8(value_type_token)
        .ok_or_else(|| Error::not_a_valid_binxml_value_type(value_type_token, cursor.position()))?;

    let ignore = optional && (value_type == BinXMLValueType::NullType);

    Ok(TemplateSubstitutionDescriptor {
        substitution_index,
        value_type,
        ignore,
    })
}

fn read_open_start_element<'a>(
    cursor: &mut Cursor<&'a [u8]>,
    ctx: &ParsingContext,
    has_attributes: bool,
) -> Result<BinXMLOpenStartElement<'a>, Error> {
    // Reserved
    let _ = try_read!(cursor, u16);
    let data_size = try_read!(cursor, u32);
    let name = BinXmlName::from_binxml_stream(cursor, ctx)?;

    let attribute_list_data_size = if has_attributes {
        try_read!(cursor, u32)
    } else {
        0
    };

    Ok(BinXMLOpenStartElement { data_size, name })
}

pub fn read_template<'a>(cursor: &mut Cursor<&'a [u8]>) -> Result<BinXmlTemplate<'a>, Error> {
    debug!("TemplateInstance at {}", cursor.position());

    let _ = try_read!(cursor, u8);
    let template_id = try_read!(cursor, u32);
    let template_definition_data_offset = try_read!(cursor, u32);

    let template_def = if template_definition_data_offset != cursor.position() as u32 {
        debug!(
            "Need to seek to offset {} to read template",
            template_definition_data_offset
        );
        let position_before_seek = cursor.position();

        cursor
            .seek(SeekFrom::Start(u64::from(template_definition_data_offset)))
            .map_err(|e| Error::io(e, cursor.position()))?;

        let template_def = Rc::new(read_template_definition(cursor)?);

        cursor
            .seek(SeekFrom::Start(position_before_seek))
            .map_err(|e| Error::io(e, cursor.position()))?;

        template_def
    } else {
        Rc::new(read_template_definition(cursor)?)
    };

    trace!("{:?}", template_def);

    let number_of_substitutions = try_read!(cursor, u32);

    let mut value_descriptors = Vec::with_capacity(number_of_substitutions as usize);

    for _ in 0..number_of_substitutions {
        let size = try_read!(cursor, u16);
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXMLValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        // Empty
        let _ = try_read!(cursor, u8);

        value_descriptors.push(TemplateValueDescriptor { size, value_type })
    }

    trace!("{:?}", value_descriptors);

    let mut substitution_array = Vec::with_capacity(number_of_substitutions as usize);

    for descriptor in value_descriptors {
        let position = cursor.position();
        debug!("Substitution: {:?} at {}", descriptor.value_type, position);
        let value = BinXMLValue::deserialize_value_type(&descriptor.value_type, cursor)?;

        debug!("\t {:?}", value);
        // NullType can mean deleted substitution (and data need to be skipped)
        if value == BinXMLValue::NullType {
            debug!("\t Skip {}", descriptor.size);
            cursor
                .seek(SeekFrom::Current(i64::from(descriptor.size)))
                .map_err(|e| Error::io(e, cursor.position()))?;
        }
        assert_eq!(
            position + u64::from(descriptor.size),
            cursor.position(),
            "{}",
            &format!(
                "Read incorrect amount of data, cursor position is at {}, but should have ended up at {}, last descriptor was {:?}.",
                cursor.position(), position + u64::from(descriptor.size), &descriptor
            )
        );
        substitution_array.push(value);
    }

    Ok(BinXmlTemplate {
        definition: template_def.clone(),
        substitution_array,
    })
}
