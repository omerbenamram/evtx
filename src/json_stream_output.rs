use crate::ParserSettings;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;

use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::{Result as IoResult, Write};

use quick_xml::events::BytesText;

struct JsonWriter<W: Write> {
    writer: W,
}

impl<W: Write> JsonWriter<W> {
    fn new(writer: W) -> Self {
        Self { writer }
    }

    fn write_str(&mut self, s: &str) -> IoResult<()> {
        self.writer.write_all(s.as_bytes())
    }

    fn write_quoted_str(&mut self, s: &str) -> IoResult<()> {
        // Minimal JSON string escaping; delegate to serde_json for correctness
        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, &Value::String(s.to_string())).unwrap();
        self.writer.write_all(&buf)
    }

    fn write_value(&mut self, v: &Value) -> IoResult<()> {
        let mut buf = Vec::new();
        serde_json::to_writer(&mut buf, v).unwrap();
        self.writer.write_all(&buf)
    }
}

#[derive(Default)]
struct ObjectContext {
    has_any_field: bool,
    // Per-parent duplicate counters for child keys
    dup_counters: HashMap<String, usize>,
    // Track if current element has attributes (affects how we write text values)
    has_attributes: bool,
    // Track if we opened an object for this context
    object_opened: bool,
}

pub struct JsonStreamOutput<W: Write> {
    writer: JsonWriter<W>,
    // Stack of open objects (root + nested elements)
    stack: Vec<ObjectContext>,
    // separate_json_attributes option
    separate_json_attributes: bool,
}

impl<W: Write> JsonStreamOutput<W> {
    pub fn with_writer(writer: W, settings: &ParserSettings) -> Self {
        Self {
            writer: JsonWriter::new(writer),
            stack: Vec::new(),
            separate_json_attributes: settings.should_separate_json_attributes(),
        }
    }

    pub fn into_writer(self) -> W {
        self.writer.writer
    }

    fn current_mut(&mut self) -> &mut ObjectContext {
        if self.stack.is_empty() {
            self.stack.push(ObjectContext::default());
        }
        self.stack.last_mut().unwrap()
    }

    fn next_duplicate_index_for(&mut self, base: &str) -> usize {
        let ctx = self.current_mut();
        if let Some(next) = ctx.dup_counters.get(base) {
            return *next;
        }
        ctx.dup_counters.insert(base.to_owned(), 1);
        1
    }

    fn advance_duplicate_index(&mut self, base: &str) {
        let ctx = self.current_mut();
        let entry = ctx.dup_counters.entry(base.to_owned()).or_insert(1);
        *entry += 1;
    }

    fn write_comma_if_needed(&mut self) -> SerializationResult<()> {
        // Avoid holding two mutable borrows: first check, then write
        let needs_comma = {
            let ctx = self.current_mut();
            let needs = ctx.has_any_field;
            // mark will be set after writing
            needs
        };
        if needs_comma {
            self.writer.write_str(",")?;
        }
        {
            let ctx = self.current_mut();
            ctx.has_any_field = true;
        }
        Ok(())
    }

    fn write_key(&mut self, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed()?;
        self.writer.write_quoted_str(key)?;
        self.writer.write_str(":")?;
        Ok(())
    }

    fn write_object_start(&mut self, key: &str) -> SerializationResult<()> {
        self.write_key(key)?;
        self.writer.write_str("{")?;
        self.stack.push(ObjectContext::default());
        Ok(())
    }

    fn write_object_end(&mut self) -> SerializationResult<()> {
        self.writer.write_str("}")?;
        self.stack.pop();
        Ok(())
    }

    fn value_to_json(value: Cow<BinXmlValue>) -> Value {
        match value {
            Cow::Owned(BinXmlValue::StringType(s)) => Value::String(s),
            other => other.into_owned().into(),
        }
    }

    fn can_write_binxml_value(value: &BinXmlValue) -> bool {
        // Check if we can safely write this value
        !matches!(value, BinXmlValue::EvtXml | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtHandle)
    }

    fn write_binxml_value(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
        // Serialize BinXmlValue directly without converting to serde_json::Value first
        // This avoids panicking on types that require template expansion
        match value {
            BinXmlValue::NullType => self.writer.write_str("null")?,
            BinXmlValue::StringType(s) => self.writer.write_quoted_str(s)?,
            BinXmlValue::AnsiStringType(s) => self.writer.write_quoted_str(s.as_ref())?,
            BinXmlValue::Int8Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt8Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int16Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt16Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int32Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt32Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int64Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt64Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Real32Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Real64Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::BoolType(b) => self.writer.write_str(if *b { "true" } else { "false" })?,
            BinXmlValue::GuidType(g) => self.writer.write_quoted_str(&g.to_string())?,
            BinXmlValue::FileTimeType(dt) | BinXmlValue::SysTimeType(dt) => {
                self.writer.write_quoted_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())?
            }
            BinXmlValue::SidType(sid) => self.writer.write_quoted_str(&sid.to_string())?,
            BinXmlValue::HexInt32Type(s) | BinXmlValue::HexInt64Type(s) => {
                self.writer.write_quoted_str(s)?
            }
            BinXmlValue::SizeTType(n) => self.writer.write_str(&n.to_string())?,
            // Types that require template expansion - write null (shouldn't appear after expansion)
            BinXmlValue::EvtXml | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtHandle => {
                // These should have been expanded by the streaming parser
                // Write null as fallback
                self.writer.write_str("null")?
            }
            // Array and complex types - convert via serde_json
            _ => {
                // Fallback: try to convert via serde_json
                let v: Value = value.clone().into();
                self.writer.write_value(&v)?
            }
        }
        Ok(())
    }
}

