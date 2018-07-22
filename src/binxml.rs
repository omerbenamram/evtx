use core::mem;
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use nom::verbose_errors::Context;
use nom::{le_u16, le_u32, le_u64, le_u8, Err as NomErr, ErrorKind, IResult};
use std::cmp::min;
use std::io::{self, Read, Result, Seek, SeekFrom};

use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};

use encoding::all::UTF_16LE;
use encoding::DecoderTrap;
use encoding::Encoding;
use evtx_parser::evtx_chunk_header;
use guid::Guid;
use std::borrow::Cow;
use std::io::Cursor;

trait FromStream {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> io::Result<Self>
    where
        Self: Sized;
}

impl FromStream for Guid {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> io::Result<Self>
    where
        Self: Sized,
    {
        let data1 = stream.read_u32::<LittleEndian>()?;
        let data2 = stream.read_u16::<LittleEndian>()?;
        let data3 = stream.read_u16::<LittleEndian>()?;
        let mut data4 = [0; 8];
        stream.read_exact(&mut data4)?;
        Ok(Guid::new(data1, data2, data3, &data4))
    }
}

enum BinXMLValueTypes {
    BinXmlTokenEOF,
    BinXmlTokenOpenStartElementTag,
    BinXmlTokenCloseStartElementTag,
    BinXmlTokenCloseEmptyElementTag,
    BinXmlTokenEndElementTag,
    BinXmlTokenValue,
    BinXmlTokenAttribute,
    BinXmlTokenCDATASection,
    BinXmlTokenCharRef,
    BinXmlTokenEntityRef,
    BinXmlTokenPITarget,
    BinXmlTokenPIData,
    BinXmlTokenTemplateInstance,
    BinXmlTokenNormalSubstitution,
    BinXmlTokenOptionalSubstitution,
    BinXmlFragmentHeaderToken,
}

