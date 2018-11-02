use std::borrow::BorrowMut;
use std::cmp::min;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::mem;
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::{Context, Error, Fail};

use evtx::datetime_from_filetime;
use evtx_chunk_header::EvtxChunk;
use evtx_chunk_header::EvtxChunkHeader;
use guid::Guid;
use model::*;
use std::borrow::{Borrow, Cow};
use std::fmt::Display;
use std::io::Cursor;
use utils::*;
use xml_builder::Visitor;

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
enum BinXmlDeserializationErrorKind {
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

pub struct BinXMLDeserializer<'a> {
    chunk: &'a EvtxChunk<'a>,
    offset_from_chunk_start: u64,
    cursor: Cursor<&'a [u8]>,
}

pub fn read_name_from_stream<'a>(stream: &mut BinXMLDeserializer) -> Result<Cow<'a, str>, Error> {
    let _ = stream.cursor.read_u32::<LittleEndian>()?;
    let name_hash = stream.cursor.read_u16::<LittleEndian>()?;

    let name = read_len_prefixed_utf16_string(&mut stream.cursor, true)?;

    Ok(Cow::Owned(name.unwrap_or_default()))
}

pub struct IntoTokens<'a> {
    chunk: &'a EvtxChunk<'a>,
    offset_from_chunk_start: u64,
    cursor: Cursor<&'a [u8]>,
}

impl<'a> IntoIterator for BinXMLDeserializer<'a> {
    type Item = Result<BinXMLDeserializedTokens<'a>, BinXmlDeserializationError>;
    type IntoIter = IntoTokens<'a>;

    fn into_iter(self) -> IntoTokens<'a> {
        unimplemented!()
    }
}

impl<'a> IntoTokens<'a> {
    /// Reads the next token from the stream, will return error if failed to read from the stream for some reason,
    /// or if reading random bytes (usually because of a bug in the code).
    fn read_next_token(&mut self) -> Result<BinXMLRawToken, BinXmlDeserializationError> {
        let token = self
            .cursor
            .read_u8()
            .map_err(BinXmlDeserializationError::unexpected_eof)?;

        Ok(BinXMLRawToken::from_u8(token)
            .ok_or_else(|| BinXmlDeserializationError::not_a_valid_binxml_token(token))?)
    }

