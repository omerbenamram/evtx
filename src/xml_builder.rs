use crate::model::*;
use log::{debug, log};
use std::{
    io::{Cursor, Read, Result, Seek, SeekFrom, Write},
    mem,
};
use xml::{
    name::Name, writer::events::StartElementBuilder, writer::XmlEvent, EmitterConfig, EventWriter,
};

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &BinXMLOpenStartElement<'a>) -> ();
    fn visit_close_start_element(&mut self) -> ();
    fn visit_close_empty_element(&mut self) -> ();
    fn visit_close_element(&mut self) -> ();
    fn visit_value(&mut self, value: &BinXMLValue<'a>) -> ();
    fn visit_attribute(&mut self, attribute: &BinXMLAttribute<'a>) -> ();
    fn visit_cdata_section(&mut self) -> ();
    fn visit_entity_reference(&mut self) -> ();
    fn visit_processing_instruction_target(&mut self) -> ();
    fn visit_processing_instruction_data(&mut self) -> ();
    fn visit_start_of_stream(&mut self, header: &BinXMLFragmentHeader) -> ();
}

pub struct BinXMLTreeBuilder<'b, W: Write> {
    writer: EventWriter<W>,
    current_element: Option<StartElementBuilder<'b>>,
}

impl<'b, W: Write> BinXMLTreeBuilder<'b, W> {
    pub fn with_writer(target: W) -> Self {
        let writer = EmitterConfig::new()
            .line_separator("\r\n")
            .perform_indent(true)
            .normalize_empty_elements(false)
            .create_writer(target);

        BinXMLTreeBuilder {
            writer,
            current_element: None,
        }
    }
}

impl<'a: 'b, 'b, W: Write> Visitor<'a> for BinXMLTreeBuilder<'b, W> {
    fn visit_end_of_stream(&mut self) {
        println!("visit_end_of_stream");
    }

    fn visit_open_start_element(&mut self, tag: &BinXMLOpenStartElement<'a>) {
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

    fn visit_value(&mut self, value: &BinXMLValue<'a>) -> () {
        debug!("visit_value");
        unimplemented!();
    }

    fn visit_attribute(&mut self, attribute: &BinXMLAttribute<'a>) -> () {
        // Return ownership to self
        self.current_element = Some(
            self.current_element
                .take()
                .expect("visit_attribute_called without calling visit_open_start_element first")
                .attr(attribute.name.as_ref(), ""),
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

    fn visit_start_of_stream(&mut self, header: &BinXMLFragmentHeader) -> () {
        debug!("visit_start_of_stream");
    }
}
