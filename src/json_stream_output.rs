use crate::ParserSettings;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{SerializationError, SerializationResult};
use crate::model::xml::{BinXmlPI, XmlElement};
use crate::xml_output::BinXmlOutput;

use std::borrow::Cow;
use std::io::{Result as IoResult, Write};

use hashbrown::HashMap as FastMap;
use hashbrown::hash_map::Entry;
use quick_xml::events::BytesText;
// itoa/ryu used via fully-qualified paths in helpers; avoid importing single components

#[inline]
fn decimal_len(mut n: usize) -> usize {
    if n == 0 {
        return 1;
    }
    let mut len = 0;
    while n > 0 {
        n /= 10;
        len += 1;
    }
    len
}

#[inline]
fn append_usize(s: &mut String, n: usize) {
    let mut buf = itoa::Buffer::new();
    s.push_str(buf.format(n));
}

#[derive(Clone)]
enum FlatScalar {
    RawNumber(String),
    Quoted(String),
    Bool(bool),
}

#[derive(Clone)]
enum TextValue {
    Scalar(FlatScalar),
    Array(Vec<FlatScalar>),
}

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
        self.writer.write_all(b"\"")?;
        let bytes = s.as_bytes();
        let mut run_start = 0usize;
        let len = bytes.len();
        let hex = b"0123456789ABCDEF";
        let mut i = 0usize;
        while i < len {
            let b = bytes[i];
            let needs_escape = matches!(b, b'"' | b'\\' | b'\n' | b'\r' | b'\t') || (b <= 0x1F);
            if needs_escape {
                if run_start < i {
                    self.writer.write_all(&bytes[run_start..i])?;
                }
                match b {
                    b'"' => self.writer.write_all(b"\\\"")?,
                    b'\\' => self.writer.write_all(b"\\\\")?,
                    b'\n' => self.writer.write_all(b"\\n")?,
                    b'\r' => self.writer.write_all(b"\\r")?,
                    b'\t' => self.writer.write_all(b"\\t")?,
                    0x00..=0x1F => {
                        let esc = [
                            b'\\',
                            b'u',
                            b'0',
                            b'0',
                            hex[(b >> 4) as usize],
                            hex[(b & 0x0F) as usize],
                        ];
                        self.writer.write_all(&esc)?;
                    }
                    _ => {}
                }
                run_start = i + 1;
            }
            i += 1;
        }
        if run_start < len {
            self.writer.write_all(&bytes[run_start..len])?;
        }
        self.writer.write_all(b"\"")
    }

    #[inline]
    fn write_i64(&mut self, n: i64) -> IoResult<()> {
        let mut buf = itoa::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    fn write_u64(&mut self, n: u64) -> IoResult<()> {
        let mut buf = itoa::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    fn write_f32(&mut self, n: f32) -> IoResult<()> {
        let mut buf = ryu::Buffer::new();
        self.write_str(buf.format(n))
    }

    #[inline]
    fn write_f64(&mut self, n: f64) -> IoResult<()> {
        let mut buf = ryu::Buffer::new();
        self.write_str(buf.format(n))
    }
}

#[derive(Default)]
struct ObjectContext {
    // Whether this JSON object (corresponding to the current XML element) has emitted at least
    // one field, for comma management.
    has_any_field: bool,
    // Per-parent duplicate counters for child keys
    dup_counters: FastMap<String, usize, ahash::RandomState>,
    // For streaming: if the current XML element's object is not yet opened, we hold the key that
    // should be used when opening it. If we end up writing scalar text directly, we will use this
    // key on the parent object instead and never open this object.
    pending_key: Option<String>,
    // Whether this context represents an opened `{}` JSON object. If false, the element is in a
    // deferred state and may either become a scalar or be opened later when the first child is
    // encountered.
    object_opened: bool,
    // Whether we have already written a scalar value directly into the parent for this element.
    wrote_scalar: bool,
    // Whether we emitted a sibling `<name>_attributes` object (when separate_json_attributes=true).
    separated_attr_emitted: bool,
    // Whether this context is specifically an EventData-like container (EventData/UserData)
    element_is_eventdata: bool,
    // If set, accumulates <Data>...</Data> text values under this container
    aggregated_data_values: Option<Vec<String>>,
    // True when this context represents a synthetic child for aggregated <Data> (we don't emit output on close)
    is_aggregated_data_child: bool,
    // Collects character content for this element until close; allows upgrading to arrays and merging
    pending_text: Option<Vec<TextValue>>,
    // For parents: hold the last unflushed scalar-only child per base key to enable last-one-wins
    suspended_scalars: Option<FastMap<String, Vec<TextValue>, ahash::RandomState>>,
    // For parents: next duplicate index for each base when flushing old suspended entries
    next_dup_index: Option<FastMap<String, usize, ahash::RandomState>>,
    // For children: if attributes exist and separate_json_attributes=false, hold attributes until we flush
    pending_attributes: Option<Vec<(String, FlatScalar)>>,
    // For parents: hold the last unflushed attributes-only child per base key
    suspended_attrs: Option<FastMap<String, Vec<(String, FlatScalar)>, ahash::RandomState>>,
}

pub struct JsonStreamOutput<W: Write> {
    writer: JsonWriter<W>,
    // Stack of open objects (root + nested elements)
    stack: Vec<ObjectContext>,
    // separate_json_attributes option
    separate_json_attributes: bool,
    // Reused hasher to avoid repeated RandomState::new costs
    hasher: ahash::RandomState,
}

impl<W: Write> JsonStreamOutput<W> {
    pub fn with_writer(writer: W, settings: &ParserSettings) -> Self {
        Self {
            writer: JsonWriter::new(writer),
            stack: Vec::new(),
            separate_json_attributes: settings.should_separate_json_attributes(),
            hasher: ahash::RandomState::new(),
        }
    }

    pub fn into_writer(self) -> W {
        self.writer.writer
    }

    fn ensure_root(&mut self) {
        if self.stack.is_empty() {
            self.stack.push(ObjectContext {
                has_any_field: false,
                dup_counters: FastMap::with_hasher(self.hasher.clone()),
                pending_key: None,
                object_opened: true,
                wrote_scalar: false,
                separated_attr_emitted: false,
                element_is_eventdata: false,
                aggregated_data_values: None,
                is_aggregated_data_child: false,
                pending_text: None,
                suspended_scalars: None,
                next_dup_index: None,
                pending_attributes: None,
                suspended_attrs: None,
            });
        }
    }

    fn current_index(&self) -> usize {
        self.stack.len() - 1
    }

    fn parent_index(&self) -> Option<usize> {
        if self.stack.len() >= 2 {
            Some(self.stack.len() - 2)
        } else {
            None
        }
    }

    fn write_comma_if_needed_at(&mut self, idx: usize) -> SerializationResult<()> {
        let needs_comma = self.stack[idx].has_any_field;
        if needs_comma {
            self.writer.write_str(",")?;
        }
        self.stack[idx].has_any_field = true;
        Ok(())
    }

    fn write_key_in(&mut self, idx: usize, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed_at(idx)?;
        self.writer.write_quoted_str(key)?;
        self.writer.write_str(":")?;
        Ok(())
    }

    #[inline]
    fn contains_array(items: &[TextValue]) -> bool {
        items.iter().any(|t| matches!(t, TextValue::Array(_)))
    }

    fn open_context_object_at(&mut self, idx: usize) -> SerializationResult<()> {
        // Open the object for context at `idx` by writing its pending key in its parent and a '{'
        if idx == 0 {
            return Ok(());
        }
        let parent_idx = idx - 1;
        let pending_key = {
            let ctx = &mut self.stack[idx];
            ctx.pending_key
                .take()
                .ok_or_else(|| SerializationError::JsonStructureError {
                    message: "Missing pending key when opening object".to_string(),
                })?
        };
        self.write_key_in(parent_idx, &pending_key)?;
        self.writer.write_str("{")?;
        self.stack[idx].object_opened = true;
        self.stack[idx].has_any_field = false;

        // If parent has suspended attributes for this base, emit them inline now as #attributes
        let base = if let Some(pos) = pending_key.rfind('_') {
            let rest = &pending_key[pos + 1..];
            if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                pending_key[..pos].to_string()
            } else {
                pending_key.clone()
            }
        } else {
            pending_key.clone()
        };
        if !self.separate_json_attributes {
            let maybe_attrs = self.stack[parent_idx]
                .suspended_attrs
                .as_mut()
                .and_then(|m| m.remove(&base));
            if let Some(attrs) = maybe_attrs {
                // write #attributes into the newly opened child object
                self.writer.write_quoted_str("#attributes")?;
                self.writer.write_str(":{")?;
                let mut first = true;
                for (name, val) in attrs.iter() {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(name)?;
                    self.writer.write_str(":")?;
                    self.write_flat_scalar(val)?;
                }
                self.writer.write_str("}")?;
                self.stack[idx].has_any_field = true;
            }
        }
        Ok(())
    }

    fn ensure_current_container_open(&mut self) -> SerializationResult<()> {
        // Ensure the top-most context (current container) is an opened JSON object, so that children
        // can be written into it. If it's deferred, open it now.
        self.ensure_root();
        let idx = self.current_index();
        if !self.stack[idx].object_opened {
            self.open_context_object_at(idx)?;
        }
        Ok(())
    }

    fn allocate_child_key_under_current(&mut self, base: &str) -> String {
        // Use parent's duplicate counters to allocate a unique key.
        self.ensure_root();
        let parent = self.stack.last_mut().unwrap();
        match parent.dup_counters.entry(base.to_owned()) {
            Entry::Occupied(mut e) => {
                let idx = *e.get();
                *e.get_mut() = idx + 1;
                // Build "base_idx" efficiently
                let mut s = String::with_capacity(base.len() + 1 + decimal_len(idx));
                s.push_str(base);
                s.push('_');
                append_usize(&mut s, idx);
                s
            }
            Entry::Vacant(e) => {
                // First occurrence: record that the next duplicate should be _2
                e.insert(2);
                base.to_owned()
            }
        }
    }

    fn flush_suspended_scalar_if_needed(
        &mut self,
        parent_idx: usize,
        base: &str,
        _current_key: &str,
    ) -> SerializationResult<()> {
        // Non-streaming keeps only the last unsuffixed node for repeated names.
        // To emulate this, when a new child for the same base arrives, we emit the previous suspended under base_{N},
        // where N starts at 1 and increments per base.
        if let Some(map) = self.stack[parent_idx].suspended_scalars.as_mut() {
            if let Some(items) = map.remove(base) {
                // Figure out N using a dedicated per-base counter starting at 1
                if self.stack[parent_idx].next_dup_index.is_none() {
                    self.stack[parent_idx].next_dup_index =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                let n = {
                    let ctrs = self.stack[parent_idx].next_dup_index.as_mut().unwrap();
                    let entry = ctrs.entry(base.to_owned()).or_insert(1);
                    let v = *entry;
                    *entry += 1;
                    v
                };
                let mut dup_key = String::with_capacity(base.len() + 1 + decimal_len(n));
                dup_key.push_str(base);
                dup_key.push('_');
                append_usize(&mut dup_key, n);
                self.flush_text_into_parent(parent_idx, &dup_key, items)?;
            }
        }
        Ok(())
    }

    fn flush_all_suspended_into_object(&mut self, idx: usize) -> SerializationResult<()> {
        // Take suspended maps out to avoid borrow conflicts during writes
        let mut vals: FastMap<String, Vec<TextValue>, ahash::RandomState> = self.stack[idx]
            .suspended_scalars
            .take()
            .unwrap_or_else(|| FastMap::with_hasher(self.hasher.clone()));
        let mut attrs: FastMap<String, Vec<(String, FlatScalar)>, ahash::RandomState> = self.stack
            [idx]
            .suspended_attrs
            .take()
            .unwrap_or_else(|| FastMap::with_hasher(self.hasher.clone()));

        // Phase A: flush values, merging attrs if present per base
        for (base, v) in vals.drain() {
            if let Some(a) = attrs.remove(&base) {
                self.write_child_object_with_attrs_and_text(idx, &base, &a, &v)?;
            } else {
                self.flush_text_into_parent(idx, &base, v)?;
            }
        }

        // Phase B: flush any remaining attributes-only entries
        for (base, a) in attrs.into_iter() {
            self.write_child_object_with_attrs_only(idx, &base, &a)?;
        }
        Ok(())
    }

    fn write_binxml_scalar(&mut self, v: &BinXmlValue) -> SerializationResult<()> {
        match v {
            BinXmlValue::NullType => self.writer.write_str("null")?,
            BinXmlValue::StringType(s) => self.writer.write_quoted_str(s)?,
            BinXmlValue::AnsiStringType(s) => self.writer.write_quoted_str(s)?,
            BinXmlValue::Int8Type(n) => self.writer.write_i64(*n as i64)?,
            BinXmlValue::UInt8Type(n) => self.writer.write_u64(*n as u64)?,
            BinXmlValue::Int16Type(n) => self.writer.write_i64(*n as i64)?,
            BinXmlValue::UInt16Type(n) => self.writer.write_u64(*n as u64)?,
            BinXmlValue::Int32Type(n) => self.writer.write_i64(*n as i64)?,
            BinXmlValue::UInt32Type(n) => self.writer.write_u64(*n as u64)?,
            BinXmlValue::Int64Type(n) => self.writer.write_i64(*n)?,
            BinXmlValue::UInt64Type(n) => self.writer.write_u64(*n)?,
            BinXmlValue::Real32Type(n) => self.writer.write_f32(*n as f32)?,
            BinXmlValue::Real64Type(n) => self.writer.write_f64(*n)?,
            BinXmlValue::BoolType(b) => self.writer.write_str(if *b { "true" } else { "false" })?,
            BinXmlValue::GuidType(g) => self.writer.write_quoted_str(&g.to_string())?,
            BinXmlValue::SizeTType(n) => self.writer.write_u64(*n as u64)?,
            BinXmlValue::FileTimeType(dt) | BinXmlValue::SysTimeType(dt) => self
                .writer
                .write_quoted_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())?,
            BinXmlValue::SidType(sid) => self.writer.write_quoted_str(&sid.to_string())?,
            BinXmlValue::HexInt32Type(s) | BinXmlValue::HexInt64Type(s) => {
                self.writer.write_quoted_str(s)?
            }
            // Arrays -> JSON arrays mirroring non-streaming writer
            BinXmlValue::StringArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for s in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(s)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Int8ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_i64(*n as i64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::UInt8ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_u64(*n as u64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Int16ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_i64(*n as i64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::UInt16ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_u64(*n as u64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Int32ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_i64(*n as i64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::UInt32ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_u64(*n as u64)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Int64ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_i64(*n)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::UInt64ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_u64(*n)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Real32ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_f32(*n as f32)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::Real64ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for n in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_f64(*n)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::BoolArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for b in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_str(if *b { "true" } else { "false" })?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::GuidArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for g in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(&g.to_string())?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::FileTimeArrayType(values) | BinXmlValue::SysTimeArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for dt in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer
                        .write_quoted_str(&dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string())?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::SidArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for sid in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(&sid.to_string())?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::HexInt32ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for s in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(s)?;
                }
                self.writer.write_str("]")?;
            }
            BinXmlValue::HexInt64ArrayType(values) => {
                self.writer.write_str("[")?;
                let mut first = true;
                for s in values {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.writer.write_quoted_str(s)?;
                }
                self.writer.write_str("]")?;
            }
            // Unsupported / non-textual in character context
            BinXmlValue::BinaryType(_)
            | BinXmlValue::EvtHandle
            | BinXmlValue::BinXmlType(_)
            | BinXmlValue::EvtXml
            | BinXmlValue::BinaryArrayType
            | BinXmlValue::SizeTArrayType
            | BinXmlValue::EvtArrayHandle
            | BinXmlValue::BinXmlArrayType
            | BinXmlValue::EvtXmlArrayType
            | BinXmlValue::AnsiStringArrayType => self.writer.write_str("null")?,
        }
        Ok(())
    }

    #[inline]
    fn should_skip_character_value(value: &BinXmlValue) -> bool {
        matches!(
            value,
            BinXmlValue::NullType
                | BinXmlValue::BinaryType(_)
                | BinXmlValue::EvtHandle
                | BinXmlValue::BinXmlType(_)
                | BinXmlValue::EvtXml
                | BinXmlValue::BinaryArrayType
                | BinXmlValue::SizeTArrayType
                | BinXmlValue::EvtArrayHandle
                | BinXmlValue::BinXmlArrayType
                | BinXmlValue::EvtXmlArrayType
                | BinXmlValue::AnsiStringArrayType
        )
    }

    fn write_key(&mut self, key: &str) -> SerializationResult<()> {
        let idx = self.current_index();
        self.write_comma_if_needed_at(idx)?;
        self.writer.write_quoted_str(key)?;
        self.writer.write_str(":")?;
        Ok(())
    }

    fn to_flat_scalar(value: &BinXmlValue) -> Option<FlatScalar> {
        match value {
            BinXmlValue::StringType(s) => Some(FlatScalar::Quoted(s.clone())),
            BinXmlValue::AnsiStringType(s) => Some(FlatScalar::Quoted(s.as_ref().to_string())),
            BinXmlValue::Int8Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as i64).to_owned()))
            }
            BinXmlValue::UInt8Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as u64).to_owned()))
            }
            BinXmlValue::Int16Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as i64).to_owned()))
            }
            BinXmlValue::UInt16Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as u64).to_owned()))
            }
            BinXmlValue::Int32Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as i64).to_owned()))
            }
            BinXmlValue::UInt32Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as u64).to_owned()))
            }
            BinXmlValue::Int64Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n).to_owned()))
            }
            BinXmlValue::UInt64Type(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n).to_owned()))
            }
            BinXmlValue::Real32Type(n) => {
                let mut b = ryu::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as f32).to_owned()))
            }
            BinXmlValue::Real64Type(n) => {
                let mut b = ryu::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n).to_owned()))
            }
            BinXmlValue::BoolType(b) => Some(FlatScalar::Bool(*b)),
            BinXmlValue::GuidType(g) => Some(FlatScalar::Quoted(g.to_string())),
            BinXmlValue::SizeTType(n) => {
                let mut b = itoa::Buffer::new();
                Some(FlatScalar::RawNumber(b.format(*n as u64).to_owned()))
            }
            BinXmlValue::FileTimeType(dt) | BinXmlValue::SysTimeType(dt) => Some(
                FlatScalar::Quoted(dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()),
            ),
            BinXmlValue::SidType(sid) => Some(FlatScalar::Quoted(sid.to_string())),
            BinXmlValue::HexInt32Type(s) => Some(FlatScalar::Quoted(s.as_ref().to_string())),
            BinXmlValue::HexInt64Type(s) => Some(FlatScalar::Quoted(s.as_ref().to_string())),
            _ => None,
        }
    }

    fn to_text_value(value: &BinXmlValue) -> Option<TextValue> {
        match value {
            // scalar
            BinXmlValue::StringType(_)
            | BinXmlValue::AnsiStringType(_)
            | BinXmlValue::Int8Type(_)
            | BinXmlValue::UInt8Type(_)
            | BinXmlValue::Int16Type(_)
            | BinXmlValue::UInt16Type(_)
            | BinXmlValue::Int32Type(_)
            | BinXmlValue::UInt32Type(_)
            | BinXmlValue::Int64Type(_)
            | BinXmlValue::UInt64Type(_)
            | BinXmlValue::Real32Type(_)
            | BinXmlValue::Real64Type(_)
            | BinXmlValue::BoolType(_)
            | BinXmlValue::GuidType(_)
            | BinXmlValue::SizeTType(_)
            | BinXmlValue::FileTimeType(_)
            | BinXmlValue::SysTimeType(_)
            | BinXmlValue::SidType(_)
            | BinXmlValue::HexInt32Type(_)
            | BinXmlValue::HexInt64Type(_) => Self::to_flat_scalar(value).map(TextValue::Scalar),
            // arrays
            BinXmlValue::StringArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|s| FlatScalar::Quoted(s.clone()))
                    .collect(),
            )),
            BinXmlValue::Int8ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as i64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::UInt8ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as u64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::Int16ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as i64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::UInt16ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as u64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::Int32ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as i64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::UInt32ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as u64).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::Int64ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::UInt64ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = itoa::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::Real32ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = ryu::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n as f32).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::Real64ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|n| {
                        let mut b = ryu::Buffer::new();
                        FlatScalar::RawNumber(b.format(*n).to_owned())
                    })
                    .collect(),
            )),
            BinXmlValue::BoolArrayType(values) => Some(TextValue::Array(
                values.iter().map(|b| FlatScalar::Bool(*b)).collect(),
            )),
            BinXmlValue::GuidArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|g| FlatScalar::Quoted(g.to_string()))
                    .collect(),
            )),
            BinXmlValue::FileTimeArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|dt| FlatScalar::Quoted(dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()))
                    .collect(),
            )),
            BinXmlValue::SysTimeArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|dt| FlatScalar::Quoted(dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string()))
                    .collect(),
            )),
            BinXmlValue::SidArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|sid| FlatScalar::Quoted(sid.to_string()))
                    .collect(),
            )),
            BinXmlValue::HexInt32ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|s| FlatScalar::Quoted(s.as_ref().to_string()))
                    .collect(),
            )),
            BinXmlValue::HexInt64ArrayType(values) => Some(TextValue::Array(
                values
                    .iter()
                    .map(|s| FlatScalar::Quoted(s.as_ref().to_string()))
                    .collect(),
            )),
            _ => None,
        }
    }

    fn write_flat_scalar(&mut self, s: &FlatScalar) -> SerializationResult<()> {
        match s {
            FlatScalar::RawNumber(n) => self.writer.write_str(n)?,
            FlatScalar::Quoted(q) => self.writer.write_quoted_str(q)?,
            FlatScalar::Bool(b) => self.writer.write_str(if *b { "true" } else { "false" })?,
        }
        Ok(())
    }

    fn write_text_items_as_array(&mut self, items: &[TextValue]) -> SerializationResult<()> {
        self.writer.write_str("[")?;
        let mut first = true;
        for item in items {
            match item {
                TextValue::Scalar(s) => {
                    if !first {
                        self.writer.write_str(",")?;
                    }
                    first = false;
                    self.write_flat_scalar(s)?;
                }
                TextValue::Array(inner) => {
                    for s in inner {
                        if !first {
                            self.writer.write_str(",")?;
                        }
                        first = false;
                        self.write_flat_scalar(s)?;
                    }
                }
            }
        }
        self.writer.write_str("]")?;
        Ok(())
    }

    fn flush_text_into_current_object(&mut self, idx: usize) -> SerializationResult<()> {
        let items_opt = self.stack[idx].pending_text.take();
        if let Some(items) = items_opt {
            self.write_key_in(idx, "#text")?;
            let needs_array =
                items.len() > 1 || items.iter().any(|t| matches!(t, TextValue::Array(_)));
            if needs_array {
                self.write_text_items_as_array(&items)?;
            } else {
                match &items[0] {
                    TextValue::Scalar(s) => self.write_flat_scalar(s)?,
                    TextValue::Array(inner) => {
                        self.write_text_items_as_array(&[TextValue::Array(inner.clone())])?;
                    }
                }
            }
        }
        Ok(())
    }

    fn flush_text_into_parent(
        &mut self,
        parent_idx: usize,
        key: &str,
        items: Vec<TextValue>,
    ) -> SerializationResult<()> {
        self.write_key_in(parent_idx, key)?;
        if items.len() == 1 {
            match &items[0] {
                TextValue::Scalar(s) => {
                    self.write_flat_scalar(s)?;
                }
                TextValue::Array(inner) => {
                    // Output a single array
                    self.write_text_items_as_array(&[TextValue::Array(inner.clone())])?;
                }
            }
            return Ok(());
        }
        if Self::contains_array(&items) {
            self.write_text_items_as_array(&items)?;
        } else {
            // Multiple scalar chunks in a single element without attributes -> concatenate into a single string
            let mut total_len: usize = 0;
            for item in items.iter() {
                if let TextValue::Scalar(s) = item {
                    match s {
                        FlatScalar::Quoted(q) => total_len += q.len(),
                        FlatScalar::RawNumber(n) => total_len += n.len(),
                        FlatScalar::Bool(b) => total_len += if *b { 4 } else { 5 },
                    }
                }
            }
            let mut concatenated = String::with_capacity(total_len);
            for item in items {
                if let TextValue::Scalar(s) = item {
                    match s {
                        FlatScalar::Quoted(q) => concatenated.push_str(&q),
                        FlatScalar::RawNumber(n) => concatenated.push_str(&n),
                        FlatScalar::Bool(b) => {
                            concatenated.push_str(if b { "true" } else { "false" })
                        }
                    }
                }
            }
            self.writer.write_quoted_str(&concatenated)?;
        }
        Ok(())
    }

    fn write_child_object_with_attrs_and_text(
        &mut self,
        parent_idx: usize,
        key: &str,
        attrs: &[(String, FlatScalar)],
        text_items: &[TextValue],
    ) -> SerializationResult<()> {
        self.write_key_in(parent_idx, key)?;
        self.writer.write_str("{")?;
        // #attributes
        self.writer.write_quoted_str("#attributes")?;
        self.writer.write_str(":{")?;
        let mut first = true;
        for (name, val) in attrs.iter() {
            if !first {
                self.writer.write_str(",")?;
            }
            first = false;
            self.writer.write_quoted_str(name)?;
            self.writer.write_str(":")?;
            self.write_flat_scalar(val)?;
        }
        self.writer.write_str("}")?;
        // separator before #text
        self.writer.write_str(",")?;
        self.writer.write_quoted_str("#text")?;
        self.writer.write_str(":")?;
        if Self::contains_array(text_items) {
            self.write_text_items_as_array(text_items)?;
        } else {
            match &text_items[0] {
                TextValue::Scalar(s) => self.write_flat_scalar(s)?,
                TextValue::Array(inner) => {
                    self.write_text_items_as_array(&[TextValue::Array(inner.clone())])?
                }
            }
        }
        self.writer.write_str("}")?;
        Ok(())
    }

    fn write_child_object_with_attrs_only(
        &mut self,
        parent_idx: usize,
        key: &str,
        attrs: &[(String, FlatScalar)],
    ) -> SerializationResult<()> {
        self.write_key_in(parent_idx, key)?;
        self.writer.write_str("{")?;
        self.writer.write_quoted_str("#attributes")?;
        self.writer.write_str(":{")?;
        let mut first = true;
        for (name, val) in attrs.iter() {
            if !first {
                self.writer.write_str(",")?;
            }
            first = false;
            self.writer.write_quoted_str(name)?;
            self.writer.write_str(":")?;
            self.write_flat_scalar(val)?;
        }
        self.writer.write_str("}")?;
        self.writer.write_str("}")?;
        Ok(())
    }

    fn flush_prev_for_base_into_suffixed_child(
        &mut self,
        parent_idx: usize,
        base: &str,
    ) -> SerializationResult<()> {
        let mut prev_vals: Option<Vec<TextValue>> = None;
        if let Some(map) = self.stack[parent_idx].suspended_scalars.as_mut() {
            if let Some(vals) = map.remove(base) {
                prev_vals = Some(vals);
            }
        }
        let mut prev_attrs: Option<Vec<(String, FlatScalar)>> = None;
        if let Some(attrs_map) = self.stack[parent_idx].suspended_attrs.as_mut() {
            if let Some(attrs) = attrs_map.remove(base) {
                prev_attrs = Some(attrs);
            }
        }

        if prev_vals.is_none() && prev_attrs.is_none() {
            return Ok(());
        }

        if self.stack[parent_idx].next_dup_index.is_none() {
            self.stack[parent_idx].next_dup_index = Some(FastMap::with_hasher(self.hasher.clone()));
        }
        let n = {
            let ctrs = self.stack[parent_idx].next_dup_index.as_mut().unwrap();
            let entry = ctrs.entry(base.to_string()).or_insert(1);
            let v = *entry;
            *entry += 1;
            v
        };
        // Build duplicate key efficiently: base + '_' + n
        let mut dup_key = String::with_capacity(base.len() + 1 + decimal_len(n));
        dup_key.push_str(base);
        dup_key.push('_');
        append_usize(&mut dup_key, n);

        match (prev_vals, prev_attrs) {
            (Some(vals), Some(attrs)) => {
                self.write_child_object_with_attrs_and_text(parent_idx, &dup_key, &attrs, &vals)?;
            }
            (Some(vals), None) => {
                self.flush_text_into_parent(parent_idx, &dup_key, vals)?;
            }
            (None, Some(attrs)) => {
                self.write_child_object_with_attrs_only(parent_idx, &dup_key, &attrs)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn push_deferred_element(&mut self, key: String, separated_attr_emitted: bool) {
        self.stack.push(ObjectContext {
            has_any_field: false,
            dup_counters: FastMap::with_hasher(self.hasher.clone()),
            pending_key: Some(key),
            object_opened: false,
            wrote_scalar: false,
            separated_attr_emitted,
            element_is_eventdata: false,
            aggregated_data_values: None,
            is_aggregated_data_child: false,
            pending_text: None,
            suspended_scalars: None,
            next_dup_index: None,
            pending_attributes: None,
            suspended_attrs: None,
        });
    }

    // streaming path does not need a Value materializer
}

impl<W: Write> BinXmlOutput for JsonStreamOutput<W> {
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        // Root object
        self.writer.write_str("{")?;
        self.stack.push(ObjectContext {
            has_any_field: false,
            dup_counters: FastMap::with_hasher(self.hasher.clone()),
            pending_key: None,
            object_opened: true,
            wrote_scalar: false,
            separated_attr_emitted: false,
            element_is_eventdata: false,
            aggregated_data_values: None,
            is_aggregated_data_child: false,
            pending_text: None,
            suspended_scalars: None,
            next_dup_index: None,
            pending_attributes: None,
            suspended_attrs: None,
        });
        Ok(())
    }

    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        // Close any open objects including root
        while self.stack.len() > 1 {
            let last_idx = self.stack.len() - 1;
            if self.stack[last_idx].object_opened {
                self.writer.write_str("}")?;
            }
            self.stack.pop();
        }
        // Flush any suspended scalars on root before closing root object
        if let Some(root_map) = self
            .stack
            .get_mut(0)
            .and_then(|r| r.suspended_scalars.as_mut())
        {
            // Emit in insertion order is not guaranteed in FastMap; but tests assert JSON structure, not key order.
            let items: Vec<(String, Vec<TextValue>)> = root_map.drain().collect();
            for (base, vals) in items {
                // flush_text_into_parent writes the key itself, so avoid double key write here
                self.flush_text_into_parent(0, &base, vals)?;
            }
        }
        self.writer.write_str("}")?;
        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        // Ensure current container is an opened object to host this element and its siblings
        self.ensure_current_container_open()?;

        let base = element.name.as_str();
        // debug logging removed in release

        // Special-case: <Data Name="X">text</Data>
        if base == "Data" {
            // First, honor Name attribute if present
            let mut data_key: Option<String> = None;
            for attr in element.attributes.iter() {
                if attr.name.as_str() == "Name" {
                    let k = attr.value.as_ref().as_cow_str().to_string();
                    data_key = Some(k);
                    break;
                }
            }
            if let Some(k) = data_key {
                // Ensure last-one-wins: flush prior unsuffixed sibling of same base to suffixed key
                let parent_idx = self.current_index();
                if self.stack[parent_idx].suspended_scalars.is_none() {
                    self.stack[parent_idx].suspended_scalars =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                if self.stack[parent_idx].suspended_attrs.is_none() {
                    self.stack[parent_idx].suspended_attrs =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                self.flush_prev_for_base_into_suffixed_child(parent_idx, &k)?;

                // Defer opening; text will be emitted under unsuffixed key k
                self.push_deferred_element(k, false);
                return Ok(());
            }

            // No Name attribute: EventData/UserData semantics
            // - separate_json_attributes=true: collect text across siblings on parent; emit once at parent close as a single concatenated string
            // - separate_json_attributes=false: open an object and emit { "#text": [...] }
            if self.separate_json_attributes {
                // Create a synthetic child that funnels text into parent's aggregated_data_values
                // Ensure parent container has aggregation enabled
                let parent_idx = self.current_index();
                if !self.stack[parent_idx].element_is_eventdata {
                    self.stack[parent_idx].element_is_eventdata = true;
                }
                if self.stack[parent_idx].aggregated_data_values.is_none() {
                    self.stack[parent_idx].aggregated_data_values = Some(Vec::new());
                }
                // Push synthetic child
                self.stack.push(ObjectContext {
                    has_any_field: false,
                    dup_counters: FastMap::with_hasher(self.hasher.clone()),
                    pending_key: None,
                    object_opened: false,
                    wrote_scalar: false,
                    separated_attr_emitted: false,
                    element_is_eventdata: false,
                    aggregated_data_values: None,
                    is_aggregated_data_child: true,
                    pending_text: None,
                    suspended_scalars: None,
                    next_dup_index: None,
                    pending_attributes: None,
                    suspended_attrs: None,
                });
                return Ok(());
            } else {
                let key = self.allocate_child_key_under_current("Data");
                self.push_deferred_element(key, false);
                self.open_context_object_at(self.current_index())?;
                return Ok(());
            }
        }

        let is_eventdata_like = base == "EventData" || base == "UserData";

        // Attributes handling
        let mut has_non_null_attr = false;
        for attr in element.attributes.iter() {
            if !matches!(*attr.value, BinXmlValue::NullType) {
                has_non_null_attr = true;
                break;
            }
        }

        if has_non_null_attr {
            if self.separate_json_attributes {
                // Emit sibling <name>_attributes at current container level
                let parent_idx = self.current_index();
                let key = self.allocate_child_key_under_current(base);
                // For separate_json_attributes, expected numbering for duplicates starts at _1 for the second occurrence.
                // Our general allocator returns base, base_2, base_3, ... so we remap attribute suffix by subtracting 1.
                let attr_key = if let Some(pos) = key.rfind('_') {
                    let rest = &key[pos + 1..];
                    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                        if let Ok(n) = rest.parse::<usize>() {
                            if n > 1 {
                                let mut s = String::with_capacity(key.len() + 12);
                                s.push_str(&key[..pos]);
                                s.push('_');
                                append_usize(&mut s, n - 1);
                                s.push_str("_attributes");
                                s
                            } else {
                                let mut s = String::with_capacity(key.len() + 11);
                                s.push_str(&key[..pos]);
                                s.push_str("_attributes");
                                s
                            }
                        } else {
                            let mut s = String::with_capacity(key.len() + 11);
                            s.push_str(&key);
                            s.push_str("_attributes");
                            s
                        }
                    } else {
                        let mut s = String::with_capacity(key.len() + 11);
                        s.push_str(&key);
                        s.push_str("_attributes");
                        s
                    }
                } else {
                    let mut s = String::with_capacity(key.len() + 11);
                    s.push_str(&key);
                    s.push_str("_attributes");
                    s
                };
                self.write_key_in(parent_idx, &attr_key)?;
                self.writer.write_str("{")?;
                let mut first = true;
                for attr in element.attributes.iter() {
                    if !matches!(*attr.value, BinXmlValue::NullType) {
                        if !first {
                            self.writer.write_str(",")?;
                        }
                        first = false;
                        self.writer.write_quoted_str(attr.name.as_str())?;
                        self.writer.write_str(":")?;
                        self.write_binxml_scalar(&attr.value)?;
                    }
                }
                self.writer.write_str("}")?;

                // Defer opening the element itself until we see text or children
                self.push_deferred_element(key.clone(), true);
                if is_eventdata_like {
                    let idx = self.current_index();
                    self.stack[idx].element_is_eventdata = true;
                    self.stack[idx].aggregated_data_values = Some(Vec::new());
                }
            } else {
                // Do not emit immediately. Suspend attributes under parent to enable last-one-wins unsuffixed.
                let parent_idx = self.current_index();
                // Flush any previous unsuffixed sibling for this base to a suffixed child (merge scalars+attrs if both)
                self.flush_prev_for_base_into_suffixed_child(parent_idx, base)?;

                // Record current attributes as suspended for this base
                let mut attrs_vec: Vec<(String, FlatScalar)> =
                    Vec::with_capacity(element.attributes.len());
                for attr in element.attributes.iter() {
                    if !matches!(*attr.value, BinXmlValue::NullType) {
                        // Convert to flat scalar for stable JSON rendering
                        if let Some(fs) = Self::to_flat_scalar(&attr.value) {
                            attrs_vec.push((attr.name.as_str().to_string(), fs));
                        }
                    }
                }
                if self.stack[parent_idx].suspended_attrs.is_none() {
                    self.stack[parent_idx].suspended_attrs =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                self.stack[parent_idx]
                    .suspended_attrs
                    .as_mut()
                    .unwrap()
                    .insert(base.to_string(), attrs_vec);
                // Still push a deferred child for potential text, using unsuffixed base name
                self.push_deferred_element(base.to_string(), false);
                if is_eventdata_like {
                    let idx = self.current_index();
                    self.stack[idx].element_is_eventdata = true;
                    self.stack[idx].aggregated_data_values = Some(Vec::new());
                }
            }
        } else {
            // No attributes – fully defer; decide later whether scalar or object
            // Before we create a new child for this base, move any previously suspended scalar at the parent
            // to a suffixed key so the previous one is preserved, and keep this new one as the latest unsuffixed.
            if self.stack.len() >= 1 {
                let parent_idx = self.current_index();
                // Ensure maps exist before suspension
                if self.stack[parent_idx].suspended_scalars.is_none() {
                    self.stack[parent_idx].suspended_scalars =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                if self.stack[parent_idx].suspended_attrs.is_none() {
                    self.stack[parent_idx].suspended_attrs =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                // Flush any prior suspended (scalars and/or attrs) for this base
                self.flush_prev_for_base_into_suffixed_child(parent_idx, base)?;
            }
            // Use unsuffixed base key for child
            self.push_deferred_element(base.to_string(), false);
            if is_eventdata_like {
                let idx = self.current_index();
                self.stack[idx].element_is_eventdata = true;
                self.stack[idx].aggregated_data_values = Some(Vec::new());
            }
        }

        Ok(())
    }

    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        // On close, either close an opened object, or write a null placeholder (only when we have
        // no attributes sibling and we haven't written any scalar/children), or nothing at all.
        let parent_idx = self.parent_index();
        let idx = self.current_index();

        // Handle synthetic aggregated Data child: flush collected values to parent under a single key
        if self.stack[idx].is_aggregated_data_child {
            if let Some(parent_idx) = parent_idx {
                if let Some(_values) = self.stack[parent_idx].aggregated_data_values.as_mut() {
                    // Only stash values during visit_characters; nothing to emit here.
                }
            }
            self.stack.pop();
            return Ok(());
        }

        // debug logging removed in release

        if self.stack[idx].object_opened {
            // Flush any pending #text and suspended children into the current object before closing it
            self.flush_text_into_current_object(idx)?;
            self.flush_all_suspended_into_object(idx)?;

            // If this is EventData/UserData and we aggregated Data values, emit them before closing
            if self.stack[idx].element_is_eventdata {
                if let Some(values) = self.stack[idx].aggregated_data_values.take() {
                    if !values.is_empty() {
                        if self.separate_json_attributes {
                            // Concatenate into single string under unsuffixed Data
                            if self.stack[idx].has_any_field {
                                self.writer.write_str(",")?;
                            }
                            self.writer.write_quoted_str("Data")?;
                            self.writer.write_str(":")?;
                            let mut total_len = 0usize;
                            for v in values.iter() {
                                total_len += v.len();
                            }
                            let mut concatenated = String::with_capacity(total_len);
                            for v in values.iter() {
                                concatenated.push_str(v);
                            }
                            self.writer.write_quoted_str(&concatenated)?;
                        } else {
                            if self.stack[idx].has_any_field {
                                self.writer.write_str(",")?;
                            }
                            self.writer.write_quoted_str("Data")?;
                            self.writer.write_str(":{")?;
                            self.writer.write_quoted_str("#text")?;
                            self.writer.write_str(":")?;
                            self.writer.write_str("[")?;
                            let mut first = true;
                            for v in values.iter() {
                                if !first {
                                    self.writer.write_str(",")?;
                                }
                                first = false;
                                self.writer.write_quoted_str(v)?;
                            }
                            self.writer.write_str("]")?;
                            self.writer.write_str("}")?;
                        }
                    }
                }
            }
            // Before closing, if this object had a bare scalar-only child previously suspended on this parent,
            // there's nothing to do here; suspension is handled when the next sibling arrives or when the parent closes.
            self.writer.write_str("}")?;
            // Now we can pop the context
            self.stack.pop();
        } else {
            // Deferred element: if we buffered any text, flush to parent under pending key
            let pending_items = self.stack[idx].pending_text.take();
            if let (Some(parent_idx), Some(key), Some(items)) = (
                parent_idx,
                self.stack[idx].pending_key.clone(),
                pending_items,
            ) {
                // For separate_json_attributes=true, Data without Name is handled via a synthetic aggregated child,
                // so no special handling is needed here. Data with Name will follow the generic suspension path below.
                // If attributes for this base are suspended, we must merge them later. So always suspend text here.
                // Determine base from the pending key (strip numeric suffix if present)
                let base_key = if let Some(pos) = key.rfind('_') {
                    let rest = &key[pos + 1..];
                    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
                        key[..pos].to_string()
                    } else {
                        key.clone()
                    }
                } else {
                    key.clone()
                };

                // Always suspend text under base to allow last-one-wins and potential merge with attributes
                if self.stack[parent_idx].suspended_scalars.is_none() {
                    self.stack[parent_idx].suspended_scalars =
                        Some(FastMap::with_hasher(self.hasher.clone()));
                }
                self.stack[parent_idx]
                    .suspended_scalars
                    .as_mut()
                    .unwrap()
                    .insert(base_key, items);
                // Mark parent comma state updated already handled by write_key_in
                self.stack.pop();
                return Ok(());
            }

            // If nothing was written for this element and we didn't emit a separate attributes
            // sibling, mirror the non-streaming behavior and emit `name: null`.
            // Exception: if parent holds suspended attributes for this base name, skip null; we'll flush attributes later.
            if !self.stack[idx].wrote_scalar && !self.stack[idx].separated_attr_emitted {
                let base = element.name.as_str();
                if let Some(pidx) = parent_idx {
                    if self.stack[pidx]
                        .suspended_attrs
                        .as_ref()
                        .map(|m| m.contains_key(base))
                        .unwrap_or(false)
                    {
                        // skip null emission; attributes will be flushed as object later
                        self.stack.pop();
                        return Ok(());
                    }
                }
                let key_cloned = self.stack[idx].pending_key.clone();
                if let (Some(parent_idx), Some(key)) = (parent_idx, key_cloned) {
                    // debug logging removed in release
                    // If this is an EventData/UserData with aggregated values collected, emit them
                    if self.stack[idx].element_is_eventdata {
                        if let Some(values) = self.stack[idx].aggregated_data_values.take() {
                            if !values.is_empty() {
                                self.write_key_in(parent_idx, &key)?;
                                self.writer.write_str("{")?;
                                self.writer.write_quoted_str("#text")?;
                                self.writer.write_str(":")?;
                                self.writer.write_str("[")?;
                                let mut first = true;
                                for v in values.iter() {
                                    if !first {
                                        self.writer.write_str(",")?;
                                    }
                                    first = false;
                                    self.writer.write_quoted_str(v)?;
                                }
                                self.writer.write_str("]")?;
                                self.writer.write_str("}")?;
                            } else {
                                self.write_key_in(parent_idx, &key)?;
                                self.writer.write_str("null")?;
                            }
                        } else {
                            self.write_key_in(parent_idx, &key)?;
                            self.writer.write_str("null")?;
                        }
                    } else {
                        self.write_key_in(parent_idx, &key)?;
                        self.writer.write_str("null")?;
                    }
                }
            }
            self.stack.pop();
        }
        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        // Ignore values that should not be emitted as character content to match non-streaming behavior
        if match &value {
            Cow::Borrowed(v) => Self::should_skip_character_value(v),
            Cow::Owned(v) => Self::should_skip_character_value(v),
        } {
            return Ok(());
        }
        // Collect text; flush on close. This allows upgrading scalars to arrays and merging array-values.
        let idx = self.current_index();
        if !self.stack[idx].object_opened {
            // If we're inside synthetic aggregated Data child, collect string and return
            if self.stack[idx].is_aggregated_data_child {
                let parent_idx = idx - 1;
                if let Some(values) = self.stack[parent_idx].aggregated_data_values.as_mut() {
                    let s = match value {
                        Cow::Borrowed(BinXmlValue::StringType(t)) => t.clone(),
                        Cow::Borrowed(BinXmlValue::AnsiStringType(t)) => t.as_ref().to_string(),
                        Cow::Borrowed(v) => v.as_cow_str().into_owned(),
                        Cow::Owned(v) => v.as_cow_str().into_owned(),
                    };
                    values.push(s);
                    // mark parent as having scalar content
                    self.stack[parent_idx].wrote_scalar = true;
                }
                return Ok(());
            }
        }
        let tv = match &value {
            Cow::Borrowed(v) => Self::to_text_value(v),
            Cow::Owned(v) => Self::to_text_value(v),
        };
        if let Some(tv) = tv {
            if self.stack[idx].pending_text.is_none() {
                self.stack[idx].pending_text = Some(Vec::new());
            }
            self.stack[idx].pending_text.as_mut().unwrap().push(tv);
            self.stack[idx].wrote_scalar = true;
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
                message: format!("Unterminated XML Entity {entity_ref}"),
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
