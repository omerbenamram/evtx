use crate::ParserSettings;
use crate::err::{SerializationError, SerializationResult};
use crate::xml_output::BinXmlOutput;

use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;
use crate::model::xml::{BinXmlPI, XmlElement};
use chrono::{Datelike, Timelike};
use hashbrown::DefaultHashBuilder;
use hashbrown::HashMap;
use quick_xml::events::BytesText;
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;

/// Zig-style fixed table size for duplicate-key tracking (see `PERF.md` H1).
///
/// We keep this small so duplicate-key tracking stays in L1 and avoids hashing.
const MAX_UNIQUE_NAMES: usize = 64;

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
struct KeyId(u32);

/// Represents how the current XML element is being rendered in JSON.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
enum ElementValueKind {
    /// We haven't decided yet if this element will be rendered as a scalar,
    /// an object, or `null`. This is the case for elements without attributes.
    Pending,
    /// The element has been rendered as a scalar JSON value (`"key": 123`).
    Scalar,
    /// The element is rendered as an object (`"key": { ... }`).
    Object,
}

/// Per-element state while streaming.
#[derive(Debug)]
struct ElementState {
    /// JSON key for this element in its parent object.
    name: KeyId,
    /// How this element's JSON value is currently represented.
    kind: ElementValueKind,
    /// Whether we've already emitted a `#text` field for this element (when `kind == Object`).
    has_text: bool,
    /// Whether we've emitted `<name>_attributes` separately (when `separate_json_attributes == true`).
    /// If true and `kind == Pending` on close, we skip emitting `null` to match legacy behavior.
    has_separate_attributes: bool,
    /// Buffered scalar values (inline fast-path avoids per-node Vec alloc).
    buffered_values: BufferedValues,
}

/// JSON object context (either the root object or any nested object).
#[derive(Debug)]
struct ObjectFrame {
    /// Whether we've already written any field in this object.
    first_field: bool,
    /// Keys already used in this object (for duplicate key handling).
    used_keys: UniqueKeyTable,
}

#[derive(Debug, Copy, Clone)]
struct NameCountEntry {
    base: KeyId,
    next_suffix: u32,
}

/// Duplicate-key tracking without hashing: a small, linear-scanned table.
///
/// This matches the Zig renderer’s spirit: track up to a small number of unique names per object,
/// use pointer identity fast paths, and only allocate suffix strings on collision.
#[derive(Debug)]
struct UniqueKeyTable {
    /// All keys that have been emitted (including suffixed forms).
    used: Vec<KeyId>,
    /// Per-base counters to generate `base_1`, `base_2`, ... without rescanning from 1.
    base_counts: Vec<NameCountEntry>,
}

impl UniqueKeyTable {
    fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.max(1);
        UniqueKeyTable {
            used: Vec::with_capacity(cap),
            base_counts: Vec::with_capacity(cap.min(MAX_UNIQUE_NAMES)),
        }
    }

    #[inline]
    fn clear(&mut self) {
        self.used.clear();
        self.base_counts.clear();
    }

    #[inline]
    fn reserve(&mut self, additional: usize) {
        self.used.reserve(additional);
        self.base_counts.reserve(additional.min(MAX_UNIQUE_NAMES));
    }

    #[inline]
    fn contains(&self, key: KeyId) -> bool {
        // Manual loop tends to compile smaller/faster than iterator combinators here.
        for &k in &self.used {
            if k == key {
                return true;
            }
        }
        false
    }

    #[inline]
    fn base_entry_index(&self, base: KeyId) -> Option<usize> {
        for (i, e) in self.base_counts.iter().enumerate() {
            if e.base == base {
                return Some(i);
            }
        }
        None
    }

    fn reserve_unique(&mut self, base: KeyId, interner: &mut KeyInterner) -> KeyId {
        // Fast path: first time we see this base key in this object.
        if !self.contains(base) {
            self.used.push(base);
            self.base_counts.push(NameCountEntry {
                base,
                next_suffix: 1,
            });
            return base;
        }

        // Duplicate base key: generate the next suffix, skipping any collisions with existing keys.
        let entry_idx = self.base_entry_index(base);
        let mut suffix = entry_idx
            .map(|i| self.base_counts[i].next_suffix)
            .unwrap_or(1);

        loop {
            let candidate_str = {
                let base_str = interner.resolve(base);
                format!("{}_{}", base_str, suffix)
            };
            let candidate = interner.intern(&candidate_str);
            suffix = suffix.saturating_add(1);

            if !self.contains(candidate) {
                self.used.push(candidate);

                if let Some(i) = entry_idx {
                    self.base_counts[i].next_suffix = suffix;
                } else {
                    // Rare: base key was present but we didn't have a counter entry yet.
                    self.base_counts.push(NameCountEntry {
                        base,
                        next_suffix: suffix,
                    });
                }

                return candidate;
            }
        }
    }
}

/// Buffer of JSON scalar values with an inline "one value" fast-path.
///
/// This avoids allocating a new `Vec` (and triggering `RawVec::grow_one`) for the common
/// case where an element has exactly one text node.
#[derive(Debug, Default)]
enum BufferedValues {
    #[default]
    Empty,
    One(JsonValue),
    Many(Vec<JsonValue>),
}

impl BufferedValues {
    #[inline]
    fn is_empty(&self) -> bool {
        matches!(self, BufferedValues::Empty)
    }

    #[inline]
    fn push(&mut self, v: JsonValue) {
        match self {
            BufferedValues::Empty => {
                *self = BufferedValues::One(v);
            }
            BufferedValues::One(prev) => {
                let prev = std::mem::replace(prev, JsonValue::Null);
                *self = BufferedValues::Many(vec![prev, v]);
            }
            BufferedValues::Many(vec) => vec.push(v),
        }
    }
}

#[derive(Debug, Default)]
struct KeyInterner {
    hasher: DefaultHashBuilder,
    buckets: HashMap<u64, Vec<KeyId>>,
    strings: Vec<Box<str>>,
}

impl KeyInterner {
    #[inline]
    fn hash_str(&self, s: &str) -> u64 {
        let mut h = self.hasher.build_hasher();
        h.write(s.as_bytes());
        h.finish()
    }

    #[inline]
    fn intern(&mut self, s: &str) -> KeyId {
        let hash = self.hash_str(s);
        if let Some(ids) = self.buckets.get(&hash) {
            for &id in ids {
                if self.resolve(id) == s {
                    return id;
                }
            }
        }

        let id = KeyId(self.strings.len() as u32);
        self.strings.push(s.into());
        self.buckets.entry(hash).or_default().push(id);
        id
    }

