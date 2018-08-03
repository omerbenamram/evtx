use core::mem;
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use std::cmp::min;
use std::io::{self, Error, ErrorKind, Read, Result, Seek, SeekFrom};

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};

use binxml::model::{
    BinXMLAttribute, BinXMLFragmentHeader, BinXMLName, BinXMLTemplate, BinXMLValueText,
};
use binxml::model::{BinXMLParsedNodes, BinXMLToken, BinXMLValueTypes};
use binxml::utils::read_len_prefixed_utf16_string;

use binxml::model::BinXMLOpenStartElement;
use binxml::model::EndOfStream;
use binxml::model::TemplateValueDescriptor;
use evtx_parser::evtx_chunk_header;
use guid::Guid;
use std::borrow::{Borrow, Cow};
use std::io::Cursor;

struct ChunkCtx<'a> {
    data: &'a [u8],
    record_number: u32,
}

struct BinXMLTokenStream<'a> {
    chunk: &'a ChunkCtx<'a>,
    offset_from_chunk_start: u64,
    cursor: Cursor<&'a [u8]>,
}

impl<'a> BinXMLTokenStream<'a> {
    pub fn new(
        xml_raw: &'a [u8],
        chunk: &'a ChunkCtx,
        offset_from_chunk_start: u64,
    ) -> BinXMLTokenStream<'a> {
        let cursor = Cursor::new(xml_raw);

