use std::io::{self, Seek, SeekFrom};
use std::rc::Rc;

use byteorder::{LittleEndian, ReadBytesExt};
use log::{debug, log, trace};

use crate::{
    evtx_chunk::EvtxChunk, guid::Guid, utils::datetime_from_filetime, utils::*,
    xml_builder::BinXMLOutput,
};

use crate::error::Error;
use crate::evtx_chunk::Offset;
use crate::evtx_chunk::StringHash;
use crate::model::deserialized::*;
use crate::model::owned::*;
use crate::model::raw::*;
use crate::ntsid::Sid;
use std::borrow::Cow;
use std::io::Cursor;
use std::io::Write;

pub struct BinXmlDeserializer<'chunk: 'record, 'record> {
    chunk: &'record EvtxChunk<'chunk>,
    offset_from_chunk_start: u64,
    // data_size is canonically u32 in the header.
    data_size: u32,
    data_read_so_far: u32,
    eof: bool,
}

impl<'chunk: 'record, 'record> BinXmlDeserializer<'chunk, 'record> {
    pub fn from_chunk_at_offset(
        chunk: &'record EvtxChunk<'chunk>,
        offset_from_chunk_starts: u64,
        expected_data_size: u32,
    ) -> Self {
        BinXmlDeserializer {
            chunk,
            offset_from_chunk_start: offset_from_chunk_starts,
            data_size: expected_data_size,
            data_read_so_far: 0,
            eof: false,
        }
    }

    /// This logic is static since it is also used in initializing the string cache.
    pub fn read_name_and_hash(cursor: &mut Cursor<&'chunk [u8]>) -> Result<StringHash, Error> {
        let position_before_read = cursor.position();

        let _ = try_read!(cursor, u32);
        let name_hash = try_read!(cursor, u16);
        let name = read_len_prefixed_utf16_string(cursor, true)
            .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
            .expect("Expected string");

        let position_after_read = cursor.position();

        Ok((
            name,
            name_hash,
            (position_after_read - position_before_read) as u16,
        ))
    }

    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&self, cursor: &mut Cursor<&'chunk [u8]>) -> Result<BinXMLRawToken, Error> {
        let token = cursor
            .read_u8()
            .map_err(|e| Error::unexpected_eof(e, cursor.position()))?;

        Ok(BinXMLRawToken::from_u8(token)
            .ok_or_else(|| Error::not_a_valid_binxml_token(token, cursor.position()))?)
    }

    fn token_from_raw(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
        raw_token: BinXMLRawToken,
    ) -> Result<BinXMLDeserializedTokens<'record>, Error> {
        match raw_token {
            BinXMLRawToken::EndOfStream => Ok(BinXMLDeserializedTokens::EndOfStream),
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    self.read_open_start_element(cursor, token_information.has_attributes)?,
                ))
            }
            BinXMLRawToken::CloseStartElement => Ok(BinXMLDeserializedTokens::CloseStartElement),
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => Ok(BinXMLDeserializedTokens::CloseElement),
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(self.read_value(cursor)?)),
            BinXMLRawToken::Attribute(token_information) => Ok(
                BinXMLDeserializedTokens::Attribute(self.read_attribute(cursor)?),
            ),
            BinXMLRawToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLRawToken::EntityReference => Ok(BinXMLDeserializedTokens::EntityRef(
                self.read_entity_ref(cursor)?,
            )),
            BinXMLRawToken::ProcessingInstructionTarget => {
                unimplemented!("BinXMLToken::ProcessingInstructionTarget")
            }
            BinXMLRawToken::ProcessingInstructionData => {
                unimplemented!("BinXMLToken::ProcessingInstructionData")
            }
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                self.read_template(cursor)?,
            )),
            BinXMLRawToken::NormalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                self.read_substitution(cursor, false)?,
            )),
            BinXMLRawToken::ConditionalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                self.read_substitution(cursor, true)?,
            )),
            BinXMLRawToken::StartOfStream => Ok(BinXMLDeserializedTokens::FragmentHeader(
                self.read_fragment_header(cursor)?,
            )),
        }
    }

    fn read_value_from_type(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
        value_type: &BinXMLValueType,
    ) -> Result<BinXMLValue<'record>, Error> {
        match value_type {
            BinXMLValueType::NullType => Ok(BinXMLValue::NullType),
            BinXMLValueType::StringType => Ok(BinXMLValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(cursor, false)
                    .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
                    .expect("String cannot be empty"),
            ))),
            BinXMLValueType::AnsiStringType => unimplemented!(),
            BinXMLValueType::Int8Type => Ok(BinXMLValue::Int8Type(try_read!(cursor, i8))),
            BinXMLValueType::UInt8Type => Ok(BinXMLValue::UInt8Type(try_read!(cursor, u8))),
            BinXMLValueType::Int16Type => Ok(BinXMLValue::Int16Type(try_read!(cursor, i16))),
            BinXMLValueType::UInt16Type => Ok(BinXMLValue::UInt16Type(try_read!(cursor, u16))),
            BinXMLValueType::Int32Type => Ok(BinXMLValue::Int32Type(try_read!(cursor, i32))),
            BinXMLValueType::UInt32Type => Ok(BinXMLValue::UInt32Type(try_read!(cursor, u32))),
            BinXMLValueType::Int64Type => Ok(BinXMLValue::Int64Type(try_read!(cursor, i64))),
            BinXMLValueType::UInt64Type => Ok(BinXMLValue::UInt64Type(try_read!(cursor, u64))),
            BinXMLValueType::Real32Type => unimplemented!("Real32Type"),
            BinXMLValueType::Real64Type => unimplemented!("Real64Type"),
            BinXMLValueType::BoolType => unimplemented!("BoolType"),
            BinXMLValueType::BinaryType => unimplemented!("BinaryType"),
            BinXMLValueType::GuidType => {
                Ok(BinXMLValue::GuidType(Guid::from_stream(cursor).map_err(
                    |e| Error::other("Failed to read GUID from stream", cursor.position()),
                )?))
            }
            BinXMLValueType::SizeTType => unimplemented!("SizeTType"),
            BinXMLValueType::FileTimeType => Ok(BinXMLValue::FileTimeType(datetime_from_filetime(
                try_read!(cursor, u64),
            ))),
            BinXMLValueType::SysTimeType => unimplemented!("SysTimeType"),
            BinXMLValueType::SidType => {
                Ok(BinXMLValue::SidType(Sid::from_stream(cursor).map_err(
                    |_| Error::other("Failed to read NTSID from stream", cursor.position()),
                )?))
            }
            BinXMLValueType::HexInt32Type => Ok(BinXMLValue::HexInt32Type(format!(
                "0x{:2x}",
                try_read!(cursor, i32)
            ))),
            BinXMLValueType::HexInt64Type => Ok(BinXMLValue::HexInt64Type(format!(
                "0x{:2x}",
                try_read!(cursor, i64)
            ))),
            BinXMLValueType::EvtHandle => unimplemented!("EvtHandle"),
            BinXMLValueType::BinXmlType => Ok(BinXMLValue::BinXmlType(
                self.read_until_end_of_stream(cursor)?,
            )),
            BinXMLValueType::EvtXml => unimplemented!("EvtXml"),
        }
    }

    /// Collects all tokens until end of stream marker, useful for handling templates.
    fn read_until_end_of_stream(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<Vec<BinXMLDeserializedTokens<'record>>, Error> {
        let mut tokens = vec![];

        loop {
            let token = self.read_next_token(cursor).and_then(|t| {
                self.token_from_raw(cursor, t)
                    .map_err(|_| Error::other("token_from_raw failed", cursor.position()))
            });

            match token {
                Err(e) => {
                    return Err(Error::other("failed", cursor.position()));
                }
                Ok(token) => {
                    if token != BinXMLDeserializedTokens::EndOfStream {
                        tokens.push(token);
                    } else {
                        break;
                    }
                }
            }
        }

        Ok(tokens)
    }

    fn read_substitution(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
        optional: bool,
    ) -> Result<TemplateSubstitutionDescriptor, Error> {
        let substitution_index = try_read!(cursor, u16);
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXMLValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        let ignore = optional && (value_type == BinXMLValueType::NullType);

        Ok(TemplateSubstitutionDescriptor {
            substitution_index,
            value_type,
            ignore,
        })
    }

    fn read_value(&self, cursor: &mut Cursor<&'chunk [u8]>) -> Result<BinXMLValue<'record>, Error> {
        let value_type_token = try_read!(cursor, u8);
        let value_type = BinXMLValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        let data = self.read_value_from_type(cursor, &value_type)?;
        Ok(data)
    }

    fn read_open_start_element(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
        has_attributes: bool,
    ) -> Result<BinXMLOpenStartElement<'record>, Error> {
        // Reserved
        let _ = try_read!(cursor, u16);
        let data_size = try_read!(cursor, u32);
        let name = self.read_name(cursor)?;

        let attribute_list_data_size = if has_attributes {
            try_read!(cursor, u32)
        } else {
            0
        };

        Ok(BinXMLOpenStartElement { data_size, name })
    }

    fn inner_read_name(
        &self,
        name_offset: Offset,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<StringHash, Error> {
        if name_offset != cursor.position() as u32 {
            trace!(
                "Current offset {}, seeking to {}",
                cursor.position(),
                name_offset
            );
            let position_before_seek = cursor.position();
            cursor
                .seek(SeekFrom::Start(u64::from(name_offset)))
                .map_err(|e| Error::io(e, position_before_seek))?;

            let (name, hash, n_bytes_read) = BinXmlDeserializer::read_name_and_hash(cursor)?;

            trace!("Restoring cursor to {}", position_before_seek);
            cursor
                .seek(SeekFrom::Start(position_before_seek as u64))
                .map_err(|e| Error::io(e, position_before_seek))?;

            Ok((name, hash, n_bytes_read))
        } else {
            trace!("Name is at current offset");
            let (name, hash, n_bytes_read) = BinXmlDeserializer::read_name_and_hash(cursor)?;
            Ok((name, hash, n_bytes_read))
        }
    }

    fn read_name(&self, cursor: &mut Cursor<&'chunk [u8]>) -> Result<Cow<'record, str>, Error> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = try_read!(cursor, u32);

        if let Some((name, _, n_bytes_read)) = self.chunk.get_string_and_hash(name_offset) {
            if name_offset == cursor.position() as u32 {
                cursor
                    .seek(SeekFrom::Current(*n_bytes_read as i64))
                    .map_err(|e| Error::io(e, cursor.position()))?;
            }
            return Ok(Cow::Borrowed(name));
        }

        let (name, _, _) = self.inner_read_name(name_offset, cursor)?;
        Ok(Cow::Owned(name))
    }

    fn read_template(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<BinXMLTemplate<'record>, Error> {
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

            let template_def = Rc::new(self.read_template_definition(cursor)?);

            cursor
                .seek(SeekFrom::Start(position_before_seek))
                .map_err(|e| Error::io(e, cursor.position()))?;

            template_def
        } else {
            Rc::new(self.read_template_definition(cursor)?)
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
            let value = match descriptor.value_type {
                BinXMLValueType::StringType => BinXMLValue::StringType(Cow::Owned(
                    read_utf16_by_size(cursor, u64::from(descriptor.size))
                        .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
                        .unwrap_or_else(|| "".to_owned()),
                )),
                _ => self.read_value_from_type(cursor, &descriptor.value_type)?,
            };
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

        Ok(BinXMLTemplate {
            definition: template_def.clone(),
            substitution_array,
        })
    }

    fn read_template_definition(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<BinXMLTemplateDefinition<'record>, Error> {
        let next_template_offset = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| Error::io(e, cursor.position()))?;
        let template_guid = Guid::from_stream(cursor)
            .map_err(|e| Error::other("Failed to read GUID from stream", cursor.position()))?;

        let data_size = cursor
            .read_u32::<LittleEndian>()
            .map_err(|e| Error::io(e, cursor.position()))?;
        // Data size includes the fragment header, element and end of file token;
        // except for the first 33 bytes of the template definition (above)
        let start_position = cursor.position();
        let element = self.read_until_end_of_stream(cursor)?;

        assert_eq!(
            cursor.position(),
            start_position + u64::from(data_size),
            "Template definition wasn't read completely"
        );
        Ok(BinXMLTemplateDefinition {
            next_template_offset,
            template_guid,
            data_size,
            tokens: element,
        })
    }

    fn read_entity_ref(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<BinXmlEntityReference<'record>, Error> {
        debug!("EntityReference at {}", cursor.position());
        let name = self.read_name(cursor)?;
        debug!("\t name: {:?}", name);

        Ok(BinXmlEntityReference { name })
    }

    fn read_attribute(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<BinXMLAttribute<'record>, Error> {
        let name = self.read_name(cursor)?;

        Ok(BinXMLAttribute { name })
    }

    fn read_fragment_header(
        &self,
        cursor: &mut Cursor<&'chunk [u8]>,
    ) -> Result<BinXMLFragmentHeader, Error> {
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
}

