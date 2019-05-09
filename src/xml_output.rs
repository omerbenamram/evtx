use crate::model::xml::XmlElement;
use log::trace;
use std::io::Write;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::binxml::value_variant::BinXmlValue;
use crate::ParserSettings;
use failure::{bail, format_err, Error};
use std::borrow::Cow;

pub trait BinXmlOutput<W: Write> {
    /// Implementors are expected to provide a `std::Write` target.
    /// The record will be written to the target.
    fn with_writer(target: W, settings: &ParserSettings) -> Self;

    /// Consumes the output, returning control of the inner writer to the caller.
    fn into_writer(self) -> Result<W, Error>;

    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> Result<(), Error>;

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(&mut self, open_start_element: &XmlElement) -> Result<(), Error>;

    /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
    fn visit_close_element(&mut self, element: &XmlElement) -> Result<(), Error>;

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
}

/// Adapter between binxml XmlModel type and quick-xml events.
impl<W: Write> BinXmlOutput<W> for XmlOutput<W> {
    fn with_writer(target: W, settings: &ParserSettings) -> Self {
        let writer = if settings.should_indent() {
            Writer::new_with_indent(target, b' ', 2)
        } else {
            Writer::new(target)
        };

        XmlOutput { writer }
    }

    fn into_writer(self) -> Result<W, Error> {
        Ok(self.writer.into_inner())
    }

    fn visit_end_of_stream(&mut self) -> Result<(), Error> {
        trace!("visit_end_of_stream");
        self.writer.write_event(Event::Eof)?;
        Ok(())
    }

    fn visit_open_start_element<'a>(&mut self, element: &XmlElement) -> Result<(), Error> {
        trace!("visit_open_start_element: {:?}", element);

        let mut event_builder =
            BytesStart::borrowed_name(element.name.as_ref().as_str().as_bytes());

        for attr in element.attributes.iter() {
            let value_cow: Cow<'_, str> = attr.value.as_ref().as_cow_str();

            if value_cow.len() > 0 {
                let name_as_str = attr.name.as_str();
                let attr = Attribute::from((name_as_str, value_cow.as_ref()));
                event_builder.push_attribute(attr);
            }
        }

        self.writer.write_event(Event::Start(event_builder))?;

        Ok(())
    }

    fn visit_close_element(&mut self, element: &XmlElement) -> Result<(), Error> {
        trace!("visit_close_element");
        let event = BytesEnd::borrowed(element.name.as_ref().as_str().as_bytes());

        self.writer.write_event(Event::End(event))?;
        Ok(())
    }

    fn visit_characters(&mut self, value: &BinXmlValue) -> Result<(), Error> {
        trace!("visit_chars");
        let cow: Cow<str> = value.as_cow_str();
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
