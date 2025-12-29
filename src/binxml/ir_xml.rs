//! XML rendering for the IR element tree.
//!
//! This module provides a fast, allocation-light XML renderer that operates
//! directly on the IR (`model::ir`). It is the XML counterpart to the JSON
//! streaming renderer in `binxml::ir_json` and intentionally avoids building any
//! intermediate XML model or token stream.
//!
//! The renderer writes directly to an `io::Write` sink, escaping text and value
//! nodes on the fly. It assumes the IR tree is fully resolved (no placeholders)
//! and reports structural errors if invalid node types appear in attribute
//! contexts.
//!
//! Rendering rules:
//! - Element/attribute names are emitted as-is (names are already validated).
//! - Text and values are XML-escaped.
//! - Entity references are preserved as `&name;`.
//! - CDATA nodes are emitted verbatim (outside of attributes).
//! - Optional indentation is controlled by `ParserSettings::should_indent()`.

use crate::ParserSettings;
use crate::binxml::value_render::ValueRenderer;
use crate::err::{EvtxError, Result};
use crate::model::ir::{ElementId, IrArena, IrTree, Name, Node, Text, is_optional_empty};
use crate::utils::Utf16LeSlice;
use sonic_rs::writer::WriteExt;

const INDENT_WIDTH: usize = 2;

/// Render a single record element to XML.
pub(crate) fn render_xml_record<W: WriteExt>(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    // Keep output stable (matches snapshot tests / legacy formatting).
    writer.write_all(b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n")?;
    let mut emitter = XmlEmitter::new(writer, settings.should_indent(), tree.arena());
    emitter.render_element(tree.root(), 0)?;
    Ok(())
}

/// Streaming XML emitter for IR nodes.
///
/// The emitter owns indentation state and writes escaped content directly to
/// the underlying writer without allocating intermediate strings.
struct XmlEmitter<'w, 'a, W: WriteExt> {
    writer: &'w mut W,
    indent: bool,
    arena: &'a IrArena<'a>,
    scratch: utf16_simd::Scratch,
    values: ValueRenderer,
}

impl<'w, 'a, W: WriteExt> XmlEmitter<'w, 'a, W> {
    fn new(writer: &'w mut W, indent: bool, arena: &'a IrArena<'a>) -> Self {
        XmlEmitter {
            writer,
            indent,
            arena,
            scratch: utf16_simd::Scratch::new(),
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
        let escaped = self.scratch.escape_xml_utf16le(bytes, units, in_attribute);
        self.writer.write_all(escaped)?;
        Ok(())
    }

    fn write_utf16_raw(&mut self, value: Utf16LeSlice<'_>) -> Result<()> {
        let bytes = value.as_bytes();
        let units = bytes.len() / 2;
        if units == 0 {
            return Ok(());
        }
        let raw = self.scratch.escape_utf16le_raw(bytes, units);
        self.writer.write_all(raw)?;
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

    fn render_element(&mut self, element_id: ElementId, indent: usize) -> Result<()> {
        let arena = self.arena;
        let element = arena.get(element_id).expect("invalid element id");
        self.write_indent(indent)?;
        self.write_bytes(b"<")?;
        self.write_name(&element.name)?;

        for attr in &element.attrs {
            if self.attribute_value_is_empty(&attr.value)? {
                continue;
            }
            self.write_bytes(b" ")?;
            self.write_name(&attr.name)?;
            self.write_bytes(b"=\"")?;
            self.render_nodes(&attr.value, true)?;
            self.write_bytes(b"\"")?;
        }

        self.write_bytes(b">")?;

        if element.children.is_empty() {
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

        if !element.has_element_child {
            self.render_nodes(&element.children, false)?;
            self.write_close_tag(&element.name)?;
            self.write_newline()?;
            return Ok(());
        }

        self.write_newline()?;

        for node in &element.children {
            match node {
                Node::Element(child_id) => {
                    self.render_element(*child_id, indent + INDENT_WIDTH)?;
                }
                _ => {
                    self.write_indent(indent + INDENT_WIDTH)?;
                    self.render_single_node(node, false)?;
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

    fn attribute_value_is_empty(&self, nodes: &[Node<'_>]) -> Result<bool> {
        for node in nodes {
            match node {
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
            }
        }
        Ok(true)
    }

    fn render_nodes(&mut self, nodes: &[Node<'_>], in_attribute: bool) -> Result<()> {
        let mut idx = 0;
        while idx < nodes.len() {
            let node = &nodes[idx];
            match node {
                Node::PITarget(name) => {
                    if in_attribute {
                        return Err(EvtxError::Unimplemented {
                            name: "processing instruction in attribute value".to_string(),
                        });
                    }
                    let next = nodes.get(idx + 1);
                    match next {
                        Some(Node::PIData(data)) => {
                            self.write_bytes(b"<?")?;
                            self.write_name(name)?;
                            self.write_bytes(b" ")?;
                            self.write_text_raw(data)?;
                            self.write_bytes(b"?>")?;
                            idx += 2;
                            continue;
                        }
                        _ => {
                            self.write_bytes(b"<?")?;
                            self.write_name(name)?;
                            self.write_bytes(b"?>")?;
                        }
                    }
                }
                Node::PIData(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "PIData without PITarget",
                    ));
                }
                _ => {
                    self.render_single_node(node, in_attribute)?;
                }
            }
            idx += 1;
        }
        Ok(())
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
