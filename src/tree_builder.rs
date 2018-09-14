use binxml::model::BinXMLTemplate;
use binxml::model::BinXMLTemplateDefinition;
use binxml::model::BinXMLValue;
use binxml::model::OpenStartElementTokenMeta;
use binxml::model::{
    BinXMLAttribute, BinXMLDeserializedTokens, BinXMLFragmentHeader, BinXMLOpenStartElement,
};
use binxml::owned_model::Element;
use hexdump::print_hexdump;
use indextree::{Arena, NodeId};
use std::io::{Cursor, Read, Result, Seek, SeekFrom};
use std::marker::PhantomData;
use std::borrow::BorrowMut;
use num_traits::Num;
use xml::{EventWriter, EmitterConfig};
use std::io::Write;

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &'a BinXMLOpenStartElement) -> ();
    fn visit_close_start_element(&mut self) -> ();
    fn visit_close_empty_element(&mut self) -> ();
    fn visit_close_element(&mut self) -> ();
    fn visit_value(&mut self, value: &'a BinXMLValue<'a>) -> ();
    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute) -> ();
    fn visit_cdata_section(&mut self) -> ();
    fn visit_entity_reference(&mut self) -> ();
    fn visit_processing_instruction_target(&mut self) -> ();
    fn visit_processing_instruction_data(&mut self) -> ();
    fn visit_normal_substitution(&mut self) -> ();
    fn visit_conditional_substitution(&mut self) -> ();
    fn visit_template_instance(&mut self, template: &'a BinXMLTemplate) -> ();
    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> ();
}

struct BinXMLTreeBuilder<'a, W: Write> {
    template: Option<&'a BinXMLTemplateDefinition<'a>>,
    writer: EventWriter<W>,
}

impl<'a, W: Write> BinXMLTreeBuilder<'a, W> {
    // TODO: pick up from here - use EventWriter to drive the visitor.
    // The crucial part is to handle template substitutions.
    pub fn with_writer(target: W) -> Self {
        let writer = EmitterConfig::new()
            .line_separator("\r\n")
            .perform_indent(true)
            .normalize_empty_elements(false)
            .create_writer(target);

        BinXMLTreeBuilder {
            template: None,
            writer,
        }
    }
}

impl<'a, W: Write> Visitor<'a> for BinXMLTreeBuilder<'a, W> {
    fn visit_end_of_stream(&mut self) {
        println!("visit_end_of_stream");
    }

    fn visit_open_start_element(&mut self, tag: &'a BinXMLOpenStartElement) {
        debug!("visit start_element {:?}", tag);
    }

    fn visit_close_start_element(&mut self) {
        println!("visit_close_start_element");
        unimplemented!();
    }

    fn visit_close_empty_element(&mut self) {
        println!("visit_close_empty_element");
        unimplemented!();
    }

    fn visit_close_element(&mut self) {
        println!("visit_close_element");
        unimplemented!();
    }

    fn visit_value(&mut self, value: &'a BinXMLValue<'a>) -> () {
        debug!("visit_value");
        unimplemented!();
    }

    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute) -> () {
        unimplemented!()
    }

    fn visit_cdata_section(&mut self) {
        println!("visit_cdata_section");
        unimplemented!();
    }

    fn visit_entity_reference(&mut self) {
        println!("visit_entity_reference");
        unimplemented!();
    }

    fn visit_processing_instruction_target(&mut self) {
        println!("visit_processing_instruction_target");
        unimplemented!();
    }

    fn visit_processing_instruction_data(&mut self) {
        println!("visit_processing_instruction_data");
        unimplemented!();
    }

    fn visit_normal_substitution(&mut self) {
        println!("visit_normal_substitution");
        unimplemented!();
    }

    fn visit_conditional_substitution(&mut self) {
        println!("visit_conditional_substitution");
        unimplemented!();
    }

    fn visit_template_instance(&mut self, template: &'a BinXMLTemplate) -> () {
        let elem_unfilled = &template.definition.element;
        let mut elem_filled = elem_unfilled.clone();
        let substitutions = &template.substitution_array;

        for (i, sub_elem) in elem_filled.iter_mut().enumerate() {
            if let BinXMLDeserializedTokens::Substitution(ref data) = sub_elem {
                *sub_elem = BinXMLDeserializedTokens::Value(
                    substitutions[data.substitution_index as usize].clone(),
                )
            }
        }
    }

    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> () {
        debug!("visit_start_of_stream");
    }
}