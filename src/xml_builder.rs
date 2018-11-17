use core::borrow::Borrow;
use crate::model::*;
use log::{debug, log};
use std::borrow::Cow;
use std::ops::Deref;
use std::{
    io::{Cursor, Read, Result, Seek, SeekFrom, Write},
    mem,
};
use xml::common::XmlVersion;
use xml::{
    name::Name, writer::events::StartElementBuilder, writer::XmlEvent, EmitterConfig, EventWriter,
};

pub trait Visitor<'a> {
    fn visit_end_of_stream(&mut self) -> ();
    fn visit_open_start_element(&mut self, open_start_element: &XmlElement<'a>) -> ();
    fn visit_close_element(&mut self) -> ();
    fn visit_characters(&mut self, value: &str) -> ();
    fn visit_cdata_section(&mut self) -> ();
    fn visit_entity_reference(&mut self) -> ();
    fn visit_processing_instruction_target(&mut self) -> ();
    fn visit_processing_instruction_data(&mut self) -> ();
    fn visit_start_of_stream(&mut self) -> ();
}

pub struct BinXMLTreeBuilder<W: Write> {
    writer: EventWriter<W>,
}

impl<W: Write> BinXMLTreeBuilder<W> {
    pub fn with_writer(target: W) -> Self {
        let writer = EmitterConfig::new()
            .line_separator("\r\n")
            .perform_indent(true)
            .normalize_empty_elements(false)
            .create_writer(target);

        BinXMLTreeBuilder { writer }
    }
}

impl<'a, W: Write> Visitor<'a> for BinXMLTreeBuilder<W> {
    fn visit_end_of_stream(&mut self) {
        self.writer.write(XmlEvent::end_element()).unwrap();
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) {
        debug!("visit_open_start_element: {:?}", element);
        let mut event_builder = XmlEvent::start_element(element.name.borrow());

//        for attr in element.attributes.iter() {
//            event_builder.attr(attr.name.borrow(), &attr.value.borrow());
//        }

        self.writer.write(event_builder).unwrap();
    }

    fn visit_close_element(&mut self) {
        debug!("visit_close_element");
        self.writer.write(XmlEvent::end_element()).unwrap();
    }

    fn visit_characters(&mut self, value: &str) -> () {
        self.writer.write(XmlEvent::characters(value)).unwrap();
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
