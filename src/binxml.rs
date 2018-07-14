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
    offset: i32,
    // TODO: should be a pointer to a template instance
    template: i32,
}

fn visit_end_of_stream() {
    println!("visit_end_of_stream");
}
fn visit_open_start_element() {
    println!("visit_open_start_element");
}
fn visit_close_start_element() {
    println!("visit_close_start_element");
}
fn visit_close_empty_element() {
    println!("visit_close_empty_element");
}
fn visit_close_element() {
    println!("visit_close_element");
}
fn visit_value() {
    println!("visit_value");
}
fn visit_attribute() {
    println!("visit_attribute");
}
fn visit_cdata_section() {
    println!("visit_cdata_section");
}
fn visit_entity_reference() {
    println!("visit_entity_reference");
}
fn visit_processing_instruction_target() {
    println!("visit_processing_instruction_target");
}
fn visit_processing_instruction_data() {
    println!("visit_processing_instruction_data");
}
fn visit_template_instance() {
    println!("visit_template_instance");
}
fn visit_normal_substitution() {
    println!("visit_normal_substitution");
}
fn visit_conditional_substitution() {
    println!("visit_conditional_substitution");
}
fn visit_start_of_stream() {
    println!("visit_start_of_stream");
}

fn parse_binxml(data: &[u8]) {
    //        set_stream(stream);
    //        current_chunk = &chunk;
    //        root = &node;
    //        is_template_definition = template_definition;
    //        stack.clear();
    //        stack_path.clear();
    //
    //        stop = false;
    let token =
        BXMLToken::from_u8(data[0]).expect(&format!("{:?} not a valid binxml token", data[0]));

    loop {
        match token {
            BXMLToken::EndOfStream => visit_end_of_stream(),
            BXMLToken::OpenStartElement => visit_open_start_element(),
            BXMLToken::CloseStartElement => visit_close_start_element(),
            BXMLToken::CloseEmptyElement => visit_close_empty_element(),
            BXMLToken::CloseElement => visit_close_element(),
            BXMLToken::Value => visit_value(),
            BXMLToken::Attribute => visit_attribute(),
            BXMLToken::CDataSection => visit_cdata_section(),
            BXMLToken::EntityReference => visit_entity_reference(),
            BXMLToken::ProcessingInstructionTarget => visit_processing_instruction_target(),
            BXMLToken::ProcessingInstructionData => visit_processing_instruction_data(),
            BXMLToken::TemplateInstance => visit_template_instance(),
            BXMLToken::NormalSubstitution => visit_normal_substitution(),
            BXMLToken::ConditionalSubstitution => visit_conditional_substitution(),
            BXMLToken::StartOfStream => visit_start_of_stream(),
            _ => panic!("Unknown token {:?}", token),
        }
    }
}

mod tests {
    use super::*;
    use hexdump;
    use html5ever::parse_document;

    #[test]
    fn test_basic_binxml() {
        let sample = include_bytes!("../samples/binxml.dat");
        let test = &sample[0..16];
        hexdump::print_hexdump(test, 0, 'x');
        println!("\n{:?}", test);

        parse_binxml(test);
    }
}