    fn read_value_from_type(
        &mut self,
        value_type: BinXMLValueType,
    ) -> Result<BinXMLValue<'a>, Error> {
        match value_type {
            BinXMLValueType::NullType => Ok(BinXMLValue::NullType),
            BinXMLValueType::StringType => Ok(BinXMLValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(&mut self.cursor, false)?
                    .expect("String cannot be empty"),
            ))),
            BinXMLValueType::AnsiStringType => unimplemented!(),
            BinXMLValueType::Int8Type => Ok(BinXMLValue::Int8Type(self.cursor.read_u8()? as i8)),
            BinXMLValueType::UInt8Type => Ok(BinXMLValue::UInt8Type(self.cursor.read_u8()?)),
            BinXMLValueType::Int16Type => Ok(BinXMLValue::Int16Type(
                self.cursor.read_u16::<LittleEndian>()? as i16,
            )),
            BinXMLValueType::UInt16Type => Ok(BinXMLValue::UInt16Type(
                self.cursor.read_u16::<LittleEndian>()?,
            )),
            BinXMLValueType::Int32Type => Ok(BinXMLValue::Int32Type(
                self.cursor.read_u32::<LittleEndian>()? as i32,
            )),
            BinXMLValueType::UInt32Type => Ok(BinXMLValue::UInt32Type(
                self.cursor.read_u32::<LittleEndian>()?,
            )),
            BinXMLValueType::Int64Type => Ok(BinXMLValue::Int64Type(
                self.cursor.read_u64::<LittleEndian>()? as i64,
            )),
            BinXMLValueType::UInt64Type => Ok(BinXMLValue::UInt64Type(
                self.cursor.read_u64::<LittleEndian>()?,
            )),
            BinXMLValueType::Real32Type => unimplemented!(),
            BinXMLValueType::Real64Type => unimplemented!(),
            BinXMLValueType::BoolType => unimplemented!(),
            BinXMLValueType::BinaryType => unimplemented!(),
            BinXMLValueType::GuidType => {
                Ok(BinXMLValue::GuidType(Guid::from_stream(&mut self.cursor)?))
            }
            BinXMLValueType::SizeTType => unimplemented!(),
            BinXMLValueType::FileTimeType => Ok(BinXMLValue::FileTimeType(datetime_from_filetime(
                self.cursor.read_u64::<LittleEndian>()?,
            ))),
            BinXMLValueType::SysTimeType => unimplemented!(),
            BinXMLValueType::SidType => unimplemented!(),
            BinXMLValueType::HexInt32Type => unimplemented!(),
            BinXMLValueType::HexInt64Type => Ok(BinXMLValue::HexInt64Type(format!(
                "0x{:2x}",
                self.cursor.read_u64::<LittleEndian>()?
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

    fn position_relative_to_chunk_start(&self) -> u64 {
        self.cursor.position() + self.offset_from_chunk_start
    }

    fn read_substitution(
        &mut self,
        optional: bool,
    ) -> Result<TemplateSubstitutionDescriptor, Error> {
        debug!(
            "Substitution at: {}, optional: {}",
            self.cursor.position(),
            optional
        );
        let substitution_index = self.cursor.read_u16::<LittleEndian>()?;
        debug!("\t Index: {}", substitution_index);
        let value_type = BinXMLValueType::from_u8(self.cursor.read_u8()?);
        debug!("\t Value Type: {:?}", value_type);
        let ignore = optional && (value_type == BinXMLValueType::NullType);
        debug!("\t Ignore: {}", ignore);

        Ok(TemplateSubstitutionDescriptor {
            substitution_index,
            value_type,
            ignore,
        })
    }

    fn read_value(&mut self) -> Result<BinXMLValue<'a>, Error> {
        debug!(
            "Value at: {} (0x{:2x})",
            self.cursor.position(),
            self.cursor.position() + 24
        );
        let value_type = BinXMLValueType::from_u8(self.cursor.read_u8()?);
        let data = self.read_value_from_type(value_type)?;
        debug!("\t Data: {:?}", data);
        Ok(data)
    }

    pub fn read_relative_to_chunk_offset<T: Sized>(
        &mut self,
        offset: u64,
        f: &FnMut() -> Result<T, Error>,
    ) -> Result<T, Error> {
        let need_to_seek = !(offset == self.position_relative_to_chunk_start());

        if need_to_seek {
            let original_position = self.cursor.position();
            self.cursor.seek(SeekFrom::Start(offset))?;
            let result = f();
            self.cursor.seek(SeekFrom::Start(original_position));

            result
        } else {
            f()
        }
    }

    fn dump_and_panic(&self, lookbehind: i32) {
        let offset = self.cursor.position();
        println!("Panicked at offset {} (0x{:2x})", offset, offset + 24);
        dump_cursor(&self.cursor, lookbehind);
        panic!();
    }

    fn read_open_start_element(
        &mut self,
        has_attributes: bool,
    ) -> Result<BinXMLOpenStartElement<'a>, Error> {
        debug!(
            "OpenStartElement at {}, has_attributes: {}",
            self.cursor.position(),
            has_attributes
        );
        self.cursor.read_u16::<LittleEndian>()?;

        let data_size = self.cursor.read_u32::<LittleEndian>()?;
        let name = self.read_name()?;
        debug!("\t Name: {:?}", name);

        let attribute_list_data_size = match has_attributes {
            true => self.cursor.read_u32::<LittleEndian>()?,
            false => 0,
        };
        debug!("\t Attributes Data Size: {:?}", attribute_list_data_size);

        Ok(BinXMLOpenStartElement { data_size, name })
    }

    fn read_name(&mut self) -> Result<Cow<'a, str>, Error> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = self.cursor.read_u32::<LittleEndian>()?;
        let name_offset = name_offset as u64;
        // TODO: check string offset cache and return reference to cached value if needed.

        let name = self.read_relative_to_chunk_offset(name_offset, &read_name_from_stream)?;
        Ok(name)
    }

    fn read_template(&mut self) -> Result<BinXMLTemplate<'a>, Error> {
        debug!("TemplateInstance at {}", self.cursor.position());
        self.cursor.read_u8()?;
        let template_id = self.cursor.read_u32::<LittleEndian>()?;
        let template_definition_data_offset = self.cursor.read_u32::<LittleEndian>()?;

        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let template_def = Rc::new(
            self.read_relative_to_chunk_offset(
                template_definition_data_offset as u64,
                &BinXMLDeserializer::read_template_definition,
            )
            .expect(&format!(
                "Failed to read template definition at offset {}",
                template_definition_data_offset
            )),
        )
        .clone();

        let number_of_substitutions = self.cursor.read_u32::<LittleEndian>()?;
        let mut value_descriptors = Vec::with_capacity(number_of_substitutions as usize);
        for _ in 0..number_of_substitutions {
            let size = self.cursor.read_u16::<LittleEndian>()?;
            let value_type = BinXMLValueType::from_u8(self.cursor.read_u8()?);
            // Empty
            self.cursor.read_u8()?;

            value_descriptors.push(TemplateValueDescriptor { size, value_type })
        }

        let mut substitution_array = Vec::with_capacity(number_of_substitutions as usize);

        for descriptor in value_descriptors {
            let position = self.cursor.position();
            debug!("Substitution: {:?}", descriptor.value_type);
            let value = match descriptor.value_type {
                BinXMLValueType::StringType => BinXMLValue::StringType(Cow::Owned(
                    read_utf16_by_size(&mut self.cursor, descriptor.size as u64)?
                        .expect("String should not be empty"),
                )),
                _ => self.read_value_from_type(descriptor.value_type)?,
            };
            debug!("\t {:?}", value);
            // NullType can mean deleted substitution (and data need to be skipped)
            if value == BinXMLValue::NullType {
                debug!("\t Skip {}", descriptor.size);
                self.cursor
                    .seek(SeekFrom::Current(descriptor.size as i64))?;
            }
            assert_eq!(
                position + descriptor.size as u64,
                self.cursor.position(),
                "Read incorrect amount of data"
            );
            substitution_array.push(value);
        }

        Ok(BinXMLTemplate {
            definition: template_def,
            substitution_array,
        })
    }

    fn read_template_definition(
        ctx: &mut BinXMLDeserializer<'a>,
    ) -> Result<BinXMLTemplateDefinition<'a>, Error> {
        let next_template_offset = ctx.cursor.read_u32::<LittleEndian>()?;
        let template_guid = Guid::from_stream(&mut ctx.cursor)?;
        let data_size = ctx.cursor.read_u32::<LittleEndian>()?;
        // Data size includes of the fragment header, element and end of file token;
        // except for the first 33 bytes of the template definition (above)
        let start_position = ctx.cursor.position();
        let element = ctx.read_until_end_of_stream()?;
        assert_eq!(
            ctx.cursor.position(),
            start_position + data_size as u64,
            "Template definition wasn't read completely"
        );
        Ok(BinXMLTemplateDefinition {
            next_template_offset,
            template_guid,
            data_size,
            element,
        })
    }

    fn read_attribute(&mut self) -> Result<BinXMLAttribute<'a>, Error> {
        debug!("Attribute at {}", self.cursor.position());
        let name = self.read_name()?;
        debug!("\t Attribute name: {:?}", name);

        Ok(BinXMLAttribute { name })
    }

    fn read_fragment_header(&mut self) -> Result<BinXMLFragmentHeader, Error> {
        debug!("FragmentHeader at {}", self.cursor.position());
        let major_version = self.cursor.read_u8()?;
        let minor_version = self.cursor.read_u8()?;
        let flags = self.cursor.read_u8()?;
        Ok(BinXMLFragmentHeader {
            major_version,
            minor_version,
            flags,
        })
    }
}

