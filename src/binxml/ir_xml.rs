//! XML rendering for the IR element tree.
//!
//! This module provides a fast, allocation-light XML renderer that operates
//! directly on the IR (`model::ir`). It is the XML counterpart to the JSON
//! streaming renderer in `binxml::ir_json` and intentionally avoids building any
//! intermediate XML model or token stream.
//!
//! The renderer writes directly to an `io::Write` sink, escaping text and value
//! nodes on the fly. It renders either a fully materialized tree or a cached
//! template definition tree with render-time substitution resolution (see
//! `binxml::render_ctx`), including render-time array expansion.
//!
//! Rendering rules:
//! - Element/attribute names are emitted as-is (names are already validated).
//! - Text and values are XML-escaped.
//! - Entity references are preserved as `&name;`.
//! - CDATA nodes are emitted verbatim (outside of attributes).
//! - Optional indentation is controlled by `ParserSettings::should_indent()`.

use crate::ParserSettings;
use crate::binxml::ir::RecordContent;
use crate::binxml::render_ctx::{Ovr, RNode, Scope, child_layout, find_expansion};
use crate::binxml::value_render::ValueRenderer;
use crate::err::{EvtxError, Result};
use crate::model::ir::{ElementId, IrTree, Name, Node, Text, is_optional_empty};
use crate::utils::Utf16LeSlice;
use sonic_rs::writer::WriteExt;

const INDENT_WIDTH: usize = 2;
const XML_DECL: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

/// Render a single record element to XML from a materialized tree.
pub(crate) fn render_xml_record<W: WriteExt>(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    // Keep output stable (matches snapshot tests / legacy formatting).
    writer.write_all(XML_DECL)?;
    let mut emitter = XmlEmitter::new(writer, settings.should_indent());
    emitter.render_element(Scope::materialized(tree.arena()), tree.root(), 0, None)?;
    Ok(())
}

/// Render record content (materialized tree or unmaterialized template instance).
pub(crate) fn render_xml_record_content<W: WriteExt>(
    content: &RecordContent<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    match content {
        RecordContent::Tree(tree) => render_xml_record(tree, settings, writer),
        RecordContent::Template(tc) => {
            writer.write_all(XML_DECL)?;
            let mut emitter = XmlEmitter::new(writer, settings.should_indent());
            // The instantiation root is never array-expanded (matches the
            // materialized path, which discards the root expansion flag).
            emitter.render_element(tc.scope(), tc.root.template.root(), 0, None)?;
            Ok(())
        }
    }
}

/// Streaming XML emitter for IR nodes.
///
/// The emitter owns indentation state and writes escaped content directly to
/// the underlying writer without allocating intermediate strings.
struct XmlEmitter<'w, W: WriteExt> {
    writer: &'w mut W,
    indent: bool,
    values: ValueRenderer,
}

