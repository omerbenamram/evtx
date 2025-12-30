//! JSON rendering for BinXML IR trees.
//!
//! This module turns the IR (`model::ir`) into JSON using a streaming renderer that
//! writes directly to a `WriteExt` sink. It avoids building any intermediate JSON
//! representation and (intentionally) matches the EVTX JSON conventions used by
//! this project's CLI.
//!
//! ## Mental model: element → JSON *value*
//!
//! Each XML element becomes a JSON value under a key named after the element:
//!
//! - **Scalar** (string/number/bool): when the element has *only* text/value nodes.
//! - **Object**: when the element has attributes and/or child elements.
//! - **null**: when the element is semantically empty (no child elements, no text,
//!   and no *non-empty* attributes).
//!
//! Numeric/bool coercion only happens for *typed* `Value(..)` nodes. Plain textual `"42"`
//! stays a JSON string.
//!
//! In object form we use two reserved keys:
//!
//! - `#attributes`: attribute bag (default mode)
//! - `#text`: concatenated non-element content (for mixed-content elements)
//!
//! Example (default mode):
//!
//! ```xml
//! <A x="1">hello<B/>world</A>
//! ```
//!
//! becomes:
//!
//! ```json
//! { "A": { "#attributes": { "x": "1" }, "#text": "helloworld", "B": null } }
//! ```
//!
//! (Note: `#text` intentionally omits markup; child elements are emitted as siblings.)
//!
//! ## Attributes: inline vs sibling (legacy compatibility)
//!
//! - **Default**: attributes live under `#attributes` inside the element object.
//! - **`separate_json_attributes`**: emit attributes as sibling keys
//!   `<ElementName>[_N]_attributes` next to `<ElementName>[_N]` (legacy output shape).
//!   Root attributes are emitted as `<Root>_attributes` at the top level.
//!
//! Example (separate mode):
//!
//! ```json
//! {
//!   "Header_attributes": { "x": "1" },
//!   "Header": "value"
//! }
//! ```
//!
//! ## Duplicate sibling names (`_N` suffixing)
//!
//! JSON objects can't represent repeated keys reliably, so for duplicate sibling
//! element names we append a numeric suffix: `Header`, `Header_1`, `Header_2`, ...
//!
//! This is implemented using a small fixed-size scan table (`MAX_UNIQUE_NAMES`)
//! to avoid allocating a `HashMap` on hot paths.
//!
//! ## `EventData` / `UserData` special handling
//!
//! These containers are common in Windows event logs and have two encodings:
//!
//! - **Named data**: `<Data Name="Foo">bar</Data>` → flatten into `"Foo": "bar"`.
//! - **Positional data**: `<Data>...</Data><Data>...</Data>` → group into:
//!   `"Data": { "#text": [ ... ] }` (or a single scalar when only one `Data` exists).
//!
//! If we detect *any* non-empty `Name` attribute on a `Data` child, we choose the
//! named/flattened form and skip unnamed `Data` nodes (to match existing behavior).
//!
//! ## Escaping / decoding
//!
//! - IR may contain `CharRef` / `EntityRef` nodes; for JSON we resolve standard entities
//!   (`quot`, `apos`, `amp`, `lt`, `gt`) to their character form and then JSON-escape.
//! - UTF-16 text is escaped using `utf16-simd` for speed.
//!
//! The emitter keeps scratch state (escaping + number formatting) so a record can be
//! rendered with minimal allocations.

use crate::ParserSettings;
use crate::binxml::value_render::ValueRenderer;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{Attr, Element, IrArena, IrTree, Name, Node, Text, is_optional_empty};
use crate::utils::Utf16LeSlice;
use sonic_rs::format::{CompactFormatter, Formatter};
use sonic_rs::writer::WriteExt;

/// Upper bound for the "unique child-name" scan table.
///
/// We deliberately use a small fixed-size array to avoid heap allocations while still
/// covering the vast majority of real-world event shapes. If a single element has more
/// unique child names than this, we stop tracking and may stop emitting `_N` suffixes
/// for duplicates beyond the tracked set.
const MAX_UNIQUE_NAMES: usize = 64;

