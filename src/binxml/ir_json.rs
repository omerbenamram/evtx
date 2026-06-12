//! JSON rendering for BinXML IR trees.
//!
//! This module turns the IR (`model::ir`) into JSON using a streaming renderer that
//! writes directly to a `WriteExt` sink. It avoids building any intermediate JSON
//! representation and (intentionally) matches the EVTX JSON conventions used by
//! this project's CLI.
//!
//! The renderer works on either a fully materialized tree or a cached template
//! definition tree with render-time substitution resolution (`binxml::render_ctx`),
//! including render-time array expansion. Expanded copies behave exactly like the
//! repeated sibling elements the materialized path would produce (they participate
//! in positional `Data` counting and `_N` suffixing).
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
use crate::binxml::ir::RecordContent;
use crate::binxml::render_ctx::{
    Ovr, RNode, Scope, content_layout, count_expansion_copies, expansion_any, find_expansion,
    for_each_expansion, has_non_empty_text_content, resolve_child_element,
};
use crate::binxml::value_render::ValueRenderer;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{Attr, Element, IrArena, IrTree, Name, Node, Text, is_optional_empty};
use crate::utils::Utf16LeSlice;
use sonic_rs::format::{CompactFormatter, Formatter};
use sonic_rs::writer::WriteExt;

/// Size of the inline "unique child-name" scan table.
///
/// Sized to cover typical event shapes (a `System` element has ~14 unique child
/// names) while keeping the per-element zeroing cost small; rarer shapes spill
/// into a heap vector, so `_N` suffixing stays correct for any name count.
const INLINE_UNIQUE_NAMES: usize = 16;

/// Render a single materialized record tree to JSON.
pub(crate) fn render_json_record<W: WriteExt>(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    render_json_with_scope(
        Scope::materialized(tree.arena()),
        tree.root_element(),
        settings,
        writer,
    )
}

/// Render record content (materialized tree or unmaterialized template instance).
pub(crate) fn render_json_record_content<W: WriteExt>(
    content: &RecordContent<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    match content {
        RecordContent::Tree(tree) => render_json_record(tree, settings, writer),
        RecordContent::Template(tc) => {
            let scope = tc.scope();
            let root = scope
                .arena
                .get(tc.root.template.root())
                .expect("invalid element id");
            // The instantiation root is never array-expanded (matches the
            // materialized path, which discards the root expansion flag).
            render_json_with_scope(scope, root, settings, writer)
        }
    }
}