/// IntoTokens yields ownership of the deserialized XML tokens.
impl<'chunk: 'record, 'record> Iterator for BinXmlDeserializer<'chunk, 'record> {
    type Item = Result<BinXMLDeserializedTokens<'record>, Error>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        trace!("offset_from_chunk_start: {}", self.offset_from_chunk_start);
        trace!(
            "need to read: {}, read so far: {}",
            self.data_size,
            self.data_read_so_far
        );

        // Finished reading
        if (self.data_read_so_far >= self.data_size) || self.eof {
            return None;
        }

        let mut cursor = Cursor::new(self.chunk.data.as_slice());

        cursor
            .seek(SeekFrom::Start(self.offset_from_chunk_start))
            .unwrap();

        match self.read_next_token(&mut cursor) {
            Ok(t) => {
                if let BinXMLRawToken::EndOfStream = t {
                    self.eof = true;
                }

                trace!("{:?} at {}", t, self.offset_from_chunk_start);
                let token = self.token_from_raw(&mut cursor, t);
                trace!("{:?} position at stream {}", token, cursor.position());

                assert!(
                    cursor.position() >= self.offset_from_chunk_start,
                    "Invalid state, cursor position at entering loop {}, now at {}",
                    self.offset_from_chunk_start,
                    cursor.position()
                );

                let total_read = cursor.position() - self.offset_from_chunk_start;
                self.offset_from_chunk_start += total_read;
                self.data_read_so_far += total_read as u32;

                Some(token)
            }
            Err(e) => {
                // Cursor might have not been moved if this error was thrown in middle of seek.
                // So seek all the way to end.
                assert!(
                    self.data_size >= self.data_read_so_far,
                    "Invalid state! read too much data! data_size is {}, read to {}",
                    self.data_size,
                    self.data_read_so_far
                );
                let total_read = self.data_size - self.data_read_so_far;
                self.offset_from_chunk_start += u64::from(total_read);
                self.data_read_so_far += total_read as u32;

                Some(Err(e))
            }
        }
    }
}

