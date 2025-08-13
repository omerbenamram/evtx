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
        self.writer.write_all(b"\"")?;
        for b in s.bytes() {
            match b {
                b'"' => self.writer.write_all(b"\\\"")?,
                b'\\' => self.writer.write_all(b"\\\\")?,
                b'\n' => self.writer.write_all(b"\\n")?,
                b'\r' => self.writer.write_all(b"\\r")?,
                b'\t' => self.writer.write_all(b"\\t")?,
                0x00..=0x1F => {
                    // \u00XX control escapes
                    let esc = [b'\\', b'u', b'0', b'0',
                        b"0123456789ABCDEF"[(b >> 4) as usize],
                        b"0123456789ABCDEF"[(b & 0x0F) as usize]
                    ];
                    self.writer.write_all(&esc)?;
                }
                _ => self.writer.write_all(&[b])?,
            }
        }
        self.writer.write_all(b"\"")
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

    fn write_binxml_scalar(&mut self, v: &BinXmlValue) -> SerializationResult<()> {
        use std::fmt::Write as _;
        match v {
            BinXmlValue::NullType => self.writer.write_str("null")?,
            BinXmlValue::StringType(s) => self.writer.write_quoted_str(s)?,
            BinXmlValue::AnsiStringType(s) => self.writer.write_quoted_str(s)?,
            BinXmlValue::Int8Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt8Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int16Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt16Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int32Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt32Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Int64Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::UInt64Type(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::Real32Type(n) => self.writer.write_str(&format!("{}", n))?,
            BinXmlValue::Real64Type(n) => self.writer.write_str(&format!("{}", n))?,
            BinXmlValue::BoolType(b) => self.writer.write_str(if *b {"true"} else {"false"})?,
            BinXmlValue::GuidType(g) => self.writer.write_quoted_str(&g.to_string())?,
            BinXmlValue::SizeTType(n) => self.writer.write_str(&n.to_string())?,
            BinXmlValue::FileTimeType(dt) | BinXmlValue::SysTimeType(dt) => self.writer.write_quoted_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())?,
            BinXmlValue::SidType(sid) => self.writer.write_quoted_str(&sid.to_string())?,
            BinXmlValue::HexInt32Type(s) | BinXmlValue::HexInt64Type(s) => self.writer.write_quoted_str(s)?,
            // Binary and arrays: fallback to string or null for now (rare in hot path)
            BinXmlValue::BinaryType(_)
            | BinXmlValue::EvtHandle
            | BinXmlValue::BinXmlType(_)
            | BinXmlValue::EvtXml
            | BinXmlValue::StringArrayType(_)
            | BinXmlValue::AnsiStringArrayType
            | BinXmlValue::Int8ArrayType(_)
            | BinXmlValue::UInt8ArrayType(_)
            | BinXmlValue::Int16ArrayType(_)
            | BinXmlValue::UInt16ArrayType(_)
            | BinXmlValue::Int32ArrayType(_)
            | BinXmlValue::UInt32ArrayType(_)
            | BinXmlValue::Int64ArrayType(_)
            | BinXmlValue::UInt64ArrayType(_)
            | BinXmlValue::Real32ArrayType(_)
            | BinXmlValue::Real64ArrayType(_)
            | BinXmlValue::BoolArrayType(_)
            | BinXmlValue::BinaryArrayType
            | BinXmlValue::GuidArrayType(_)
            | BinXmlValue::SizeTArrayType
            | BinXmlValue::FileTimeArrayType(_)
            | BinXmlValue::SysTimeArrayType(_)
            | BinXmlValue::SidArrayType(_)
            | BinXmlValue::HexInt32ArrayType(_)
            | BinXmlValue::HexInt64ArrayType(_)
            | BinXmlValue::EvtArrayHandle
            | BinXmlValue::BinXmlArrayType
            | BinXmlValue::EvtXmlArrayType => self.writer.write_str("null")?,
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
                    if !first { /* nothing yet */ }
                    // Only write non-null
                    if !matches!(*attr.value, BinXmlValue::NullType) {
                        if !first { self.writer.write_str(",")?; }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.write_binxml_scalar(&attr.value)?;
                    }
                }
                self.writer.write_str("}")?;
            } else {
                self.write_key("#attributes")?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    if !matches!(*attr.value, BinXmlValue::NullType) {
                        if !first { self.writer.write_str(",")?; }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.write_binxml_scalar(&attr.value)?;
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
        match value {
            Cow::Borrowed(v) => self.write_binxml_scalar(v)?,
            Cow::Owned(v) => self.write_binxml_scalar(&v)?,
        }
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