/// IntoTokens yields ownership of the deserialized XML tokens.
impl<'a> Iterator for IntoTokens<'a> {
    type Item = Result<BinXMLDeserializedTokens<'a>, BinXmlDeserializationError>;

    /// yields tokens from the chunk, will return once the chunk is finished.
    fn next(&mut self) -> Option<<Self as Iterator>::Item> {
        let token = self.read_next_token();

        if let Err(e) = token {
            return Some(Err(e));
        }

        let raw_token = token.unwrap();

        if self.chunk.data.len() == self.offset_from_chunk_start as usize {
            return None;
        }

        match raw_token {
            BinXMLRawToken::EndOfStream => {
                debug!("End of stream");
                Some(Ok(BinXMLDeserializedTokens::EndOfStream))
            }
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Some(Ok(BinXMLDeserializedTokens::OpenStartElement(
                    self.read_open_start_element(token_information.has_attributes)?,
                )))
            }
            BinXMLRawToken::CloseStartElement => {
                debug!("Close start element");
                Some(Ok(BinXMLDeserializedTokens::CloseStartElement))
            }
            BinXMLRawToken::CloseEmptyElement => Ok(BinXMLDeserializedTokens::CloseEmptyElement),
            BinXMLRawToken::CloseElement => {
                debug!("Close element");
                Some(Ok(BinXMLDeserializedTokens::CloseElement))
            }
            BinXMLRawToken::Value => Some(Ok(BinXMLDeserializedTokens::Value(self.read_value()?))),
            BinXMLRawToken::Attribute(token_information) => Some(Ok(
                BinXMLDeserializedTokens::Attribute(self.read_attribute()?),
            )),
            BinXMLRawToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLRawToken::EntityReference => unimplemented!("BinXMLToken::EntityReference"),
            BinXMLRawToken::ProcessingInstructionTarget => {
                unimplemented!("BinXMLToken::ProcessingInstructionTarget")
            }
            BinXMLRawToken::ProcessingInstructionData => {
                unimplemented!("BinXMLToken::ProcessingInstructionData")
            }
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                Some(self.read_template()),
            )),
            BinXMLRawToken::NormalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(Some(
                self.read_substitution(false),
            ))),
            BinXMLRawToken::ConditionalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                Some(self.read_substitution(true)),
            )),
            BinXMLRawToken::StartOfStream => Ok(BinXMLDeserializedTokens::FragmentHeader(Some(
                self.read_fragment_header(),
            ))),
        }
    }
}