impl<'w, W: WriteExt> XmlEmitter<'w, W> {
    fn new(writer: &'w mut W, indent: bool) -> Self {
        XmlEmitter {
            writer,
            indent,
            values: ValueRenderer::new(),
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        Ok(())
    }

    fn write_utf16_escaped(&mut self, value: Utf16LeSlice<'_>, in_attribute: bool) -> Result<()> {
        let bytes = value.as_bytes();
        let units = bytes.len() / 2;
        if units == 0 {
            return Ok(());
        }
        utf16_simd::write_xml_utf16le(self.writer, bytes, units, in_attribute)?;
        Ok(())
    }

    fn write_utf16_raw(&mut self, value: Utf16LeSlice<'_>) -> Result<()> {
        let bytes = value.as_bytes();
        let units = bytes.len() / 2;
        if units == 0 {
            return Ok(());
        }
        utf16_simd::write_utf16le_raw(self.writer, bytes, units)?;
        Ok(())
    }

    fn write_indent(&mut self, level: usize) -> Result<()> {
        if !self.indent {
            return Ok(());
        }
        for _ in 0..level {
            self.write_bytes(b" ")?;
        }
        Ok(())
    }

    fn write_newline(&mut self) -> Result<()> {
        if self.indent {
            self.write_bytes(b"\n")?;
        }
        Ok(())
    }

    /// Render a child element, applying render-time array expansion
    /// (containing-element repetition; cross-product via recursion).
    fn render_child_element(
        &mut self,
        scope: Scope<'_, '_>,
        element_id: ElementId,
        indent: usize,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        if let Some(ctx) = &scope.ctx {
            let element = scope.arena.get(element_id).expect("invalid element id");
            if let Some((slot, len)) = find_expansion(element, ctx, ovr) {
                for idx in 0..len {
                    let frame = Ovr::frame(slot, idx, ovr);
                    self.render_child_element(scope, element_id, indent, Some(&frame))?;
                }
                return Ok(());
            }
        }
        self.render_element(scope, element_id, indent, ovr)
    }

    fn render_element(
        &mut self,
        scope: Scope<'_, '_>,
        element_id: ElementId,
        indent: usize,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        let element = scope.arena.get(element_id).expect("invalid element id");
        self.write_indent(indent)?;
        self.write_bytes(b"<")?;
        self.write_name(&element.name)?;

        for attr in &element.attrs {
            if self.attribute_value_is_empty(scope, &attr.value, ovr)? {
                continue;
            }
            self.write_bytes(b" ")?;
            self.write_name(&attr.name)?;
            self.write_bytes(b"=\"")?;
            self.render_nodes(scope, &attr.value, true, ovr)?;
            self.write_bytes(b"\"")?;
        }

        self.write_bytes(b">")?;

        let (logically_empty, has_element_child) = child_layout(&scope, element, ovr);

        if logically_empty {
            // Preserve legacy formatting: most empty elements are rendered as:
            //   <Tag ...>
            //   </Tag>
            //
            // But `<Binary>` is emitted on a single line to match existing snapshots.
            if element.name.as_str() == "Binary" {
                self.write_close_tag(&element.name)?;
                self.write_newline()?;
            } else {
                self.write_newline()?;
                self.write_indent(indent)?;
                self.write_close_tag(&element.name)?;
                self.write_newline()?;
            }
            return Ok(());
        }

        if !has_element_child {
            self.render_nodes(scope, &element.children, false, ovr)?;
            self.write_close_tag(&element.name)?;
            self.write_newline()?;
            return Ok(());
        }

        self.write_newline()?;

        for node in &element.children {
            match scope.resolve(node, ovr)? {
                RNode::Skip => {}
                RNode::Plain(Node::Element(child_id)) => {
                    self.render_child_element(scope, *child_id, indent + INDENT_WIDTH, ovr)?;
                }
                RNode::Frag(child_id) => {
                    self.render_element(scope.frag_scope(), child_id, indent + INDENT_WIDTH, None)?;
                }
                RNode::Nested(idx) => {
                    let (nested_scope, root) = scope.nested_scope_root(idx);
                    self.render_element(nested_scope, root, indent + INDENT_WIDTH, None)?;
                }
                rnode => {
                    self.write_indent(indent + INDENT_WIDTH)?;
                    self.render_rnode(&rnode, false)?;
                    self.write_newline()?;
                }
            }
        }

        self.write_indent(indent)?;
        self.write_close_tag(&element.name)?;
        self.write_newline()?;
        Ok(())
    }

    fn write_name(&mut self, name: &Name<'_>) -> Result<()> {
        self.write_bytes(name.as_str().as_bytes())
    }

    fn write_close_tag(&mut self, name: &Name<'_>) -> Result<()> {
        self.write_bytes(b"</")?;
        self.write_name(name)?;
        self.write_bytes(b">")
    }

    fn attribute_value_is_empty(
        &self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        ovr: Option<&Ovr<'_>>,
    ) -> Result<bool> {
        for node in nodes {
            match scope.resolve(node, ovr)? {
                RNode::Skip => {}
                RNode::Text(text) => {
                    if !text.is_empty() {
                        return Ok(false);
                    }
                }
                RNode::Value(value) => {
                    if !is_optional_empty(value) {
                        return Ok(false);
                    }
                }
                RNode::OwnValue(value) => {
                    if !is_optional_empty(&value) {
                        return Ok(false);
                    }
                }
                RNode::Frag(_) | RNode::Nested(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "element node inside attribute value",
                    ));
                }
                RNode::Plain(node) => match node {
                    Node::Text(text) => {
                        if !text.is_empty() {
                            return Ok(false);
                        }
                    }
                    Node::Value(value) => {
                        if !is_optional_empty(value) {
                            return Ok(false);
                        }
                    }
                    Node::EntityRef(_) | Node::CharRef(_) | Node::CData(_) => {
                        return Ok(false);
                    }
                    Node::Element(_) => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "element node inside attribute value",
                        ));
                    }
                    Node::PITarget(_) | Node::PIData(_) => {
                        return Err(EvtxError::Unimplemented {
                            name: "processing instruction in attribute value".to_string(),
                        });
                    }
                    Node::Placeholder(_) => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "unresolved placeholder in attribute value",
                        ));
                    }
                },
            }
        }
        Ok(true)
    }

    fn render_nodes(
        &mut self,
        scope: Scope<'_, '_>,
        nodes: &[Node<'_>],
        in_attribute: bool,
        ovr: Option<&Ovr<'_>>,
    ) -> Result<()> {
        let mut idx = 0;
        while idx < nodes.len() {
            match scope.resolve(&nodes[idx], ovr)? {
                RNode::Plain(Node::PITarget(name)) => {
                    if in_attribute {
                        return Err(EvtxError::Unimplemented {
                            name: "processing instruction in attribute value".to_string(),
                        });
                    }
                    // Pair with the next (non-omitted) node when it is PIData.
                    let mut next_idx = idx + 1;
                    let mut data = None;
                    while next_idx < nodes.len() {
                        match scope.resolve(&nodes[next_idx], ovr)? {
                            RNode::Skip => next_idx += 1,
                            RNode::Plain(Node::PIData(pi_data)) => {
                                data = Some(*pi_data);
                                break;
                            }
                            _ => break,
                        }
                    }
                    self.write_bytes(b"<?")?;
                    self.write_name(name)?;
                    match data {
                        Some(pi_data) => {
                            self.write_bytes(b" ")?;
                            self.write_text_raw(&pi_data)?;
                            self.write_bytes(b"?>")?;
                            idx = next_idx + 1;
                            continue;
                        }
                        None => {
                            self.write_bytes(b"?>")?;
                        }
                    }
                }
                RNode::Plain(Node::PIData(_)) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "PIData without PITarget",
                    ));
                }
                rnode => {
                    self.render_rnode(&rnode, in_attribute)?;
                }
            }
            idx += 1;
        }
        Ok(())
    }

    fn render_rnode(&mut self, rnode: &RNode<'_, '_>, in_attribute: bool) -> Result<()> {
        match rnode {
            RNode::Skip => Ok(()),
            RNode::Plain(node) => self.render_single_node(node, in_attribute),
            RNode::Text(text) => self.write_text_escaped(text, in_attribute),
            RNode::Value(value) => {
                self.values
                    .write_xml_value_text(self.writer, value, in_attribute)
            }
            RNode::OwnValue(value) => {
                self.values
                    .write_xml_value_text(self.writer, value, in_attribute)
            }
            RNode::Frag(_) | RNode::Nested(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unexpected element node in text context",
            )),
        }
    }

    fn render_single_node(&mut self, node: &Node<'_>, in_attribute: bool) -> Result<()> {
        match node {
            Node::Element(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unexpected element node in text context",
            )),
            Node::Text(text) => self.write_text_escaped(text, in_attribute),
            Node::Value(value) => {
                self.values
                    .write_xml_value_text(self.writer, value, in_attribute)
            }
            Node::EntityRef(name) => {
                self.write_bytes(b"&")?;
                self.write_name(name)?;
                self.write_bytes(b";")
            }
            Node::CharRef(ch) => {
                let value = format!("&#{};", ch);
                self.write_bytes(value.as_bytes())
            }
            Node::CData(text) => {
                if in_attribute {
                    self.write_text_escaped(text, true)
                } else {
                    self.write_bytes(b"<![CDATA[")?;
                    self.write_text_raw(text)?;
                    self.write_bytes(b"]]>")
                }
            }
            Node::PITarget(_) | Node::PIData(_) => Ok(()),
            Node::Placeholder(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unresolved placeholder in tree",
            )),
        }
    }

    fn write_text_raw(&mut self, text: &Text<'_>) -> Result<()> {
        match text {
            Text::Utf16(value) => self.write_utf16_raw(*value),
            Text::Utf8(value) => self.write_bytes(value.as_bytes()),
        }
    }

    fn write_text_escaped(&mut self, text: &Text<'_>, in_attribute: bool) -> Result<()> {
        match text {
            Text::Utf16(value) => self.write_utf16_escaped(*value, in_attribute),
            Text::Utf8(value) => self.write_escaped_str(value.as_ref(), in_attribute),
        }
    }

    fn write_escaped_str(&mut self, text: &str, in_attribute: bool) -> Result<()> {
        for ch in text.chars() {
            match ch {
                '&' => self.write_bytes(b"&amp;")?,
                '<' => self.write_bytes(b"&lt;")?,
                '>' => self.write_bytes(b"&gt;")?,
                '"' if in_attribute => self.write_bytes(b"&quot;")?,
                '\'' if in_attribute => self.write_bytes(b"&apos;")?,
                _ => {
                    let mut buf = [0_u8; 4];
                    let slice = ch.encode_utf8(&mut buf).as_bytes();
                    self.write_bytes(slice)?;
                }
            }
        }
        Ok(())
    }
}
