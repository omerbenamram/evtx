use core::mem;
use indextree::{Arena, NodeId};
use num_traits::FromPrimitive;

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
#[derive(Debug, PartialOrd, PartialEq)]
struct BinXMLFragmentHeader {
    major_version: u8,
    minor_version: u8,
    flags: u8,
}

enum BinXMLTokens {
    FragmentHeader(BinXMLFragmentHeader),
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
    TemplateInstanceToken,
    NormalSubstitutionToken,
    OptionalSubstitutionToken,
}

//struct BXMLTemplate {
//    data_len: i32,
//
//    "dword", "next_offset", 0x0)
//    "dword", "template_id")
//    "guid",  "guid", 0x04)  # unsure why this overlaps
//    "dword", "data_length")
//}

struct BXMLParseCtx<'a> {
    data: &'a [u8],
    offset: usize,
    // TODO: should be a pointer to a template instance
    template: i32,
    xml: Arena<BinXMLTokens>,
    current_parent: Option<NodeId>,
}

impl<'a> BXMLParseCtx<'a> {
    fn new(data: &'a [u8]) -> BXMLParseCtx {
        BXMLParseCtx {
            data,
            offset: 0,
            template: 0,
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

fn parse_binxml(data: &[u8]) {
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
}

mod tests {
    use super::*;
    use hexdump;
    extern crate env_logger;

    #[test]
    fn test_basic_binxml() {
        let _ = env_logger::try_init().expect("Failed to init logger");
        let sample = include_bytes!("../samples/binxml.dat");

        let test = &sample[0..16];
        hexdump::print_hexdump(test, 0, 'x');
        println!("\n{:?}", test);

        parse_binxml(test);
    }
}