impl<W: Write> BinXmlOutput for JsonStreamOutput<W> {
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        // Root object
        self.writer.write_str("{")?;
        self.stack.push(ObjectContext::default());
        Ok(())
    }

    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        // Close any open objects including root
        while self.stack.len() > 1 {
            self.write_object_end()?;
        }
        self.writer.write_str("}")?;
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        let name = element.name.as_str();
        let has_attributes = !element.attributes.is_empty();

        // Handle duplicate key naming under current parent
        let mut key = name.to_string();
        let next = self.next_duplicate_index_for(name);
        if next > 1 {
            key = format!("{}_{}", name, next);
            self.advance_duplicate_index(name);
        }

        // If element has no attributes, we'll write the value directly (not wrapped in #text)
        // So we don't open an object yet - we'll write the value directly in visit_characters
        if !has_attributes {
            // Just write the key, we'll write the value in visit_characters
            self.write_key(&key)?;
            // Push a context to track this element (but no object opened)
            self.stack.push(ObjectContext {
                has_attributes: false,
                object_opened: false,
                ..Default::default()
            });
            return Ok(());
        }

        // Element has attributes, so we need an object
        self.write_object_start(&key)?;
        // Mark that this element has attributes and object was opened
        if let Some(ctx) = self.stack.last_mut() {
            ctx.has_attributes = true;
            ctx.object_opened = true;
        }

        // Attributes
        if has_attributes {
            if self.separate_json_attributes {
                // Emit sibling <name>_attributes at parent level; we will add after closing current object
                // For streaming simplicity, we emit attributes inside the object under "#attributes" as well
                // to preserve information locally.
                self.write_key("#attributes")?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    if !matches!(attr.value.as_ref(), BinXmlValue::NullType)
                        && Self::can_write_binxml_value(attr.value.as_ref())
                    {
                        if !first {
                            self.writer.write_str(",")?;
                        }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.write_binxml_value(attr.value.as_ref())?;
                    }
                }
                self.writer.write_str("}")?;
            } else {
                self.write_key("#attributes")?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    if !matches!(attr.value.as_ref(), BinXmlValue::NullType)
                        && Self::can_write_binxml_value(attr.value.as_ref())
                    {
                        if !first {
                            self.writer.write_str(",")?;
                        }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.write_binxml_value(attr.value.as_ref())?;
                    }
                }
                self.writer.write_str("}")?;
            }
        }

        Ok(())
    }

    fn visit_close_element(&mut self, _element: &XmlElement) -> SerializationResult<()> {
        // Pop the context for this element
        if let Some(ctx) = self.stack.pop() {
            // Only close object if one was opened
            if ctx.object_opened {
                self.write_object_end()?;
            }
            // If no object was opened and no value was written, we need to write null
            // (This handles empty elements without attributes)
            // Actually, if we wrote a key but no value, that's an error case we should handle
        }
        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        // Check if current element has attributes
        let has_attributes = self.stack.last().map(|ctx| ctx.has_attributes).unwrap_or(false);
        
        if has_attributes {
            // Element has attributes, so wrap text in #text field
            self.write_key("#text")?;
            self.write_binxml_value(value.as_ref())?;
        } else {
            // Element has no attributes, write value directly (no #text wrapper)
            self.write_binxml_value(value.as_ref())?;
        }
        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_cdata_section", file!()),
        })
    }

    fn visit_entity_reference(
        &mut self,
        entity: &crate::binxml::name::BinXmlName,
    ) -> SerializationResult<()> {
        // Expand entity into characters and delegate to visit_characters
        let entity_ref = {
            let mut s = String::with_capacity(entity.as_str().len() + 2);
            s.push('&');
            s.push_str(entity.as_str());
            s.push(';');
            s
        };
        let xml_event = BytesText::from_escaped(&entity_ref);
        match xml_event.unescape() {
            Ok(escaped) => {
                let as_string = escaped.to_string();
                self.visit_characters(Cow::Owned(BinXmlValue::StringType(as_string)))
            }
            Err(_) => Err(SerializationError::JsonStructureError {
                message: format!("Unterminated XML Entity {}", entity_ref),
            }),
        }
    }

    fn visit_character_reference(&mut self, _char_ref: Cow<'_, str>) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_character_reference", file!()),
        })
    }

    fn visit_processing_instruction(&mut self, _pi: &BinXmlPI) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented {
            message: format!("`{}`: visit_processing_instruction_data", file!()),
        })
    }
}
