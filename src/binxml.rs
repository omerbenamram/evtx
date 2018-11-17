use std::borrow::BorrowMut;
use std::cmp::min;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::mem;
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::{Context, Error, Fail, format_err, bail};
use log::{debug, log};

use crate::{
    evtx_chunk::{EvtxChunk, EvtxChunkHeader},
    guid::Guid,
    model::*,
    utils::datetime_from_filetime,
    utils::*,
    xml_builder::Visitor,
};

use std::borrow::{Borrow, Cow};
use std::collections::hash_map::Entry;
use std::fmt::Display;
use std::io::Cursor;

#[derive(Debug)]
pub struct BinXmlDeserializationError {
    inner: Context<BinXmlDeserializationErrorKind>,
}

impl BinXmlDeserializationError {
    pub fn new(ctx: Context<BinXmlDeserializationErrorKind>) -> BinXmlDeserializationError {
        BinXmlDeserializationError { inner: ctx }
    }

    pub fn unexpected_eof(e: impl Fail) -> Self {
        BinXmlDeserializationError::new(e.context(BinXmlDeserializationErrorKind::UnexpectedEOF))
    }

    pub fn not_a_valid_binxml_token(token: u8) -> Self {
        let err = BinXmlDeserializationErrorKind::NotAValidBinXMLToken { token };
        BinXmlDeserializationError::new(Context::new(err))
    }
}

#[derive(Fail, Debug)]
pub enum BinXmlDeserializationErrorKind {
    #[fail(
        display = "Expected attribute token to follow attribute name at position {}",
        position
    )]
    ExpectedValue { position: u64 },
    #[fail(display = "{:2x} not a valid binxml token", token)]
    NotAValidBinXMLToken { token: u8 },
    #[fail(display = "Unexpected EOF")]
    UnexpectedEOF,
}

pub struct BinXmlDeserializer<'a, 'b> {
    pub chunk: &'b mut EvtxChunk<'a>,
    pub offset_from_chunk_start: u64,
}

impl<'a, 'b> BinXmlDeserializer<'a, 'b> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLRawToken, BinXmlDeserializationError> {
        let token = cursor
            .read_u8()
            .map_err(BinXmlDeserializationError::unexpected_eof)?;