    #[inline]
    fn resolve(&self, id: KeyId) -> &str {
        &self.strings[id.0 as usize]
    }
}

pub struct JsonStreamOutput<W: Write> {
    writer: Option<W>,
    /// Whether pretty-printing was requested. Currently unused – streaming
    /// output is always compact, and callers compare via `serde_json::Value`.
    #[allow(dead_code)]
    indent: bool,
    separate_json_attributes: bool,

    /// Stack of JSON object frames. The root object is at index 0.
    frames: Vec<ObjectFrame>,
    /// Stack of currently open XML elements.
    elements: Vec<ElementState>,
    /// Recycled object frames to reuse per-object key tracking allocations across records.
    recycled_frames: Vec<ObjectFrame>,

    /// Interned key strings to avoid per-record/per-key heap churn.
    key_interner: KeyInterner,

    /// Optional depth (in `elements`) of an `EventData` element that owns a
    /// synthetic `"Data": { "#text": [...] }` aggregator, used to model
    /// `<EventData><Data>...</Data>...</EventData>` without building an
    /// intermediate tree.
    data_owner_depth: Option<usize>,
    /// Collected values for the aggregated `"Data": { "#text": [...] }` array.
    data_values: BufferedValues,
    /// Whether we are currently inside a `<Data>` element that contributes to
    /// the aggregated `"Data"` array.
    data_inside_element: bool,
}

impl<W: Write> JsonStreamOutput<W> {
    pub fn with_writer(writer: W, settings: &ParserSettings) -> Self {
        JsonStreamOutput {
            writer: Some(writer),
            indent: settings.should_indent(),
            separate_json_attributes: settings.should_separate_json_attributes(),
            frames: Vec::new(),
            elements: Vec::new(),
            recycled_frames: Vec::new(),
            key_interner: KeyInterner::default(),
            data_owner_depth: None,
            data_values: BufferedValues::default(),
            data_inside_element: false,
        }
    }

    /// Finalize the JSON stream and return the underlying writer.
    pub fn finish(mut self) -> SerializationResult<W> {
        // If the caller didn't drive the parser fully, we may still have an
        // open root object; try to close it gracefully.
        if !self.frames.is_empty() {
            // Close any remaining open element objects.
            while let Some(elem) = self.elements.pop() {
                if elem.kind == ElementValueKind::Object {
                    self.end_object()?;
                }
            }

            // Close the root object.
            self.write_bytes(b"}")?;
            self.frames.clear();
        }

        self.writer
            .take()
            .ok_or_else(|| SerializationError::JsonStructureError {
                message: "Writer already taken".to_string(),
            })
    }

    pub fn into_writer(self) -> W {
        self.finish()
            .expect("failed to finalize JSON output in JsonStreamOutput")
    }

