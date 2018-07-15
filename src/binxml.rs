use core::mem;
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use nom::{le_u16, le_u32, le_u64, le_u8, IResult};
use std::cmp::min;
use std::fmt;
use std::fmt::{Debug, Display};

/// Represents how much size should the parser skip for this struct.
trait BinarySize {
    fn size() -> usize;
}

#[derive(Debug, PartialOrd, PartialEq)]
enum BinXMLToken {
    EndOfStream,
    OpenStartElement,
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value,
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
            0x01 | 0x41 => Some(BinXMLToken::OpenStartElement),
            0x02 => Some(BinXMLToken::CloseStartElement),
            0x03 => Some(BinXMLToken::CloseEmptyElement),
            0x04 => Some(BinXMLToken::CloseElement),
            0x05 | 0x45 => Some(BinXMLToken::Value),
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

pub struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

impl Guid {
    pub fn new(data1: u32, data2: u16, data3: u16, data4: &[u8]) -> Guid {
        let mut data4_owned = [0; 8];
        data4_owned.clone_from_slice(&data4[0..8]);
        Guid {
            data1,
            data2,
            data3,
            data4: data4_owned,
        }
    }

    pub fn to_string(&self) -> String {
        format!(
            "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
            self.data1,
            self.data2,
            self.data3,
            self.data4[0],
            self.data4[1],
            self.data4[2],
            self.data4[3],
            self.data4[4],
            self.data4[5],
            self.data4[6],
            self.data4[7]
        )
    }
}

impl Display for Guid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl Debug for Guid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

fn guid(input: &[u8]) -> IResult<&[u8], Guid> {
    // https://msdn.microsoft.com/en-us/library/windows/desktop/aa373931(v=vs.85).aspx
    return do_parse!(
        input,
        data1: le_u32
            >> data2: le_u16
            >> data3: le_u16
            >> data4: take!(8)
            >> (Guid::new(data1, data2, data3, data4))
    );
}

#[derive(Debug)]
struct BinXmlTemplate {
    template_id: u32,
    template_offset: u32,
    next_template_offset: u32,
    template_guid: Guid,
    // This includes the size of the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition.
    data_size: u32,
}

impl BinarySize for BinXmlTemplate {
    fn size() -> usize {
        // Don't forget the skipped (first) byte!!!
        mem::size_of::<BinXmlTemplate>() + 1
    }
}

fn binxml_template(input: &[u8]) -> IResult<&[u8], BinXmlTemplate> {
    return do_parse!(
        input,
        take!(1) // Unknown
       >> template_id: le_u32
       >> template_offset: le_u32
       >> next_template_offset: le_u32
       >> template_guid: guid
       // Currently this is redundant
       >> data_size: le_u32 >> (BinXmlTemplate {
            template_id,
            template_offset,
            next_template_offset,
            template_guid,
            data_size,
        })
    );
}

#[derive(Debug)]
enum BinXMLTokens {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstanceToken(BinXmlTemplate),
    OpenStartElementTag,
    AttributeList,
    Attribute,
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
    offset: usize,
    template: Option<BinXmlTemplate>,
    xml: Arena<BinXMLTokens>,
    current_parent: Option<NodeId>,
}

impl<'a> BinXMLParseCtx<'a> {
    fn new(data: &'a [u8]) -> BinXMLParseCtx {
        BinXMLParseCtx {
            data,
            offset: 0,
            template: None,
            xml: Arena::new(),
            current_parent: None,
        }
    }
}

fn visit_end_of_stream(ctx: &mut BinXMLParseCtx) {
    println!("visit_end_of_stream");
    unimplemented!();
}
fn visit_open_start_element(ctx: &mut BinXMLParseCtx) {
    println!("visit_open_start_element");
    unimplemented!();
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
    println!("visit_value");
    unimplemented!();
}
fn visit_attribute(ctx: &mut BinXMLParseCtx) {
    println!("visit_attribute");
    unimplemented!();
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
    let (_, template) = binxml_template(ctx.data).expect("Failed to parse template");
    ctx.template = Some(template);
    println!("{:?}", &ctx.template);
    ctx.offset += BinXmlTemplate::size();
}

fn visit_start_of_stream(ctx: &mut BinXMLParseCtx) {
    debug!("visit_start_of_stream");

    // TODO: actually extract this header from stream instead of just creating it.
    let fragment_header = BinXMLTokens::FragmentHeader(BinXMLFragmentHeader {
        major_version: 0x01,
        minor_version: 0x01,
        flags: 0x00,
    });

    let node = ctx.xml.new_node(fragment_header);

    match ctx.current_parent {
        Some(parent) => {
            parent.append(node, &mut ctx.xml);
            ctx.current_parent = Some(node);
        }
        None => ctx.current_parent = Some(node),
    }

    ctx.offset += mem::size_of::<BinXMLFragmentHeader>();
}

fn parse_binxml(data: &[u8]) -> Arena<BinXMLTokens> {
    let mut ctx = BinXMLParseCtx::new(data);

    // TODO: actually break
    for _ in 0..10 {
        let token = data[ctx.offset];
        ctx.offset += 1;

        let token = BinXMLToken::from_u8(token)
            .or_else(|| {
                println!("\n\n");
                println!("-------------------------------");
                println!("Panicked at offset {}", ctx.offset);
                println!("{:2x} not a valid binxml token", token);

                let m = (ctx.offset as i32) - 10;
                let start = if m < 0 { 0 } else { m };
                print_hexdump(&ctx.data[start as usize..ctx.offset + 100], 0, 'C');

                println!("\n-------------------------------");
                println!("\n\n");
                panic!();
            })
            .unwrap();

        match token {
            BinXMLToken::EndOfStream => visit_end_of_stream(&mut ctx),
            BinXMLToken::OpenStartElement => visit_open_start_element(&mut ctx),
            BinXMLToken::CloseStartElement => visit_close_start_element(&mut ctx),
            BinXMLToken::CloseEmptyElement => visit_close_empty_element(&mut ctx),
            BinXMLToken::CloseElement => visit_close_element(&mut ctx),
            BinXMLToken::Value => visit_value(&mut ctx),
            BinXMLToken::Attribute => visit_attribute(&mut ctx),
            BinXMLToken::CDataSection => visit_cdata_section(&mut ctx),
            BinXMLToken::EntityReference => visit_entity_reference(&mut ctx),
            BinXMLToken::ProcessingInstructionTarget => visit_processing_instruction_target(&mut ctx),
            BinXMLToken::ProcessingInstructionData => visit_processing_instruction_data(&mut ctx),
            BinXMLToken::TemplateInstance => visit_template_instance(&mut ctx),
            BinXMLToken::NormalSubstitution => visit_normal_substitution(&mut ctx),
            BinXMLToken::ConditionalSubstitution => visit_conditional_substitution(&mut ctx),
            BinXMLToken::StartOfStream => visit_start_of_stream(&mut ctx),
        }
    }

    ctx.xml
}

mod tests {
    use super::*;
    use hexdump;
    extern crate env_logger;

    #[test]
    fn test_basic_binxml() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let sample = include_bytes!("../samples/binxml.dat");

        let xml = parse_binxml(&sample[..]);
        println!("{:?}", xml);
    }
}
