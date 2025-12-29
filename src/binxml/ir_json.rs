//! JSON rendering for BinXML IR trees.
//!
//! This module turns the IR (`model::ir`) into JSON using a streaming renderer
//! that writes directly to a `WriteExt` sink. It avoids building any
//! intermediate JSON representation and matches the EVTX JSON conventions used
//! by the CLI:
//!
//! - Element names become object keys.
//! - Attributes are emitted under the `#attributes` object.
//! - Text/value nodes are serialized as JSON strings (or numbers when the
//!   content is a single numeric value).
//! - `EventData`/`UserData` containers are flattened to the `Data` elements
//!   they contain.
//! - Templates are instantiated during IR build; resolved trees contain no placeholders.
//!
//! The renderer keeps scratch buffers for JSON escaping, number formatting, and
//! datetime formatting to minimize allocations during hot loops.

use crate::ParserSettings;
use crate::binxml::value_render::ValueRenderer;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{Attr, Element, IrArena, IrTree, Name, Node, Text, is_optional_empty};
use crate::utils::Utf16LeSlice;
use sonic_rs::format::{CompactFormatter, Formatter};
use sonic_rs::writer::WriteExt;

const MAX_UNIQUE_NAMES: usize = 64;

/// Render a single record tree to JSON.
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

    fn eql(self, other: NameKey<'a>) -> bool {
        if self.bytes.as_ptr() == other.bytes.as_ptr() && self.bytes.len() == other.bytes.len() {
            return true;
        }
        self.bytes == other.bytes
    }
}

/// Tracks how often a child name appears so arrays are emitted once.
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

    fn write_json_key_from_nodes(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(nodes)?;
        self.write_bytes(b"\":")
    }

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

    fn try_write_as_number(&mut self, nodes: &[Node<'_>]) -> Result<bool> {
        if nodes.len() != 1 {
            return Ok(false);
        }
        let Node::Value(value) = &nodes[0] else {
            return Ok(false);
        };
        self.write_value_as_number(value)
    }

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

    fn write_signed_number(&mut self, value: i64) -> Result<bool> {
        self.formatter
            .write_i64(self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    fn write_unsigned_number(&mut self, value: u64) -> Result<bool> {
        self.formatter
            .write_u64(self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    fn render_text_to_json_string(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content(nodes)?;
        self.write_byte(b'\"')
    }

    fn render_text_to_json_string_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'\"')?;
        self.write_json_text_content_skip_elements(nodes)?;
        self.write_byte(b'\"')
    }

    fn render_content_as_json_value(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        if self.try_write_as_number(nodes)? {
            return Ok(());
        }
        self.render_text_to_json_string(nodes)
    }

    fn render_content_as_json_value_skip_elements(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        if self.try_write_as_number_skip_elements(nodes)? {
            return Ok(());
        }
        self.render_text_to_json_string_skip_elements(nodes)
    }

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

    fn write_element_body_json(
        &mut self,
        element: &Element<'_>,
        in_data_container: bool,
        omit_attributes: bool,
    ) -> Result<()> {
        let arena = self.arena;

        // Detect whether `EventData`/`UserData` can be flattened into Name-keyed pairs.
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

        // Count child element names on the fly so we can apply legacy `_N` suffixes (Header, Header_1, ...).
        let mut name_counts: [Option<NameCount<'_>>; MAX_UNIQUE_NAMES] =
            std::array::from_fn(|_| None);
        let mut num_unique = 0usize;

        self.write_byte(b'{')?;
        let mut wrote_any = false;

        if !omit_attributes {
            if !element.attrs.is_empty() && self.render_attributes_object(&element.attrs)? {
                wrote_any = true;
            }
        }

        // If we're emitting an object, any non-element content becomes `#text`.
        if self.has_non_empty_text_content(&element.children) {
            if wrote_any {
                self.write_byte(b',')?;
            }
            wrote_any = true;
            self.write_bytes(b"\"#text\":")?;
            self.render_content_as_json_value_skip_elements(&element.children)?;
        }

        // Pre-count positional `Data` nodes for the non-flattened container case.
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
                    // Some EventData containers encode positional values as a single `StringArrayType`
                    // substitution within one `<Data>` element. For JSON, preserve the original
                    // item boundaries by emitting an array of strings.
                    if positional_data_count == 1 {
                        if let Some(items) = extract_string_array_value(&child.children) {
                            self.write_byte(b'[')?;
                            let mut first = true;
                            for item in items.iter() {
                                if !first {
                                    self.write_byte(b',')?;
                                }
                                first = false;
                                self.write_byte(b'\"')?;
                                self.write_json_escaped_utf16(*item)?;
                                self.write_byte(b'\"')?;
                            }
                            self.write_byte(b']')?;
                        } else {
                            self.render_data_element_value(child)?;
                        }
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

fn extract_string_array_value<'a>(nodes: &[Node<'a>]) -> Option<&'a [Utf16LeSlice<'a>]> {
    use crate::model::ir::is_optional_empty;

    let mut found: Option<&'a [Utf16LeSlice<'a>]> = None;
    for node in nodes {
        match node {
            Node::Element(_) => return None,
            Node::PITarget(_) | Node::PIData(_) => {}
            Node::Text(text) | Node::CData(text) => {
                if !text.is_empty() {
                    return None;
                }
            }
            Node::Value(value) => {
                if is_optional_empty(value) {
                    continue;
                }
                match value {
                    BinXmlValue::StringArrayType(items) => {
                        if found.is_some() {
                            return None;
                        }
                        found = Some(items);
                    }
                    _ => return None,
                }
            }
            Node::CharRef(_) | Node::EntityRef(_) => return None,
            Node::Placeholder(_) => return None,
        }
    }
    found
}