    fn writer_mut(&mut self) -> &mut W {
        self.writer
            .as_mut()
            .expect("JsonStreamOutput writer missing")
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> SerializationResult<()> {
        self.writer_mut()
            .write_all(bytes)
            .map_err(SerializationError::from)
    }

    /// Write a JSON string directly without escaping.
    /// Only safe for NCName strings (XML element/attribute names) which don't contain
    /// characters that need JSON escaping (no quotes, backslashes, control chars).
    #[inline]
    fn write_json_string_ncname(&mut self, s: &str) -> SerializationResult<()> {
        self.write_bytes(b"\"")?;
        self.write_bytes(s.as_bytes())?;
        self.write_bytes(b"\"")
    }

    /// Write a JSON string with proper escaping for special characters.
    /// Uses a fast path for strings that don't need escaping.
    fn write_json_string_escaped(&mut self, s: &str) -> SerializationResult<()> {
        // Fast path: check if escaping is needed
        let needs_escape = s
            .bytes()
            .any(|b| matches!(b, b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0..=0x1F));

        if !needs_escape {
            return self.write_json_string_ncname(s);
        }

        // Slow path: escape special characters
        self.write_bytes(b"\"")?;
        for c in s.chars() {
            match c {
                '"' => self.write_bytes(b"\\\"")?,
                '\\' => self.write_bytes(b"\\\\")?,
                '\n' => self.write_bytes(b"\\n")?,
                '\r' => self.write_bytes(b"\\r")?,
                '\t' => self.write_bytes(b"\\t")?,
                c if c.is_control() => {
                    write!(self.writer_mut(), "\\u{:04x}", c as u32)
                        .map_err(SerializationError::from)?;
                }
                c => {
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    self.write_bytes(encoded.as_bytes())?;
                }
            }
        }
        self.write_bytes(b"\"")
    }

    /// Write a BinXmlValue directly to JSON output without creating intermediate JsonValue.
    /// This is the zero-allocation path for value serialization.
    fn write_binxml_value(&mut self, value: &BinXmlValue) -> SerializationResult<()> {
        match value {
            BinXmlValue::NullType => self.write_bytes(b"null"),
            BinXmlValue::StringType(s) => self.write_json_string_escaped(s.as_str()),
            BinXmlValue::AnsiStringType(s) => self.write_json_string_escaped(s.as_str()),
            BinXmlValue::Int8Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::UInt8Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::Int16Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::UInt16Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::Int32Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::UInt32Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::Int64Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::UInt64Type(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::Real32Type(n) => {
                let mut buf = ryu::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::Real64Type(n) => {
                let mut buf = ryu::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::BoolType(b) => self.write_bytes(if *b { b"true" } else { b"false" }),
            BinXmlValue::BinaryType(bytes) => {
                self.write_bytes(b"\"")?;
                for byte in *bytes {
                    write!(self.writer_mut(), "{:02X}", byte).map_err(SerializationError::from)?;
                }
                self.write_bytes(b"\"")
            }
            BinXmlValue::GuidType(guid) => {
                // Use Guid's Display impl, write as JSON string
                write!(self.writer_mut(), "\"{}\"", guid).map_err(SerializationError::from)
            }
            BinXmlValue::SizeTType(n) => {
                let mut buf = itoa::Buffer::new();
                self.write_bytes(buf.format(*n).as_bytes())
            }
            BinXmlValue::FileTimeType(dt) | BinXmlValue::SysTimeType(dt) => {
                // Fast ISO-8601 with microseconds (avoids strftime parser overhead):
                // YYYY-MM-DDTHH:MM:SS.ffffffZ
                write!(
                    self.writer_mut(),
                    "\"{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:06}Z\"",
                    dt.year(),
                    dt.month(),
                    dt.day(),
                    dt.hour(),
                    dt.minute(),
                    dt.second(),
                    dt.timestamp_subsec_micros()
                )
                .map_err(SerializationError::from)
            }
            BinXmlValue::SidType(sid) => {
                self.write_bytes(b"\"")?;
                write!(self.writer_mut(), "{}", sid).map_err(SerializationError::from)?;
                self.write_bytes(b"\"")
            }
            BinXmlValue::HexInt32Type(s) | BinXmlValue::HexInt64Type(s) => {
                self.write_json_string_escaped(s.as_str())
            }
            BinXmlValue::EvtHandle | BinXmlValue::EvtXml => self.write_bytes(b"null"),
            // Arrays
            BinXmlValue::StringArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, s) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    self.write_json_string_escaped(s.as_str())?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::Int8ArrayType(arr) => self.write_int_array(arr.iter().map(|n| *n as i64)),
            BinXmlValue::UInt8ArrayType(arr) => {
                self.write_uint_array(arr.iter().map(|n| *n as u64))
            }
            BinXmlValue::Int16ArrayType(arr) => self.write_int_array(arr.iter().map(|n| *n as i64)),
            BinXmlValue::UInt16ArrayType(arr) => {
                self.write_uint_array(arr.iter().map(|n| *n as u64))
            }
            BinXmlValue::Int32ArrayType(arr) => self.write_int_array(arr.iter().map(|n| *n as i64)),
            BinXmlValue::UInt32ArrayType(arr) => {
                self.write_uint_array(arr.iter().map(|n| *n as u64))
            }
            BinXmlValue::Int64ArrayType(arr) => self.write_int_array(arr.iter().copied()),
            BinXmlValue::UInt64ArrayType(arr) => self.write_uint_array(arr.iter().copied()),
            BinXmlValue::Real32ArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, n) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    let mut buf = ryu::Buffer::new();
                    self.write_bytes(buf.format(*n).as_bytes())?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::Real64ArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, n) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    let mut buf = ryu::Buffer::new();
                    self.write_bytes(buf.format(*n).as_bytes())?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::BoolArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, b) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    self.write_bytes(if *b { b"true" } else { b"false" })?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::GuidArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, guid) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    write!(self.writer_mut(), "\"{}\"", guid).map_err(SerializationError::from)?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::FileTimeArrayType(arr) | BinXmlValue::SysTimeArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, dt) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    write!(
                        self.writer_mut(),
                        "\"{}\"",
                        dt.format("%Y-%m-%dT%H:%M:%S%.6fZ")
                    )
                    .map_err(SerializationError::from)?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::SidArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, sid) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    self.write_bytes(b"\"")?;
                    write!(self.writer_mut(), "{}", sid).map_err(SerializationError::from)?;
                    self.write_bytes(b"\"")?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::HexInt32ArrayType(arr) | BinXmlValue::HexInt64ArrayType(arr) => {
                self.write_bytes(b"[")?;
                for (i, s) in arr.iter().enumerate() {
                    if i > 0 {
                        self.write_bytes(b",")?;
                    }
                    self.write_json_string_escaped(s.as_str())?;
                }
                self.write_bytes(b"]")
            }
            BinXmlValue::AnsiStringArrayType
            | BinXmlValue::BinaryArrayType
            | BinXmlValue::SizeTArrayType
            | BinXmlValue::EvtArrayHandle
            | BinXmlValue::BinXmlArrayType
            | BinXmlValue::EvtXmlArrayType => self.write_bytes(b"null"),
            BinXmlValue::BinXmlType(_) => self.write_bytes(b"null"),
        }
    }

    /// Helper for writing integer arrays
    fn write_int_array(&mut self, iter: impl Iterator<Item = i64>) -> SerializationResult<()> {
        self.write_bytes(b"[")?;
        let mut buf = itoa::Buffer::new();
        let mut first = true;
        for n in iter {
            if !first {
                self.write_bytes(b",")?;
            }
            first = false;
            self.write_bytes(buf.format(n).as_bytes())?;
        }
        self.write_bytes(b"]")
    }

    /// Helper for writing unsigned integer arrays
    fn write_uint_array(&mut self, iter: impl Iterator<Item = u64>) -> SerializationResult<()> {
        self.write_bytes(b"[")?;
        let mut buf = itoa::Buffer::new();
        let mut first = true;
        for n in iter {
            if !first {
                self.write_bytes(b",")?;
            }
            first = false;
            self.write_bytes(buf.format(n).as_bytes())?;
        }
        self.write_bytes(b"]")
    }

    fn current_frame_mut(&mut self) -> &mut ObjectFrame {
        self.frames
            .last_mut()
            .expect("no current JSON object frame available")
    }

    #[inline]
    fn push_object_frame(&mut self, used_keys_capacity: usize) {
        let mut frame = self.recycled_frames.pop().unwrap_or_else(|| ObjectFrame {
            first_field: true,
            used_keys: UniqueKeyTable::with_capacity(used_keys_capacity),
        });

        frame.first_field = true;
        frame.used_keys.clear();
        frame.used_keys.reserve(used_keys_capacity);
        self.frames.push(frame);
    }

    #[inline]
    fn pop_object_frame(&mut self) {
        let mut frame = self
            .frames
            .pop()
            .expect("attempted to pop JSON frame when none exist");
        frame.first_field = true;
        frame.used_keys.clear();
        self.recycled_frames.push(frame);
    }

    /// Write a comma if needed for the current JSON object.
    fn write_comma_if_needed(&mut self) -> SerializationResult<()> {
        let frame = self.current_frame_mut();
        if frame.first_field {
            frame.first_field = false;
            Ok(())
        } else {
            self.write_bytes(b",")
        }
    }

    /// Reserve a unique key in the current frame from an already-interned key.
    /// This avoids hashing the same key again on hot paths.
    #[inline]
    fn reserve_unique_key(&mut self, key: KeyId) -> KeyId {
        let frame = self
            .frames
            .last_mut()
            .expect("no current JSON object frame");
        frame.used_keys.reserve_unique(key, &mut self.key_interner)
    }

    /// Write a JSON string key (with surrounding quotes and escaping).
    /// Write a JSON string key, handling duplicates by appending `_1`, `_2`, etc.
    fn write_key(&mut self, key: &str) -> SerializationResult<()> {
        let key = self.key_interner.intern(key);
        self.write_key_id(key)
    }

    #[inline]
    fn write_key_id(&mut self, key: KeyId) -> SerializationResult<()> {
        self.write_comma_if_needed()?;

        let unique_key = {
            let frame = self
                .frames
                .last_mut()
                .expect("no current JSON object frame");
            frame.used_keys.reserve_unique(key, &mut self.key_interner)
        };

        // Keys derived from XML NCName don't need escaping.
        let key_str = self.key_interner.resolve(unique_key);
        let writer = self
            .writer
            .as_mut()
            .expect("JsonStreamOutput writer missing");
        writer.write_all(b"\"").map_err(SerializationError::from)?;
        writer
            .write_all(key_str.as_bytes())
            .map_err(SerializationError::from)?;
        writer.write_all(b"\":").map_err(SerializationError::from)
    }

    #[inline]
    fn write_reserved_key_id(&mut self, key: KeyId) -> SerializationResult<()> {
        self.write_comma_if_needed()?;

        // Keys derived from XML NCName don't need escaping.
        let key_str = self.key_interner.resolve(key);
        let writer = self
            .writer
            .as_mut()
            .expect("JsonStreamOutput writer missing");
        writer.write_all(b"\"").map_err(SerializationError::from)?;
        writer
            .write_all(key_str.as_bytes())
            .map_err(SerializationError::from)?;
        writer.write_all(b"\":").map_err(SerializationError::from)
    }

    /// Write a pre-reserved key directly (no duplicate checking needed).
    fn write_reserved_key(&mut self, key: &str) -> SerializationResult<()> {
        self.write_comma_if_needed()?;
        // Keys derived from XML NCName don't need escaping
        self.write_json_string_ncname(key)?;
        self.write_bytes(b":")
    }

    /// Start a new nested JSON object as the value of `key` in the current object.
    fn start_object_value(&mut self, key: &str) -> SerializationResult<()> {
        let key = self.key_interner.intern(key);
        self.start_object_value_id(key)
    }

    #[inline]
    fn start_object_value_id(&mut self, key: KeyId) -> SerializationResult<()> {
        self.write_key_id(key)?;
        self.write_bytes(b"{")?;
        // Heuristic: nested objects tend to have a moderate number of keys.
        self.push_object_frame(32);
        Ok(())
    }

    /// End the current JSON object frame.
    fn end_object(&mut self) -> SerializationResult<()> {
        self.write_bytes(b"}")?;
        self.pop_object_frame();
        Ok(())
    }

    /// For elements without attributes, if their first child is another element
    /// we need to materialize this element as an object (`"name": { ... }`).
    fn ensure_parent_is_object(&mut self) -> SerializationResult<()> {
        let Some(parent_index) = self.elements.len().checked_sub(1) else {
            return Ok(());
        };

        let parent_kind = self.elements[parent_index].kind;

        match parent_kind {
            ElementValueKind::Pending => {
                // Turn `"parent": null` into `"parent": { ... }` by starting an
                // object value for it now.
                let key = self.elements[parent_index].name;
                let was_reserved = self.elements[parent_index].has_separate_attributes;

                // If the key was pre-reserved (separate_json_attributes mode), use
                // write_reserved_key to avoid double-reservation.
                if was_reserved {
                    self.write_reserved_key_id(key)?;
                } else {
                    self.write_key_id(key)?;
                }
                self.write_bytes(b"{")?;
                self.push_object_frame(32);

                self.elements[parent_index].kind = ElementValueKind::Object;
            }
            ElementValueKind::Scalar => {
                // Element had text content but now has child elements too.
                // Turn it into an object and move buffered text to #text field.
                let key = self.elements[parent_index].name;
                let was_reserved = self.elements[parent_index].has_separate_attributes;
                let buffered = std::mem::take(&mut self.elements[parent_index].buffered_values);

                if was_reserved {
                    self.write_reserved_key_id(key)?;
                } else {
                    self.write_key_id(key)?;
                }
                self.write_bytes(b"{")?;
                self.push_object_frame(32);

                // Write the buffered text as #text if not in separate mode
                // (in separate mode, text in mixed-content elements is dropped).
                if !buffered.is_empty() && !self.separate_json_attributes {
                    self.write_key("#text")?;
                    match buffered {
                        BufferedValues::Empty => {}
                        BufferedValues::One(v) => {
                            serde_json::to_writer(self.writer_mut(), &v)
                                .map_err(SerializationError::from)?;
                        }
                        BufferedValues::Many(vs) => {
                            serde_json::to_writer(self.writer_mut(), &vs)
                                .map_err(SerializationError::from)?;
                        }
                    }
                }

                self.elements[parent_index].kind = ElementValueKind::Object;
            }
            ElementValueKind::Object => {
                // Already an object, nothing to do.
            }
        }

        Ok(())
    }

    /// Append a value into the aggregated `"Data": { "#text": [...] }` under an
    /// `EventData` element. The BinXml value may itself be an array (e.g.
    /// `StringArrayType`), in which case it is stored as-is, matching the
    /// behaviour of `JsonOutput::value_to_json`.
    fn write_data_aggregated_value(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        let json_value: JsonValue = match &value {
            Cow::Borrowed(v) => JsonValue::from(*v),
            Cow::Owned(v) => JsonValue::from(v),
        };

        self.data_values.push(json_value);
        Ok(())
    }

    /// Finalize the aggregated `"Data"` value, if any.
    /// With `separate_json_attributes == false`: outputs `"Data": { "#text": ... }`
    /// With `separate_json_attributes == true`: outputs `"Data": ...` directly
    fn finalize_data_aggregator(&mut self) -> SerializationResult<()> {
        if self.data_owner_depth.is_some() && !self.data_values.is_empty() {
            // Avoid aliasing `self` while iterating by taking the values out.
            let values = std::mem::take(&mut self.data_values);

            if self.separate_json_attributes {
                // In separate_json_attributes mode, output directly without wrapper.
                // Legacy concatenates multiple string values into one.
                self.write_key("Data")?;
                match values {
                    BufferedValues::Empty => {
                        // Nothing to write (shouldn't happen given the outer check).
                        self.write_bytes(b"null")?;
                    }
                    BufferedValues::One(v) => {
                        serde_json::to_writer(self.writer_mut(), &v)
                            .map_err(SerializationError::from)?;
                    }
                    BufferedValues::Many(vs) => {
                        // Concatenate multiple values as strings (legacy behavior).
                        let mut concat = String::new();
                        for v in &vs {
                            match v {
                                JsonValue::String(s) => concat.push_str(s),
                                JsonValue::Number(n) => concat.push_str(&n.to_string()),
                                JsonValue::Bool(b) => {
                                    concat.push_str(if *b { "true" } else { "false" })
                                }
                                JsonValue::Null => {}
                                _ => concat.push_str(&v.to_string()),
                            }
                        }

                        // Avoid `serde_json` for emitting the final JSON string.
                        self.write_json_string_escaped(&concat)?;
                    }
                }
            } else {
                // With `#attributes` mode, wrap in `"Data": { "#text": ... }`.
                self.start_object_value("Data")?;
                self.write_key("#text")?;

                match values {
                    BufferedValues::Empty => {
                        self.write_bytes(b"null")?;
                    }
                    BufferedValues::One(v) => {
                        serde_json::to_writer(self.writer_mut(), &v)
                            .map_err(SerializationError::from)?;
                    }
                    BufferedValues::Many(vs) => {
                        // Multiple `<Data>` children: aggregate into an array.
                        self.write_bytes(b"[")?;
                        for (idx, json_value) in vs.iter().enumerate() {
                            if idx > 0 {
                                self.write_bytes(b",")?;
                            }
                            serde_json::to_writer(self.writer_mut(), json_value)
                                .map_err(SerializationError::from)?;
                        }
                        self.write_bytes(b"]")?;
                    }
                }

                self.end_object()?;
            }
        }

        // Reset aggregator state.
        self.data_owner_depth = None;
        self.data_inside_element = false;
        Ok(())
    }

    /// Helper to handle entity reference strings without needing arena for BinXmlValue
    fn handle_entity_string(&mut self, s: &str) -> SerializationResult<()> {
        // Aggregated `<EventData><Data>...</Data>...</EventData>` case.
        if let Some(owner_depth) = self.data_owner_depth {
            let current_depth = self.elements.len();
            if self.data_inside_element && current_depth == owner_depth {
                self.write_json_string_escaped(s)?;
                return Ok(());
            }
        }

        let Some(index) = self.elements.len().checked_sub(1) else {
            return Ok(());
        };

        let kind = self.elements[index].kind;
        let json_value = JsonValue::String(s.to_string());

        match kind {
            ElementValueKind::Pending => {
                self.elements[index].buffered_values.push(json_value);
                self.elements[index].kind = ElementValueKind::Scalar;
            }
            ElementValueKind::Scalar => {
                self.elements[index].buffered_values.push(json_value);
            }
            ElementValueKind::Object => {
                // Match legacy `JsonOutput`: once an element has been materialized as an object
                // (attributes and/or child elements), entity references are ignored.
                let _ = json_value;
                return Ok(());
            }
        }

        Ok(())
    }
}

