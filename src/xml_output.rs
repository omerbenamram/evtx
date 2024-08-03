use crate::binxml::value_variant::BinXmlValue;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::ParserSettings;

use log::trace;
use std::io::Write;

use quick_xml::events::attributes::Attribute;
use quick_xml::events::{BytesDecl, BytesEnd, BytesPI, BytesStart, BytesText, Event};
use quick_xml::Writer;

use crate::binxml::name::BinXmlName;
use std::borrow::Cow;

pub trait BinXmlOutput {
    /// Called once when EOF is reached.
    fn visit_end_of_stream(&mut self) -> SerializationResult<()>;

    /// Called on <Tag attr="value" another_attr="value">.
    fn visit_open_start_element(
        &mut self,
        open_start_element: &XmlElement,
    ) -> SerializationResult<()>;

    /// Called on </Tag>, implementor may want to keep a stack to properly close tags.
    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()>;

    ///
    /// Called with value on xml text node,  (ex. <Computer>DESKTOP-0QT8017</Computer>)
    ///                                                     ~~~~~~~~~~~~~~~
    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()>;

    /// Unimplemented
    fn visit_cdata_section(&mut self) -> SerializationResult<()>;

    /// Emit the character "&" and the text.
    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> SerializationResult<()>;

    /// Emit the characters "&" and "#" and the decimal string representation of the value.
    fn visit_character_reference(&mut self, char_ref: Cow<'_, str>) -> SerializationResult<()>;

    /// Unimplemented
    fn visit_processing_instruction(&mut self, pi: &BinXmlPI) -> SerializationResult<()>;

    /// Called once on beginning of parsing.
    fn visit_start_of_stream(&mut self) -> SerializationResult<()>;
}

pub struct XmlOutput<W: Write> {
    writer: Writer<W>,
}

impl<W: Write> XmlOutput<W> {
    pub fn with_writer(target: W, settings: &ParserSettings) -> Self {
        let writer = if settings.should_indent() {
            Writer::new_with_indent(target, b' ', 2)
        } else {
            Writer::new(target)
        };

        XmlOutput { writer }
    }

    pub fn into_writer(self) -> W {
        self.writer.into_inner()
    }
}

/// Adapter between binxml XmlModel type and quick-xml events.
impl<W: Write> BinXmlOutput for XmlOutput<W> {
    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        trace!("visit_end_of_stream");
        self.writer.write_event(Event::Eof)?;

        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        trace!("visit_open_start_element: {:?}", element);

        let mut event_builder = BytesStart::new(element.name.as_ref().as_str());

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

    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        trace!("visit_close_element");
        let event = BytesEnd::new(element.name.as_ref().as_str());

        self.writer.write_event(Event::End(event))?;

        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        trace!("visit_chars");
        let cow: Cow<str> = value.as_cow_str();
        let event = BytesText::new(&cow);
        self.writer.write_event(Event::Text(event))?;

        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_cdata_section", file!()),
        })
    }

    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> Result<(), SerializationError> {
        let xml_ref = "&".to_string() + entity.as_str() + ";";
        // xml_ref is already escaped
        let event = Event::Text(BytesText::from_escaped(&xml_ref));
        self.writer.write_event(event)?;

        Ok(())
    }

    fn visit_character_reference(
        &mut self,
        _char_ref: Cow<'_, str>,
    ) -> Result<(), SerializationError> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_character_reference", file!()),
        })
    }

    fn visit_processing_instruction(&mut self, pi: &BinXmlPI) -> SerializationResult<()> {
        // PITARGET - Emit the text "<?", the text (as specified by the Name rule in 2.2.12), and then the space character " ".
        // Emit the text (as specified by the NullTerminatedUnicodeString rule in 2.2.12), and then the text "?>".
        let concat = pi.name.as_str().to_owned() + pi.data.as_ref(); // only `String` supports concatenation.
        let event = Event::PI(BytesPI::new(concat.as_str()));
        self.writer.write_event(event)?;

        Ok(())
    }

    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        trace!("visit_start_of_stream");
        let event = BytesDecl::new("1.0", Some("utf-8"), None);

        self.writer.write_event(Event::Decl(event))?;

        Ok(())
    }
}