impl BinXMLValueTypes {
    fn from_u8(byte: u8) -> Option<BinXMLValueTypes> {
        match byte {
            0x00 => Some(BinXMLValueTypes::BinXmlTokenEOF),
            0x01 | 0x41 => Some(BinXMLValueTypes::BinXmlTokenOpenStartElementTag),
            0x02 => Some(BinXMLValueTypes::BinXmlTokenCloseStartElementTag),
            0x03 => Some(BinXMLValueTypes::BinXmlTokenCloseEmptyElementTag),
            0x04 => Some(BinXMLValueTypes::BinXmlTokenEndElementTag),
            0x05 | 0x45 => Some(BinXMLValueTypes::BinXmlTokenValue),
            0x06 | 0x46 => Some(BinXMLValueTypes::BinXmlTokenAttribute),
            0x07 | 0x47 => Some(BinXMLValueTypes::BinXmlTokenCDATASection),
            0x08 | 0x48 => Some(BinXMLValueTypes::BinXmlTokenCharRef),
            0x09 | 0x49 => Some(BinXMLValueTypes::BinXmlTokenEntityRef),
            0x0a => Some(BinXMLValueTypes::BinXmlTokenPITarget),
            0x0b => Some(BinXMLValueTypes::BinXmlTokenPIData),
            0x0c => Some(BinXMLValueTypes::BinXmlTokenTemplateInstance),
            0x0d => Some(BinXMLValueTypes::BinXmlTokenNormalSubstitution),
            0x0e => Some(BinXMLValueTypes::BinXmlTokenOptionalSubstitution),
            0x0f => Some(BinXMLValueTypes::BinXmlFragmentHeaderToken),
            _ => None,
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq)]
enum BinXMLToken {
    EndOfStream,
    // True if has attributes, otherwise false.
    OpenStartElement(bool),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    TextValue,
    Attribute,
    CDataSection,
    EntityReference,
    ProcessingInstructionTarget,
    ProcessingInstructionData,
    TemplateInstance,
    NormalSubstitution,
    ConditionalSubstitution,
    StartOfStream,
}

impl BinXMLToken {
    fn from_u8(byte: u8) -> Option<BinXMLToken> {
        match byte {
            0x00 => Some(BinXMLToken::EndOfStream),
            0x01 => Some(BinXMLToken::OpenStartElement(false)),
            0x41 => Some(BinXMLToken::OpenStartElement(true)),
            0x02 => Some(BinXMLToken::CloseStartElement),
            0x03 => Some(BinXMLToken::CloseEmptyElement),
            0x04 => Some(BinXMLToken::CloseElement),
            0x05 | 0x45 => Some(BinXMLToken::TextValue),
            0x06 | 0x46 => Some(BinXMLToken::Attribute),
            0x07 | 0x47 => Some(BinXMLToken::CDataSection),
            0x08 | 0x48 => Some(BinXMLToken::EntityReference),
            0x0a | 0x49 => Some(BinXMLToken::ProcessingInstructionTarget),
            0x0b => Some(BinXMLToken::ProcessingInstructionData),
            0x0c => Some(BinXMLToken::TemplateInstance),
            0x0d => Some(BinXMLToken::NormalSubstitution),
            0x0e => Some(BinXMLToken::ConditionalSubstitution),
            0x0f => Some(BinXMLToken::StartOfStream),
            _ => None,
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct BinXMLFragmentHeader {
    major_version: u8,
    minor_version: u8,
    flags: u8,
}

impl FromStream for BinXMLFragmentHeader {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> io::Result<Self>
    where
        Self: Sized,
    {
        let major_version = stream.read_u8()?;
        let minor_version = stream.read_u8()?;
        let flags = stream.read_u8()?;
        Ok(BinXMLFragmentHeader {
            major_version,
            minor_version,
            flags,
        })
    }
}

struct BinXMLValue {}

#[derive(Debug)]
struct BinXMLTemplate {
    template_id: u32,
    template_offset: u32,
    next_template_offset: u32,
    template_guid: Guid,
    // This includes the size of the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition.
    data_size: u32,
}

#[derive(Debug)]
struct TemplateValueDescriptor {
    value_size: u16,
    value_type: u8,
}

impl FromStream for BinXMLTemplate {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> Result<Self>
    where
        Self: Sized,
    {
        stream.read_u8()?;
        let template_id = stream.read_u32::<LittleEndian>()?;
        let template_offset = stream.read_u32::<LittleEndian>()?;
        let next_template_offset = stream.read_u32::<LittleEndian>()?;
        let template_guid = Guid::read(stream)?;
        let data_size = stream.read_u32::<LittleEndian>()?;

        let element = parse_binxml(&stream.get_ref(), (template_offset + 24) as u64);
        let number_of_template_values = stream.read_u32::<LittleEndian>()?;

        assert_eq!(number_of_template_values, 4);

        let mut value_descriptors: Vec<TemplateValueDescriptor> = Vec::new();
        for _ in number_of_template_values.. {
            let value_size = stream.read_u16::<LittleEndian>()?;
            let value_type = stream.read_u8()?;
            stream.read_u8()?;
            value_descriptors.push(TemplateValueDescriptor {
                value_size,
                value_type,
            });
        }

        println!("{:?}", value_descriptors);
        Ok(BinXMLTemplate {
            template_id,
            template_offset,
            next_template_offset,
            template_guid,
            data_size,
        })
    }
}

#[derive(Debug)]
struct BinXMLAttribute {
    name: BinXMLName,
}

impl FromStream for BinXMLAttribute {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> Result<Self>
    where
        Self: Sized,
    {
        let name = BinXMLName::read(stream)?;
        Ok(BinXMLAttribute { name })
    }
}

#[derive(Debug)]
struct BinXMLName {
    name: Option<String>,
}

impl FromStream for BinXMLName {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> Result<Self>
    where
        Self: Sized,
    {
        // Important!!
        // The "offset_from_start" refers to the offset where the name struct begins.
        let offset_from_start_of_chunk = stream.read_u32::<LittleEndian>()?;
        let offset_from_start_of_chunk = offset_from_start_of_chunk as u64;

        let mut rollback = false;
        let orig_position = stream.position();

        // TODO: test this.
        if offset_from_start_of_chunk != stream.position() {
            debug!("Seeking to {}", offset_from_start_of_chunk);
            stream.seek(SeekFrom::Start(offset_from_start_of_chunk))?;
            rollback = true;
        }

        let _ = stream.read_u32::<LittleEndian>()?;
        let name_hash = stream.read_u16::<LittleEndian>()?;

        let expected_number_of_characters = stream.read_u16::<LittleEndian>()?;
        let needed_bytes = (expected_number_of_characters * 2) as u64;

        let name: Option<String> = match expected_number_of_characters {
            0 => None,
            _ => {
                let s = UTF_16LE
                    .decode(
                        &stream.get_ref()[stream.position() as usize
                                              ..(stream.position() + needed_bytes) as usize],
                        DecoderTrap::Strict,
                    )
                    .expect("Failed to read UTF-16 string");

                assert_eq!(
                    s.len(),
                    expected_number_of_characters as usize,
                    "UTF-16 string mismatch"
                );

                stream.seek(SeekFrom::Current(needed_bytes as i64))?;
                // TODO: only do this if string is null terminated, not all strings are.
                stream.seek(SeekFrom::Current(2))?;
                Some(s)
            }
        };

        if rollback {
            stream.seek(SeekFrom::Start(orig_position))?;
        }

        println!("{:?}", name);
        // TODO: do i need move the cursor somehow in here?
        Ok(BinXMLName { name })
    }
}

struct BinXMLValueText<'a> {
    raw: Cow<'a, str>,
}

//impl<'a> FromStream for BinXMLValueText<'a> {
//    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> Result<Self>
//    where
//        R: Read,
//        Self: Sized,
//    {
//
//    }
//}

#[derive(Debug)]
struct BinXMLOpenElementStartTag {
    data_size: u32,
    name: BinXMLName,
    attribute_list: Option<Vec<BinXMLAttribute>>,
}

impl FromStream for BinXMLOpenElementStartTag {
    fn read<'a>(stream: &mut Cursor<&'a [u8]>) -> Result<Self>
    where
        Self: Sized,
    {
        // Unused
        stream.read_u16::<LittleEndian>()?;
        let data_size = stream.read_u32::<LittleEndian>()?;
        let name = BinXMLName::read(stream)?;
        let attribute_list_data_size = stream.read_u32::<LittleEndian>()?;

        println!("{}, {:?}", attribute_list_data_size, name);

        let attribute_list = match attribute_list_data_size {
            0 => None,
            _ => Some(Vec::new()),
        };

        Ok(BinXMLOpenElementStartTag {
            data_size,
            name,
            attribute_list,
        })
    }
}

#[derive(Debug)]
enum BinXMLNodes {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstanceToken(BinXMLTemplate),
    OpenStartElementTag(BinXMLOpenElementStartTag),
    AttributeList,
    Attribute(BinXMLAttribute),
    FragmentHeaderToken,
    OpenStartElementToken,
    CloseStartElementToken,
    CloseEmptyElementToken,
    CloseElementToken,
    ValueTextToken,
    AttributeToken,
    CDATASectionToken,
    CharRefToken,
    EntityRefToken,
    PITargetToken,
    PIDataToken,
    NormalSubstitutionToken,
    OptionalSubstitutionToken,
}

struct BinXMLParseCtx<'a> {
    data: &'a [u8],
    cursor: Cursor<&'a [u8]>,
    template: Option<BinXMLTemplate>,
    xml: Arena<BinXMLNodes>,
    current_parent: Option<NodeId>,
}

impl<'a> BinXMLParseCtx<'a> {
    fn new(data: &'a [u8], offset: u64) -> BinXMLParseCtx {
        let mut cursor = Cursor::new(data);
        cursor
            .seek(SeekFrom::Start(offset))
            .expect("Not enough data");
        BinXMLParseCtx {
            data,
            cursor,
            template: None,
            xml: Arena::new(),
            current_parent: None,
        }
    }
    fn add_leaf(&mut self, node: NodeId) -> () {
        self.current_parent.unwrap().append(node, &mut self.xml);
    }