impl JsonStreamOutput<Vec<u8>> {
    /// Get the currently written JSON bytes.
    #[inline]
    pub fn buffer(&self) -> &[u8] {
        self.writer.as_deref().unwrap_or(&[])
    }

    /// Clear the underlying buffer while retaining its capacity.
    #[inline]
    pub fn clear_buffer(&mut self) {
        if let Some(buf) = self.writer.as_mut() {
            buf.clear();
        }
    }

    /// Reserve additional capacity in the underlying buffer.
    #[inline]
    pub fn reserve_buffer(&mut self, additional: usize) {
        if let Some(buf) = self.writer.as_mut() {
            buf.reserve(additional);
        }
    }
}

impl<W: Write> BinXmlOutput for JsonStreamOutput<W> {
    fn visit_start_of_stream(&mut self) -> SerializationResult<()> {
        // Be defensive: if a previous record failed mid-stream, try to reset state
        // so we can continue emitting subsequent records.
        while !self.elements.is_empty() {
            let elem = self.elements.pop().expect("checked non-empty");
            if elem.kind == ElementValueKind::Object {
                // Close any dangling object frames.
                if !self.frames.is_empty() {
                    self.pop_object_frame();
                }
            }
        }
        while !self.frames.is_empty() {
            self.pop_object_frame();
        }

        // Open the root JSON object.
        self.write_bytes(b"{")?;
        // Root objects can have many keys; pre-reserve to reduce rehashing.
        self.push_object_frame(128);
        Ok(())
    }