fn render_json_with_scope<W: WriteExt>(
    scope: Scope<'_, '_>,
    root: &Element<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(writer, settings.should_separate_json_attributes());
    emitter.write_bytes(b"{")?;
    if emitter.separate_json_attributes {
        // Root attributes are emitted as a sibling key `<Root>_attributes` at the top level.
        if !root.attrs.is_empty()
            && emitter.render_separate_attributes_for_element(scope, root, 0, None)?
        {
            emitter.write_byte(b',')?;
        }
        emitter.write_json_key_from_name_with_suffix(&root.name, 0)?;
        emitter.write_element_value_no_attrs(scope, root, None, false)?;
    } else {
        emitter.write_byte(b'\"')?;
        emitter.write_name(root.name.as_str())?;
        emitter.write_bytes(b"\":")?;
        emitter.write_element_value(scope, root, None, false)?;
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

/// Per-element child-name table: a small inline array for the common case,
/// spilling to the heap so `_N` suffixing stays correct for any name count.
struct NameCounts<'a> {
    inline: [Option<NameCount<'a>>; INLINE_UNIQUE_NAMES],
    num_inline: usize,
    spill: Vec<NameCount<'a>>,
}

impl<'a> NameCounts<'a> {
    /// Return the `_N` suffix for this occurrence of `key` and bump its count.
    fn next_suffix(&mut self, key: NameKey<'a>) -> u16 {
        for nc_opt in self.inline.iter_mut().take(self.num_inline) {
            let Some(nc) = nc_opt.as_mut() else {
                continue;
            };
            if nc.key.eql(key) {
                let suffix = nc.emitted_count;
                nc.emitted_count = nc.emitted_count.saturating_add(1);
                return suffix;
            }
        }
        if !self.spill.is_empty() {
            for nc in self.spill.iter_mut() {
                if nc.key.eql(key) {
                    let suffix = nc.emitted_count;
                    nc.emitted_count = nc.emitted_count.saturating_add(1);
                    return suffix;
                }
            }
        }
        let entry = NameCount {
            key,
            emitted_count: 1,
        };
        if self.num_inline < INLINE_UNIQUE_NAMES {
            self.inline[self.num_inline] = Some(entry);
            self.num_inline += 1;
        } else {
            self.spill.push(entry);
        }
        0
    }
}

/// Quick attribute presence flags.
///
/// Returns `(has_any_attrs, has_any_non_empty_attr_value)`.
///
/// This distinction matters because we treat "attribute exists but is empty" as
/// ignorable for deciding between `null` vs `{ "#attributes": ... }`.
fn attr_flags(scope: &Scope<'_, '_>, attrs: &[Attr<'_>], ovr: Option<&Ovr<'_>>) -> (bool, bool) {
    if attrs.is_empty() {
        return (false, false);
    }
    for attr in attrs {
        if has_non_empty_text_content(scope, &attr.value, ovr) {
            return (true, true);
        }
    }
    (true, false)
}

/// Return the raw node slice for `Data[@Name]` if present.
///
/// We keep this as a `&[Node]` (rather than resolving to a `String`) to avoid
/// allocations; the same JSON-escaping rules that apply to text content are used
/// when the attribute is turned into a JSON key.
fn get_name_attr_nodes<'b, 'a>(element: &'b Element<'a>) -> Option<&'b [Node<'a>]> {
    for attr in &element.attrs {
        if attr.name.as_str() == "Name" {
            return Some(&attr.value);
        }
    }
    None
}

/// Streaming JSON emitter for IR nodes.
///
/// The emitter owns formatter state and scratch buffers so callers can reuse
/// allocations while traversing a record tree.
struct JsonEmitter<'w, W: WriteExt> {
    writer: &'w mut W,
    values: ValueRenderer,
    formatter: CompactFormatter,
    separate_json_attributes: bool,
}

impl<'w, W: WriteExt> JsonEmitter<'w, W> {
    fn new(writer: &'w mut W, separate_json_attributes: bool) -> Self {
        JsonEmitter {
            writer,
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
    fn write_json_key_from_nodes(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(scope, nodes, false, ovr)?;
        self.write_bytes(b"\":")
    }

    /// Write JSON-escaped UTF-16LE contents (no surrounding quotes).
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

    fn write_json_text(&mut self, text: &Text<'_>) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        match text {
            Text::Utf16(value) => self.write_json_escaped_utf16(*value),
            Text::Utf8(value) => self
                .formatter
                .write_string_fast(self.writer, value.as_ref(), false)
                .map_err(EvtxError::from),
        }
    }

    /// Write the *contents* of a JSON string for a node slice.
    ///
    /// This function **does not** write surrounding quotes (`"`). The slice is
    /// interpreted as "text-like" content:
    /// - `Text` / `CData`: appended (after JSON escaping)
    /// - `Value`: appended using the BinXML value renderer (e.g. substitutions)
    /// - `CharRef` / `EntityRef`: resolved to characters when possible
    ///
    /// Errors:
    /// - element-like nodes are rejected because they belong in object context, unless
    ///   `skip_elements` is set (used for mixed-content `#text`, where child elements are
    ///   emitted as separate object keys and only the "loose text" is concatenated).
    /// - `Placeholder` nodes in a materialized tree indicate a bug in IR construction.
    fn write_json_text_content(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        skip_elements: bool,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        for node in nodes {
            match scope.resolve(node, ovr)? {
                RNode::Skip => {}
                RNode::Text(text) => self.write_json_text(&text)?,
                RNode::Value(value) => {
                    self.values.write_json_value_text(self.writer, value)?;
                }
                RNode::OwnValue(value) => {
                    self.values.write_json_value_text(self.writer, &value)?;
                }
                RNode::Frag(_) | RNode::Nested(_) => {
                    if skip_elements {
                        continue;
                    }
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "unexpected element node in text context",
                    ));
                }
                RNode::Plain(node) => match node {
                    Node::Text(text) | Node::CData(text) => self.write_json_text(text)?,
                    Node::Value(value) => {
                        self.values.write_json_value_text(self.writer, value)?;
                    }
                    Node::CharRef(ch) => {
                        // In JSON, emit the resolved character (not an XML `&#...;` sequence).
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
                        let resolved = match name.as_str() {
                            "quot" => Some("\""),
                            "apos" => Some("'"),
                            "amp" => Some("&"),
                            "lt" => Some("<"),
                            "gt" => Some(">"),
                            _ => None,
                        };
                        match resolved {
                            Some(s) => {
                                self.formatter
                                    .write_string_fast(self.writer, s, false)
                                    .map_err(EvtxError::from)?;
                            }
                            None => {
                                // Unknown entity: keep as literal `&name;`.
                                self.write_byte(b'&')?;
                                self.write_bytes(name.as_str().as_bytes())?;
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
                        if skip_elements {
                            continue;
                        }
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "unexpected element node in text context",
                        ));
                    }
                },
            }
        }
        Ok(())
    }

    /// Try to write a BinXML value as a JSON primitive (number/bool).
    ///
    /// Returns `true` when the value was written as a non-string JSON token, `false`
    /// when the caller should fall back to string rendering.
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

    /// If the resolved content is exactly one `Value(..)` node, try to emit it as a
    /// JSON primitive.
    ///
    /// Matches the materialized rule "the (resolved) node slice has exactly one node
    /// and it is a typed value" — omitted substitutions don't count as nodes.
    fn try_write_as_number(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        // Fast path: the dominant shape is a single node (typically one substitution).
        let single = if let [node] = nodes {
            scope.resolve(node, ovr)?
        } else {
            let mut single: Option<RNode<'_, '_>> = None;
            for node in nodes {
                match scope.resolve(node, ovr)? {
                    RNode::Skip => {}
                    rnode => {
                        if single.is_some() {
                            return Ok(false);
                        }
                        single = Some(rnode);
                    }
                }
            }
            match single {
                Some(rnode) => rnode,
                None => return Ok(false),
            }
        };
        match single {
            RNode::Value(value) => self.write_value_as_number(value),
            RNode::OwnValue(value) => self.write_value_as_number(&value),
            RNode::Plain(Node::Value(value)) => self.write_value_as_number(value),
            _ => Ok(false),
        }
    }

    /// Numeric coercion for mixed-content slices where element nodes should be ignored.
    ///
    /// We treat the slice as numeric only when:
    /// - there is exactly one non-empty `Value(..)` node, and
    /// - there is no non-empty `Text`/`CData`, and
    /// - there are no `CharRef`/`EntityRef` nodes.
    fn try_write_as_number_skip_elements(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        let mut single: Option<RNode<'_, '_>> = None;

        for node in nodes {
            let rnode = scope.resolve(node, ovr)?;
            match &rnode {
                RNode::Skip | RNode::Frag(_) | RNode::Nested(_) => continue,
                RNode::Text(text) => {
                    if !text.is_empty() {
                        return Ok(false);
                    }
                }
                RNode::Value(value) => {
                    if is_optional_empty(value) {
                        continue;
                    }
                    if single.is_some() {
                        return Ok(false);
                    }
                    single = Some(rnode);
                }
                RNode::OwnValue(value) => {
                    if is_optional_empty(value) {
                        continue;
                    }
                    if single.is_some() {
                        return Ok(false);
                    }
                    single = Some(rnode);
                }
                RNode::Plain(node) => match node {
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
                        single = Some(rnode);
                    }
                    Node::CharRef(_) | Node::EntityRef(_) => return Ok(false),
                    Node::PITarget(_) | Node::PIData(_) => {}
                    Node::Placeholder(_) => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "unresolved placeholder in tree",
                        ));
                    }
                },
            }
        }

        match single {
            Some(RNode::Value(value)) => self.write_value_as_number(value),
            Some(RNode::OwnValue(value)) => self.write_value_as_number(&value),
            Some(RNode::Plain(Node::Value(value))) => self.write_value_as_number(value),
            _ => Ok(false),
        }
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
    fn render_text_to_json_string(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        skip_elements: bool,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(scope, nodes, skip_elements, ovr)?;
        self.write_byte(b'\"')
    }

    /// Render a node slice as a JSON value, applying numeric/bool coercion where possible.
    fn render_content_as_json_value(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        skip_elements: bool,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        let wrote_number = if skip_elements {
            self.try_write_as_number_skip_elements(scope, nodes, ovr)?
        } else {
            self.try_write_as_number(scope, nodes, ovr)?
        };
        if wrote_number {
            return Ok(());
        }
        self.render_text_to_json_string(scope, nodes, skip_elements, ovr)
    }

    /// Emit a `"#attributes": { ... }` object into the current element object.
    ///
    /// Only attributes with non-empty values are emitted.
    fn render_attributes_object(
        &mut self,
        scope: Scope<'_, '_>,
        attrs: &[Attr<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        let (_has_any, has_text) = attr_flags(&scope, attrs, ovr);
        if !has_text {
            return Ok(false);
        }
        self.write_bytes(b"\"#attributes\":{")?;
        let wrote_any = self.render_attributes_object_body(scope, attrs, ovr)?;
        self.write_byte(b'}')?;
        Ok(wrote_any)
    }

    /// Emit `{ ... }` attribute members without the outer wrapper.
    fn render_attributes_object_body(
        &mut self,
        scope: Scope<'_, '_>,
        attrs: &[Attr<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        let mut wrote_any = false;
        let mut first = true;
        for attr in attrs {
            if !has_non_empty_text_content(&scope, &attr.value, ovr) {
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
            if self.try_write_as_number(scope, &attr.value, ovr)? {
                continue;
            }
            self.render_text_to_json_string(scope, &attr.value, false, ovr)?;
        }
        Ok(wrote_any)
    }

    /// Emit `<ElementName>[_N]_attributes` for the provided element into the current object.
    fn render_separate_attributes_for_element(
        &mut self,
        scope: Scope<'_, '_>,
        element: &Element<'_>,
        suffix: u16,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        if element.attrs.is_empty() {
            return Ok(false);
        }
        let (_has_any, has_text) = attr_flags(&scope, &element.attrs, ovr);
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
        let wrote_any = self.render_attributes_object_body(scope, &element.attrs, ovr)?;
        self.write_byte(b'}')?;
        Ok(wrote_any)
    }

    /// Render the JSON value for a `<Data>` element inside `EventData`/`UserData`.
    ///
    /// The empty case is intentionally `""` (empty string) rather than `null` to match
    /// established EVTX JSON output expectations.
    fn render_data_element_value(
        &mut self,
        scope: Scope<'_, '_>,
        element: &Element<'_>,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        if self.try_write_leaf_value(scope, element, ovr, true)? {
            return Ok(());
        }
        let (has_text, has_element_child) = content_layout(&scope, element, ovr);

        if !has_text && !has_element_child {
            return self.write_bytes(b"\"\"");
        }

        if has_element_child {
            self.write_element_body_json(scope, element, ovr, false, true, has_text)
        } else {
            self.render_content_as_json_value(scope, &element.children, false, ovr)
        }
    }

    /// Fast path for the dominant leaf shape: one child node, no element
    /// children. Resolves the child once and writes the JSON value directly.
    ///
    /// Returns `false` (nothing written) for shapes that need the general path.
    /// `empty_as_string` selects the `Data`-element convention (`""`) over `null`.
    fn try_write_leaf_value(
        &mut self,
        scope: Scope<'_, '_>,
        element: &Element<'_>,
        ovr: Option<&Ovr<'_>>,
        empty_as_string: bool,
    ) -> Result<bool> {
        if element.has_element_child || element.children.len() != 1 {
            return Ok(false);
        }
        let empty: &[u8] = if empty_as_string { b"\"\"" } else { b"null" };
        match scope.resolve(&element.children[0], ovr)? {
            RNode::Skip => {
                self.write_bytes(empty)?;
                Ok(true)
            }
            RNode::Text(text) => {
                self.write_leaf_text(&text, empty)?;
                Ok(true)
            }
            RNode::Value(value) => {
                self.write_leaf_value(value, empty)?;
                Ok(true)
            }
            RNode::OwnValue(value) => {
                self.write_leaf_value(&value, empty)?;
                Ok(true)
            }
            RNode::Plain(Node::Text(text)) => {
                self.write_leaf_text(text, empty)?;
                Ok(true)
            }
            RNode::Plain(Node::Value(value)) => {
                self.write_leaf_value(value, empty)?;
                Ok(true)
            }
            // Element-like, CData/CharRef/EntityRef/PI, and error shapes take
            // the general path.
            _ => Ok(false),
        }
    }

    fn write_leaf_text(&mut self, text: &Text<'_>, empty: &[u8]) -> Result<()> {
        if text.is_empty() {
            return self.write_bytes(empty);
        }
        self.write_byte(b'\"')?;
        self.write_json_text(text)?;
        self.write_byte(b'\"')
    }

    fn write_leaf_value(&mut self, value: &BinXmlValue<'_>, empty: &[u8]) -> Result<()> {
        if is_optional_empty(value) {
            return self.write_bytes(empty);
        }
        if self.write_value_as_number(value)? {
            return Ok(());
        }
        self.write_byte(b'\"')?;
        self.values.write_json_value_text(self.writer, value)?;
        self.write_byte(b'\"')
    }

    /// Render an element value when attributes are handled elsewhere (separate-attrs mode).
    ///
    /// Shape rules:
    /// - no children + no text → `null`
    /// - only text/value nodes → scalar (string/number/bool)
    /// - any child elements → object
    fn write_element_value_no_attrs(
        &mut self,
        scope: Scope<'_, '_>,
        element: &Element<'_>,
        ovr: Option<&Ovr<'_>>,
        child_is_container: bool,
    ) -> Result<()> {
        if self.try_write_leaf_value(scope, element, ovr, false)? {
            return Ok(());
        }
        let (has_text, has_element_child) = content_layout(&scope, element, ovr);

        if !has_element_child && !has_text {
            self.write_bytes(b"null")
        } else if !has_element_child {
            self.render_content_as_json_value(scope, &element.children, false, ovr)
        } else {
            self.write_element_body_json(scope, element, ovr, child_is_container, true, has_text)
        }
    }

    /// Render an element value in the default mode (attributes inline under `#attributes`).
    fn write_element_value(
        &mut self,
        scope: Scope<'_, '_>,
        element: &Element<'_>,
        ovr: Option<&Ovr<'_>>,
        child_is_container: bool,
    ) -> Result<()> {
        if element.attrs.is_empty() && self.try_write_leaf_value(scope, element, ovr, false)? {
            return Ok(());
        }
        let (has_text, has_element_child) = content_layout(&scope, element, ovr);
        let (_has_attrs_any, has_attrs_text) = attr_flags(&scope, &element.attrs, ovr);

        if !has_element_child && !has_text && !has_attrs_text {
            self.write_bytes(b"null")
        } else if !has_element_child && !has_attrs_text {
            self.render_content_as_json_value(scope, &element.children, false, ovr)
        } else {
            self.write_element_body_json(scope, element, ovr, child_is_container, false, has_text)
        }
    }

    /// Render an element in "object form".
    ///
    /// This is the core routine: it emits `#attributes`, `#text`, the
    /// `EventData`/`UserData` special forms, and suffixed child keys. Array
    /// expansion is applied per child element; each copy behaves exactly like a
    /// separate sibling (suffix counting, positional `Data` items, named pairs).
    fn write_element_body_json<'t, 'a>(
        &mut self,
        scope: Scope<'t, 'a>,
        element: &'t Element<'a>,
        ovr: Option<&Ovr<'_>>,
        in_data_container: bool,
        omit_attributes: bool,
        has_text: bool,
    ) -> Result<()> {
        // Detect whether `EventData`/`UserData` should be flattened into Name-keyed pairs.
        //
        // Important nuance: we only need to see *one* non-empty `Data[@Name]` (on any
        // expansion copy) to select the named/flattened form. Once selected, unnamed
        // `Data` nodes are skipped.
        let mut should_flatten_named_data = false;
        if in_data_container {
            for node in &element.children {
                let Some(ce) = resolve_child_element(scope, node, ovr)? else {
                    continue;
                };
                if !is_data_element(ce.element.name.as_str()) {
                    continue;
                }
                let Some(name_nodes) = get_name_attr_nodes(ce.element) else {
                    continue;
                };
                let child_scope = ce.scope;
                let has_named =
                    if ce.expand && child_scope.ctx.as_ref().is_some_and(|ctx| ctx.may_expand) {
                        expansion_any(&child_scope, ce.element, ovr, &mut |copy_ovr| {
                            Ok(has_non_empty_text_content(
                                &child_scope,
                                name_nodes,
                                copy_ovr,
                            ))
                        })?
                    } else {
                        let copy_ovr = if ce.expand { ovr } else { None };
                        has_non_empty_text_content(&child_scope, name_nodes, copy_ovr)
                    };
                if has_named {
                    should_flatten_named_data = true;
                    break;
                }
            }
        }

        // Count child element names on the fly so we can apply legacy `_N` suffixes
        // (Header, Header_1, Header_2, ...).
        let mut name_counts = NameCounts {
            inline: std::array::from_fn(|_| None),
            num_inline: 0,
            spill: Vec::new(),
        };

        self.write_byte(b'{')?;
        let mut wrote_any = false;

        if !omit_attributes
            && !element.attrs.is_empty()
            && self.render_attributes_object(scope, &element.attrs, ovr)?
        {
            wrote_any = true;
        }

        // If we're emitting an object, any non-element content becomes `#text`.
        if has_text {
            if wrote_any {
                self.write_byte(b',')?;
            }
            wrote_any = true;
            self.write_bytes(b"\"#text\":")?;
            self.render_content_as_json_value(scope, &element.children, true, ovr)?;
        }

        // Pre-count positional `Data` nodes (including expansion copies) for the
        // non-flattened container case: we only emit `"Data": { "#text": ... }` once,
        // and its scalar-vs-array shape depends on the total count.
        let positional_data_count = if in_data_container && !should_flatten_named_data {
            let mut count = 0usize;
            for node in &element.children {
                let Some(ce) = resolve_child_element(scope, node, ovr)? else {
                    continue;
                };
                if !is_data_element(ce.element.name.as_str()) {
                    continue;
                }
                count += if ce.expand {
                    count_expansion_copies(&ce.scope, ce.element, ovr)
                } else {
                    1
                };
            }
            count
        } else {
            0
        };
        let mut positional_data_emitted = false;

        for node in &element.children {
            let Some(ce) = resolve_child_element(scope, node, ovr)? else {
                continue;
            };
            let child = ce.element;
            let child_scope = ce.scope;
            // Expansion only applies under a template ctx with arrays present.
            let expansion = if ce.expand {
                child_scope
                    .ctx
                    .as_ref()
                    .and_then(|ctx| find_expansion(child, ctx, ovr))
            } else {
                None
            };
            let direct_ovr = if ce.expand { ovr } else { None };

            // EventData/UserData special-case.
            if in_data_container && is_data_element(child.name.as_str()) {
                if should_flatten_named_data {
                    let Some(name_nodes) = get_name_attr_nodes(child) else {
                        continue;
                    };
                    if expansion.is_none() {
                        self.emit_named_data_copy(
                            child_scope,
                            child,
                            name_nodes,
                            direct_ovr,
                            &mut wrote_any,
                        )?;
                    } else {
                        let emitter = &mut *self;
                        let wrote_any_ref = &mut wrote_any;
                        for_each_expansion(&child_scope, child, ovr, &mut |copy_ovr| {
                            emitter.emit_named_data_copy(
                                child_scope,
                                child,
                                name_nodes,
                                copy_ovr,
                                wrote_any_ref,
                            )
                        })?;
                    }
                } else if !positional_data_emitted && positional_data_count > 0 {
                    if wrote_any {
                        self.write_byte(b',')?;
                    }
                    wrote_any = true;
                    positional_data_emitted = true;

                    self.write_bytes(b"\"Data\":{")?;
                    self.write_bytes(b"\"#text\":")?;
                    if positional_data_count == 1 {
                        // A single copy: expansion cannot apply (it implies >1 copies).
                        self.render_data_element_value(child_scope, child, direct_ovr)?;
                    } else {
                        self.write_byte(b'[')?;
                        let mut first = true;
                        for node2 in &element.children {
                            let Some(ce2) = resolve_child_element(scope, node2, ovr)? else {
                                continue;
                            };
                            if !is_data_element(ce2.element.name.as_str()) {
                                continue;
                            }
                            let candidate = ce2.element;
                            let candidate_scope = ce2.scope;
                            let item_ovr = if ce2.expand { ovr } else { None };
                            let emitter = &mut *self;
                            let mut emit_item = |copy_ovr: Option<&Ovr<'_>>| -> Result<()> {
                                if !first {
                                    emitter.write_byte(b',')?;
                                }
                                first = false;
                                emitter.render_data_element_value(
                                    candidate_scope,
                                    candidate,
                                    copy_ovr,
                                )
                            };
                            if ce2.expand {
                                for_each_expansion(
                                    &candidate_scope,
                                    candidate,
                                    ovr,
                                    &mut emit_item,
                                )?;
                            } else {
                                emit_item(item_ovr)?;
                            }
                        }
                        self.write_byte(b']')?;
                    }
                    self.write_byte(b'}')?;
                }
                continue;
            }

            // Normal child element: apply `_N` suffixes, once per expansion copy.
            if expansion.is_none() {
                self.emit_normal_child(
                    child_scope,
                    child,
                    direct_ovr,
                    &mut name_counts,
                    &mut wrote_any,
                )?;
            } else {
                let emitter = &mut *self;
                let name_counts_ref = &mut name_counts;
                let wrote_any_ref = &mut wrote_any;
                for_each_expansion(&child_scope, child, ovr, &mut |copy_ovr| {
                    emitter.emit_normal_child(
                        child_scope,
                        child,
                        copy_ovr,
                        name_counts_ref,
                        wrote_any_ref,
                    )
                })?;
            }
        }

        self.write_byte(b'}')?;
        Ok(())
    }

    /// Emit one `"<Name>": <value>` pair for a named `Data` element copy
    /// (skipped when this copy's `Name` is empty).
    fn emit_named_data_copy(
        &mut self,
        scope: Scope<'_, '_>,
        child: &Element<'_>,
        name_nodes: &[Node<'_>],
        copy_ovr: Option<&Ovr<'_>>,
        wrote_any: &mut bool,
    ) -> Result<()> {
        if !has_non_empty_text_content(&scope, name_nodes, copy_ovr) {
            return Ok(());
        }
        if *wrote_any {
            self.write_byte(b',')?;
        }
        *wrote_any = true;
        self.write_json_key_from_nodes(scope, name_nodes, copy_ovr)?;
        self.render_data_element_value(scope, child, copy_ovr)
    }

    /// Emit one suffixed `"<name>[_N]": <value>` member for a child element copy.
    fn emit_normal_child<'t, 'a>(
        &mut self,
        scope: Scope<'t, 'a>,
        child: &'t Element<'a>,
        copy_ovr: Option<&Ovr<'_>>,
        name_counts: &mut NameCounts<'t>,
        wrote_any: &mut bool,
    ) -> Result<()> {
        let suffix = name_counts.next_suffix(NameKey::from_name(&child.name));

        if *wrote_any {
            self.write_byte(b',')?;
        }
        *wrote_any = true;

        let child_is_container = is_data_container(child.name.as_str());
        if self.separate_json_attributes {
            // Emit `<name>_attributes` sibling before the value, matching legacy output.
            let wrote_attrs =
                self.render_separate_attributes_for_element(scope, child, suffix, copy_ovr)?;

            // Omit `<name>: null` when the element only contains attributes.
            let (has_text, has_element_child) = content_layout(&scope, child, copy_ovr);
            let child_has_value = has_element_child || has_text;
            let write_value = child_has_value || !wrote_attrs;

            if wrote_attrs && write_value {
                self.write_byte(b',')?;
            }
            if write_value {
                self.write_json_key_from_name_with_suffix(&child.name, suffix)?;
                self.write_element_value_no_attrs(scope, child, copy_ovr, child_is_container)?;
            }
        } else {
            self.write_json_key_from_name_with_suffix(&child.name, suffix)?;
            self.write_element_value(scope, child, copy_ovr, child_is_container)?;
        }
        Ok(())
    }
}

/// Compiled-template helpers: render materialized (placeholder-free) pieces
/// with the real emitter so compile-time literal output is parity by
/// construction.
pub(crate) fn render_json_element_value_materialized(
    arena: &IrArena<'_>,
    element: &Element<'_>,
    child_is_container: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(out, false);
    emitter.write_element_value(
        Scope::materialized(arena),
        element,
        None,
        child_is_container,
    )?;
    emitter.flush()
}

/// Render one literal attribute as a `"name":value` member (no commas).
pub(crate) fn render_json_attr_member_materialized(
    arena: &IrArena<'_>,
    attr: &Attr<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(out, false);
    let scope = Scope::materialized(arena);
    emitter.write_byte(b'\"')?;
    emitter.write_name(attr.name.as_str())?;
    emitter.write_bytes(b"\":")?;
    if !emitter.try_write_as_number(scope, &attr.value, None)? {
        emitter.render_text_to_json_string(scope, &attr.value, false, None)?;
    }
    emitter.flush()
}

/// Render a full `"#attributes":{...}` member for literal attributes.
pub(crate) fn render_json_attributes_object_materialized(
    arena: &IrArena<'_>,
    attrs: &[Attr<'_>],
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(out, false);
    emitter.render_attributes_object(Scope::materialized(arena), attrs, None)?;
    emitter.flush()
}

/// Render a JSON object key (escaped) from literal nodes, with trailing `:`.
pub(crate) fn render_json_key_from_nodes_materialized(
    arena: &IrArena<'_>,
    nodes: &[Node<'_>],
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(out, false);
    emitter.write_json_key_from_nodes(Scope::materialized(arena), nodes, None)?;
    emitter.flush()
}

/// Render a literal `<Data>` element's JSON value (`""` when empty).
pub(crate) fn render_json_data_value_materialized(
    arena: &IrArena<'_>,
    element: &Element<'_>,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(out, false);
    emitter.render_data_element_value(Scope::materialized(arena), element, None)?;
    emitter.flush()
}

/// Benchmark-only helper to measure JSON text rendering.
#[cfg(feature = "bench")]
pub(crate) fn bench_write_json_text_content<'a, W: WriteExt>(
    writer: &mut W,
    arena: &'a IrArena<'a>,
    nodes: &[Node<'a>],
) -> Result<()> {
    let mut emitter = JsonEmitter::new(writer, false);
    emitter.write_json_text_content(Scope::materialized(arena), nodes, false, None)?;
    emitter.flush()
}

fn is_data_container(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn is_data_element(name: &str) -> bool {
    name == "Data"
}