        BinXMLTokenStream {
            chunk,
            offset_from_chunk_start,
            cursor,
        }
    }

    fn read_next_token(&mut self) -> Option<BinXMLToken> {
        let token = self.cursor.read_u8().expect("EOF");

        BinXMLToken::from_u8(token)
            // Unknown token.
            .or_else(|| {
                error!("{:2x} not a valid binxml token", token);
                &self.dump_and_panic(10);
                None
            })
    }

    pub fn get_next_token(&mut self) -> io::Result<BinXMLParsedNodes> {
        let token = self.read_next_token().unwrap();

        match token {
            BinXMLToken::EndOfStream => Ok(BinXMLParsedNodes::EndOfStream),
            BinXMLToken::OpenStartElement(token_information) => {
                Ok(BinXMLParsedNodes::OpenStartElement(
                    self.read_open_start_element(token_information.has_attributes)?,
                ))
            }
            BinXMLToken::CloseStartElement => unimplemented!("BinXMLToken::CloseStartElement"),
            BinXMLToken::CloseEmptyElement => unimplemented!("BinXMLToken::CloseEmptyElement"),
            BinXMLToken::CloseElement => unimplemented!("BinXMLToken::CloseElement"),
            BinXMLToken::TextValue => Ok(BinXMLParsedNodes::ValueText(self.read_value_text()?)),
            BinXMLToken::Attribute(token_information) => {
                Ok(BinXMLParsedNodes::Attribute(self.read_attribute()?))
            }
            BinXMLToken::CDataSection => unimplemented!("BinXMLToken::CDataSection"),
            BinXMLToken::EntityReference => unimplemented!("BinXMLToken::EntityReference"),
            BinXMLToken::ProcessingInstructionTarget => {
                unimplemented!("BinXMLToken::ProcessingInstructionTarget")
            }
            BinXMLToken::ProcessingInstructionData => {
                unimplemented!("BinXMLToken::ProcessingInstructionData")
            }
            BinXMLToken::TemplateInstance => {
                Ok(BinXMLParsedNodes::TemplateInstance(self.read_template()?))
            }
            BinXMLToken::NormalSubstitution => unimplemented!("BinXMLToken::NormalSubstitution"),
            BinXMLToken::ConditionalSubstitution => {
                unimplemented!("BinXMLToken::ConditionalSubstitution")
            }
            BinXMLToken::StartOfStream => Ok(BinXMLParsedNodes::FragmentHeader(
                self.read_fragment_header()?,
            )),
        }
    }

    fn position_relative_to_chunk_start(&self) -> u64 {
        // 24 is record header size, 512 is chunk header.
        self.cursor.position() + self.offset_from_chunk_start
    }

    fn read_tokens_until_eof(&mut self) -> io::Result<Arena<BinXMLParsedNodes>> {
        // TODO: think about how this can be a tree
        let mut tokens = Arena::new();

        while self.get_next_token()? != BinXMLParsedNodes::EndOfStream {
            tokens.new_node(self.get_next_token()?);
        }

        Ok(tokens)
    }

    fn read_element_relative_to_chunk_offset(
        &mut self,
        offset: u64,
    ) -> io::Result<Arena<BinXMLParsedNodes>> {
        debug!("reading next element relative to offset {}", offset);
        if offset == self.position_relative_to_chunk_start() {
            debug!("offset asked is current offset");
            return self.read_tokens_until_eof();
        }

        let mut temp_ctx =
            BinXMLTokenStream::new(&self.chunk.data[offset as usize..], self.chunk, offset);

        temp_ctx.read_tokens_until_eof()
    }

    // TODO: fix my return type!
    fn parse_value(&mut self) -> io::Result<BinXMLValueText> {
        let value_type_token = self.cursor.read_u8().expect("EOF");
        let value_type = BinXMLValueTypes::from_u8(value_type_token)
            .or_else(|| {
                println!("{:2x} not a valid value type", value_type_token);
                None
            }).unwrap();

        let value = match value_type {
            BinXMLValueTypes::StringType => self.read_value_text().expect("Failed to read value"),
            _ => unimplemented!(),
        };

        debug!("visit_value returned {:?}", value);

        Ok(value)
    }

    fn read_relative_to_chunk_offset<T: Sized>(
        &mut self,
        offset: u64,
        f: Box<Fn(&mut BinXMLTokenStream) -> io::Result<T>>,
    ) -> io::Result<T> {
        debug!(
            "Offset {}, position {}",
            offset,
            self.position_relative_to_chunk_start()
        );
        if offset == self.position_relative_to_chunk_start() {
            f(self)
        } else {
            let mut temp_ctx =
                BinXMLTokenStream::new(&self.chunk.data[offset as usize..], &self.chunk, offset);
            f(&mut temp_ctx)
        }
    }

    fn dump_and_panic(&self, lookbehind: i32) {
        let offset = self.cursor.position();
        println!("Panicked at offset {}", offset);
        self.dump(lookbehind);
        panic!();
    }

    fn dump(&self, lookbehind: i32) {
        let offset = self.cursor.position();
        let data = self.cursor.get_ref();
        println!("-------------------------------");
        println!("Current Value {:2X}", data[offset as usize]);
        let m = (offset as i32) - lookbehind;
        let start = if m < 0 { 0 } else { m };
        print_hexdump(&data[start as usize..(offset + 100) as usize], 0, 'C');
        println!("\n-------------------------------");
    }

    fn read_open_start_element(
        &mut self,
        has_attributes: bool,
    ) -> io::Result<BinXMLOpenStartElement> {
        debug!(
            "OpenStartElement at {}, has_attributes: {}",
            self.cursor.position(),
            has_attributes
        );
        self.cursor.read_u16::<LittleEndian>()?;
        let data_size = self.cursor.read_u32::<LittleEndian>()?;
        let name = self.read_name()?;

        let attribute_list = match has_attributes {
            true => {
                let attribute_list_data_size = self.cursor.read_u32::<LittleEndian>()?;
                debug!("attribute list data_size: {}", attribute_list_data_size);
                let initial_position = self.cursor.position();
                let mut attributes = vec![];

                loop {
                    if let Some(token) = self.read_next_token() {
                        match token {
                            BinXMLToken::Attribute(token_meta) => {
                                attributes.push(self.read_attribute()?);
                                if !token_meta.more_attributes_expected {
                                    assert_eq!(
                                        self.cursor.position(),
                                        initial_position + attribute_list_data_size as u64
                                    );
                                    break;
                                }
                            },
                            _ => self.dump_and_panic(10)
                        }
                    }
                }
                Some(attributes)
            }
            false => None,
        };

        Ok(BinXMLOpenStartElement {
            data_size,
            name,
            attribute_list,
        })
    }

    fn read_name(&mut self) -> io::Result<BinXMLName> {
        debug!("Name at {}", self.cursor.position());
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let name_offset = self.cursor.read_u32::<LittleEndian>()?;
        let name_offset = name_offset as u64;

        let _read_name = |ctx: &mut BinXMLTokenStream| {
            let _ = ctx.cursor.read_u32::<LittleEndian>()?;
            let name_hash = ctx.cursor.read_u16::<LittleEndian>()?;

            let name = read_len_prefixed_utf16_string(&mut ctx.cursor, true)?;

            Ok(BinXMLName { name })
        };

        self.read_relative_to_chunk_offset(name_offset, box _read_name)
    }

    fn read_template(&mut self) -> io::Result<BinXMLTemplate> {
        debug!("TemplateInstance at {}", self.cursor.position());
        self.cursor.read_u8()?;
        let template_id = self.cursor.read_u32::<LittleEndian>()?;
        let template_definition_data_offset = self.cursor.read_u32::<LittleEndian>()?;

        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let _read_template = move |ctx: &mut BinXMLTokenStream| {
            // Used to assert that we read all data;
            let start_position = ctx.cursor.position();

            let next_template_offset = ctx.cursor.read_u32::<LittleEndian>()?;
            let template_guid = Guid::from_stream(&mut ctx.cursor)?;
            let data_size = ctx.cursor.read_u32::<LittleEndian>()?;

            let element = ctx.read_tokens_until_eof()?;
            //            let number_of_template_values = stream.read_u32::<LittleEndian>()?;
            //
            //            let mut value_descriptors: Vec<TemplateValueDescriptor> = Vec::new();
            //            for _ in number_of_template_values.. {
            //                let value_size = stream.read_u16::<LittleEndian>()?;
            //                let value_type = stream.read_u8()?;
            //                stream.read_u8()?;
            //                value_descriptors.push(TemplateValueDescriptor {
            //                    value_size,
            //                    value_type,
            //                });
            //            }
            assert_eq!(
                ctx.cursor.position(),
                ctx.cursor.position() + data_size as u64,
                "Template wasn't read completely"
            );
            Ok(BinXMLTemplate {
                template_id,
                template_offset: template_definition_data_offset,
                next_template_offset,
                template_guid,
                data_size,
            })
        };

        self.read_relative_to_chunk_offset(
            template_definition_data_offset as u64,
            box _read_template,
        )
    }

    fn read_attribute_list(&mut self) {}
    fn read_attribute(&mut self) -> io::Result<BinXMLAttribute> {
        debug!("Attribute at {}", self.cursor.position());
        let name = self.read_name()?;
        let attribute_data = self.get_next_token()?;
        debug!("{:?}", attribute_data);
        Ok(BinXMLAttribute { name, data: attribute_data})
    }

    fn read_value_text(&mut self) -> io::Result<BinXMLValueText> {
        debug!("TextValue at {}", self.cursor.position());
        let value_type = BinXMLValueTypes::from_u8(self.cursor.read_u8()?).unwrap();
        assert_eq!(value_type, BinXMLValueTypes::StringType, "TextValue must be a StringType");

        let raw = read_len_prefixed_utf16_string(&mut self.cursor, false)?
            .expect("Value cannot be empty");
        Ok(BinXMLValueText { raw })
    }

    fn read_fragment_header(&mut self) -> io::Result<BinXMLFragmentHeader> {
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
    use evtx_parser::evtx_chunk_header;
    use hexdump;

    extern crate env_logger;

    #[test]
    fn test_basic_binxml() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];
        let chunk = ChunkCtx {
            data: &from_start_of_chunk,
            record_number: 1,
        };
        let mut token_stream =
            BinXMLTokenStream::new(&from_start_of_chunk[512 + 24..], &chunk, 512 + 24);
        let token = token_stream.get_next_token().unwrap();

        assert_eq!(
            token,
            BinXMLParsedNodes::FragmentHeader(BinXMLFragmentHeader {
                major_version: 1,
                minor_version: 1,
                flags: 0,
            })
        );

        let token = token_stream.get_next_token().unwrap();
        assert_eq!(
            token,
            BinXMLParsedNodes::FragmentHeader(BinXMLFragmentHeader {
                major_version: 1,
                minor_version: 1,
                flags: 0,
            })
        );
    }
}