    fn visit_end_of_stream(&mut self) -> SerializationResult<()> {
        // Close any remaining elements that own JSON object frames.
        while let Some(elem) = self.elements.pop() {
            if elem.kind == ElementValueKind::Object {
                self.end_object()?;
            }
        }

        // Close the root JSON object.
        if !self.frames.is_empty() {
            self.write_bytes(b"}")?;
            while !self.frames.is_empty() {
                self.pop_object_frame();
            }
        }

        Ok(())
    }

    fn visit_open_start_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        // If we're nested under an element without attributes, and this is the
        // first child element, we must represent the parent as an object.
        self.ensure_parent_is_object()?;

        // Determine JSON key for this element.
        let element_name = element.name.as_str();

        // Special handling for `<Data>` nodes: they use their "Name" attribute
        // as the JSON key when present, and ignore attributes entirely.
        let is_data = element_name == "Data";
        let data_name_attr = if is_data {
            element
                .attributes
                .iter()
                .find(|a| a.name.as_ref().as_str() == "Name")
        } else {
            None
        };

        let key = if let Some(name_attr) = data_name_attr {
            let value: Cow<'_, str> = name_attr.value.as_cow_str();
            self.key_interner.intern(value.as_ref())
        } else {
            self.key_interner.intern(element_name)
        };

        // Aggregated `<EventData><Data>...</Data>...</EventData>` case:
        // multiple `<Data>` children without a `Name` attribute become a single
        // `"Data": { "#text": [ ... ] }` object under their `EventData` parent.
        if is_data
            && data_name_attr.is_none()
            && let Some(parent) = self.elements.last()
            && self.key_interner.resolve(parent.name) == "EventData"
        {
            // Depth of the owning `EventData` element.
            let owner_depth = self.elements.len();

            // Initialize a new aggregator for this `EventData`, if needed.
            if self.data_owner_depth != Some(owner_depth) {
                self.data_owner_depth = Some(owner_depth);
                self.data_values = BufferedValues::default();
            }

            // We're now inside a `<Data>` element that contributes to
            // the aggregated array.
            self.data_inside_element = true;

            // Do NOT push a new `ElementState` for this `<Data>` node;
            // its values are handled by the aggregator.
            return Ok(());
        }

        // In the JSON representation, `<Data Name="...">` behaves like a
        // regular node without attributes. Attributes whose JSON value is
        // `null` are ignored (this matches `JsonOutput`).
        let mut has_json_attributes = false;
        if !is_data {
            for attr in &element.attributes {
                if !matches!(attr.value.as_ref(), BinXmlValue::NullType) {
                    has_json_attributes = true;
                    break;
                }
            }
        }