pub fn parse_tokens<'chunk: 'record, 'record, W: Write, T: BinXMLOutput<'chunk, W>>(
    tokens: Vec<BinXMLDeserializedTokens<'chunk>>,
    visitor: &'record mut T,
) {
    let expanded_tokens = expand_templates(tokens);
    let record_model = create_record_model(expanded_tokens);

    for owned_token in record_model {
        match owned_token {
            OwnedModel::OpenElement(open_element) => {
                visitor.visit_open_start_element(&open_element)
            }
            OwnedModel::CloseElement => visitor.visit_close_element(),
            OwnedModel::String(s) => visitor.visit_characters(&s),
            OwnedModel::EndOfStream => visitor.visit_end_of_stream(),
            OwnedModel::StartOfStream => visitor.visit_start_of_stream(),
        }
    }
}

pub fn create_record_model(tokens: Vec<BinXMLDeserializedTokens>) -> Vec<OwnedModel> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut model: Vec<OwnedModel> = vec![];

    for token in tokens {
        match token {
            BinXMLDeserializedTokens::FragmentHeader(_) => {}
            BinXMLDeserializedTokens::TemplateInstance(_) => {
                panic!("Call `expand_templates` before calling this function")
            }
            BinXMLDeserializedTokens::AttributeList => {}
            BinXMLDeserializedTokens::Attribute(attr) => {
                debug!("BinXMLDeserializedTokens::Attribute(attr) - {:?}", attr);
                match current_element.take() {
                    None => panic!("attribute - Bad parser state"),
                    Some(builder) => {
                        current_element = Some(builder.attribute_name(attr.name));
                    }
                };
            }
            BinXMLDeserializedTokens::OpenStartElement(elem) => {
                debug!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let builder = XmlElementBuilder::new();
                current_element = Some(builder.name(elem.name));
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                debug!("BinXMLDeserializedTokens::CloseStartElement");
                match current_element.take() {
                    None => panic!("close start - Bad parser state"),
                    Some(builder) => model.push(OwnedModel::OpenElement(builder.finish())),
                };
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                debug!("BinXMLDeserializedTokens::CloseEmptyElement");
                match current_element.take() {
                    None => panic!("close empty - Bad parser state"),
                    Some(builder) => {
                        model.push(OwnedModel::OpenElement(builder.finish()));
                        model.push(OwnedModel::CloseElement);
                    }
                };
            }
            BinXMLDeserializedTokens::CloseElement => {
                model.push(OwnedModel::CloseElement);
            }
            BinXMLDeserializedTokens::Value(value) => {
                debug!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                match current_element.take() {
                    // A string that is not inside any element, yield it
                    None => match value {
                        BinXMLValue::StringType(cow) => {
                            model.push(OwnedModel::String(cow.clone()));
                        }
                        BinXMLValue::EvtXml => {
                            panic!("Call `expand_templates` before calling this function")
                        }
                        _ => {
                            model.push(OwnedModel::String(value.into()));
                        }
                    },
                    // A string that is bound to an attribute
                    Some(builder) => {
                        current_element =
                            Some(builder.attribute_value(BinXMLValue::StringType(value.into())));
                    }
                };
            }
            BinXMLDeserializedTokens::CDATASection => {}
            BinXMLDeserializedTokens::CharRef => {}
            BinXMLDeserializedTokens::EntityRef(e) => unimplemented!("{}", &format!("{:?}", e)),
            BinXMLDeserializedTokens::PITarget => {}
            BinXMLDeserializedTokens::PIData => {}
            BinXMLDeserializedTokens::Substitution(_) => {
                panic!("Call `expand_templates` before calling this function")
            }
            BinXMLDeserializedTokens::EndOfStream => model.push(OwnedModel::EndOfStream),
            BinXMLDeserializedTokens::StartOfStream => model.push(OwnedModel::StartOfStream),
        }
    }
    model
}

