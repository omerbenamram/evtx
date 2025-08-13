use crate::err::{SerializationError, SerializationResult};
use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;
use crate::ParserSettings;

use serde_json::Value;
use std::borrow::Cow;
use std::io::{Result as IoResult, Write};

use hashbrown::HashMap as FastMap;

struct JsonWriter<W: Write> {
    writer: W,
}

impl<W: Write> JsonWriter<W> {
    fn new(writer: W) -> Self { Self { writer } }

    fn write_str(&mut self, s: &str) -> IoResult<()> { self.writer.write_all(s.as_bytes()) }

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
    dup_counters: FastMap<String, usize, ahash::RandomState>,
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
        if self.stack.is_empty() { self.stack.push(ObjectContext::default()); }
        self.stack.last_mut().unwrap()
    }

    fn next_duplicate_index_for(&mut self, base: &str) -> usize {
        let ctx = self.current_mut();
        if let Some(next) = ctx.dup_counters.get(base) { return *next; }
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
        if needs_comma { self.writer.write_str(",")?; }
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

        // Handle duplicate key naming under current parent
        let mut key = name.to_string();
        let next = self.next_duplicate_index_for(name);
        if next > 1 {
            key = format!("{}_{}", name, next);
            self.advance_duplicate_index(name);
        }

        // Always open an object for the element
        self.write_object_start(&key)?;

        // Attributes
        if !element.attributes.is_empty() {
            if self.separate_json_attributes {
                // Emit sibling <name>_attributes at parent level; we will add after closing current object
                // For streaming simplicity, we emit attributes inside the object under "#attributes" as well
                // to preserve information locally.
                self.write_key("#attributes")?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    if let Some(v) = Some(attr.value.clone().into_owned().into()) {
                        if !matches!(v, Value::Null) {
                            if !first { self.writer.write_str(",")?; }
                            first = false;
                            self.writer.write_quoted_str(attr.name.as_str())?;
                            self.writer.write_str(":")?;
                            self.writer.write_value(&v)?;
                        }
                    }
                }
                self.writer.write_str("}")?;
            } else {
                self.write_key("#attributes")?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    let v: Value = attr.value.clone().into_owned().into();
                    if !matches!(v, Value::Null) {
                        if !first { self.writer.write_str(",")?; }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.writer.write_value(&v)?;
                    }
                }
                self.writer.write_str("}")?;
            }
        }

        Ok(())
    }

    fn visit_close_element(&mut self, _element: &XmlElement) -> SerializationResult<()> {
        self.write_object_end()
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        // Serialize as #text field within the current object
        self.write_key("#text")?;
        let v = Self::value_to_json(value);
        self.writer.write_value(&v)?;
        Ok(())
    }

    fn visit_cdata_section(&mut self) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented { message: format!("`{}`: visit_cdata_section", file!()) })
    }

    fn visit_entity_reference(&mut self, _entity: &crate::binxml::name::BinXmlName) -> SerializationResult<()> {
        // Entity references should be expanded earlier; treat as unimplemented for now
        Err(SerializationError::Unimplemented { message: format!("`{}`: visit_entity_reference", file!()) })
    }

    fn visit_character_reference(&mut self, _char_ref: Cow<'_, str>) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented { message: format!("`{}`: visit_character_reference", file!()) })
    }

    fn visit_processing_instruction(&mut self, _pi: &BinXmlPI) -> SerializationResult<()> {
        Err(SerializationError::Unimplemented { message: format!("`{}`: visit_processing_instruction_data", file!()) })
    }
}