/// Render a single record tree to JSON.
///
/// When `settings.should_separate_json_attributes()` is enabled, this renderer emits attribute
/// objects as sibling keys named `<ElementName>_attributes`, matching the legacy CLI output.
pub(crate) fn render_json_record<W: WriteExt>(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(
        writer,
        tree.arena(),
        settings.should_separate_json_attributes(),
    );
    let root = tree.root_element();
    emitter.write_bytes(b"{")?;
    if emitter.separate_json_attributes {
        // Root attributes are emitted as a sibling key `<Root>_attributes` at the top level.
        if !root.attrs.is_empty() && emitter.render_separate_attributes_for_element(root, 0)? {
            emitter.write_byte(b',')?;
        }
        emitter.write_json_key_from_name_with_suffix(&root.name, 0)?;
        emitter.write_element_value_no_attrs(root, false)?;
    } else {
        emitter.write_byte(b'\"')?;
        emitter.write_name(root.name.as_str())?;
        emitter.write_bytes(b"\":")?;
        emitter.write_element_value(root, false)?;
    }
    emitter.write_bytes(b"}")?;
    emitter.flush()?;
    Ok(())
}

/// Borrowed key for comparing element names without allocating.
///
/// This uses pointer/length equality as a fast path before falling back to
/// byte-wise comparison.
#[derive(Clone, Copy)]
struct NameKey<'a> {
    bytes: &'a str,
}

impl<'a> NameKey<'a> {
    fn from_name(name: &'a Name<'a>) -> Self {
        NameKey {
            bytes: name.as_str(),
        }
    }

    /// Equality optimized for the common "interned" case.
    ///
    /// Many IR names ultimately come from arena-backed slices, so pointer+len equality
    /// often short-circuits without scanning bytes. We still fall back to full string
    /// equality to remain correct for non-interned names.
    fn eql(self, other: NameKey<'a>) -> bool {
        if self.bytes.as_ptr() == other.bytes.as_ptr() && self.bytes.len() == other.bytes.len() {
            return true;
        }
        self.bytes == other.bytes
    }
}

/// Tracks how often a child element name has been emitted.
///
/// Used to generate stable legacy suffixes: `Header`, `Header_1`, `Header_2`, ...
struct NameCount<'a> {
    key: NameKey<'a>,
    emitted_count: u16,
}

/// Streaming JSON emitter for IR nodes.
///
/// The emitter owns formatter state and scratch buffers so callers can reuse
/// allocations while traversing a record tree.
struct JsonEmitter<'w, 'a, W: WriteExt> {
    writer: &'w mut W,
    arena: &'a IrArena<'a>,
    values: ValueRenderer,
    formatter: CompactFormatter,
    separate_json_attributes: bool,
}

impl<'w, 'a, W: WriteExt> JsonEmitter<'w, 'a, W> {
    fn new(writer: &'w mut W, arena: &'a IrArena<'a>, separate_json_attributes: bool) -> Self {
        JsonEmitter {
            writer,
            arena,
            values: ValueRenderer::new(),
            formatter: CompactFormatter,
            separate_json_attributes,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes).map_err(EvtxError::from)
    }

    fn write_byte(&mut self, byte: u8) -> Result<()> {
        self.writer.write_all(&[byte]).map_err(EvtxError::from)
    }

    fn flush(&mut self) -> Result<()> {
        self.writer.flush().map_err(EvtxError::from)
    }

    fn write_name(&mut self, name: &str) -> Result<()> {
        self.write_bytes(name.as_bytes())
    }

    /// Write a JSON object key from an element name with an optional legacy suffix.
    ///
    /// - `suffix == 0` → `"Name":`
    /// - `suffix == 1` → `"Name_1":`
    ///
    /// This is used to disambiguate duplicate sibling element names in a JSON object.
    fn write_json_key_from_name_with_suffix(&mut self, name: &Name<'_>, suffix: u16) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_name(name.as_str())?;
        if suffix > 0 {
            self.write_byte(b'_')?;
            self.formatter
                .write_u64(self.writer, u64::from(suffix))
                .map_err(EvtxError::from)?;
        }
        self.write_bytes(b"\":")
    }

    /// Write a JSON object key derived from IR nodes (escaped), followed by `:`.
    ///
    /// This is used for the `EventData`/`UserData` flattening case where the key comes
    /// from `Data[@Name]` and can itself contain entity/character references.
    ///
    /// Example:
    /// - `Name="A&amp;B"` → `"A&B":`
    fn write_json_key_from_nodes(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(nodes)?;
        self.write_bytes(b"\":")
    }