pub fn expand_templates(
    token_tree: Vec<BinXMLDeserializedTokens>,
) -> Vec<BinXMLDeserializedTokens> {
    let mut stack = Vec::new();

    fn _expand_templates<'chunk: 'local, 'local>(
        token: BinXMLDeserializedTokens<'chunk>,
        stack: &mut Vec<BinXMLDeserializedTokens<'local>>,
    ) {
        match token {
            BinXMLDeserializedTokens::Value(ref value) => {
                if let BinXMLValue::BinXmlType(tokens) = value {
                    for token in tokens.iter() {
                        _expand_templates(token.clone(), stack);
                    }
                } else {
                    stack.push(token)
                }
            }
            BinXMLDeserializedTokens::TemplateInstance(template) => {
                // We have to clone here since the templates **definitions** are shared.
                for token in template.definition.tokens.iter().cloned() {
                    if let BinXMLDeserializedTokens::Substitution(ref substitution_descriptor) =
                        token
                    {
                        if substitution_descriptor.ignore {
                            continue;
                        } else {
                            // TODO: see if we can avoid this copy
                            let value = &template.substitution_array
                                [substitution_descriptor.substitution_index as usize];

                            _expand_templates(
                                BinXMLDeserializedTokens::Value(value.clone()),
                                stack,
                            );
                        }
                    } else {
                        _expand_templates(token, stack);
                    }
                }
            }
            _ => stack.push(token),
        }
    }

    for token in token_tree {
        _expand_templates(token, &mut stack)
    }

    stack
}

