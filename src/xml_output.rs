use crate::model::xml::XmlElement;
use core::borrow::Borrow;
use log::trace;

use std::io::Write;

use xml::common::XmlVersion;
use xml::{writer::XmlEvent, EmitterConfig, EventWriter};

use failure::{bail, format_err, Error};

pub trait BinXMLOutput<'a, W: Write> {
    fn with_writer(target: W) -> Self;
    fn into_writer(self) -> Result<W, Error>;

    fn visit_end_of_stream(&mut self) -> Result<(), Error>;
    fn visit_open_start_element(
        &mut self,
        open_start_element: &XmlElement<'a>,
    ) -> Result<(), Error>;
    fn visit_close_element(&mut self) -> Result<(), Error>;
    fn visit_characters(&mut self, value: &str) -> Result<(), Error>;
    fn visit_cdata_section(&mut self) -> Result<(), Error>;
    fn visit_entity_reference(&mut self) -> Result<(), Error>;
    fn visit_processing_instruction_target(&mut self) -> Result<(), Error>;
    fn visit_processing_instruction_data(&mut self) -> Result<(), Error>;
    fn visit_start_of_stream(&mut self) -> Result<(), Error>;
}

pub struct XMLOutput<W: Write> {
    writer: EventWriter<W>,
    eof_reached: bool,
}

/// Adapter between binxml XmlModel type and rust-xml output stream.
impl<'a, W: Write> BinXMLOutput<'a, W> for XMLOutput<W> {
    fn with_writer(target: W) -> Self {
        let config = EmitterConfig {
            line_separator: "\r\n".into(),
            indent_string: "  ".into(),
            perform_indent: true,
            perform_escaping: false,
            write_document_declaration: true,
            normalize_empty_elements: false,
            cdata_to_characters: false,
            keep_element_names_stack: true,
            autopad_comments: true,
        };

        let writer = EventWriter::new_with_config(target, config);

        XMLOutput {
            writer,
            eof_reached: false,
        }
    }

    fn into_writer(self) -> Result<W, Error> {
        if self.eof_reached {
            Ok(self.writer.into_inner())
        } else {
            Err(format_err!(
                "Tried to return writer before EOF marked, incomplete output."
            ))
        }
    }

    fn visit_end_of_stream(&mut self) -> Result<(), Error> {
        trace!("visit_end_of_stream");
        self.eof_reached = true;
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> Result<(), Error> {
        trace!("visit_open_start_element: {:?}", element);
        if self.eof_reached {
            bail!("Impossible state - `visit_open_start_element` after EOF");
        }

        let mut event_builder = XmlEvent::start_element(element.name.borrow());

        for attr in element.attributes.iter() {
            event_builder = event_builder.attr(attr.name.borrow(), &attr.value.borrow());
        }

        self.writer.write(event_builder)?;

        Ok(())
    }

    fn visit_close_element(&mut self) -> Result<(), Error> {
        trace!("visit_close_element");
        self.writer.write(XmlEvent::end_element())?;
        Ok(())
    }

    fn visit_characters(&mut self, value: &str) -> Result<(), Error> {
        trace!("visit_chars");
        self.writer.write(XmlEvent::characters(value))?;
        Ok(())
    }

    fn visit_cdata_section(&mut self) -> Result<(), Error> {
        bail!("Unimplemented: visit_cdata_section")
    }

    fn visit_entity_reference(&mut self) -> Result<(), Error> {
        bail!("Unimplemented: visit_entity_reference")
    }

    fn visit_processing_instruction_target(&mut self) -> Result<(), Error> {
        bail!("Unimplemented: visit_processing_instruction_target")
    }

    fn visit_processing_instruction_data(&mut self) -> Result<(), Error> {
        bail!("Unimplemented: visit_processing_instruction_data")
    }

    fn visit_start_of_stream(&mut self) -> Result<(), Error> {
        trace!("visit_start_of_stream");
        if self.eof_reached {
            bail!("Impossible state - `visit_start_of_stream` after EOF");
        }

        self.writer.write(XmlEvent::StartDocument {
            version: XmlVersion::Version10,
            encoding: None,
            standalone: None,
        })?;

        Ok(())
    }
}