        // Elements with attributes and `separate_json_attributes == false` are
        // materialized as objects with a `#attributes` field.
        if has_json_attributes && !self.separate_json_attributes {
            // `"key": { "#attributes": { ... } }`
            self.start_object_value_id(key)?;

            // Write `#attributes` object.
            {
                // Update first-field state for the element object.
                let first_field = {
                    let frame = self.current_frame_mut();
                    let first = frame.first_field;
                    if first {
                        frame.first_field = false;
                    }
                    first
                };
                if !first_field {
                    self.write_bytes(b",")?;
                }
                // "#attributes" is a fixed ASCII key, no escaping needed
                self.write_bytes(b"\"#attributes\":")?;

                // Start attributes object.
                self.write_bytes(b"{")?;
                self.push_object_frame(0);

                {
                    for attr in &element.attributes {
                        let attr_key = attr.name.as_str();
                        // Skip the `Name` attribute on `<Data>`; it is only
                        // used as the field name, not as an attribute.
                        if is_data && attr_key == "Name" {
                            continue;
                        }

                        if matches!(attr.value.as_ref(), BinXmlValue::NullType) {
                            continue;
                        }

                        let is_first = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !is_first {
                            self.write_bytes(b",")?;
                        }
                        // Attribute keys are XML NCName, no escaping needed
                        self.write_json_string_ncname(attr_key)?;
                        self.write_bytes(b":")?;
                        self.write_binxml_value(attr.value.as_ref())?;
                    }
                }

                // Close `#attributes` object.
                self.end_object()?;
            }

            self.elements.push(ElementState {
                name: key,
                kind: ElementValueKind::Object,
                has_text: false,
                has_separate_attributes: false,
                buffered_values: BufferedValues::default(),
            });
        } else {
            // `separate_json_attributes == true` or element has no attributes.
            let wrote_separate_attrs = has_json_attributes && self.separate_json_attributes;

            // If we're writing `_attributes`, pre-reserve the element key so both
            // the `_attributes` and the element itself use matching suffixes.
            let element_key = if wrote_separate_attrs {
                let unique_key = self.reserve_unique_key(key);

                // Emit `"<unique_key>_attributes": { ... }` into the parent object.
                let attr_key = {
                    let s = self.key_interner.resolve(unique_key);
                    format!("{}_attributes", s)
                };
                self.write_reserved_key(&attr_key)?;
                self.write_bytes(b"{")?;
                self.push_object_frame(0);

                {
                    for attr in &element.attributes {
                        let attr_name = attr.name.as_str();
                        if matches!(attr.value.as_ref(), BinXmlValue::NullType) {
                            continue;
                        }

                        let is_first = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !is_first {
                            self.write_bytes(b",")?;
                        }
                        // Attribute names are XML NCName, no escaping needed
                        self.write_json_string_ncname(attr_name)?;
                        self.write_bytes(b":")?;
                        self.write_binxml_value(attr.value.as_ref())?;
                    }
                }

                self.end_object()?;
                unique_key
            } else {
                // No attributes to write - use original key (will be deduped on write).
                key
            };

            // We delay emitting the actual `"key": ...` until we see either
            // a character node or a child element, so we can decide whether
            // this element is a scalar, an object, or `null`.
            self.elements.push(ElementState {
                name: element_key,
                kind: ElementValueKind::Pending,
                has_text: false,
                has_separate_attributes: wrote_separate_attrs,
                buffered_values: BufferedValues::default(),
            });
        }

