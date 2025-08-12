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
    scratch: String,
}

impl<W: Write> XmlOutput<W> {
    pub fn with_writer(target: W, settings: &ParserSettings) -> Self {
        let writer = if settings.should_indent() {
            Writer::new_with_indent(target, b' ', 2)
        } else {
            Writer::new(target)
        };

        XmlOutput { writer, scratch: String::with_capacity(64) }
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

        let name = element.name.as_ref().as_str();

        // Prebuild attributes with exact capacity to avoid RawVec growth during emission
        if element.attributes.is_empty() {
            let event_builder = BytesStart::new(name);
            self.writer.write_event(Event::Start(event_builder))?;
            return Ok(());
        }

        // Pass 1: for attributes whose values cannot be borrowed as &str directly, precompute owned strings
        let mut owned_values_by_idx: Vec<Option<String>> = vec![None; element.attributes.len()];
        for (i, attr) in element.attributes.iter().enumerate() {
            let needs_owned = match attr.value.as_ref() {
                BinXmlValue::StringType(_) | BinXmlValue::AnsiStringType(_) |
                BinXmlValue::HexInt32Type(_) | BinXmlValue::HexInt64Type(_) |
                BinXmlValue::NullType => false,
                _ => true,
            };
            if needs_owned {
                let s = attr.value.as_ref().as_cow_str().into_owned();
                if !s.is_empty() {
                    owned_values_by_idx[i] = Some(s);
                }
            }
        }

        // Pass 2: build attributes using borrowed slices where possible, otherwise from owned storage
        let mut attrs: Vec<Attribute> = Vec::with_capacity(element.attributes.len());
        for (i, attr) in element.attributes.iter().enumerate() {
            let name_as_str = attr.name.as_str();
            if let Some(ref s) = owned_values_by_idx[i] {
                // Non-borrowable value; use the owned string we materialized in pass 1
                attrs.push(Attribute::from((name_as_str, s.as_str())));
                continue;
            }

            // Borrowable: reference directly from the source without creating temporaries
            match attr.value.as_ref() {
                BinXmlValue::StringType(s) => {
                    if !s.is_empty() { attrs.push(Attribute::from((name_as_str, s.as_str()))); }
                }
                BinXmlValue::AnsiStringType(s) => {
                    let v = s.as_ref();
                    if !v.is_empty() { attrs.push(Attribute::from((name_as_str, v))); }
                }
                BinXmlValue::HexInt32Type(s) => {
                    let v = s.as_ref();
                    if !v.is_empty() { attrs.push(Attribute::from((name_as_str, v))); }
                }
                BinXmlValue::HexInt64Type(s) => {
                    let v = s.as_ref();
                    if !v.is_empty() { attrs.push(Attribute::from((name_as_str, v))); }
                }
                BinXmlValue::NullType => {
                    // Skip empty
                }
                _ => {
                    // Should have been handled in owned_values_by_idx; skip if empty
                }
            }
        }

        let event_builder = BytesStart::new(name).with_attributes(attrs);
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
        match value {
            Cow::Borrowed(BinXmlValue::StringType(s)) => {
                let event = BytesText::new(s.as_ref());
                self.writer.write_event(Event::Text(event))?;
            }
            Cow::Borrowed(BinXmlValue::AnsiStringType(s)) => {
                let v = s.as_ref();
                let event = BytesText::new(v);
                self.writer.write_event(Event::Text(event))?;
            }
            // Numeric and bool fast path: content has no XML special chars, so treat as escaped
            Cow::Borrowed(BinXmlValue::Int8Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as i64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::UInt8Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as u64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::Int16Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as i64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::UInt16Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as u64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::Int32Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as i64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::UInt32Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n as u64);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::Int64Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::UInt64Type(n)) => {
                let mut buf = itoa::Buffer::new();
                let s = buf.format(*n);
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            Cow::Borrowed(BinXmlValue::BoolType(b)) => {
                let s = if *b { "true" } else { "false" };
                let event = Event::Text(BytesText::from_escaped(s));
                self.writer.write_event(event)?;
            }
            _ => {
                let cow: Cow<str> = value.as_cow_str();
                if cow.len() <= 128 {
                    let s = &mut self.scratch;
                    s.clear();
                    s.reserve(cow.len());
                    s.push_str(&cow);
                    let event = BytesText::new(s.as_str());
                    self.writer.write_event(Event::Text(event))?;
                } else {
                    let event = BytesText::new(&cow);
                    self.writer.write_event(Event::Text(event))?;
                }
            }
        }

        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_cdata_section", file!()),
        })
    }

    fn visit_entity_reference(&mut self, entity: &BinXmlName) -> SerializationResult<()> {
        let name = entity.as_str();
        let s = &mut self.scratch;
        s.clear();
        s.reserve(2 + name.len());
        s.push('&');
        s.push_str(name);
        s.push(';');
        // xml_ref is already escaped
        let event = Event::Text(BytesText::from_escaped(s.as_str()));
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
        let s = &mut self.scratch;
        s.clear();
        s.reserve(pi.name.as_str().len() + pi.data.as_ref().len());
        s.push_str(pi.name.as_str());
        s.push_str(pi.data.as_ref());
        let event = Event::PI(BytesPI::new(s.as_str()));
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
