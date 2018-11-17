use crate::model::*;
use log::{debug, log};
use std::{
    io::{Cursor, Read, Result, Seek, SeekFrom, Write},
    mem,
};
use xml::common::XmlVersion;
use xml::{
    name::Name, writer::events::StartElementBuilder, writer::XmlEvent, EmitterConfig, EventWriter,
};
use std::borrow::Cow;
use core::borrow::Borrow;
use std::ops::Deref;

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
    fn visit_start_of_stream(&mut self) -> ();
}

pub struct BinXMLTreeBuilder<'b, W: Write> {
    writer: EventWriter<W>,
    current_element: Option<StartElementBuilder<'b>>,
    current_attribute_name: Option<&'b str>,
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
            current_attribute_name: None,
        }
    }
}

impl<'a: 'b, 'b, W: Write> Visitor<'a> for BinXMLTreeBuilder<'b, W> {
    fn visit_end_of_stream(&mut self) {
        self.writer.write(XmlEvent::end_element()).unwrap();
    }

    fn visit_open_start_element(&mut self, tag: &BinXMLOpenStartElement<'a>) {
        debug!("visit_open_start_element: {:?}", tag);
        //        let event_builder = XmlEvent::start_element(tag.name.as_ref());

        let event_builder = XmlEvent::start_element("test");
        self.current_element = Some(event_builder);
    }

    fn visit_close_start_element(&mut self) {
        debug!("visit_close_start_element");
        let current_elem = self.current_element.take().expect("Invalid state: visit_close_start_element called without calling visit_open_start_element first");
        self.writer.write(current_elem).expect("Failed to write");
    }

    fn visit_close_empty_element(&mut self) {
        debug!("visit_close_empty_element");
        self.writer
            .write(self.current_element.take().expect("It should be here"))
            .unwrap();
        self.writer.write(XmlEvent::end_element()).unwrap();
    }

    fn visit_close_element(&mut self) {
        debug!("visit_close_element");
        self.writer.write(XmlEvent::end_element()).unwrap();
    }

    fn visit_value(&mut self, value: &BinXMLValue<'a>) -> () {
        match &self.current_attribute_name {
            Some(ref attribute) => {
                self.current_element = Some(
                    self.current_element
                        .take()
                        .expect("It should be here")
                        .attr(attribute.deref(), "a value"),
                );
            }
            None => {}
        }
        debug!("visit_value {:?}", value);
    }

    fn visit_attribute(&mut self, attribute: &BinXMLAttribute<'a>) -> () {
        debug!("visit_attribute: {:?}", attribute);
        // Return ownership to self
        self.current_attribute_name = Some(&*attribute.name);
    }

    fn visit_cdata_section(&mut self) {
        unimplemented!("visit_cdata_section");
    }

    fn visit_entity_reference(&mut self) {
        unimplemented!("visit_entity_reference");
    }

    fn visit_processing_instruction_target(&mut self) {
        unimplemented!("visit_processing_instruction_target");
    }

    fn visit_processing_instruction_data(&mut self) {
        unimplemented!("visit_processing_instruction_data");
    }

    fn visit_start_of_stream(&mut self) -> () {
        self.writer
            .write(XmlEvent::StartDocument {
                version: XmlVersion::Version10,
                encoding: None,
                standalone: None,
            })
            .unwrap();
    }
}