    fn add_node(&mut self, node: NodeId) -> () {
        match self.current_parent {
            Some(parent) => {
                parent.append(node, &mut self.xml);
                self.current_parent = Some(node);
            }
            None => self.current_parent = Some(node),
        }
    }
}

fn visit_end_of_stream(ctx: &mut BinXMLParseCtx) {
    println!("visit_end_of_stream");
}

fn visit_open_start_element(ctx: &mut BinXMLParseCtx, tok: &BinXMLToken) {
    debug!("visit start_element {:?}", tok);
    let tag = BinXMLOpenElementStartTag::read(&mut ctx.cursor).expect("Failed to parse open tag");
    let node = ctx.xml.new_node(BinXMLNodes::OpenStartElementTag(tag));
    ctx.add_node(node);
}

fn visit_close_start_element(ctx: &mut BinXMLParseCtx) {
    println!("visit_close_start_element");
    unimplemented!();
}

fn visit_close_empty_element(ctx: &mut BinXMLParseCtx) {
    println!("visit_close_empty_element");
    unimplemented!();
}

fn visit_close_element(ctx: &mut BinXMLParseCtx) {
    println!("visit_close_element");
    unimplemented!();
}

fn visit_value(ctx: &mut BinXMLParseCtx) {
    debug!("visit_value");
    unimplemented!()
}

fn visit_attribute(ctx: &mut BinXMLParseCtx) {
    debug!("visit_attribute");
    let attribute = BinXMLAttribute::read(&mut ctx.cursor).expect("Failed to parse attribute");
    let node = ctx.xml.new_node(BinXMLNodes::Attribute(attribute));
    ctx.add_leaf(node);
}

fn visit_cdata_section(ctx: &mut BinXMLParseCtx) {
    println!("visit_cdata_section");
    unimplemented!();
}

fn visit_entity_reference(ctx: &mut BinXMLParseCtx) {
    println!("visit_entity_reference");
    unimplemented!();
}

fn visit_processing_instruction_target(ctx: &mut BinXMLParseCtx) {
    println!("visit_processing_instruction_target");
    unimplemented!();
}

fn visit_processing_instruction_data(ctx: &mut BinXMLParseCtx) {
    println!("visit_processing_instruction_data");
    unimplemented!();
}

fn visit_normal_substitution(ctx: &mut BinXMLParseCtx) {
    println!("visit_normal_substitution");
    unimplemented!();
}

fn visit_conditional_substitution(ctx: &mut BinXMLParseCtx) {
    println!("visit_conditional_substitution");
    unimplemented!();
}

fn visit_template_instance(ctx: &mut BinXMLParseCtx) {
    debug!("visit_template_instance");
    let template = BinXMLTemplate::read(&mut ctx.cursor).expect("Failed to parse template");
    ctx.template = Some(template);
    println!("{:?}", &ctx.template);
}

fn visit_start_of_stream(ctx: &mut BinXMLParseCtx) {
    debug!("visit_start_of_stream");

    let fragment_header = BinXMLNodes::FragmentHeader(
        BinXMLFragmentHeader::read(&mut ctx.cursor).expect("Failed to read fragment_header"),
    );
    let node = ctx.xml.new_node(fragment_header);
    ctx.add_node(node);
}

pub type BinXML = Arena<BinXMLNodes>;

fn parse_binxml(data: &[u8], offset: u64) -> BinXML {
    let mut ctx = BinXMLParseCtx::new(data, offset);

    //    dump(&mut ctx, 0);
    // TODO: actually break
    for _ in 0..10 {
        let token = ctx.cursor.read_u8().expect("EOF");

        let token = BinXMLToken::from_u8(token)
            // Unknown token.
            .or_else(|| {
                println!("{:2x} not a valid binxml token", token);
                dump_and_panic(&mut ctx, 10);
                None
            })
            .unwrap();

        match token {
            BinXMLToken::EndOfStream => {
                visit_end_of_stream(&mut ctx);
                break;
            }
            BinXMLToken::OpenStartElement(_) => visit_open_start_element(&mut ctx, &token),
            BinXMLToken::CloseStartElement => visit_close_start_element(&mut ctx),
            BinXMLToken::CloseEmptyElement => visit_close_empty_element(&mut ctx),
            BinXMLToken::CloseElement => visit_close_element(&mut ctx),
            BinXMLToken::TextValue => visit_value(&mut ctx),
            BinXMLToken::Attribute => visit_attribute(&mut ctx),
            BinXMLToken::CDataSection => visit_cdata_section(&mut ctx),
            BinXMLToken::EntityReference => visit_entity_reference(&mut ctx),
            BinXMLToken::ProcessingInstructionTarget => {
                visit_processing_instruction_target(&mut ctx)
            }
            BinXMLToken::ProcessingInstructionData => visit_processing_instruction_data(&mut ctx),
            BinXMLToken::TemplateInstance => visit_template_instance(&mut ctx),
            BinXMLToken::NormalSubstitution => visit_normal_substitution(&mut ctx),
            BinXMLToken::ConditionalSubstitution => visit_conditional_substitution(&mut ctx),
            BinXMLToken::StartOfStream => visit_start_of_stream(&mut ctx),
        }
    }

    ctx.xml
}

fn dump_and_panic(ctx: &mut BinXMLParseCtx, lookbehind: i32) {
    let offset = ctx.cursor.position();
    println!("Panicked at offset {}", offset);
    dump(ctx, lookbehind);
    panic!();
}

fn dump(ctx: &mut BinXMLParseCtx, lookbehind: i32) {
    let offset = ctx.cursor.position();
    println!("-------------------------------");
    println!("Current Value {:2X}", ctx.data[offset as usize]);
    let m = (offset as i32) - lookbehind;
    let start = if m < 0 { 0 } else { m };
    print_hexdump(&ctx.data[start as usize..(offset + 100) as usize], 0, 'C');
    println!("\n-------------------------------");
}

mod tests {
    use super::*;
    use evtx_parser::evtx_chunk_header;
    use hexdump;

    extern crate env_logger;

    #[test]
    fn test_basic_binxml() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let evtx_file = include_bytes!("../samples/security.evtx");
        let from_start_of_chunk = &evtx_file[4096..];
        let xml = parse_binxml(from_start_of_chunk, 512 + 24);

        println!("{:?}", xml);
    }
}
