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

struct BinXMLTokenStream<'a> {
    cursor: Cursor<&'a [u8]>,
}

impl<'a> BinXMLTokenStream<'a> {
    pub fn new(data: &'a [u8]) -> BinXMLTokenStream {
        let cursor = Cursor::new(data);

        BinXMLTokenStream { cursor }
    }

    pub fn get_next_token(&mut self) -> io::Result<BinXMLParsedNodes> {
        let token = self.cursor.read_u8().expect("EOF");

        let token = BinXMLToken::from_u8(token)
            // Unknown token.
            .or_else(|| {
                error!("{:2x} not a valid binxml token", token);
                &self.dump_and_panic(10);
                None
            })
            .unwrap();

        match token {
            BinXMLToken::EndOfStream => Ok(BinXMLParsedNodes::EndOfStream),
            BinXMLToken::OpenStartElement(_) => Ok(BinXMLParsedNodes::OpenStartElement(
                self.read_open_start_element()?,
            )),
            BinXMLToken::CloseStartElement => unimplemented!("BinXMLToken::CloseStartElement"),
            BinXMLToken::CloseEmptyElement => unimplemented!("BinXMLToken::CloseEmptyElement"),
            BinXMLToken::CloseElement => unimplemented!("BinXMLToken::CloseElement"),
            BinXMLToken::TextValue => Ok(BinXMLParsedNodes::ValueText(self.read_value_text()?)),
            BinXMLToken::Attribute => Ok(BinXMLParsedNodes::Attribute(self.read_attribute()?)),
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
        self.cursor.position() + 512
    }

    fn read_element_relative_to_chunk_offset(
        &mut self,
        offset: u64,
    ) -> io::Result<BinXMLParsedNodes> {
        if offset == self.position_relative_to_chunk_start() {
            return self.get_next_token();
        }

        let mut temp_cursor = Cursor::new(*self.cursor.get_ref());

        temp_cursor.seek(SeekFrom::Start(offset))?;
        let mut temp_ctx = BinXMLTokenStream {
            cursor: temp_cursor,
        };

        temp_ctx.get_next_token()
    }

    // TODO: fix my return type!
    fn parse_value(&mut self) -> io::Result<BinXMLValueText> {
        let value_type_token = self.cursor.read_u8().expect("EOF");
        let value_type = BinXMLValueTypes::from_u8(value_type_token)
            .or_else(|| {
                println!("{:2x} not a valid value type", value_type_token);
                None
            })
            .unwrap();

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
        f: Box<Fn(&mut Cursor<&'a [u8]>) -> io::Result<T>>,
    ) -> io::Result<T> {
        let mut temp_cursor = Cursor::new(*self.cursor.get_ref());
        temp_cursor.seek(SeekFrom::Start(offset + 24))?;
        f(&mut temp_cursor)
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

    fn read_open_start_element(&mut self) -> io::Result<BinXMLOpenStartElement> {
        debug!("OpenStartElement");
        self.cursor.read_u16::<LittleEndian>()?;
        let data_size = self.cursor.read_u32::<LittleEndian>()?;
        let name = self.read_name()?;
        let attribute_list_data_size = self.cursor.read_u32::<LittleEndian>()?;

        let attribute_list = match attribute_list_data_size {
            0 => None,
            _ => Some(Vec::new()),
        };

        Ok(BinXMLOpenStartElement {
            data_size,
            name,
            attribute_list,
        })
    }

    fn read_name(&mut self) -> io::Result<BinXMLName> {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let offset_from_start_of_chunk = self.cursor.read_u32::<LittleEndian>()?;
        let offset_from_start_of_chunk = offset_from_start_of_chunk as u64;

        let _read_name = |stream: &mut Cursor<&[u8]>| {
            let _ = stream.read_u32::<LittleEndian>()?;
            let name_hash = stream.read_u16::<LittleEndian>()?;

            let name = read_len_prefixed_utf16_string(stream, true)?;

            Ok(BinXMLName { name })
        };

        if offset_from_start_of_chunk == self.position_relative_to_chunk_start() {
            _read_name(&mut self.cursor)
        } else {
            self.read_relative_to_chunk_offset(offset_from_start_of_chunk, Box::new(_read_name))
        }
    }

    fn read_template(&mut self) -> io::Result<BinXMLTemplate> {
        debug!("TemplateInstance");
        self.cursor.read_u8()?;
        let template_id = self.cursor.read_u32::<LittleEndian>()?;
        let template_offset = self.cursor.read_u32::<LittleEndian>()?;
        let next_template_offset = self.cursor.read_u32::<LittleEndian>()?;

        let template_guid = Guid::from_stream(&mut self.cursor)?;
        let data_size = self.cursor.read_u32::<LittleEndian>()?;

        // TODO: make sure this works
        let element = self.read_element_relative_to_chunk_offset(template_offset as u64);

        println!("{:?}", element);
        match element {
            Ok(BinXMLParsedNodes::EndOfStream) => {}
            _ => unimplemented!("Only end of stream is expected for now."),
        }

        let number_of_template_values = self.cursor.read_u32::<LittleEndian>()?;

        assert_eq!(number_of_template_values, 4, "Too many elements");

        let mut value_descriptors: Vec<TemplateValueDescriptor> = Vec::new();
        for _ in number_of_template_values.. {
            let value_size = self.cursor.read_u16::<LittleEndian>()?;
            let value_type = self.cursor.read_u8()?;
            self.cursor.read_u8()?;
            value_descriptors.push(TemplateValueDescriptor {
                value_size,
                value_type,
            });
        }

        Ok(BinXMLTemplate {
            template_id,
            template_offset,
            next_template_offset,
            template_guid,
            data_size,
        })
    }

    fn read_attribute(&mut self) -> io::Result<BinXMLAttribute> {
        debug!("Attribute");
        let name = self.read_name()?;
        Ok(BinXMLAttribute { name })
    }

    fn read_value_text(&mut self) -> io::Result<BinXMLValueText> {
        debug!("TextValue");
        let raw = read_len_prefixed_utf16_string(&mut self.cursor, false)?
            .expect("Value cannot be empty");
        Ok(BinXMLValueText { raw })
    }

    fn read_fragment_header(&mut self) -> io::Result<BinXMLFragmentHeader> {
        debug!("FragmentHeader");
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
        let mut token_stream = BinXMLTokenStream::new(&from_start_of_chunk[512 + 24..]);
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
