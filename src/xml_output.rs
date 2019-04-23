use crate::model::xml::XmlElement;
use log::trace;
use std::io::Write;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::binxml::value_variant::BinXmlValue;
use failure::{bail, format_err, Error};
use std::borrow::{Borrow, Cow};

pub trait BinXmlOutput<W: Write> {
    /// Implementors are expected to provide a `std::Write` target.
    /// The record will be written to the target.
    fn with_writer(target: W) -> Self;

    /// Consumes the output, returning control of the inner writer to the caller.
    fn into_writer(self) -> Result<W, Error>;

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> Result<(), Error>;

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(&mut self, open_start_element: &XmlElement) -> Result<(), Error>;

    /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
    fn visit_close_element(&mut self) -> Result<(), Error>;

    ///
    /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
    ///                                                     ~~~~~~~~~~~~~~~
    fn visit_characters(&mut self, value: &BinXmlValue) -> Result<(), Error>;

    /// Unimplemented
    fn visit_cdata_section(&mut self) -> Result<(), Error>;

    /// Unimplemented
    fn visit_entity_reference(&mut self) -> Result<(), Error>;

    /// Unimplemented
    fn visit_processing_instruction_target(&mut self) -> Result<(), Error>;

    /// Unimplemented
    fn visit_processing_instruction_data(&mut self) -> Result<(), Error>;

    /// Called once on beginning of parsing.
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

    fn visit_open_start_element<'a>(&mut self, element: &XmlElement) -> Result<(), Error> {
        trace!("visit_open_start_element: {:?}", element);
        if self.eof_reached {
            bail!("Impossible state - `visit_open_start_element` after EOF");
        }

        // TODO: we could improve performance even further if we could somehow avoid this clone,
        // and share this borrow to the end element.
        self.stack.push(element.name.as_str().to_owned());

        let mut event_builder = BytesStart::from(element.name.as_ref().borrow().into());

        for attr in element.attributes.iter() {
            let value_cow: Cow<'_, str> = attr.value.as_ref().into();

            if value_cow.len() > 0 {
                let name_as_str = attr.name.as_str();
                let attr = Attribute::from((name_as_str, value_cow.as_ref()));
                event_builder.push_attribute(attr);
            }
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

    fn visit_characters(&mut self, value: &BinXmlValue) -> Result<(), Error> {
        trace!("visit_chars");
        let cow: Cow<'_, str> = value.into();
        let event = BytesText::from_plain_str(&cow);
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
