use crate::model::xml::XmlElement;
use core::borrow::Borrow;
use log::trace;
use std::io::Write;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use failure::{bail, format_err, Error};

pub trait BinXmlOutput<W: Write> {
    fn with_writer(target: W) -> Self;
    fn into_writer(self) -> Result<W, Error>;
    fn visit_end_of_stream(&mut self) -> Result<(), Error>;
    fn visit_open_start_element(&mut self, open_start_element: &XmlElement) -> Result<(), Error>;
    fn visit_close_element(&mut self) -> Result<(), Error>;
    fn visit_characters(&mut self, value: &str) -> Result<(), Error>;
    fn visit_cdata_section(&mut self) -> Result<(), Error>;
    fn visit_entity_reference(&mut self) -> Result<(), Error>;
    fn visit_processing_instruction_target(&mut self) -> Result<(), Error>;
    fn visit_processing_instruction_data(&mut self) -> Result<(), Error>;
    fn visit_start_of_stream(&mut self) -> Result<(), Error>;
}

pub struct XmlOutput<W: Write> {
    writer: Writer<W>,
    eof_reached: bool,
    // TODO: Bring back Vec<BinXmlName<'a>> if possible.
    stack: Vec<String>,
}

/// Adapter between binxml XmlModel type and quick-xml events.
impl<W: Write> BinXmlOutput<W> for XmlOutput<W> {
    fn with_writer(target: W) -> Self {
        let writer = Writer::new_with_indent(target, b' ', 2);

        XmlOutput {
            writer,
            eof_reached: false,
            stack: vec![],
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
        self.writer.write_event(Event::Eof)?;
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> Result<(), Error> {
        trace!("visit_open_start_element: {:?}", element);
        if self.eof_reached {
            bail!("Impossible state - `visit_open_start_element` after EOF");
        }

        // TODO: we could improve performance even further if we could somehow avoid this clone,
        // and share this borrow to the end element.
        self.stack.push(element.name.as_str().to_owned());

        let mut event_builder = BytesStart::from(element.name.borrow().into());

        for attr in element.attributes.iter() {
            let name_as_str = attr.name.as_str();

            let attr = Attribute::from((name_as_str, attr.value.as_ref()));
            event_builder.push_attribute(attr);
        }

        self.writer.write_event(Event::Start(event_builder))?;

        Ok(())
    }

    fn visit_close_element(&mut self) -> Result<(), Error> {
        trace!("visit_close_element");
        let name = self
            .stack
            .pop()
            .ok_or_else(|| format_err!("invalid stack state"))?;

        let event = BytesEnd::owned(name.into_bytes());

        self.writer.write_event(Event::End(event))?;
        Ok(())
    }

    fn visit_characters(&mut self, value: &str) -> Result<(), Error> {
        trace!("visit_chars");
        let event = BytesText::from_plain_str(value);
        self.writer.write_event(Event::Text(event))?;
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
        let event = BytesDecl::new(b"1.0", Some(b"utf-8"), None);

        self.writer.write_event(Event::Decl(event))?;

        Ok(())
    }
}