mod tests {
    use super::*;
    use crate::ensure_env_logger_initialized;
    use crate::evtx_record::EvtxRecordHeader;
    use std::borrow::BorrowMut;
    use std::io::Read;

    const EVTX_CHUNK_SIZE: usize = 65536;
    const EVTX_HEADER_SIZE: usize = 4096;
    const EVTX_RECORD_HEADER_SIZE: usize = 24;

    #[test]
    fn test_read_name_bug() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");

        let mut cursor = Cursor::new(&evtx_file[EVTX_HEADER_SIZE + EVTX_CHUNK_SIZE..]);
        let mut chunk_data = Vec::with_capacity(EVTX_CHUNK_SIZE);
        cursor
            .borrow_mut()
            .take(EVTX_CHUNK_SIZE as u64)
            .read_to_end(&mut chunk_data)
            .unwrap();

        let chunk = EvtxChunk::new(chunk_data).unwrap();
        let mut cursor = Cursor::new(chunk.data.as_slice());

        // Seek to bad record position
        cursor.seek(SeekFrom::Start(3872)).unwrap();

        let record_header = EvtxRecordHeader::from_reader(&mut cursor).unwrap();
        let mut data = Vec::with_capacity(record_header.data_size as usize);

        cursor
            .take(u64::from(record_header.data_size))
            .read_to_end(&mut data)
            .unwrap();

        let deser = BinXmlDeserializer::from_chunk_at_offset(
            &chunk,
            (3872_usize + EVTX_RECORD_HEADER_SIZE) as u64,
            record_header.data_size - 4 - 4 - 4 - 8 - 8,
        );

        for token in deser {
            if let Err(e) = token {
                let mut cursor = Cursor::new(chunk.data.as_slice());
                println!("{}", e);
                cursor.seek(SeekFrom::Start(e.offset())).unwrap();
                dump_cursor(&mut cursor, 10);
                panic!();
            }
        }
    }

    #[test]
    fn test_reads_a_single_record() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(from_start_of_chunk.to_vec()).unwrap();

        for record in chunk.into_iter().take(1) {
            assert!(record.is_ok(), record.unwrap())
        }
    }

    #[test]
    fn test_reads_a_ten_records() {
        ensure_env_logger_initialized();
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(from_start_of_chunk.to_vec()).unwrap();

        for record in chunk.into_iter().take(10) {
            println!("{:?}", record);
        }
    }

}