        Ok(())
    }

    fn visit_close_element(&mut self, element: &XmlElement) -> SerializationResult<()> {
        let element_name = element.name.as_str();

        // Closing an aggregated `<Data>` node: we only need to mark that we
        // are no longer inside a contributing `<Data>`; the owning `EventData`
        // element remains on the stack.
        if element_name == "Data" && self.data_owner_depth.is_some() && self.data_inside_element {
            self.data_inside_element = false;
            return Ok(());
        }

        let current_depth = self.elements.len();
        let is_data_owner = self.data_owner_depth == Some(current_depth);

        if let Some(elem) = self.elements.pop() {
            if is_data_owner {
                // Finalize the aggregated `"Data": { "#text": [...] }` object.
                self.finalize_data_aggregator()?;
            }

            match elem.kind {
                ElementValueKind::Pending => {
                    // No text and no children – render as `null`, unless we already
                    // emitted `<name>_attributes` separately (legacy omits the null).
                    if !elem.has_separate_attributes {
                        self.write_key_id(elem.name)?;
                        self.write_bytes(b"null")?;
                    }
                }
                ElementValueKind::Scalar => {
                    // Write the buffered scalar value(s) now.
                    if !elem.buffered_values.is_empty() {
                        // If key was pre-reserved (separate_json_attributes mode), use reserved writer.
                        if elem.has_separate_attributes {
                            self.write_reserved_key_id(elem.name)?;
                        } else {
                            self.write_key_id(elem.name)?;
                        }
                        match elem.buffered_values {
                            BufferedValues::Empty => {}
                            BufferedValues::One(v) => {
                                // Single value: preserve original type.
                                serde_json::to_writer(self.writer_mut(), &v)
                                    .map_err(SerializationError::from)?;
                            }
                            BufferedValues::Many(vs) => {
                                // Multiple values: concatenate as strings (legacy behavior).
                                let mut concat = String::new();
                                for v in &vs {
                                    // Convert JSON value back to string for concatenation
                                    match v {
                                        JsonValue::String(s) => concat.push_str(s),
                                        JsonValue::Number(n) => concat.push_str(&n.to_string()),
                                        JsonValue::Bool(b) => {
                                            concat.push_str(if *b { "true" } else { "false" })
                                        }
                                        JsonValue::Null => concat.push_str("null"),
                                        _ => concat.push_str(&v.to_string()),
                                    }
                                }

                                // Avoid `serde_json` for emitting the final JSON string.
                                self.write_json_string_escaped(&concat)?;
                            }
                        }
                    }
                }
                ElementValueKind::Object => {
                    // Write buffered #text values if any, then close the object.
                    // In separate_json_attributes mode, elements with child elements
                    // drop text content (legacy behavior - no #text field).
                    if !elem.buffered_values.is_empty() && !self.separate_json_attributes {
                        let is_first = {
                            let frame = self.current_frame_mut();
                            let first = frame.first_field;
                            if first {
                                frame.first_field = false;
                            }
                            first
                        };
                        if !is_first {
                            self.write_bytes(b",")?;
                        }
                        // "#text" is a fixed ASCII key, no escaping needed
                        self.write_bytes(b"\"#text\":")?;

                        match elem.buffered_values {
                            BufferedValues::Empty => {}
                            BufferedValues::One(v) => {
                                // Single value: write directly.
                                serde_json::to_writer(self.writer_mut(), &v)
                                    .map_err(SerializationError::from)?;
                            }
                            BufferedValues::Many(vs) => {
                                // Multiple values: write as array (legacy behavior).
                                serde_json::to_writer(self.writer_mut(), &vs)
                                    .map_err(SerializationError::from)?;
                            }
                        }
                    }
                    // Close the element's object.
                    self.end_object()?;
                }
            }
        }
        Ok(())
    }

    fn visit_characters(&mut self, value: Cow<BinXmlValue>) -> SerializationResult<()> {
        // Aggregated `<EventData><Data>...</Data>...</EventData>` case.
        if let Some(owner_depth) = self.data_owner_depth {
            let current_depth = self.elements.len();
            if self.data_inside_element && current_depth == owner_depth {
                self.write_data_aggregated_value(value)?;
                return Ok(());
            }
        }

        // Characters belong to the innermost open XML element.
        let Some(index) = self.elements.len().checked_sub(1) else {
            return Ok(());
        };

        let kind = self.elements[index].kind;

        match kind {
            ElementValueKind::Pending => {
                // First content for this element and it has no attributes:
                // buffer the value (we'll write on close to support concatenation).
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
                self.elements[index].kind = ElementValueKind::Scalar;
            }
            ElementValueKind::Scalar => {
                // Multiple character nodes: add to the buffer.
                // On close, we'll concatenate string representations to match legacy.
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
            }
            ElementValueKind::Object => {
                // Elements with attributes: we store text under a `#text` key.
                // In separate_json_attributes mode, skip null #text values.
                if self.elements[index].has_separate_attributes {
                    let is_null = matches!(
                        &value,
                        Cow::Borrowed(BinXmlValue::NullType) | Cow::Owned(BinXmlValue::NullType)
                    );
                    if is_null {
                        return Ok(());
                    }
                }

                // Buffer text values to support multiple text nodes (legacy creates an array).
                let json_value: JsonValue = match value {
                    Cow::Borrowed(v) => JsonValue::from(v),
                    Cow::Owned(v) => JsonValue::from(&v),
                };
                self.elements[index].buffered_values.push(json_value);
                self.elements[index].has_text = true;
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
        // Match JsonOutput behaviour: use quick-xml's unescape to resolve the entity.
        let entity_ref = "&".to_string() + entity.as_str() + ";";
        let xml_event = BytesText::from_escaped(&entity_ref);
        match xml_event.unescape() {
            Ok(escaped) => {
                // Directly handle string without creating BinXmlValue (which would need arena)
                self.handle_entity_string(escaped.as_ref())
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

#[cfg(test)]
mod tests {
    use super::JsonStreamOutput;
    use crate::binxml::name::BinXmlName;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::model::xml::{XmlAttribute, XmlElement};
    use crate::{BinXmlOutput, JsonOutput, ParserSettings};
    use bumpalo::Bump;
    use bumpalo::collections::String as BumpString;
    use pretty_assertions::assert_eq;
    use quick_xml::Reader;
    use quick_xml::events::{BytesStart, Event};
    use std::borrow::Cow;

    fn bytes_to_string(bytes: &[u8]) -> String {
        String::from_utf8(bytes.to_vec()).expect("UTF8 Input")
    }

    fn event_to_element<'a>(event: BytesStart, arena: &'a Bump) -> XmlElement<'a> {
        let mut attrs = vec![];

        for attr in event.attributes() {
            let attr = attr.expect("Failed to read attribute.");
            attrs.push(XmlAttribute {
                name: Cow::Owned(BinXmlName::from_string(bytes_to_string(attr.key.as_ref()))),
                // We have to compromise here and assume all values are strings.
                value: Cow::Owned(BinXmlValue::StringType(BumpString::from_str_in(
                    &bytes_to_string(&attr.value),
                    arena,
                ))),
            });
        }

        XmlElement {
            name: Cow::Owned(BinXmlName::from_string(bytes_to_string(
                event.name().as_ref(),
            ))),
            attributes: attrs,
        }
    }

    /// Converts an XML string to JSON using the legacy `JsonOutput`.
    fn xml_to_json_legacy(xml: &str, settings: &ParserSettings) -> String {
        let arena = Bump::new();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut output = JsonOutput::new(settings);
        output.visit_start_of_stream().expect("Start of stream");

        let mut element_stack: Vec<XmlElement> = Vec::new();

        loop {
            match reader.read_event() {
                Ok(event) => match event {
                    Event::Start(start) => {
                        let elem = event_to_element(start, &arena);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Open start element");
                        element_stack.push(elem);
                    }
                    Event::End(_) => {
                        let elem = element_stack.pop().expect("Unbalanced XML (End)");
                        output.visit_close_element(&elem).expect("Close element");
                    }
                    Event::Empty(empty) => {
                        let elem = event_to_element(empty, &arena);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Empty Open start element");
                        output.visit_close_element(&elem).expect("Empty Close");
                    }
                    Event::Text(text) => output
                        .visit_characters(Cow::Owned(BinXmlValue::StringType(
                            BumpString::from_str_in(&bytes_to_string(text.as_ref()), &arena),
                        )))
                        .expect("Text element"),
                    Event::Comment(_) => {}
                    Event::CData(_) => unimplemented!(),
                    Event::Decl(_) => {}
                    Event::PI(_) => unimplemented!(),
                    Event::DocType(_) => {}
                    Event::Eof => {
                        output.visit_end_of_stream().expect("End of stream");
                        break;
                    }
                },
                Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            }
        }

        serde_json::to_string_pretty(&output.into_value().expect("Output")).expect("To serialize")
    }

    /// Converts an XML string to JSON using the streaming `JsonStreamOutput`.
    fn xml_to_json_streaming(xml: &str, settings: &ParserSettings) -> String {
        let arena = Bump::new();
        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let writer = Vec::new();
        let mut output = JsonStreamOutput::with_writer(writer, settings);
        output.visit_start_of_stream().expect("Start of stream");

        let mut element_stack: Vec<XmlElement> = Vec::new();

        loop {
            match reader.read_event() {
                Ok(event) => match event {
                    Event::Start(start) => {
                        let elem = event_to_element(start, &arena);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Open start element");
                        element_stack.push(elem);
                    }
                    Event::End(_) => {
                        let elem = element_stack.pop().expect("Unbalanced XML (End)");
                        output.visit_close_element(&elem).expect("Close element");
                    }
                    Event::Empty(empty) => {
                        let elem = event_to_element(empty, &arena);
                        output
                            .visit_open_start_element(&elem)
                            .expect("Empty Open start element");
                        output.visit_close_element(&elem).expect("Empty Close");
                    }
                    Event::Text(text) => output
                        .visit_characters(Cow::Owned(BinXmlValue::StringType(
                            BumpString::from_str_in(&bytes_to_string(text.as_ref()), &arena),
                        )))
                        .expect("Text element"),
                    Event::Comment(_) => {}
                    Event::CData(_) => unimplemented!(),
                    Event::Decl(_) => {}
                    Event::PI(_) => unimplemented!(),
                    Event::DocType(_) => {}
                    Event::Eof => {
                        output.visit_end_of_stream().expect("End of stream");
                        break;
                    }
                },
                Err(e) => panic!("Error at position {}: {:?}", reader.buffer_position(), e),
            }
        }

        let bytes = output.finish().expect("finish streaming JSON");
        String::from_utf8(bytes).expect("UTF8 JSON")
    }

    #[test]
    fn test_unnamed_data_interspersed_with_binary_matches_legacy() {
        let xml = r#"
<Event>
  <EventData>
    <Data>v1</Data>
    <Binary>00AA</Binary>
    <Data>v2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new().num_threads(1);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        assert_eq!(
            legacy_value, streaming_value,
            "streaming JSON must match legacy JSON for unnamed <Data> elements interspersed with <Binary>"
        );
    }

    /// Regression test for Issue 1: Data aggregation format in separate_json_attributes mode.
    /// Legacy outputs `"Data": [...]` but streaming was outputting `"Data": { "#text": [...] }`.
    #[test]
    fn test_data_aggregation_separate_attributes_mode() {
        let xml = r#"
<Event>
  <EventData>
    <Data>v1</Data>
    <Data>v2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new()
            .num_threads(1)
            .separate_json_attributes(true);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        assert_eq!(
            legacy_value, streaming_value,
            "Data aggregation in separate_json_attributes mode: streaming must match legacy.\nLegacy: {}\nStreaming: {}",
            legacy_json, streaming_json
        );
    }

    /// Regression test for Issue 2: Duplicate element key handling.
    /// Legacy outputs `"LogonGuid": "...", "LogonGuid_1": "..."` but streaming was losing duplicates.
    ///
    /// NOTE: Legacy and streaming have different key ordering for duplicates:
    /// - Legacy: last value gets unsuffixed key (LogonGuid: guid2, LogonGuid_1: guid1)
    /// - Streaming: first value gets unsuffixed key (LogonGuid: guid1, LogonGuid_1: guid2)
    ///
    /// Both preserve all data, just with different key assignments. This is acceptable
    /// for streaming since we can't retroactively rename already-written keys.
    #[test]
    fn test_duplicate_element_keys() {
        let xml = r#"
<Event>
  <EventData>
    <Data Name="LogonGuid">guid1</Data>
    <Data Name="LogonGuid">guid2</Data>
  </EventData>
</Event>
        "#
        .trim();

        let settings = ParserSettings::new().num_threads(1);

        let legacy_json = xml_to_json_legacy(xml, &settings);
        let streaming_json = xml_to_json_streaming(xml, &settings);

        let legacy_value: serde_json::Value =
            serde_json::from_str(&legacy_json).expect("legacy JSON should be valid");
        let streaming_value: serde_json::Value =
            serde_json::from_str(&streaming_json).expect("streaming JSON should be valid");

        // Extract the set of LogonGuid values from EventData (regardless of key ordering)
        let legacy_event_data = &legacy_value["Event"]["EventData"];
        let streaming_event_data = &streaming_value["Event"]["EventData"];

        // Collect all values for LogonGuid* keys
        let mut legacy_values: Vec<&str> = Vec::new();
        let mut streaming_values: Vec<&str> = Vec::new();

        if let serde_json::Value::Object(obj) = legacy_event_data {
            for (key, val) in obj {
                if key.starts_with("LogonGuid")
                    && let serde_json::Value::String(s) = val
                {
                    legacy_values.push(s);
                }
            }
        }
        if let serde_json::Value::Object(obj) = streaming_event_data {
            for (key, val) in obj {
                if key.starts_with("LogonGuid")
                    && let serde_json::Value::String(s) = val
                {
                    streaming_values.push(s);
                }
            }
        }

        legacy_values.sort();
        streaming_values.sort();

        assert_eq!(
            legacy_values, streaming_values,
            "Duplicate element keys: both parsers must preserve all values.\nLegacy: {}\nStreaming: {}",
            legacy_json, streaming_json
        );
    }

    /// Regression test for Issue 3: Multiple character nodes concatenation.
    /// Legacy concatenates multiple text nodes, streaming was only keeping the first.
    /// This test directly invokes visit_characters multiple times to simulate the real case.
    #[test]
    fn test_multiple_character_nodes_concatenation() {
        use crate::model::xml::XmlElement;

        let arena = Bump::new();

        // Test by directly calling the visitor methods to simulate multiple character nodes
        let settings = ParserSettings::new().num_threads(1);

        // Legacy parser
        let mut legacy_output = JsonOutput::new(&settings);
        legacy_output.visit_start_of_stream().unwrap();
        let event_elem = XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Event")),
            attributes: vec![],
        };
        let msg_elem = XmlElement {
            name: Cow::Owned(BinXmlName::from_str("Message")),
            attributes: vec![],
        };
        legacy_output.visit_open_start_element(&event_elem).unwrap();
        legacy_output.visit_open_start_element(&msg_elem).unwrap();
        legacy_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType(
                BumpString::from_str_in("Part1", &arena),
            )))
            .unwrap();
        legacy_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType(
                BumpString::from_str_in("Part2", &arena),
            )))
            .unwrap();
        legacy_output.visit_close_element(&msg_elem).unwrap();
        legacy_output.visit_close_element(&event_elem).unwrap();
        legacy_output.visit_end_of_stream().unwrap();
        let legacy_value = legacy_output.into_value().unwrap();

        // Streaming parser
        let writer = Vec::new();
        let mut streaming_output = JsonStreamOutput::with_writer(writer, &settings);
        streaming_output.visit_start_of_stream().unwrap();
        streaming_output
            .visit_open_start_element(&event_elem)
            .unwrap();
        streaming_output
            .visit_open_start_element(&msg_elem)
            .unwrap();
        streaming_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType(
                BumpString::from_str_in("Part1", &arena),
            )))
            .unwrap();
        streaming_output
            .visit_characters(Cow::Owned(BinXmlValue::StringType(
                BumpString::from_str_in("Part2", &arena),
            )))
            .unwrap();
        streaming_output.visit_close_element(&msg_elem).unwrap();
        streaming_output.visit_close_element(&event_elem).unwrap();
        streaming_output.visit_end_of_stream().unwrap();
        let bytes = streaming_output.finish().unwrap();
        let streaming_json = String::from_utf8(bytes).unwrap();
        let streaming_value: serde_json::Value = serde_json::from_str(&streaming_json).unwrap();

        assert_eq!(
            legacy_value, streaming_value,
            "Multiple character nodes: streaming must match legacy.\nLegacy: {:?}\nStreaming: {}",
            legacy_value, streaming_json
        );
    }
}
