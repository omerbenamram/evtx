use core::mem;
use indextree::{Arena, NodeId};
use nom::{le_u16, le_u32, le_u64, le_u8, IResult};
use num_traits::FromPrimitive;
use std::fmt;
use std::fmt::{Debug, Display};

#[repr(u8)]
#[derive(Primitive, Debug, PartialOrd, PartialEq)]
enum BXMLToken {
    EndOfStream = 0x00,
    OpenStartElement = 0x01,
    CloseStartElement = 0x02,
    CloseEmptyElement = 0x03,
    CloseElement = 0x04,
    Value = 0x05,
    Attribute = 0x06,
    CDataSection = 0x07,
    EntityReference = 0x08,
    ProcessingInstructionTarget = 0x0a,
    ProcessingInstructionData = 0x0b,
    TemplateInstance = 0xc,
    NormalSubstitution = 0x0d,
    ConditionalSubstitution = 0x0e,
    StartOfStream = 0x0f,
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
    // This includes the size of the fragment header, element and end of file token,
    // except for the first 33 bytes of the template definition.
    data_size: u32,
}

fn binxml_template(input: &[u8]) -> IResult<&[u8], BinXmlTemplate> {
    return do_parse!(
        input,
        take!(1) // Unknown
       >> template_id: le_u32
       >> template_offset: le_u32
       >> next_template_offset: le_u32
       >> template_guid: guid
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

struct BXMLParseCtx<'a> {
    data: &'a [u8],
    offset: usize,
    template: Option<&'a BinXmlTemplate>,
    xml: Arena<BinXMLTokens>,
    current_parent: Option<NodeId>,
}

impl<'a> BXMLParseCtx<'a> {
    fn new(data: &'a [u8]) -> BXMLParseCtx {
        BXMLParseCtx {
            data,
            offset: 0,
            template: None,
            xml: Arena::new(),
            current_parent: None,
        }
    }
}

fn visit_end_of_stream(ctx: &mut BXMLParseCtx) {
    println!("visit_end_of_stream");
}
fn visit_open_start_element(ctx: &mut BXMLParseCtx) {
    println!("visit_open_start_element");
}
fn visit_close_start_element(ctx: &mut BXMLParseCtx) {
    println!("visit_close_start_element");
}
fn visit_close_empty_element(ctx: &mut BXMLParseCtx) {
    println!("visit_close_empty_element");
}
fn visit_close_element(ctx: &mut BXMLParseCtx) {
    println!("visit_close_element");
}
fn visit_value(ctx: &mut BXMLParseCtx) {
    println!("visit_value");
}
fn visit_attribute(ctx: &mut BXMLParseCtx) {
    println!("visit_attribute");
}
fn visit_cdata_section(ctx: &mut BXMLParseCtx) {
    println!("visit_cdata_section");
}
fn visit_entity_reference(ctx: &mut BXMLParseCtx) {
    println!("visit_entity_reference");
}
fn visit_processing_instruction_target(ctx: &mut BXMLParseCtx) {
    println!("visit_processing_instruction_target");
}
fn visit_processing_instruction_data(ctx: &mut BXMLParseCtx) {
    println!("visit_processing_instruction_data");
}
fn visit_template_instance(ctx: &mut BXMLParseCtx) {
    println!("visit_template_instance");
}
fn visit_normal_substitution(ctx: &mut BXMLParseCtx) {
    println!("visit_normal_substitution");
}
fn visit_conditional_substitution(ctx: &mut BXMLParseCtx) {
    println!("visit_conditional_substitution");
}
fn visit_start_of_stream(ctx: &mut BXMLParseCtx) {
    debug!("visit_start_of_stream");
    // Skip signature
    ctx.offset += mem::size_of::<BinXMLFragmentHeader>();
    let root = ctx
        .xml
        .new_node(BinXMLTokens::FragmentHeader(BinXMLFragmentHeader {
            major_version: 0x01,
            minor_version: 0x01,
            flags: 0x00,
        }));
    ctx.current_parent = Some(root);
}

fn parse_binxml(data: &[u8]) -> Arena<BinXMLTokens> {
    let mut ctx = BXMLParseCtx::new(data);

    loop {
        let token = data[ctx.offset];
        let token =
            BXMLToken::from_u8(token).expect(&format!("{:?} not a valid binxml token", token));

        match token {
            BXMLToken::EndOfStream => visit_end_of_stream(&mut ctx),
            BXMLToken::OpenStartElement => visit_open_start_element(&mut ctx),
            BXMLToken::CloseStartElement => visit_close_start_element(&mut ctx),
            BXMLToken::CloseEmptyElement => visit_close_empty_element(&mut ctx),
            BXMLToken::CloseElement => visit_close_element(&mut ctx),
            BXMLToken::Value => visit_value(&mut ctx),
            BXMLToken::Attribute => visit_attribute(&mut ctx),
            BXMLToken::CDataSection => visit_cdata_section(&mut ctx),
            BXMLToken::EntityReference => visit_entity_reference(&mut ctx),
            BXMLToken::ProcessingInstructionTarget => visit_processing_instruction_target(&mut ctx),
            BXMLToken::ProcessingInstructionData => visit_processing_instruction_data(&mut ctx),
            BXMLToken::TemplateInstance => visit_template_instance(&mut ctx),
            BXMLToken::NormalSubstitution => visit_normal_substitution(&mut ctx),
            BXMLToken::ConditionalSubstitution => visit_conditional_substitution(&mut ctx),
            BXMLToken::StartOfStream => visit_start_of_stream(&mut ctx),
            _ => panic!("Unknown token {:?}", token),
        }
        break;
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
