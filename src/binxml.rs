use std::borrow::BorrowMut;
use std::cmp::min;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom};
use std::mem;
use std::rc::Rc;

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use failure::Error;

use evtx::datetime_from_filetime;
use evtx_chunk_header::EvtxChunk;
use evtx_chunk_header::EvtxChunkHeader;
use guid::Guid;
use model::*;
use std::borrow::{Borrow, Cow};
use std::io::Cursor;
use utils::*;
use xml_builder::Visitor;

#[derive(Fail, Debug)]
enum BinXmlDeserializationError {
    #[fail(
        display = "Expected attribute token to follow attribute name at position {}",
        position
    )]
    ExpectedValue { position: u64 },
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

    /// Reads an element from the serialized XML
    /// An Element Begins with a BinXMLFragmentHeader, and ends with an EOF.
    pub fn read_until_end_of_stream(&mut self) -> Result<Vec<BinXMLDeserializedTokens<'a>>, Error> {
        let mut tokens = vec![];

        loop {
            let token = self.get_next_token()?;
            if token != BinXMLDeserializedTokens::EndOfStream {
                tokens.push(token);
            } else {
                break;
            }
        }

        Ok(tokens)
    }

    pub fn visit_next(&mut self, visitor: &mut impl Visitor<'a>) -> Result<(), Error> {
        let next_token = self.get_next_token()?;
        self.visit_token(&next_token, visitor);
        Ok(())
    }

    fn visit_token(&mut self, token: &BinXMLDeserializedTokens, visitor: &mut impl Visitor<'a>) -> Result<(), Error> {
        match token {
            // Fill the template
            BinXMLDeserializedTokens::TemplateInstance(t) => {
                let template = self.read_until_end_of_stream()?;
                for e in t.definition.element.iter() {
                    let replacement = t.substitute_token_if_needed(e);
                    match replacement {
                        Replacement::Token(token) => self.visit_token(token, visitor)?,
                        Replacement::Value(value) => visitor.visit_value(value)
                    }
                }
            },
            _ => unimplemented!("{:?}", token)
        }
        Ok(())
    }

    fn read_next_token(&mut self) -> Option<BinXMLRawToken> {
        let token = self.cursor.read_u8().expect("Unexpected EOF");

        BinXMLRawToken::from_u8(token)
            // Unknown token.
            .or_else(|| {
                error!("{:2x} not a valid binxml token", token);
                &self.dump_and_panic(10);
                None
            })
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
            BinXMLValueType::BinXmlType => Ok(BinXMLValue::BinXmlType(self.read_until_end_of_stream()?)),
            BinXMLValueType::EvtXml => unimplemented!(),
            _ => unimplemented!("{:?}", value_type),
        }
    }

    fn get_next_token(&mut self) -> Result<BinXMLDeserializedTokens<'a>, Error> {
        let token = self.read_next_token().unwrap();

        match token {
            BinXMLRawToken::EndOfStream => {
                debug!("End of stream");
                Ok(BinXMLDeserializedTokens::EndOfStream)
            }
            BinXMLRawToken::OpenStartElement(token_information) => {
                // Debug print inside
                Ok(BinXMLDeserializedTokens::OpenStartElement(
                    self.read_open_start_element(token_information.has_attributes)?,
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
            BinXMLRawToken::Value => Ok(BinXMLDeserializedTokens::Value(self.read_value()?)),
            BinXMLRawToken::Attribute(token_information) => {
                Ok(BinXMLDeserializedTokens::Attribute(self.read_attribute()?))
            }
            BinXMLRawToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLRawToken::EntityReference => unimplemented!("BinXMLToken::EntityReference"),
            BinXMLRawToken::ProcessingInstructionTarget => {
                unimplemented!("BinXMLToken::ProcessingInstructionTarget")
            }
            BinXMLRawToken::ProcessingInstructionData => {
                unimplemented!("BinXMLToken::ProcessingInstructionData")
            }
            BinXMLRawToken::TemplateInstance => Ok(BinXMLDeserializedTokens::TemplateInstance(
                self.read_template()?,
            )),
            BinXMLRawToken::NormalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                self.read_substitution(false)?,
            )),
            BinXMLRawToken::ConditionalSubstitution => Ok(BinXMLDeserializedTokens::Substitution(
                self.read_substitution(true)?,
            )),
            BinXMLRawToken::StartOfStream => Ok(BinXMLDeserializedTokens::FragmentHeader(
                self.read_fragment_header()?,
            )),
        }
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
        f: &Fn(&mut BinXMLDeserializer<'a>) -> Result<T, Error>,
    ) -> Result<T, Error> {
        if offset == self.position_relative_to_chunk_start() {
            f(self)
        } else {
            // Fork a new context at the given offset, and read there.
            // This ensures our state will not be mutated.
            let mut temp_ctx = BinXMLDeserializer::new(&self.chunk, offset);
            f(&mut temp_ctx)
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