        Ok(BinXMLRawToken::from_u8(token).ok_or_else(|| {
            self.dump_and_panic(cursor, 100);
            BinXmlDeserializationError::not_a_valid_binxml_token(token)
        })?)
    }

    fn token_from_raw(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
        raw_token: BinXMLRawToken,
    ) -> Result<BinXMLDeserializedTokens<'a>, Error> {
        match raw_token {
            BinXMLRawToken::EndOfStream => {
                debug!("End of stream");
                Ok(BinXMLDeserializedTokens::EndOfStream)
            }
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    self.read_open_start_element(cursor, token_information.has_attributes)?,
                ))
            }
            BinXMLRawToken::CloseStartElement => {
                debug!("Close start element");
                Ok(BinXMLDeserializedTokens::CloseStartElement)
            }
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => {
                debug!("Close element");
                Ok(BinXMLDeserializedTokens::CloseElement)
            }
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(self.read_value(cursor)?)),
            BinXMLRawToken::Attribute(token_information) => Ok(
                BinXMLDeserializedTokens::Attribute(self.read_attribute(cursor)?),
            ),
            BinXMLRawToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLRawToken::EntityReference => unimplemented!("BinXMLToken::EntityReference"),
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
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
        value_type: BinXMLValueType,
    ) -> Result<BinXMLValue<'a>, Error> {
        match value_type {
            BinXMLValueType::NullType => Ok(BinXMLValue::NullType),
            BinXMLValueType::StringType => Ok(BinXMLValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(cursor, false)?.expect("String cannot be empty"),
            ))),
            BinXMLValueType::AnsiStringType => unimplemented!(),
            BinXMLValueType::Int8Type => Ok(BinXMLValue::Int8Type(cursor.read_u8()? as i8)),
            BinXMLValueType::UInt8Type => Ok(BinXMLValue::UInt8Type(cursor.read_u8()?)),
            BinXMLValueType::Int16Type => Ok(BinXMLValue::Int16Type(
                cursor.read_u16::<LittleEndian>()? as i16,
            )),
            BinXMLValueType::UInt16Type => {
                Ok(BinXMLValue::UInt16Type(cursor.read_u16::<LittleEndian>()?))
            }
            BinXMLValueType::Int32Type => Ok(BinXMLValue::Int32Type(
                cursor.read_u32::<LittleEndian>()? as i32,
            )),
            BinXMLValueType::UInt32Type => {
                Ok(BinXMLValue::UInt32Type(cursor.read_u32::<LittleEndian>()?))
            }
            BinXMLValueType::Int64Type => Ok(BinXMLValue::Int64Type(
                cursor.read_u64::<LittleEndian>()? as i64,
            )),
            BinXMLValueType::UInt64Type => {
                Ok(BinXMLValue::UInt64Type(cursor.read_u64::<LittleEndian>()?))
            }
            BinXMLValueType::Real32Type => unimplemented!(),
            BinXMLValueType::Real64Type => unimplemented!(),
            BinXMLValueType::BoolType => unimplemented!(),
            BinXMLValueType::BinaryType => unimplemented!(),
            BinXMLValueType::GuidType => Ok(BinXMLValue::GuidType(Guid::from_stream(cursor)?)),
            BinXMLValueType::SizeTType => unimplemented!(),
            BinXMLValueType::FileTimeType => Ok(BinXMLValue::FileTimeType(datetime_from_filetime(
                cursor.read_u64::<LittleEndian>()?,
            ))),
            BinXMLValueType::SysTimeType => unimplemented!(),
            BinXMLValueType::SidType => unimplemented!(),
            BinXMLValueType::HexInt32Type => unimplemented!(),
            BinXMLValueType::HexInt64Type => Ok(BinXMLValue::HexInt64Type(format!(
                "0x{:2x}",
                cursor.read_u64::<LittleEndian>()?
            ))),
            BinXMLValueType::EvtHandle => unimplemented!(),
            BinXMLValueType::BinXmlType => {
                Ok(BinXMLValue::BinXmlType(self.read_until_end_of_stream()?))
            }
            BinXMLValueType::EvtXml => unimplemented!(),
            _ => unimplemented!("{:?}", value_type),
        }
    }

    /// Collects all tokens until end of stream marker, useful for handling templates.
    fn read_until_end_of_stream(&mut self) -> Result<Vec<BinXMLDeserializedTokens<'a>>, Error> {
        let mut tokens = vec![];

        loop {
            let token = self.next();
            match token {
                None => bail!("Unexpected EOF"),
                Some(token) => match token {
                    Err(e) => bail!("Unexpected error {:?}", e),
                    Ok(token) => {
                        if token != BinXMLDeserializedTokens::EndOfStream {
                            tokens.push(token);
                        } else {
                            break;
                        }
                    }
                },
            }
        }

        Ok(tokens)
    }

    fn position_relative_to_chunk_start(&mut self, cursor: &mut Cursor<&'a [u8]>) -> u64 {
        cursor.position() + self.offset_from_chunk_start
    }

    fn read_substitution(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
        optional: bool,
    ) -> Result<TemplateSubstitutionDescriptor, Error> {
        debug!(
            "Substitution at: {}, optional: {}",
            cursor.position(),
            optional
        );
        let substitution_index = cursor.read_u16::<LittleEndian>()?;
        debug!("\t Index: {}", substitution_index);
        let value_type = BinXMLValueType::from_u8(cursor.read_u8()?);
        debug!("\t Value Type: {:?}", value_type);
        let ignore = optional && (value_type == BinXMLValueType::NullType);
        debug!("\t Ignore: {}", ignore);

        Ok(TemplateSubstitutionDescriptor {
            substitution_index,
            value_type,
            ignore,
        })
    }

    fn read_value(&mut self, cursor: &mut Cursor<&'a [u8]>) -> Result<BinXMLValue<'a>, Error> {
        debug!(
            "Value at: {} (0x{:2x})",
            cursor.position(),
            cursor.position() + 24
        );
        let value_type = BinXMLValueType::from_u8(cursor.read_u8()?);
        let data = self.read_value_from_type(cursor, value_type)?;
        debug!("\t Data: {:?}", data);
        Ok(data)
    }

    pub fn dump_and_panic(&mut self, cursor: &Cursor<&'a [u8]>, lookbehind: i32) {
        let offset = self.offset_from_chunk_start;
        println!("Panicked at offset {} (0x{:2x})", offset, offset + 24);
        dump_cursor(&cursor, lookbehind);
        panic!();
    }

    fn read_open_start_element(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
        has_attributes: bool,
    ) -> Result<BinXMLOpenStartElement<'a>, Error> {
        debug!(
            "OpenStartElement at {}, has_attributes: {}",
            self.offset_from_chunk_start, has_attributes
        );
        // Reserved
        cursor.read_u16::<LittleEndian>()?;

        let data_size = cursor.read_u32::<LittleEndian>()?;
        debug!("data size: {}, {}", data_size, cursor.position());
        let name = self.read_name(cursor)?;
        debug!("\t Name: {:?}", name);

        let attribute_list_data_size = match has_attributes {
            true => cursor.read_u32::<LittleEndian>()?,
            false => 0,
        };
        debug!("\t Attributes Data Size: {:?}", attribute_list_data_size);

        Ok(BinXMLOpenStartElement { data_size, name })
    }

    fn read_name(&mut self, cursor: &mut Cursor<&'a [u8]>) -> Result<Cow<'a, str>, Error> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = cursor.read_u32::<LittleEndian>()?;
        //        let name = match self.chunk.string_table.entry(name_offset) {
        //            Entry::Occupied(ref e) => e.get(),
        //            Entry::Vacant(e) => {
        //                let current_pos = cursor.position();
        //
        //                cursor.seek(SeekFrom::Start(name_offset as u64));
        //
        //                let _ = cursor.read_u32::<LittleEndian>()?;
        //                let name_hash = cursor.read_u16::<LittleEndian>()?;
        //
        //                let name = read_len_prefixed_utf16_string(cursor, true)?;
        //
        //                cursor.seek(SeekFrom::Start(current_pos));
        //
        //                e.insert((name_hash, name.unwrap_or_default()))
        //            }
        //        };

        debug!("{}", name_offset);
        let name = {
            let mut cursor = Cursor::new(self.chunk.data);
            cursor.seek(SeekFrom::Start(name_offset as u64))?;
            read_len_prefixed_utf16_string(&mut cursor, true)?.expect("Expected string")
        };

        Ok(Cow::Owned(name))
    }

    fn read_template(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLTemplate<'a>, Error> {
        debug!("TemplateInstance at {}", cursor.position());
        cursor.read_u8()?;
        let template_id = cursor.read_u32::<LittleEndian>()?;
        let template_definition_data_offset = cursor.read_u32::<LittleEndian>()?;

        //        let template_def = match self.chunk.template_table.entry(template_id) {
        //            Entry::Occupied(e) => e.get(),
        //            Entry::Vacant(e) => {
        //                let position = cursor.position();
        //                let template = self.read_template_definition()?;
        //                e.insert(Rc::new(template))
        //            }
        //        };

        let template_def = {
            let mut temp_cursor =
                Cursor::new(&self.chunk.data[template_definition_data_offset as usize..]);
            let template = self.read_template_definition(&mut temp_cursor)?;
            Rc::new(template)
        };

        let number_of_substitutions = cursor.read_u32::<LittleEndian>()?;
        let mut value_descriptors = Vec::with_capacity(number_of_substitutions as usize);
        for _ in 0..number_of_substitutions {
            let size = cursor.read_u16::<LittleEndian>()?;
            let value_type = BinXMLValueType::from_u8(cursor.read_u8()?);
            // Empty
            cursor.read_u8()?;

            value_descriptors.push(TemplateValueDescriptor { size, value_type })
        }

        let mut substitution_array = Vec::with_capacity(number_of_substitutions as usize);

        for descriptor in value_descriptors {
            let position = cursor.position();
            debug!("Substitution: {:?}", descriptor.value_type);
            let value = match descriptor.value_type {
                BinXMLValueType::StringType => BinXMLValue::StringType(Cow::Owned(
                    read_utf16_by_size(cursor, descriptor.size as u64)?
                        .expect("String should not be empty"),
                )),
                _ => self.read_value_from_type(cursor, descriptor.value_type)?,
            };
            debug!("\t {:?}", value);
            // NullType can mean deleted substitution (and data need to be skipped)
            if value == BinXMLValue::NullType {
                debug!("\t Skip {}", descriptor.size);
                cursor.seek(SeekFrom::Current(descriptor.size as i64))?;
            }
            assert_eq!(
                position + descriptor.size as u64,
                cursor.position(),
                "Read incorrect amount of data"
            );
            substitution_array.push(value);
        }

        Ok(BinXMLTemplate {
            definition: template_def.clone(),
            substitution_array,
        })
    }

    fn read_template_definition(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLTemplateDefinition<'a>, Error> {
        let next_template_offset = cursor.read_u32::<LittleEndian>()?;
        let template_guid = Guid::from_stream(cursor)?;
        let data_size = cursor.read_u32::<LittleEndian>()?;
        // Data size includes of the fragment header, element and end of file token;
        // except for the first 33 bytes of the template definition (above)
        let start_position = cursor.position();
        let element = self.read_until_end_of_stream()?;
        assert_eq!(
            cursor.position(),
            start_position + data_size as u64,
            "Template definition wasn't read completely"
        );
        Ok(BinXMLTemplateDefinition {
            next_template_offset,
            template_guid,
            data_size,
            tokens: element,
        })
    }

    fn read_attribute(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLAttribute<'a>, Error> {
        debug!("Attribute at {}", cursor.position());
        let name = self.read_name(cursor)?;
        debug!("\t Attribute name: {:?}", name);

        Ok(BinXMLAttribute { name })
    }

    fn read_fragment_header(
        &mut self,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLFragmentHeader, Error> {
        debug!("FragmentHeader at {}", cursor.position());
        let major_version = cursor.read_u8()?;
        let minor_version = cursor.read_u8()?;
        let flags = cursor.read_u8()?;
        Ok(BinXMLFragmentHeader {
            major_version,
            minor_version,
            flags,
        })
    }
}