/// The BinXMLDeserializer struct deserialized a chunk of EVTX data into a stream of elements.
/// It is used by initializing it with an EvtxChunk.
/// It yields back a deserialized token stream, while also taking care of substitutions of templates.
impl<'a> BinXMLDeserializer<'a> {
    pub fn new(chunk: &'a EvtxChunk<'a>, offset_from_chunk_start: u64) -> BinXMLDeserializer<'a> {
        let binxml_data = &chunk.data[offset_from_chunk_start as usize..];
        let cursor = Cursor::new(binxml_data);

        BinXMLDeserializer {
            chunk,
            offset_from_chunk_start,
            cursor,
        }
    }

    fn visit_token(
        &mut self,
        token: &'a BinXMLDeserializedTokens<'a>,
        visitor: &mut impl Visitor<'a>,
    ) -> Result<(), Error> {
        match token {
            // Encountered a template, we need to fill the template, replacing values as needed and
            // presenting them to the visitor.
            BinXMLDeserializedTokens::TemplateInstance(template) => {
                let template_tokens = self.read_until_end_of_stream()?;

                for token in template_tokens.iter() {
                    let replacement = template.substitute_token_if_needed(token);
                    match replacement {
                        Replacement::Token(token) => self.visit_token(token, visitor)?,
                        Replacement::Value(value) => visitor.visit_value(value),
                    }
                }
            }
            _ => unimplemented!(),
        }
        Ok(())
    }
}

mod tests {
    use super::*;
    use evtx::evtx_record_header;

    extern crate env_logger;

    #[test]
    fn test_reads_one_element() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(&from_start_of_chunk).unwrap();

        debug!("{:#?}", &chunk);

        let mut deserializer = BinXMLDeserializer::new(&chunk, 512 + 24);

        let element = deserializer.read_until_end_of_stream().unwrap();
        println!("{:?}", element);
        assert_eq!(
            element.len(),
            2,
            "Element should contain a fragment and a template"
        );

        let is_template = match element[1] {
            BinXMLDeserializedTokens::TemplateInstance(_) => true,
            _ => false,
        };

        assert!(is_template, "Element should be a template");

        // Weird zeroes (padding?)
        let mut zeroes = [0_u8; 3];

        let c = &mut deserializer.cursor;
        c.take(3)
            .read_exact(&mut zeroes)
            .expect("Failed to read zeroes");

        let copy_of_size = c.read_u32::<LittleEndian>().unwrap();
        //        assert_eq!(
        //            record_header.data_size, copy_of_size,
        //            "Didn't read expected amount of bytes."
        //        );
    }

    #[test]
    fn test_reads_simple_template_without_substitutions() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];

        let chunk = EvtxChunk::new(&from_start_of_chunk).unwrap();

        let template = &from_start_of_chunk[1979..2064];
        let mut d = BinXMLDeserializer::new(&chunk, 1979);

        let element = d.read_until_end_of_stream().unwrap();
        assert_eq!(
            d.position_relative_to_chunk_start(),
            2064,
            "Template was not fully read."
        )
    }
}