    /// Write JSON-escaped UTF-16LE contents (no surrounding quotes).
    ///
    /// Callers are responsible for writing `"` before/after if they want a JSON string.
    fn write_json_escaped_utf16(&mut self, value: Utf16LeSlice<'_>) -> Result<()> {
        let bytes = value.as_bytes();
        let units = bytes.len() / 2;
        if units == 0 {
            return Ok(());
        }
        utf16_simd::write_json_utf16le(self.writer, bytes, units, false)
            .map_err(EvtxError::from)?;
        Ok(())
    }

    /// Write the *contents* of a JSON string for a node slice.
    ///
    /// This function **does not** write surrounding quotes (`"`). Callers typically do:
    ///
    /// - `"` + `write_json_text_content(..)` + `"` for JSON string values
    /// - `"` + `write_json_text_content(..)` + `":` for "dynamic" keys (e.g. `Data[@Name]`)
    ///
    /// The slice is interpreted as "text-like" content:
    /// - `Text` / `CData`: appended (after JSON escaping)
    /// - `Value`: appended using the BinXML value renderer (e.g. substitutions)
    /// - `CharRef` / `EntityRef`: resolved to characters when possible
    ///
    /// Errors:
    /// - `Element` nodes are rejected because they belong in object context.
    /// - `Placeholder` nodes indicate a bug in IR construction (templates not resolved).
    ///
    /// Example (conceptual IR → JSON string contents):
    ///
    /// - `[Text("A"), EntityRef("amp"), Text("B")]` → `A&B`
    /// - `[Value(Int32Type(42))]` → `42` (still string contents; numeric coercion happens elsewhere)
    fn write_json_text_content(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(text) | Node::CData(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    match text {
                        Text::Utf16(value) => {
                            self.write_json_escaped_utf16(*value)?;
                        }
                        Text::Utf8(value) => {
                            self.formatter
                                .write_string_fast(self.writer, value.as_ref(), false)
                                .map_err(EvtxError::from)?;
                        }
                    }
                }
                Node::Value(value) => {
                    self.values.write_json_value_text(self.writer, value)?;
                }
                Node::CharRef(ch) => {
                    // In JSON, emit the resolved character (not an XML `&#...;` sequence).
                    // This keeps JSON values as plain text rather than XML markup.
                    if let Some(ch) = char::from_u32(u32::from(*ch)) {
                        let mut buf = [0_u8; 4];
                        let s = ch.encode_utf8(&mut buf);
                        self.formatter
                            .write_string_fast(self.writer, s, false)
                            .map_err(EvtxError::from)?;
                    } else {
                        // Preserve invalid code units as an XML-like escape sequence.
                        self.write_bytes(b"&#")?;
                        self.formatter
                            .write_u64(self.writer, u64::from(*ch))
                            .map_err(EvtxError::from)?;
                        self.write_byte(b';')?;
                    }
                }
                Node::EntityRef(name) => {
                    // In JSON, resolve standard XML entities to their character form.
                    // (XML escaping is only relevant when serializing back to XML.)
                    match name.as_str() {
                        "quot" => {
                            self.formatter
                                .write_string_fast(self.writer, "\"", false)
                                .map_err(EvtxError::from)?;
                        }
                        "apos" => {
                            self.formatter
                                .write_string_fast(self.writer, "'", false)
                                .map_err(EvtxError::from)?;
                        }
                        "amp" => {
                            self.formatter
                                .write_string_fast(self.writer, "&", false)
                                .map_err(EvtxError::from)?;
                        }
                        "lt" => {
                            self.formatter
                                .write_string_fast(self.writer, "<", false)
                                .map_err(EvtxError::from)?;
                        }
                        "gt" => {
                            self.formatter
                                .write_string_fast(self.writer, ">", false)
                                .map_err(EvtxError::from)?;
                        }
                        other => {
                            // Unknown entity: keep as literal `&name;` so the information isn't lost.
                            self.write_byte(b'&')?;
                            self.write_bytes(other.as_bytes())?;
                            self.write_byte(b';')?;
                        }
                    }
                }
                Node::PITarget(_) | Node::PIData(_) => {}
                Node::Placeholder(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "unresolved placeholder in tree",
                    ));
                }
                Node::Element(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "unexpected element node in text context",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Like [`write_json_text_content`], but ignores `Element` nodes.
    ///
    /// This is used for mixed-content elements where we emit child elements as separate
    /// object keys and put the concatenated "loose text" into `#text`.
    ///
    /// Example:
    ///
    /// ```xml
    /// <A>hello<B/>world</A>
    /// ```
    ///
    /// The `#text` value should be `"helloworld"` (no markup), while `B` is emitted as
    /// its own key.
    fn write_json_text_content_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        for node in nodes {
            match node {
                Node::Element(_) => continue,
                Node::PITarget(_) | Node::PIData(_) => {}
                Node::Text(_)
                | Node::CData(_)
                | Node::Value(_)
                | Node::CharRef(_)
                | Node::EntityRef(_) => {
                    // Reuse the existing implementation by writing one node at a time.
                    // This keeps the escaping behavior identical.
                    self.write_json_text_content(std::slice::from_ref(node))?;
                }
                Node::Placeholder(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "unresolved placeholder in tree",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Try to write a BinXML value as a JSON primitive (number/bool).
    ///
    /// Returns `true` when the value was written as a non-string JSON token, `false`
    /// when the caller should fall back to string rendering.
    ///
    /// Example:
    /// - `UInt32Type(7)` → `7`
    /// - `BoolType(true)` → `true`
    /// - `StringType("7")` → `false` (will be rendered as `"7"`)
    fn write_value_as_number(&mut self, value: &BinXmlValue<'_>) -> Result<bool> {
        match value {
            BinXmlValue::Int8Type(v) => self.write_signed_number(i64::from(*v)),
            BinXmlValue::Int16Type(v) => self.write_signed_number(i64::from(*v)),
            BinXmlValue::Int32Type(v) => self.write_signed_number(i64::from(*v)),
            BinXmlValue::Int64Type(v) => self.write_signed_number(*v),
            BinXmlValue::UInt8Type(v) => self.write_unsigned_number(u64::from(*v)),
            BinXmlValue::UInt16Type(v) => self.write_unsigned_number(u64::from(*v)),
            BinXmlValue::UInt32Type(v) => self.write_unsigned_number(u64::from(*v)),
            BinXmlValue::UInt64Type(v) => self.write_unsigned_number(*v),
            BinXmlValue::BoolType(v) => {
                self.write_bytes(if *v { b"true" } else { b"false" })?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// If `nodes` is exactly one `Value(..)` node, try to emit it as a JSON primitive.
    ///
    /// This is the fast-path used for elements/attributes whose content is purely a
    /// single typed substitution.
    ///
    /// Example:
    /// - `nodes = [Value(Int64Type(1))]` → writes `1` and returns `true`
    /// - `nodes = [Text("1")]` → returns `false` (will be rendered as `"1"`)
    fn try_write_as_number(&mut self, nodes: &[Node<'_>]) -> Result<bool> {
        if nodes.len() != 1 {
            return Ok(false);
        }
        let Node::Value(value) = &nodes[0] else {
            return Ok(false);
        };
        self.write_value_as_number(value)
    }

    /// Numeric coercion for mixed-content slices where `Element` nodes should be ignored.
    ///
    /// We treat the slice as numeric only when:
    /// - there is exactly one non-empty `Value(..)` node, and
    /// - there is no non-empty `Text`/`CData`, and
    /// - there are no `CharRef`/`EntityRef` nodes.
    ///
    /// This lets `<X> <Sub/> 42 </X>` still coerce to a number for `#text` if the IR was
    /// represented as a single `Value(..)` surrounded by optional empties.
    fn try_write_as_number_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<bool> {
        let mut single: Option<&BinXmlValue<'_>> = None;

        for node in nodes {
            match node {
                Node::Element(_) => continue,
                Node::Text(text) | Node::CData(text) => {
                    if !text.is_empty() {
                        return Ok(false);
                    }
                }
                Node::Value(value) => {
                    if is_optional_empty(value) {
                        continue;
                    }
                    if single.is_some() {
                        return Ok(false);
                    }
                    single = Some(value);
                }
                Node::CharRef(_) | Node::EntityRef(_) => return Ok(false),
                Node::PITarget(_) | Node::PIData(_) => {}
                Node::Placeholder(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "unresolved placeholder in tree",
                    ));
                }
            }
        }

        let Some(value) = single else {
            return Ok(false);
        };

        self.write_value_as_number(value)
    }

    /// Emit an `i64` JSON token using the formatter.
    fn write_signed_number(&mut self, value: i64) -> Result<bool> {
        self.formatter
            .write_i64(self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    /// Emit a `u64` JSON token using the formatter.
    fn write_unsigned_number(&mut self, value: u64) -> Result<bool> {
        self.formatter
            .write_u64(self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    /// Render `nodes` as a JSON string (always quoted).
    fn render_text_to_json_string(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(nodes)?;
        self.write_byte(b'\"')
    }

    /// Render `nodes` as a JSON string (always quoted), ignoring `Element` nodes.
    fn render_text_to_json_string_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content_skip_elements(nodes)?;
        self.write_byte(b'\"')
    }

    /// Render a node slice as a JSON value, applying numeric/bool coercion where possible.
    ///
    /// This is the common "leaf" renderer for element bodies and attribute values.
    fn render_content_as_json_value(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        if self.try_write_as_number(nodes)? {
            return Ok(());
        }
        self.render_text_to_json_string(nodes)
    }

    /// Variant of [`render_content_as_json_value`] used for mixed-content `#text` rendering.
    fn render_content_as_json_value_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        if self.try_write_as_number_skip_elements(nodes)? {
            return Ok(());
        }
        self.render_text_to_json_string_skip_elements(nodes)
    }

    /// Returns true if `nodes` contains any semantically non-empty "text-like" content.
    ///
    /// This is *not* the same as `!nodes.is_empty()`:
    /// - Empty `Text`/`CData` nodes are ignored.
    /// - "Optional empty" typed substitutions are ignored (see `is_optional_empty`).
    /// - `CharRef` / `EntityRef` always count as content, even if they might resolve to
    ///   whitespace, because they are explicit in the source.
    fn has_non_empty_text_content(&self, nodes: &[Node<'_>]) -> bool {
        for node in nodes {
            match node {
                Node::Text(text) | Node::CData(text) => {
                    if !text.is_empty() {
                        return true;
                    }
                }
                Node::Value(value) => {
                    if !is_optional_empty(value) {
                        return true;
                    }
                }
                Node::CharRef(_) | Node::EntityRef(_) => return true,
                _ => {}
            }
        }
        false
    }

    /// Quick attribute presence flags.
    ///
    /// Returns `(has_any_attrs, has_any_non_empty_attr_value)`.
    ///
    /// This distinction matters because we treat "attribute exists but is empty" as
    /// ignorable for deciding between `null` vs `{ "#attributes": ... }`.
    fn attr_flags(&self, attrs: &[Attr<'_>]) -> (bool, bool) {
        if attrs.is_empty() {
            return (false, false);
        }
        for attr in attrs {
            if self.has_non_empty_text_content(&attr.value) {
                return (true, true);
            }
        }
        (true, false)
    }

    /// Emit a `"#attributes": { ... }` object into the current element object.
    ///
    /// Only attributes with non-empty values are emitted.
    ///
    /// Returns `true` if anything was written (used to drive comma placement).
    fn render_attributes_object(&mut self, attrs: &[Attr<'_>]) -> Result<bool> {
        let mut wrote_any = false;
        let mut first = true;
        for attr in attrs {
            if !self.has_non_empty_text_content(&attr.value) {
                continue;
            }
            if !wrote_any {
                self.write_bytes(b"\"#attributes\":{")?;
                wrote_any = true;
            }
            if !first {
                self.write_byte(b',')?;
            }
            first = false;
            self.write_byte(b'\"')?;
            self.write_name(attr.name.as_str())?;
            self.write_bytes(b"\":")?;
            if self.try_write_as_number(&attr.value)? {
                continue;
            }
            self.render_text_to_json_string(&attr.value)?;
        }
        if wrote_any {
            self.write_byte(b'}')?;
        }
        Ok(wrote_any)
    }

    /// Emit `{ ... }` attribute members without the outer wrapper.
    ///
    /// This is used for the legacy sibling-attributes mode where the key is
    /// `<Element>_attributes` instead of `#attributes`.
    fn render_attributes_object_body(&mut self, attrs: &[Attr<'_>]) -> Result<bool> {
        let mut wrote_any = false;
        let mut first = true;
        for attr in attrs {
            if !self.has_non_empty_text_content(&attr.value) {
                continue;
            }
            if !first {
                self.write_byte(b',')?;
            }
            first = false;
            wrote_any = true;
            self.write_byte(b'\"')?;
            self.write_name(attr.name.as_str())?;
            self.write_bytes(b"\":")?;
            if self.try_write_as_number(&attr.value)? {
                continue;
            }
            self.render_text_to_json_string(&attr.value)?;
        }
        Ok(wrote_any)
    }

    /// Emit `<ElementName>_attributes` for the provided element into the current object.
    ///
    /// Returns `true` if anything was written.
    ///
    /// Example (element name `Header`, suffix `2`):
    ///
    /// ```text
    /// "Header_2_attributes": { ... }
    /// ```
    fn render_separate_attributes_for_element(
        &mut self,
        element: &Element<'_>,
        suffix: u16,
    ) -> Result<bool> {
        if element.attrs.is_empty() {
            return Ok(false);
        }
        let (_has_any, has_text) = self.attr_flags(&element.attrs);
        if !has_text {
            return Ok(false);
        }

        // Write key: "<name>[_N]_attributes":
        self.write_byte(b'\"')?;
        self.write_name(element.name.as_str())?;
        if suffix > 0 {
            self.write_byte(b'_')?;
            self.formatter
                .write_u64(self.writer, u64::from(suffix))
                .map_err(EvtxError::from)?;
        }
        self.write_bytes(b"_attributes\":{")?;
        let wrote_any = self.render_attributes_object_body(&element.attrs)?;
        self.write_byte(b'}')?;
        Ok(wrote_any)
    }

    /// Render the JSON value for a `<Data>` element inside `EventData`/`UserData`.
    ///
    /// The empty case is intentionally `""` (empty string) rather than `null` to match
    /// established EVTX JSON output expectations.
    ///
    /// Example:
    /// - `<Data/>` → `""`
    /// - `<Data>42</Data>` (typed substitution) → `42` (number)
    /// - `<Data><X>y</X></Data>` → `{ "X": "y" }`
    fn render_data_element_value(&mut self, element: &'a Element<'a>) -> Result<()> {
        if !self.has_non_empty_text_content(&element.children) && !element.has_element_child {
            return self.write_bytes(b"\"\"");
        }

        if element.has_element_child {
            self.write_element_body_json(element, false, true)
        } else {
            self.render_content_as_json_value(&element.children)
        }
    }

    /// Render an element value when attributes are handled elsewhere (separate-attrs mode).
    ///
    /// Shape rules:
    /// - no children + no text → `null`
    /// - only text/value nodes → scalar (string/number/bool)
    /// - any child elements → object
    fn write_element_value_no_attrs(
        &mut self,
        element: &'a Element<'a>,
        child_is_container: bool,
    ) -> Result<()> {
        let has_text = self.has_non_empty_text_content(&element.children);
        let has_element_child = element.has_element_child;

        if !has_element_child && !has_text {
            self.write_bytes(b"null")
        } else if !has_element_child {
            self.render_content_as_json_value(&element.children)
        } else {
            self.write_element_body_json(element, child_is_container, true)
        }
    }

    /// Render an element value in the default mode (attributes inline under `#attributes`).
    ///
    /// This decides between scalar/object/null and delegates to [`write_element_body_json`]
    /// when object form is required.
    fn write_element_value(
        &mut self,
        element: &'a Element<'a>,
        child_is_container: bool,
    ) -> Result<()> {
        let has_text = self.has_non_empty_text_content(&element.children);
        let (_has_attrs_any, has_attrs_text) = self.attr_flags(&element.attrs);
        let has_element_child = element.has_element_child;

        if !has_element_child && !has_text && !has_attrs_text {
            self.write_bytes(b"null")
        } else if !has_element_child && !has_attrs_text {
            self.render_content_as_json_value(&element.children)
        } else {
            self.write_element_body_json(element, child_is_container, false)
        }
    }

    /// Render an element in "object form".
    ///
    /// This is the core routine that explains most of the complexity in this module.
    /// It performs a single pass over `element.children` while maintaining a few small
    /// pieces of scan state:
    ///
    /// - **Attributes**: optionally emit `#attributes` first.
    /// - **Mixed content**: if the element has non-element text/value nodes, emit `#text`.
    /// - **`EventData`/`UserData`**: detect "named data" vs "positional data" and render:
    ///   - named: `"Foo": <value>` (flattened)
    ///   - positional: `"Data": { "#text": [ ... ] }` (grouped)
    /// - **Duplicate sibling names**: track counts in a fixed scan table to append `_N`.
    ///
    /// Example (`EventData` positional):
    ///
    /// ```xml
    /// <EventData>
    ///   <Data>one</Data>
    ///   <Data>two</Data>
    /// </EventData>
    /// ```
    ///
    /// becomes:
    ///
    /// ```json
    /// { "EventData": { "Data": { "#text": ["one","two"] } } }
    /// ```
    ///
    /// Example (`EventData` named):
    ///
    /// ```xml
    /// <EventData>
    ///   <Data Name="Foo">bar</Data>
    ///   <Data Name="Baz">qux</Data>
    /// </EventData>
    /// ```
    ///
    /// becomes:
    ///
    /// ```json
    /// { "EventData": { "Foo": "bar", "Baz": "qux" } }
    /// ```
    fn write_element_body_json(
        &mut self,
        element: &Element<'_>,
        in_data_container: bool,
        omit_attributes: bool,
    ) -> Result<()> {
        let arena = self.arena;

        // Detect whether `EventData`/`UserData` should be flattened into Name-keyed pairs.
        //
        // Important nuance: we only need to see *one* non-empty `Data[@Name]` to select
        // the named/flattened form. Once selected, unnamed `Data` nodes are skipped.
        let should_flatten_named_data = if in_data_container {
            element.children.iter().any(|node| {
                let Node::Element(child_id) = node else {
                    return false;
                };
                let child = arena.get(*child_id).expect("invalid element id");
                if !is_data_element(child.name.as_str()) {
                    return false;
                }
                let Some(name_nodes) = Self::get_name_attr_nodes(child) else {
                    return false;
                };
                self.has_non_empty_text_content(name_nodes)
            })
        } else {
            false
        };

        // Count child element names on the fly so we can apply legacy `_N` suffixes
        // (Header, Header_1, Header_2, ...).
        //
        // Why a fixed array?
        // - Avoids per-record heap allocations (common hot path).
        // - We expect a small number of unique sibling names for typical Event XML.
        let mut name_counts: [Option<NameCount<'_>>; MAX_UNIQUE_NAMES] =
            std::array::from_fn(|_| None);
        let mut num_unique = 0usize;

        self.write_byte(b'{')?;
        let mut wrote_any = false;

        if !omit_attributes
            && !element.attrs.is_empty()
            && self.render_attributes_object(&element.attrs)?
        {
            wrote_any = true;
        }

        // If we're emitting an object, any non-element content becomes `#text`.
        //
        // This follows the common "mixed content" convention used by XML→JSON mappings:
        // element children become keys, and free text is captured under a reserved key.
        if self.has_non_empty_text_content(&element.children) {
            if wrote_any {
                self.write_byte(b',')?;
            }
            wrote_any = true;
            self.write_bytes(b"\"#text\":")?;
            self.render_content_as_json_value_skip_elements(&element.children)?;
        }

        // Pre-count positional `Data` nodes for the non-flattened container case.
        //
        // We only emit `"Data": { "#text": ... }` once, so we need to know whether it
        // should be an array and (if so) how many items it will contain.
        let positional_data_count = if in_data_container && !should_flatten_named_data {
            element
                .children
                .iter()
                .filter(|node| {
                    let Node::Element(child_id) = node else {
                        return false;
                    };
                    let child = arena.get(*child_id).expect("invalid element id");
                    is_data_element(child.name.as_str())
                })
                .count()
        } else {
            0
        };
        let mut positional_data_emitted = false;

        for node in &element.children {
            let Node::Element(child_id) = node else {
                continue;
            };

            let child = arena.get(*child_id).expect("invalid element id");

            // EventData/UserData special-case.
            if in_data_container && is_data_element(child.name.as_str()) {
                if should_flatten_named_data {
                    let Some(name_nodes) = Self::get_name_attr_nodes(child) else {
                        continue;
                    };
                    if !self.has_non_empty_text_content(name_nodes) {
                        continue;
                    }
                    if wrote_any {
                        self.write_byte(b',')?;
                    }
                    wrote_any = true;
                    self.write_json_key_from_nodes(name_nodes)?;
                    self.render_data_element_value(child)?;
                } else if !positional_data_emitted && positional_data_count > 0 {
                    if wrote_any {
                        self.write_byte(b',')?;
                    }
                    wrote_any = true;
                    positional_data_emitted = true;

                    self.write_bytes(b"\"Data\":{")?;
                    self.write_bytes(b"\"#text\":")?;
                    if positional_data_count == 1 {
                        self.render_data_element_value(child)?;
                    } else {
                        self.write_byte(b'[')?;
                        let mut first = true;
                        for node2 in &element.children {
                            let Node::Element(candidate_id) = node2 else {
                                continue;
                            };
                            let candidate = arena.get(*candidate_id).expect("invalid element id");
                            if !is_data_element(candidate.name.as_str()) {
                                continue;
                            }
                            if !first {
                                self.write_byte(b',')?;
                            }
                            first = false;
                            self.render_data_element_value(candidate)?;
                        }
                        self.write_byte(b']')?;
                    }
                    self.write_byte(b'}')?;
                }
                continue;
            }

            // Normal child element: apply `_N` suffixes.
            let key = NameKey::from_name(&child.name);
            let mut suffix: u16 = 0;
            let mut found = false;

            for nc_opt in name_counts.iter_mut().take(num_unique) {
                let Some(nc) = nc_opt.as_mut() else {
                    continue;
                };
                if nc.key.eql(key) {
                    suffix = nc.emitted_count;
                    nc.emitted_count = nc.emitted_count.saturating_add(1);
                    found = true;
                    break;
                }
            }

            if !found && num_unique < MAX_UNIQUE_NAMES {
                name_counts[num_unique] = Some(NameCount {
                    key,
                    emitted_count: 1,
                });
                num_unique += 1;
                suffix = 0;
            }

            if wrote_any {
                self.write_byte(b',')?;
            }
            wrote_any = true;

            if self.separate_json_attributes {
                // Emit `<name>_attributes` sibling before the value, matching legacy output.
                let wrote_attrs = self.render_separate_attributes_for_element(child, suffix)?;

                // Omit `<name>: null` when the element only contains attributes.
                let child_has_value =
                    child.has_element_child || self.has_non_empty_text_content(&child.children);
                let write_value = child_has_value || !wrote_attrs;

                if wrote_attrs && write_value {
                    self.write_byte(b',')?;
                }
                if write_value {
                    self.write_json_key_from_name_with_suffix(&child.name, suffix)?;
                    let child_is_container = is_data_container(child.name.as_str());
                    self.write_element_value_no_attrs(child, child_is_container)?;
                }
            } else {
                self.write_json_key_from_name_with_suffix(&child.name, suffix)?;
                let child_is_container = is_data_container(child.name.as_str());
                self.write_element_value(child, child_is_container)?;
            }
        }

        self.write_byte(b'}')?;
        Ok(())
    }

    /// Return the raw node slice for `Data[@Name]` if present.
    ///
    /// We keep this as a `&[Node]` (rather than resolving to a `String`) to avoid
    /// allocations; the same JSON-escaping rules that apply to text content are used
    /// when the attribute is turned into a JSON key.
    ///
    /// Example:
    /// - `<Data Name="Foo">bar</Data>` → returns nodes representing `"Foo"`
    fn get_name_attr_nodes<'b>(element: &'b Element<'a>) -> Option<&'b [Node<'a>]> {
        for attr in &element.attrs {
            if attr.name.as_str() == "Name" {
                return Some(&attr.value);
            }
        }
        None
    }
}

/// Benchmark-only helper to measure JSON text rendering.
#[cfg(feature = "bench")]
pub(crate) fn bench_write_json_text_content<'a, W: WriteExt>(
    writer: &mut W,
    arena: &'a IrArena<'a>,
    nodes: &[Node<'a>],
) -> Result<()> {
    let mut emitter = JsonEmitter::new(writer, arena, false);
    emitter.write_json_text_content(nodes)?;
    emitter.flush()
}

fn is_data_container(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn is_data_element(name: &str) -> bool {
    name == "Data"
}

// NOTE: Array substitution expansion is handled during template instantiation
// (`binxml::ir::clone_and_resolve`), not in the JSON renderer.