/// IntoTokens yields ownership of the deserialized XML tokens.
impl<'a, 'b> Iterator for BinXmlDeserializer<'a, 'b> {
    type Item = Result<BinXMLDeserializedTokens<'a>, Error>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        debug!("offset_from_chunk_start: {}", self.offset_from_chunk_start);
        let mut cursor = Cursor::new(&self.chunk.data[self.offset_from_chunk_start as usize..]);

        let token = self.read_next_token(&mut cursor);
        self.offset_from_chunk_start += cursor.position();
        debug!("offset_from_chunk_start: {}", self.offset_from_chunk_start);

        if let Err(e) = token {
            return Some(Err(e.inner.into()));
        }

        let raw_token = token.unwrap();

        // Finished reading
        if self.chunk.data.len() == self.offset_from_chunk_start as usize {
            return None;
        }

        debug!("{:?} at {}", raw_token, self.offset_from_chunk_start);
        return Some(self.token_from_raw(&mut cursor, raw_token));
    }
}

///// The BinXMLDeserializer struct deserialized a chunk of EVTX data into a stream of elements.
///// It is used by initializing it with an EvtxChunk.
///// It yields back a deserialized token stream, while also taking care of substitutions of templates.
//impl<'a> BinXMLDeserializer<'a> {
//    pub fn new(chunk: &'a EvtxChunk<'a>, offset_from_chunk_start: u64) -> BinXMLDeserializer<'a> {
//        let binxml_data = &chunk.data[offset_from_chunk_start as usize..];
//        let cursor = Cursor::new(binxml_data);
//
//        BinXMLDeserializer {
//            chunk,
//            offset_from_chunk_start,
//            cursor,
//        }
//    }
//
//    fn visit_token(
//        &mut self,
//        token: &'a BinXMLDeserializedTokens<'a>,
//        visitor: &mut impl Visitor<'a>,
//    ) -> Result<(), Error> {
//        match token {
//            // Encountered a template, we need to fill the template, replacing values as needed and
//            // presenting them to the visitor.
//            BinXMLDeserializedTokens::TemplateInstance(template) => {
//                for token in template.definition.tokens.iter() {
//                    let replacement = template.substitute_token_if_needed(token);
//                    match replacement {
//                        Replacement::Token(token) => self.visit_token(token, visitor)?,
//                        Replacement::Value(value) => visitor.visit_value(value),
//                    }
//                }
//            }
//            _ => unimplemented!(),
//        }
//        Ok(())
//    }
//}

