use model::*;
use std::io::{Cursor, Read, Result, Seek, SeekFrom, Write};
use std::mem;
use xml::name::Name;
use xml::writer::events::StartElementBuilder;
use xml::writer::XmlEvent;
use xml::{EmitterConfig, EventWriter};

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &'a BinXMLOpenStartElement) -> ();
    fn visit_close_start_element(&mut self) -> ();
    fn visit_close_empty_element(&mut self) -> ();
    fn visit_close_element(&mut self) -> ();
    fn visit_value(&mut self, value: &'a BinXMLValue<'a>) -> ();
    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute<'a>) -> ();
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

    current_element: Option<StartElementBuilder<'a>>,
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
            current_element: None,
        }
    }
}
//
//impl<'a> Into<Name<'a>> for BinXMLName {
//    fn into(self) -> Name<'a> {
//        Name
//    }
//}

impl<'a, W: Write> Visitor<'a> for BinXMLTreeBuilder<'a, W> {
    fn visit_end_of_stream(&mut self) {
        println!("visit_end_of_stream");
    }

    fn visit_open_start_element(&mut self, tag: &'a BinXMLOpenStartElement) {
        let event_builder = XmlEvent::start_element(tag.name.as_ref());
        self.current_element = Some(event_builder);
    }

    fn visit_close_start_element(&mut self) {
        let current_elem = self.current_element.take().expect("Invalid state: visit_close_start_element called without calling visit_open_start_element first");
        self.writer.write(current_elem).expect("Failed to write");
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

    fn visit_attribute(&mut self, attribute: &'a BinXMLAttribute<'a>) -> () {
        let value = match attribute.value {
            BinXMLValue::StringType(ref s) => s,
            _ => unimplemented!("Attribute values other than text currently not supported."),
        };

        // Return ownership to self
        self.current_element = Some(
            self.current_element
                .take()
                .expect("visit_attribute_called without calling visit_open_start_element first")
                .attr(attribute.name.as_ref(), value),
        );
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

        ()
    }

    fn visit_start_of_stream(&mut self, header: &'a BinXMLFragmentHeader) -> () {
        debug!("visit_start_of_stream");
    }
}