mod tests {
    use super::*;

    extern crate env_logger;

    //    #[test]
    //    fn test_reads_one_element() {
    //        let _ = env_logger::try_init().expect("Failed to init logger");
    //        let evtx_file = include_bytes!("../samples/security.evtx");
    //        let from_start_of_chunk = &evtx_file[4096..];
    //
    //        let chunk = EvtxChunk::new(&from_start_of_chunk).unwrap();
    //
    //
    //        let element = deserializer.read_until_end_of_stream().unwrap();
    //        println!("{:?}", element);
    //        assert_eq!(
    //            element.len(),
    //            2,
    //            "Element should contain a fragment and a template"
    //        );
    //
    //        let is_template = match element[1] {
    //            BinXMLDeserializedTokens::TemplateInstance(_) => true,
    //            _ => false,
    //        };
    //
    //        assert!(is_template, "Element should be a template");
    //
    //        // Weird zeroes (padding?)
    //        let mut zeroes = [0_u8; 3];
    //
    //        let c = &mut deserializer.cursor;
    //        c.take(3)
    //            .read_exact(&mut zeroes)
    //            .expect("Failed to read zeroes");
    //
    //        let copy_of_size = c.read_u32::<LittleEndian>().unwrap();
    //        //        assert_eq!(
    //        //            record_header.data_size, copy_of_size,
    //        //            "Didn't read expected amount of bytes."
    //        //        );
    //    }

    #[test]
    fn test_reads_simple_template_without_substitutions() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(&from_start_of_chunk).unwrap();

        for record in chunk {
            record.unwrap();
        }
    }

}
