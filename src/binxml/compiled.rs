//! Compiled per-template XML splice programs (round-4 fast path).
//!
//! Each cached template definition compiles once per (base indent, root?) key
//! into a flat program: owned literal output bytes interleaved with
//! substitution ops. Per-record rendering is then a descriptor scan (no
//! `BinXmlValue` materialization, no IR walk, no render-time scans) plus a
//! linear op loop that formats values straight from chunk bytes into the
//! output buffer.
//!
//! Coverage is deliberately partial: the compiler bails on shapes whose
//! output depends on values in ways the op set doesn't model (array
//! expansion, processing instructions, multi-placeholder content,
//! runtime-forked layouts), and the per-record pre-flight bails on anything
//! irregular (mis-sized scalars, unknown or array types, non-EOF trailers).
//! Every bail routes the record through the existing render-direct path,
//! which remains the behavioral source of truth. The pre-flight runs before
//! any output is written, so the executor never unwinds a partial record.

use crate::ParserSettings;
use crate::binxml::ir::{IrTemplateCache, build_tree_from_binxml_bytes_direct};
use crate::binxml::value_render::{StringEscapeMode, ValueRenderer};
use crate::err::Result;
use crate::evtx_chunk::EvtxChunk;
use crate::model::ir::{Attr, Element, ElementId, IrTree, Node, Placeholder, Text};
use ahash::AHashMap;
use std::sync::Arc;

const INDENT_WIDTH: u16 = 2;
const XML_DECL: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

/// A `lits` byte range.
type LitRange = (u32, u32);
/// A `Preflight::slots` index range (one instance's slots).
type SlotRange = (u32, u32);

/// One compiled op. `slot` indexes the instance's substitution array.
#[derive(Debug, Clone)]
enum XOp {
    /// Emit `lits[range]`.
    Lit(LitRange),
    /// Escaped value text in an always-emitted context (attribute value with
    /// static non-empty parts). No emptiness branch.
    Val { slot: u16, in_attr: bool },
    /// ` name="` + escaped value + `"`, all omitted when the value is
    /// empty-ish (mirrors `attribute_value_is_empty`: optionality ignored).
    AttrVal { slot: u16, pre: LitRange },
    /// `<Tag ...attrs` has been emitted (sans `>`). Emit `>` and the single
    /// placeholder content, branching on the runtime slot class:
    /// Skip -> `tail_empty`; text-ish -> text + `tail_text`;
    /// element -> newline + nested/frag at `indent + 2` + `tail_elem`.
    Body {
        slot: u16,
        optional: bool,
        indent: u16,
        tail_text: LitRange,
        tail_empty: LitRange,
        tail_elem: LitRange,
    },
    /// Placeholder in element-child position under a statically line-formed
    /// parent. Skip -> nothing; element -> nested/frag at `indent`;
    /// text-ish -> `ind` + text + newline.
    ChildSlot {
        slot: u16,
        optional: bool,
        indent: u16,
        ind: LitRange,
    },
}

/// A compiled template program for one (def offset, base indent, root?) key.
pub(crate) struct XmlProgram {
    lits: Vec<u8>,
    ops: Vec<XOp>,
    indent_on: bool,
    /// `(slot, child_indent)` for every slot rendered in element position;
    /// the pre-flight uses this to resolve nested-instance programs up front.
    elem_slots: Vec<(u16, u16)>,
}

/// Per-chunk program cache. `None` marks templates that failed to compile so
/// they are not retried for every record.
pub(crate) type ProgramCache<P> = AHashMap<(u32, u16, bool), Option<Arc<P>>>;

/// Cross-chunk program store: templates are identical across chunks (same
/// GUID + size + definition bytes), so programs compile once per file/parser
/// instead of once per chunk. Shared across worker threads.
#[derive(Default)]
pub(crate) struct ProgramStore {
    xml: std::sync::RwLock<AHashMap<StoreKey, Option<Arc<XmlProgram>>>>,
    json: std::sync::RwLock<AHashMap<StoreKey, Option<Arc<JsonProgram>>>>,
    hasher: ahash::RandomState,
}

/// Content identity of a compiled program: template identity (GUID, size,
/// definition-bytes hash) plus the compile parameters.
type StoreKey = ([u8; 16], u32, u64, u16, bool);

impl std::fmt::Debug for ProgramStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgramStore").finish_non_exhaustive()
    }
}

/// Per-format shard access used by `get_or_compile`.
pub(crate) trait StoredProgram: TemplateProgram {
    fn shard(store: &ProgramStore) -> &std::sync::RwLock<AHashMap<StoreKey, Option<Arc<Self>>>>;
}

impl StoredProgram for XmlProgram {
    fn shard(store: &ProgramStore) -> &std::sync::RwLock<AHashMap<StoreKey, Option<Arc<Self>>>> {
        &store.xml
    }
}

impl StoredProgram for JsonProgram {
    fn shard(store: &ProgramStore) -> &std::sync::RwLock<AHashMap<StoreKey, Option<Arc<Self>>>> {
        &store.json
    }
}

/// Per-chunk render state for the per-record APIs (`EvtxRecord::into_*`).
/// Fully owned (programs carry their own bytes), so `EvtxChunk` can hold it
/// behind a `RefCell` without self-referential lifetimes.
#[derive(Default)]
pub(crate) struct RenderCaches {
    pub(crate) xml: XmlProgramCache,
    pub(crate) json: JsonProgramCache,
    pub(crate) pf_xml: Preflight<XmlProgram>,
    pub(crate) pf_json: Preflight<JsonProgram>,
}

impl std::fmt::Debug for RenderCaches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderCaches")
            .field("xml_programs", &self.xml.len())
            .field("json_programs", &self.json.len())
            .finish()
    }
}
pub(crate) type XmlProgramCache = ProgramCache<XmlProgram>;
pub(crate) type JsonProgramCache = ProgramCache<JsonProgram>;

/// Per-slot validity constraint checked by the pre-flight (record falls back
/// when violated), so executors stay infallible.
#[derive(Debug, Clone, Copy)]
pub(crate) enum SlotConstraint {
    /// Slot must not be a non-empty embedded BinXml value (0x21).
    ForbidElem(u16),
    /// Slot must be a (single-instance) embedded BinXml value or empty.
    ElemOrEmpty(u16),
}

/// What the generic pre-flight needs from a compiled program.
pub(crate) trait TemplateProgram: Sized {
    /// Whether the executor can render non-instance BinXml fragments in
    /// element position (via a materialized fallback).
    const ALLOW_GENERIC_FRAGS: bool;
    /// `(slot, child_indent)` pairs rendered in element position.
    fn elem_slots(&self) -> &[(u16, u16)];
    fn constraints(&self) -> &[SlotConstraint] {
        &[]
    }
    fn compile(
        tree: &IrTree<'_>,
        has_literal_array: bool,
        base_indent: u16,
        is_root: bool,
        settings: &ParserSettings,
    ) -> Option<Self>;
}

impl TemplateProgram for XmlProgram {
    const ALLOW_GENERIC_FRAGS: bool = true;
    fn elem_slots(&self) -> &[(u16, u16)] {
        &self.elem_slots
    }
    fn compile(
        tree: &IrTree<'_>,
        has_literal_array: bool,
        base_indent: u16,
        is_root: bool,
        settings: &ParserSettings,
    ) -> Option<Self> {
        compile_xml_template(tree, has_literal_array, base_indent, is_root, settings)
    }
}

// ---------------------------------------------------------------------------
// Compilation (template definition IR -> program)
// ---------------------------------------------------------------------------

struct Bail;

struct XmlCompiler<'t, 'a> {
    tree: &'t IrTree<'a>,
    lits: Vec<u8>,
    ops: Vec<XOp>,
    /// Start of the not-yet-flushed literal run (`lits[run_start..]`).
    run_start: usize,
    elem_slots: Vec<(u16, u16)>,
    indent_on: bool,
    vr: ValueRenderer,
    /// Walking a fully materialized tree (slow lane / fragments): placeholder
    /// sites are errors instead of ops, and any error is a real record error.
    /// When false (template compilation), any error just means "not cacheable".
    materialized: bool,
}

/// Compile-lane bail sentinel (mapped to `None` by `compile_xml_template`).
fn bail_err() -> crate::err::EvtxError {
    crate::err::EvtxError::FailedToCreateRecordModel("compiled-template bail")
}

fn unresolved_placeholder() -> crate::err::EvtxError {
    crate::err::EvtxError::FailedToCreateRecordModel("unresolved placeholder in tree")
}

/// Compile a cached template definition into an XML program. Returns `None`
/// for shapes the op set doesn't model.
pub(crate) fn compile_xml_template(
    tree: &IrTree<'_>,
    has_literal_array: bool,
    base_indent: u16,
    is_root: bool,
    settings: &ParserSettings,
) -> Option<XmlProgram> {
    if has_literal_array {
        return None;
    }
    let mut c = XmlCompiler {
        tree,
        lits: Vec::with_capacity(512),
        ops: Vec::with_capacity(32),
        run_start: 0,
        elem_slots: Vec::new(),
        indent_on: settings.should_indent(),
        vr: ValueRenderer::new(),
        materialized: false,
    };
    if is_root {
        c.lits.extend_from_slice(XML_DECL);
    }
    match c.compile_element(tree.root(), base_indent) {
        Ok(()) => {
            c.flush_lit_run();
            Some(XmlProgram {
                lits: c.lits,
                ops: c.ops,
                indent_on: c.indent_on,
                elem_slots: c.elem_slots,
            })
        }
        Err(_) => None,
    }
}

/// Render a fully materialized record tree to XML: the single-walker slow
/// lane (irregular records, after materialization) writing straight into
/// `out`. Byte-compatible with the cached-program lane by construction —
/// it IS the same walk, with zero placeholder sites.
pub(crate) fn render_tree_xml(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut c = XmlCompiler {
        tree,
        lits: std::mem::take(out),
        ops: Vec::new(),
        run_start: 0,
        elem_slots: Vec::new(),
        indent_on: settings.should_indent(),
        vr: ValueRenderer::new(),
        materialized: true,
    };
    c.lits.extend_from_slice(XML_DECL);
    let res = c.compile_element(tree.root(), 0);
    debug_assert!(c.ops.is_empty(), "materialized walk produced ops");
    *out = c.lits;
    res
}

/// Render a materialized fragment subtree at `indent` (executor cold path).
fn render_subtree_xml(
    tree: &IrTree<'_>,
    indent: u16,
    indent_on: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut c = XmlCompiler {
        tree,
        lits: std::mem::take(out),
        ops: Vec::new(),
        run_start: 0,
        elem_slots: Vec::new(),
        indent_on,
        vr: ValueRenderer::new(),
        materialized: true,
    };
    let res = c.compile_element(tree.root(), indent);
    debug_assert!(c.ops.is_empty(), "materialized walk produced ops");
    *out = c.lits;
    res
}

impl<'t, 'a> XmlCompiler<'t, 'a> {
    fn element_ref(&self, id: ElementId) -> &'t Element<'a> {
        self.tree.arena().get(id).expect("invalid element id")
    }

    fn flush_lit_run(&mut self) {
        let end = self.lits.len();
        if end > self.run_start {
            self.ops.push(XOp::Lit((self.run_start as u32, end as u32)));
        }
        self.run_start = end;
    }

    /// Emit bytes via `f` as a side range (not part of any literal run).
    /// The current run must be flushed first.
    fn side_range(&mut self, f: impl FnOnce(&mut Self)) -> LitRange {
        debug_assert_eq!(self.run_start, self.lits.len(), "unflushed lit run");
        let start = self.lits.len() as u32;
        f(self);
        self.run_start = self.lits.len();
        (start, self.lits.len() as u32)
    }

    fn indent_str(&mut self, level: u16) {
        if self.indent_on {
            self.lits.extend(std::iter::repeat_n(b' ', level as usize));
        }
    }

    fn newline(&mut self) {
        if self.indent_on {
            self.lits.push(b'\n');
        }
    }

    fn compile_element(&mut self, id: ElementId, indent: u16) -> Result<()> {
        let element = self.element_ref(id);

        // Note: even placeholder-free subtrees are walked here (not delegated
        // to the materialized emitter): template-scope layout classification
        // (scan rule) differs from the materialized rule for present-but-empty
        // literal children, and this walk is the template-lane source of truth.
        self.indent_str(indent);
        self.lits.push(b'<');
        self.lits
            .extend_from_slice(element.name.as_str().as_bytes());

        for attr in &element.attrs {
            self.compile_attr(attr)?;
        }

        match classify_children(element) {
            ChildrenKind::SinglePlaceholder(ph) => {
                if self.materialized {
                    return Err(unresolved_placeholder());
                }
                let name: Vec<u8> = element.name.as_str().as_bytes().to_vec();
                let is_binary = element.name.as_str() == "Binary";
                self.lits.push(b'>');
                self.flush_lit_run();
                let tail_text = self.side_range(|c| {
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(&name);
                    c.lits.push(b'>');
                    c.newline();
                });
                let tail_empty = self.side_range(|c| {
                    if !is_binary {
                        c.newline();
                        c.indent_str(indent);
                    }
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(&name);
                    c.lits.push(b'>');
                    c.newline();
                });
                let tail_elem = self.side_range(|c| {
                    c.indent_str(indent);
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(&name);
                    c.lits.push(b'>');
                    c.newline();
                });
                self.elem_slots.push((ph.id, indent + INDENT_WIDTH));
                self.ops.push(XOp::Body {
                    slot: ph.id,
                    optional: ph.optional,
                    indent,
                    tail_text,
                    tail_empty,
                    tail_elem,
                });
            }
            ChildrenKind::Empty => {
                self.lits.push(b'>');
                if element.name.as_str() != "Binary" {
                    self.newline();
                    self.indent_str(indent);
                }
                self.close_tag_inline(element);
            }
            ChildrenKind::StaticInline => {
                self.lits.push(b'>');
                let nodes = &element.children;
                let mut idx = 0;
                while idx < nodes.len() {
                    match &nodes[idx] {
                        // Mirror `render_nodes`' processing-instruction
                        // pairing: `<?target data?>` / `<?target?>`.
                        Node::PITarget(name) => {
                            self.lits.extend_from_slice(b"<?");
                            self.lits.extend_from_slice(name.as_str().as_bytes());
                            if let Some(Node::PIData(data)) = nodes.get(idx + 1) {
                                self.lits.push(b' ');
                                self.compile_raw_text(data);
                                self.lits.extend_from_slice(b"?>");
                                idx += 2;
                                continue;
                            }
                            self.lits.extend_from_slice(b"?>");
                        }
                        Node::PIData(_) => {
                            return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                                "PIData without PITarget",
                            ));
                        }
                        node => self.compile_literal_content_node(node, false)?,
                    }
                    idx += 1;
                }
                self.close_tag_inline(element);
            }
            ChildrenKind::StaticLines => {
                self.lits.push(b'>');
                self.newline();
                for node in &element.children {
                    match node {
                        Node::Element(child) => {
                            self.compile_element(*child, indent + INDENT_WIDTH)?;
                        }
                        Node::Placeholder(ph) => {
                            if self.materialized {
                                return Err(unresolved_placeholder());
                            }
                            self.flush_lit_run();
                            let ind = self.side_range(|c| c.indent_str(indent + INDENT_WIDTH));
                            self.elem_slots.push((ph.id, indent + INDENT_WIDTH));
                            self.ops.push(XOp::ChildSlot {
                                slot: ph.id,
                                optional: ph.optional,
                                indent: indent + INDENT_WIDTH,
                                ind,
                            });
                        }
                        other => {
                            self.indent_str(indent + INDENT_WIDTH);
                            self.compile_literal_content_node(other, false)?;
                            self.newline();
                        }
                    }
                }
                self.indent_str(indent);
                self.close_tag_inline(element);
            }
            ChildrenKind::Bail => {
                // Only placeholder-bearing shapes classify as Bail; on a
                // materialized tree that means an unresolved placeholder.
                return Err(if self.materialized {
                    unresolved_placeholder()
                } else {
                    bail_err()
                });
            }
        }
        Ok(())
    }

    fn close_tag_inline(&mut self, element: &Element<'_>) {
        self.lits.extend_from_slice(b"</");
        self.lits
            .extend_from_slice(element.name.as_str().as_bytes());
        self.lits.push(b'>');
        self.newline();
    }

    fn compile_attr(&mut self, attr: &Attr<'a>) -> Result<()> {
        // Placeholders are dynamic; everything else is compile-time constant.
        // Mirrors `attribute_value_is_empty` + `render_nodes`.
        let mut has_nonempty_const = false;
        let mut n_placeholders = 0usize;
        for node in attr.value.iter() {
            match node {
                Node::Placeholder(_) => {
                    if self.materialized {
                        return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                            "unresolved placeholder in attribute value",
                        ));
                    }
                    n_placeholders += 1;
                }
                Node::Element(_) => {
                    return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                        "element node inside attribute value",
                    ));
                }
                Node::PITarget(_) | Node::PIData(_) => {
                    return Err(crate::err::EvtxError::Unimplemented {
                        name: "processing instruction in attribute value".to_string(),
                    });
                }
                Node::Text(t) => {
                    if !t.is_empty() {
                        has_nonempty_const = true;
                    }
                }
                // `attribute_value_is_empty` treats CData (even zero-length)
                // as non-empty.
                Node::CData(_) | Node::EntityRef(_) | Node::CharRef(_) => has_nonempty_const = true,
                Node::Value(v) => {
                    if !crate::model::ir::is_optional_empty(v) {
                        has_nonempty_const = true;
                    }
                }
            }
        }

        if n_placeholders == 0 {
            if !has_nonempty_const {
                return Ok(()); // statically empty attribute: omitted
            }
            self.lits.push(b' ');
            self.lits.extend_from_slice(attr.name.as_str().as_bytes());
            self.lits.extend_from_slice(b"=\"");
            for node in attr.value.iter() {
                self.compile_literal_content_node(node, true)?;
            }
            self.lits.push(b'"');
            return Ok(());
        }

        if has_nonempty_const {
            // Attribute is always emitted; placeholders write inline.
            self.lits.push(b' ');
            self.lits.extend_from_slice(attr.name.as_str().as_bytes());
            self.lits.extend_from_slice(b"=\"");
            for node in attr.value.iter() {
                match node {
                    Node::Placeholder(ph) => {
                        self.flush_lit_run();
                        self.ops.push(XOp::Val {
                            slot: ph.id,
                            in_attr: true,
                        });
                    }
                    other => self.compile_literal_content_node(other, true)?,
                }
            }
            self.lits.push(b'"');
            return Ok(());
        }

        if n_placeholders > 1 {
            // Joint emptiness across several placeholders: not modeled.
            return Err(bail_err());
        }

        // Exactly one placeholder, no non-empty constants: conditional attr.
        // (Constant empty nodes contribute nothing in either branch.)
        let ph = attr
            .value
            .iter()
            .find_map(|n| match n {
                Node::Placeholder(ph) => Some(ph),
                _ => None,
            })
            .expect("counted placeholder");
        let name: Vec<u8> = attr.name.as_str().as_bytes().to_vec();
        self.flush_lit_run();
        let pre = self.side_range(|c| {
            c.lits.push(b' ');
            c.lits.extend_from_slice(&name);
            c.lits.extend_from_slice(b"=\"");
        });
        self.ops.push(XOp::AttrVal { slot: ph.id, pre });
        Ok(())
    }

    /// Render one literal (placeholder-free) node into `lits`, mirroring
    /// `XmlEmitter::render_single_node`.
    fn compile_literal_content_node(&mut self, node: &Node<'a>, in_attribute: bool) -> Result<()> {
        match node {
            Node::Text(text) => self.compile_literal_text(text, in_attribute),
            Node::Value(value) => {
                let mut sink = std::mem::take(&mut self.lits);
                let res = self.vr.write_xml_value_text(&mut sink, value, in_attribute);
                self.lits = sink;
                res
            }
            Node::EntityRef(name) => {
                self.lits.push(b'&');
                self.lits.extend_from_slice(name.as_str().as_bytes());
                self.lits.push(b';');
                Ok(())
            }
            Node::CharRef(ch) => {
                self.lits.extend_from_slice(format!("&#{};", ch).as_bytes());
                Ok(())
            }
            Node::CData(text) => {
                if in_attribute {
                    self.compile_literal_text(text, true)
                } else {
                    self.lits.extend_from_slice(b"<![CDATA[");
                    self.compile_raw_text(text);
                    self.lits.extend_from_slice(b"]]>");
                    Ok(())
                }
            }
            // PIs contribute nothing in content position (`render_single_node`).
            Node::PITarget(_) | Node::PIData(_) => Ok(()),
            Node::Element(_) => Err(crate::err::EvtxError::FailedToCreateRecordModel(
                "unexpected element node in text context",
            )),
            Node::Placeholder(_) => Err(unresolved_placeholder()),
        }
    }

    fn compile_literal_text(&mut self, text: &Text<'a>, in_attribute: bool) -> Result<()> {
        match text {
            Text::Utf16(value) => {
                let bytes = value.as_bytes();
                let units = bytes.len() / 2;
                if units == 0 {
                    return Ok(());
                }
                let mut sink = std::mem::take(&mut self.lits);
                let res = utf16_simd::write_xml_utf16le(&mut sink, bytes, units, in_attribute);
                self.lits = sink;
                res.map_err(crate::err::EvtxError::from)?;
                Ok(())
            }
            Text::Utf8(value) => {
                xml_escape_str_into(&mut self.lits, value, in_attribute);
                Ok(())
            }
        }
    }

    fn compile_raw_text(&mut self, text: &Text<'a>) {
        match text {
            Text::Utf16(value) => {
                let bytes = value.as_bytes();
                let units = bytes.len() / 2;
                if units > 0 {
                    let mut sink = std::mem::take(&mut self.lits);
                    let _ = utf16_simd::write_utf16le_raw(&mut sink, bytes, units);
                    self.lits = sink;
                }
            }
            Text::Utf8(value) => self.lits.extend_from_slice(value.as_bytes()),
        }
    }
}

/// Mirrors `XmlEmitter::write_escaped_str` for compile-time UTF-8 literals.
fn xml_escape_str_into(out: &mut Vec<u8>, text: &str, in_attribute: bool) {
    for ch in text.chars() {
        match ch {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '"' if in_attribute => out.extend_from_slice(b"&quot;"),
            '\'' if in_attribute => out.extend_from_slice(b"&apos;"),
            _ => {
                let mut buf = [0_u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

enum ChildrenKind<'n> {
    /// No children (or only statically-empty literals): logically empty.
    Empty,
    /// Exactly one placeholder child (the `<Data>%1</Data>` shape).
    SinglePlaceholder(&'n Placeholder),
    /// All literal, no element children, at least one non-empty content node.
    StaticInline,
    /// Statically line-formed: literal elements (>=1 when placeholders are
    /// present), placeholders, and literal nodes each on their own line.
    StaticLines,
    Bail,
}

fn classify_children<'n>(element: &'n Element<'_>) -> ChildrenKind<'n> {
    if element.children.is_empty() {
        return ChildrenKind::Empty;
    }
    if element.children.len() == 1
        && let Node::Placeholder(ph) = &element.children[0]
    {
        return ChildrenKind::SinglePlaceholder(ph);
    }
    let mut has_placeholder = false;
    let mut has_literal_element = false;
    let mut has_literal_content = false;
    for node in &element.children {
        match node {
            Node::Placeholder(_) => has_placeholder = true,
            Node::Element(_) => has_literal_element = true,
            // PIs are neither content nor element (scan_class: Empty); they
            // render inline (paired) or as bare lines depending on layout.
            Node::PITarget(_) | Node::PIData(_) => {}
            Node::Text(t) | Node::CData(t) => {
                if !t.is_empty() {
                    has_literal_content = true;
                }
            }
            Node::EntityRef(_) | Node::CharRef(_) => has_literal_content = true,
            Node::Value(v) => {
                if !crate::model::ir::is_optional_empty(v) {
                    has_literal_content = true;
                }
            }
        }
    }
    if !has_placeholder {
        if has_literal_element {
            return ChildrenKind::StaticLines;
        }
        // Present-but-empty literal children are NOT logically empty
        // (`child_layout` counts Empty-class nodes as `any`): inline form.
        return ChildrenKind::StaticInline;
    }
    // Placeholders mixed with other children: the layout must be statically
    // line-formed, which requires a literal element child. Literal content
    // would render inline if no element materializes -> runtime layout fork.
    if has_literal_element && !has_literal_content {
        return ChildrenKind::StaticLines;
    }
    ChildrenKind::Bail
}

fn subtree_has_placeholder(tree: &IrTree<'_>, element: &Element<'_>) -> bool {
    for attr in &element.attrs {
        if attr.value.iter().any(|n| matches!(n, Node::Placeholder(_))) {
            return true;
        }
    }
    for node in &element.children {
        match node {
            Node::Placeholder(_) => return true,
            Node::Element(id)
                if subtree_has_placeholder(tree, tree.arena().get(*id).expect("element id")) =>
            {
                return true;
            }
            _ => {}
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Per-record pre-flight (raw instance scan)
// ---------------------------------------------------------------------------

/// One raw substitution slot: a typed window into the chunk data.
#[derive(Clone, Copy)]
struct RawSlot {
    off: u32,
    len: u16,
    ty: u8,
    /// Index into `Preflight::nested` for single-instance BinXml slots;
    /// `u16::MAX` otherwise.
    nested: u16,
    /// Index into `Preflight::ansi` (`ty == 0x02` with payload only).
    ansi: u32,
}

const NO_NESTED: u16 = u16::MAX;
const NO_ANSI: u32 = u32::MAX;

struct NestedInst<P> {
    prog: Arc<P>,
    slots: SlotRange,
}

/// Reusable per-chunk pre-flight scratch.
pub(crate) struct Preflight<P> {
    slots: Vec<RawSlot>,
    nested: Vec<NestedInst<P>>,
    ansi: Vec<String>,
}

impl<P> Default for Preflight<P> {
    fn default() -> Self {
        Preflight {
            slots: Vec::new(),
            nested: Vec::new(),
            ansi: Vec::new(),
        }
    }
}

/// Fixed wire width for fixed-size scalar types.
fn fixed_width(ty: u8) -> Option<u16> {
    Some(match ty {
        0x03 | 0x04 => 1,
        0x05 | 0x06 => 2,
        0x07 | 0x08 | 0x0b | 0x14 => 4,
        0x09 | 0x0a | 0x0c | 0x15 => 8,
        0x0d => 4,  // BoolType is a 4-byte i32 on the wire
        0x0f => 16, // GUID
        0x11 => 8,  // FILETIME
        0x12 => 16, // SYSTEMTIME
        _ => return None,
    })
}

struct PreflightBail;

impl<P: StoredProgram> Preflight<P> {
    fn clear(&mut self) {
        self.slots.clear();
        self.nested.clear();
        self.ansi.clear();
    }

    /// Emptiness of a slot, mirroring `is_optional_empty` over the value the
    /// regular path would have decoded (string NUL-truncation included).
    fn slot_empty(&self, s: &RawSlot, data: &[u8]) -> bool {
        match s.ty {
            0x00 => true,
            0x01 => {
                s.len == 0 || {
                    let o = s.off as usize;
                    data[o] == 0 && data[o + 1] == 0
                }
            }
            0x02 => s.ansi == NO_ANSI || self.ansi[s.ansi as usize].is_empty(),
            0x0e | 0x21 => s.len == 0,
            _ => false,
        }
    }

    /// Scan a `TemplateInstance` whose header starts at absolute `pos` (the
    /// byte after the 0x0c token). Appends slots/nested entries and returns
    /// `(program, slot_range, end_pos)`.
    #[allow(clippy::too_many_arguments)]
    fn scan_instance<'a>(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        pos: usize,
        depth: usize,
        cache: &mut IrTemplateCache<'a>,
        progs: &mut ProgramCache<P>,
        settings: &ParserSettings,
        base_indent: u16,
        is_root: bool,
    ) -> std::result::Result<(Arc<P>, SlotRange, usize), PreflightBail> {
        if depth > 8 {
            return Err(PreflightBail);
        }
        let data = chunk.data;
        let mut p = pos;

        // Mirrors `read_template_values_cursor` header handling.
        if p >= data.len() {
            return Err(PreflightBail);
        }
        p += 1; // unknown byte
        let _template_id = read_u32(data, p)?;
        let def_offset = read_u32(data, p + 4)?;
        p += 8;
        if p as u32 == def_offset {
            // Inline definition: skip the 24-byte header + payload.
            let data_size = read_u32(data, p + 20)?;
            p = p
                .checked_add(24 + data_size as usize)
                .ok_or(PreflightBail)?;
        }
        let n = read_u32(data, p)? as usize;
        p += 4;
        if n > 4096 {
            return Err(PreflightBail);
        }

        let prog = get_or_compile(
            chunk,
            def_offset,
            base_indent,
            is_root,
            cache,
            progs,
            settings,
        )
        .ok_or(PreflightBail)?;

        // Descriptor table: n x (u16 size, u8 type, u8 pad).
        let desc_base = p;
        let values_base = p + n * 4;
        if values_base > data.len() {
            return Err(PreflightBail);
        }
        let mut off = values_base;
        let slot_start = self.slots.len() as u32;
        for i in 0..n {
            let d = desc_base + i * 4;
            let len = read_u16(data, d)?;
            let ty = data[d + 2];
            if off + usize::from(len) > data.len() {
                return Err(PreflightBail);
            }
            match ty {
                0x00 | 0x02 | 0x0e | 0x21 => {}
                0x01 => {
                    if len % 2 != 0 {
                        return Err(PreflightBail);
                    }
                }
                0x10 => {
                    if !(len == 4 || len == 8) {
                        return Err(PreflightBail);
                    }
                }
                0x13 => {
                    // SID: 8 + 4 * sub_authority_count bytes.
                    if len < 8 {
                        return Err(PreflightBail);
                    }
                    let count = data[off + 1];
                    if usize::from(len) != 8 + 4 * usize::from(count) {
                        return Err(PreflightBail);
                    }
                }
                t => match fixed_width(t) {
                    Some(w) if w == len => {}
                    // Arrays (0x80 bit) and exotic/mis-sized types: fallback.
                    _ => return Err(PreflightBail),
                },
            }
            let mut slot = RawSlot {
                off: off as u32,
                len,
                ty,
                nested: NO_NESTED,
                ansi: NO_ANSI,
            };
            if ty == 0x02 && len > 0 {
                // Decode ANSI now so the executor stays infallible. Mirrors
                // `deserialize_value_type_cursor_in` (NUL filter + strict).
                let raw = &data[off..off + usize::from(len)];
                let filtered: Vec<u8> = raw.iter().copied().filter(|&b| b != 0).collect();
                let decoded = settings
                    .get_ansi_codec()
                    .decode(&filtered, encoding::DecoderTrap::Strict)
                    .map_err(|_| PreflightBail)?;
                slot.ansi = self.ansi.len() as u32;
                self.ansi.push(decoded);
            }
            self.slots.push(slot);
            off += usize::from(len);
        }
        let slot_range = (slot_start, self.slots.len() as u32);

        // Resolve nested instances for slots this program renders as elements.
        for &(slot_id, child_indent) in prog.elem_slots() {
            if u32::from(slot_id) >= slot_range.1 - slot_range.0 {
                continue; // out-of-range -> Skip at exec
            }
            let idx = slot_start as usize + slot_id as usize;
            let s = self.slots[idx];
            if s.ty != 0x21 || s.len == 0 {
                continue;
            }
            let fo = s.off as usize;
            let frag = &data[fo..fo + usize::from(s.len)];
            let inst_off = match frag.first() {
                Some(0x0f) if frag.len() > 5 && frag[4] == 0x0c => 5,
                Some(0x0c) => 1,
                // Generic fragment: rendered via the materialized fallback
                // where the executor supports it; otherwise fall back.
                _ if P::ALLOW_GENERIC_FRAGS => continue,
                _ => return Err(PreflightBail),
            };
            let (nprog, nslots, nend) = self.scan_instance(
                chunk,
                fo + inst_off,
                depth + 1,
                cache,
                progs,
                settings,
                child_indent,
                false,
            )?;
            // The nested instance must span its whole payload (allow EOF pad).
            let nconsumed = nend - fo;
            if nconsumed > usize::from(s.len)
                || (nconsumed < usize::from(s.len) && frag[nconsumed] != 0x00)
            {
                return Err(PreflightBail);
            }
            if self.nested.len() >= usize::from(u16::MAX) {
                return Err(PreflightBail);
            }
            self.slots[idx].nested = self.nested.len() as u16;
            self.nested.push(NestedInst {
                prog: nprog,
                slots: nslots,
            });
        }

        // Per-slot constraints (kept rare; violations route to the fallback).
        for &c in prog.constraints() {
            let (slot_id, forbid_elem) = match c {
                SlotConstraint::ForbidElem(s) => (s, true),
                SlotConstraint::ElemOrEmpty(s) => (s, false),
            };
            if u32::from(slot_id) >= slot_range.1 - slot_range.0 {
                continue; // out-of-range resolves to Skip everywhere
            }
            let s = self.slots[slot_start as usize + slot_id as usize];
            let is_elem = s.ty == 0x21 && s.len > 0;
            let empty = self.slot_empty(&s, data);
            if (forbid_elem && is_elem) || (!forbid_elem && !is_elem && !empty) {
                return Err(PreflightBail);
            }
        }

        Ok((prog, slot_range, off))
    }
}

fn read_u16(data: &[u8], pos: usize) -> std::result::Result<u16, PreflightBail> {
    data.get(pos..pos + 2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .ok_or(PreflightBail)
}

fn read_u32(data: &[u8], pos: usize) -> std::result::Result<u32, PreflightBail> {
    data.get(pos..pos + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or(PreflightBail)
}

fn get_or_compile<'a, P: StoredProgram>(
    chunk: &'a EvtxChunk<'a>,
    def_offset: u32,
    base_indent: u16,
    is_root: bool,
    cache: &mut IrTemplateCache<'a>,
    progs: &mut ProgramCache<P>,
    settings: &ParserSettings,
) -> Option<Arc<P>> {
    let key = (def_offset, base_indent, is_root);
    if let Some(entry) = progs.get(&key) {
        return entry.clone();
    }

    // Cross-chunk store, keyed by template content identity.
    let store = &*chunk.program_store;
    let store_key = template_content_key(chunk, def_offset, &store.hasher)
        .map(|(guid, size, hash)| (guid, size, hash, base_indent, is_root));
    if let Some(sk) = store_key.as_ref()
        && let Some(entry) = P::shard(store)
            .read()
            .expect("program store poisoned")
            .get(sk)
    {
        progs.insert(key, entry.clone());
        return entry.clone();
    }

    let compiled =
        cache
            .template_for_compile(chunk, def_offset)
            .ok()
            .and_then(|(tree, has_literal_array)| {
                P::compile(&tree, has_literal_array, base_indent, is_root, settings).map(Arc::new)
            });
    if let Some(sk) = store_key {
        P::shard(store)
            .write()
            .expect("program store poisoned")
            .insert(sk, compiled.clone());
    }
    progs.insert(key, compiled.clone());
    compiled
}

/// Template content identity at `def_offset`: (GUID, data size, bytes hash).
fn template_content_key(
    chunk: &EvtxChunk<'_>,
    def_offset: u32,
    hasher: &ahash::RandomState,
) -> Option<([u8; 16], u32, u64)> {
    let data = chunk.data;
    let off = def_offset as usize;
    // Header: u32 next_offset, [u8;16] guid, u32 data_size (24 bytes).
    let guid: [u8; 16] = data.get(off + 4..off + 20)?.try_into().ok()?;
    let size = u32::from_le_bytes(data.get(off + 20..off + 24)?.try_into().ok()?);
    let body = data.get(off + 24..(off + 24).checked_add(size as usize)?)?;
    Some((guid, size, hasher.hash_one(body)))
}

// ---------------------------------------------------------------------------
// Per-record entry + executor
// ---------------------------------------------------------------------------

/// Try to render one record's BinXML via the compiled-template path.
///
/// Returns `false` with `out` untouched when the record isn't covered (the
/// caller then uses the regular path). On `true` the record was rendered
/// byte-identically to `render_xml_record_content`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_render_xml_compiled<'a>(
    bytes: &[u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    progs: &mut XmlProgramCache,
    pf: &mut Preflight<XmlProgram>,
    settings: &ParserSettings,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> bool {
    // Single-instance stream shape (mirrors `read_single_instance_stream`).
    let inst_off = match bytes.first() {
        Some(0x0f) if bytes.len() > 5 && bytes[4] == 0x0c => 5,
        Some(0x0c) => 1,
        _ => return false,
    };
    let data_start = chunk.data.as_ptr() as usize;
    let slice_start = bytes.as_ptr() as usize;
    if slice_start < data_start || slice_start + bytes.len() > data_start + chunk.data.len() {
        return false;
    }
    let stream_offset = slice_start - data_start;

    pf.clear();
    let (prog, slot_range, end) = match pf.scan_instance(
        chunk,
        stream_offset + inst_off,
        0,
        cache,
        progs,
        settings,
        0,
        true,
    ) {
        Ok(v) => v,
        Err(PreflightBail) => return false,
    };

    // Anything after the instance other than EOF (0x00) is unhandled here.
    let consumed = end - stream_offset;
    if consumed > bytes.len() || (consumed < bytes.len() && bytes[consumed] != 0x00) {
        return false;
    }

    let start = out.len();
    match exec(&prog, slot_range, pf, chunk, cache, vr, out) {
        Ok(()) => true,
        Err(_) => {
            // Unreachable post-preflight except for attribute-position BinXml
            // elements, which the regular path also rejects per record.
            out.truncate(start);
            false
        }
    }
}

/// Runtime slot class for `Body`/`ChildSlot` branching.
enum SlotClass {
    Skip,
    TextLike,
    Element,
}

fn exec<'a>(
    prog: &XmlProgram,
    slot_range: SlotRange,
    pf: &Preflight<XmlProgram>,
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> Result<()> {
    let lits = &prog.lits;
    let slot_at = |slot: u16| -> Option<RawSlot> {
        if u32::from(slot) < slot_range.1 - slot_range.0 {
            Some(pf.slots[slot_range.0 as usize + slot as usize])
        } else {
            None
        }
    };
    let classify = |slot: u16, optional: bool| -> SlotClass {
        match slot_at(slot) {
            None => SlotClass::Skip,
            Some(s) => {
                if pf.slot_empty(&s, chunk.data) {
                    if optional {
                        SlotClass::Skip
                    } else {
                        SlotClass::TextLike
                    }
                } else if s.ty == 0x21 {
                    SlotClass::Element
                } else {
                    SlotClass::TextLike
                }
            }
        }
    };

    macro_rules! write_lit {
        ($r:expr) => {{
            let (a, b) = $r;
            out.extend_from_slice(&lits[a as usize..b as usize]);
        }};
    }
    macro_rules! write_val {
        ($s:expr, $in_attr:expr) => {{
            let s = $s;
            let vb = &chunk.data[s.off as usize..s.off as usize + usize::from(s.len)];
            let ansi = (s.ansi != NO_ANSI).then(|| pf.ansi[s.ansi as usize].as_str());
            vr.write_raw_value_text(
                out,
                s.ty,
                vb,
                ansi,
                StringEscapeMode::Xml {
                    in_attribute: $in_attr,
                },
            )?;
        }};
    }

    for op in &prog.ops {
        match op {
            XOp::Lit(r) => write_lit!(*r),
            XOp::Val { slot, in_attr } => {
                if let Some(s) = slot_at(*slot) {
                    if s.ty == 0x21 && s.len > 0 {
                        return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                            "element node inside attribute value",
                        ));
                    }
                    write_val!(s, *in_attr);
                }
            }
            XOp::AttrVal { slot, pre } => {
                if let Some(s) = slot_at(*slot)
                    && !pf.slot_empty(&s, chunk.data)
                {
                    if s.ty == 0x21 {
                        return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                            "element node inside attribute value",
                        ));
                    }
                    write_lit!(*pre);
                    write_val!(s, true);
                    out.push(b'"');
                }
            }
            XOp::Body {
                slot,
                optional,
                indent,
                tail_text,
                tail_empty,
                tail_elem,
            } => {
                // The opening `<Tag ...>` (including `>`) came from the
                // preceding Lit run; only the content + close remain here.
                match classify(*slot, *optional) {
                    SlotClass::Skip => write_lit!(*tail_empty),
                    SlotClass::TextLike => {
                        if let Some(s) = slot_at(*slot) {
                            write_val!(s, false);
                        }
                        write_lit!(*tail_text);
                    }
                    SlotClass::Element => {
                        let s = slot_at(*slot).expect("element class implies present");
                        if prog.indent_on {
                            out.push(b'\n');
                        }
                        render_element_slot(
                            &s,
                            pf,
                            chunk,
                            cache,
                            vr,
                            *indent + INDENT_WIDTH,
                            prog.indent_on,
                            out,
                        )?;
                        write_lit!(*tail_elem);
                    }
                }
            }
            XOp::ChildSlot {
                slot,
                optional,
                indent,
                ind,
            } => match classify(*slot, *optional) {
                SlotClass::Skip => {}
                SlotClass::TextLike => {
                    write_lit!(*ind);
                    if let Some(s) = slot_at(*slot) {
                        write_val!(s, false);
                    }
                    if prog.indent_on {
                        out.push(b'\n');
                    }
                }
                SlotClass::Element => {
                    let s = slot_at(*slot).expect("element class implies present");
                    render_element_slot(&s, pf, chunk, cache, vr, *indent, prog.indent_on, out)?;
                }
            },
        }
    }
    Ok(())
}

/// Render an element-class slot: a nested compiled instance, or a generic
/// BinXml fragment via the materialized fallback renderer.
#[allow(clippy::too_many_arguments)]
fn render_element_slot<'a>(
    s: &RawSlot,
    pf: &Preflight<XmlProgram>,
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    vr: &mut ValueRenderer,
    indent: u16,
    indent_on: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    if s.nested != NO_NESTED {
        let inst = &pf.nested[s.nested as usize];
        return exec(&inst.prog, inst.slots, pf, chunk, cache, vr, out);
    }
    // Generic (non-instance) fragment: materialize and render. Cold path.
    let fo = s.off as usize;
    let frag = &chunk.data[fo..fo + usize::from(s.len)];
    let tree = build_tree_from_binxml_bytes_direct(frag, chunk, cache)?;
    render_subtree_xml(&tree, indent, indent_on, out)
}

// ---------------------------------------------------------------------------
// JSON programs
// ---------------------------------------------------------------------------

/// One attribute part inside a `JOp::Elem` `#attributes` object: either a
/// pre-rendered literal member (`lit_member`) or a conditional placeholder
/// member (`key` + value from `slot`).
#[derive(Debug, Clone)]
struct JAttrPart {
    /// `"name":` key bytes for placeholder attrs; empty for literal members.
    key: LitRange,
    /// Pre-rendered `"name":value` member for literal attrs.
    lit_member: LitRange,
    /// Placeholder slot (`u16::MAX` for literal members).
    slot: u16,
}

#[derive(Debug, Clone)]
enum JOp {
    /// Emit `lits[range]`.
    Lit(LitRange),
    /// A leaf element value: `null`/`""` when empty, bare number for
    /// int/bool-typed slots, nested-instance object for BinXml slots,
    /// quoted escaped string otherwise.
    LeafVal { slot: u16, empty: LitRange },
    /// `write_element_value` for an element with placeholder attributes and
    /// at most one placeholder content child (no element children possible).
    Elem {
        attrs: Box<[JAttrPart]>,
        content: Option<u16>,
        /// `null` / `""` for the all-empty case.
        empty: LitRange,
    },
    /// A placeholder in element-child position inside an object: emits
    /// `,"<NestedRootName>[_N]": { ... }` when the slot is a nested instance,
    /// nothing when empty. Pre-flight guarantees elem-or-empty.
    SlotChild {
        slot: u16,
        /// `(name bytes in lits, static emission count)` of preceding static
        /// members, for `_N` suffix seeding.
        static_names: Box<[(LitRange, u16)]>,
        /// Whether any object member unconditionally precedes this op.
        lead_comma: bool,
    },
}

/// A compiled JSON template program.
pub(crate) struct JsonProgram {
    lits: Vec<u8>,
    ops: Vec<JOp>,
    elem_slots: Vec<(u16, u16)>,
    constraints: Vec<SlotConstraint>,
    /// Raw root element name bytes (member key for nested-instance values).
    root_name: Vec<u8>,
}

impl TemplateProgram for JsonProgram {
    const ALLOW_GENERIC_FRAGS: bool = false;
    fn elem_slots(&self) -> &[(u16, u16)] {
        &self.elem_slots
    }
    fn constraints(&self) -> &[SlotConstraint] {
        &self.constraints
    }
    fn compile(
        tree: &IrTree<'_>,
        has_literal_array: bool,
        _base_indent: u16,
        is_root: bool,
        settings: &ParserSettings,
    ) -> Option<Self> {
        compile_json_template(tree, has_literal_array, is_root, settings)
    }
}

struct JsonCompiler<'t, 'a> {
    tree: Option<&'t IrTree<'a>>,
    lits: Vec<u8>,
    ops: Vec<JOp>,
    run_start: usize,
    elem_slots: Vec<(u16, u16)>,
    constraints: Vec<SlotConstraint>,
    vr: ValueRenderer,
    formatter: sonic_rs::format::CompactFormatter,
    /// `--separate-json-attributes` (slow lane only; such templates bail).
    separate: bool,
}

fn compile_json_template(
    tree: &IrTree<'_>,
    has_literal_array: bool,
    is_root: bool,
    settings: &ParserSettings,
) -> Option<JsonProgram> {
    if has_literal_array || settings.should_separate_json_attributes() {
        return None;
    }
    let root = tree.root_element();
    let root_container = is_data_container_name(root.name.as_str());
    let mut c = JsonCompiler {
        tree: Some(tree),
        lits: Vec::with_capacity(512),
        ops: Vec::with_capacity(32),
        run_start: 0,
        elem_slots: Vec::new(),
        constraints: Vec::new(),
        vr: ValueRenderer::new(),
        formatter: sonic_rs::format::CompactFormatter,
        separate: false,
    };
    if is_root {
        c.lits.push(b'{');
        c.lits.push(b'"');
        c.lits.extend_from_slice(root.name.as_str().as_bytes());
        c.lits.extend_from_slice(b"\":");
    }
    match c.compile_element_value(tree.root(), root_container) {
        Ok(()) => {
            if is_root {
                c.lits.push(b'}');
            }
            c.flush_lit_run();
            Some(JsonProgram {
                lits: c.lits,
                ops: c.ops,
                elem_slots: c.elem_slots,
                constraints: c.constraints,
                root_name: root.name.as_str().as_bytes().to_vec(),
            })
        }
        Err(Bail) => None,
    }
}

fn is_data_container_name(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn is_data_element_name(name: &str) -> bool {
    name == "Data"
}

/// Compile-time emptiness of a literal (placeholder-free) node, mirroring
/// `scan_class` Content-detection for literals.
fn literal_nonempty(node: &Node<'_>) -> bool {
    match node {
        Node::Text(t) | Node::CData(t) => !t.is_empty(),
        Node::EntityRef(_) | Node::CharRef(_) => true,
        Node::Value(v) => !crate::model::ir::is_optional_empty(v),
        Node::Element(_) | Node::Placeholder(_) | Node::PITarget(_) | Node::PIData(_) => false,
    }
}

impl<'t, 'a> JsonCompiler<'t, 'a> {
    fn tree(&self) -> &'t IrTree<'a> {
        self.tree.expect("tree-bound walk")
    }

    fn flush_lit_run(&mut self) {
        let end = self.lits.len();
        if end > self.run_start {
            self.ops.push(JOp::Lit((self.run_start as u32, end as u32)));
        }
        self.run_start = end;
    }

    fn side_range(&mut self, f: impl FnOnce(&mut Self)) -> LitRange {
        debug_assert_eq!(self.run_start, self.lits.len(), "unflushed lit run");
        let start = self.lits.len() as u32;
        f(self);
        self.run_start = self.lits.len();
        (start, self.lits.len() as u32)
    }

    /// `write_element_value` equivalent: emits the VALUE of `element` (the
    /// member key is the caller's responsibility).
    fn compile_element_value(
        &mut self,
        id: ElementId,
        container: bool,
    ) -> std::result::Result<(), Bail> {
        let element = self.element_ref(id);

        // Placeholder-free subtree: render via the materialized layer (the
        // same implementation the slow lane uses).
        if !subtree_has_placeholder(self.tree(), element) {
            return self
                .write_element_value_plain(element, container)
                .map_err(|_| Bail);
        }

        let ph_attrs = element
            .attrs
            .iter()
            .any(|a| a.value.iter().any(|n| matches!(n, Node::Placeholder(_))));
        let static_attr_text = element.attrs.iter().any(|a| {
            !a.value.iter().any(|n| matches!(n, Node::Placeholder(_)))
                && a.value.iter().any(literal_nonempty)
        });

        match classify_children(element) {
            ChildrenKind::SinglePlaceholder(ph) => {
                if !ph_attrs && !static_attr_text {
                    // Leaf shape: `null` when empty, primitive otherwise.
                    self.compile_leaf_val(ph.id, b"null", container)
                } else {
                    self.compile_elem_op(element, Some(ph.id), static_attr_text)
                }
            }
            ChildrenKind::Empty => {
                if !ph_attrs {
                    // Statically resolvable: `null` or attrs-only object.
                    // (subtree_has_placeholder was true, so this can't happen.)
                    Err(Bail)
                } else {
                    self.compile_elem_op(element, None, static_attr_text)
                }
            }
            ChildrenKind::StaticInline => Err(Bail), // literal text + ph attrs: rare
            ChildrenKind::StaticLines => self.compile_object_body(element, container, ph_attrs),
            ChildrenKind::Bail => Err(Bail),
        }
    }

    /// Leaf value op (single placeholder content, no attribute text).
    fn compile_leaf_val(
        &mut self,
        slot: u16,
        empty_form: &[u8],
        container: bool,
    ) -> std::result::Result<(), Bail> {
        // A Data-container leaf (`<UserData>%n</UserData>`) re-enters the
        // flattening rules through its nested root; keep it on the fallback.
        if container {
            return Err(Bail);
        }
        self.flush_lit_run();
        let empty = self.side_range(|c| c.lits.extend_from_slice(empty_form));
        self.elem_slots.push((slot, 0));
        self.ops.push(JOp::LeafVal { slot, empty });
        Ok(())
    }

    /// `JOp::Elem` for `<Tag attr=%a ...>%c?</Tag>` shapes.
    fn compile_elem_op(
        &mut self,
        element: &'t Element<'a>,
        content: Option<u16>,
        static_attr_text: bool,
    ) -> std::result::Result<(), Bail> {
        // `static_attr_text` forces the object form unconditionally, which is
        // a shape the op models as "attrs always present"; handled below by
        // emitting literal members. Build attr parts in order.
        let _ = static_attr_text;
        let mut parts: Vec<JAttrPart> = Vec::with_capacity(element.attrs.len());
        self.flush_lit_run();
        for attr in &element.attrs {
            let n_ph = attr
                .value
                .iter()
                .filter(|n| matches!(n, Node::Placeholder(_)))
                .count();
            match n_ph {
                0 => {
                    if !attr.value.iter().any(literal_nonempty) {
                        continue; // statically empty: never a member
                    }
                    // Pre-render `"name":value` via the materialized layer.
                    let start = self.lits.len() as u32;
                    self.lits.push(b'"');
                    self.lits.extend_from_slice(attr.name.as_str().as_bytes());
                    self.lits.extend_from_slice(b"\":");
                    let number = self
                        .try_as_number_plain(&attr.value, false)
                        .map_err(|_| Bail)?;
                    if !number {
                        self.lits.push(b'"');
                        self.text_content_plain(&attr.value, false)
                            .map_err(|_| Bail)?;
                        self.lits.push(b'"');
                    }
                    let lit_member = (start, self.lits.len() as u32);
                    self.run_start = self.lits.len();
                    parts.push(JAttrPart {
                        key: (0, 0),
                        lit_member,
                        slot: u16::MAX,
                    });
                }
                1 if attr.value.len() == 1 => {
                    let Node::Placeholder(ph) = &attr.value[0] else {
                        return Err(Bail);
                    };
                    let name = attr.name.as_str().as_bytes().to_vec();
                    let key = self.side_range(|c| {
                        c.lits.push(b'"');
                        c.lits.extend_from_slice(&name);
                        c.lits.extend_from_slice(b"\":");
                    });
                    self.constraints.push(SlotConstraint::ForbidElem(ph.id));
                    parts.push(JAttrPart {
                        key,
                        lit_member: (0, 0),
                        slot: ph.id,
                    });
                }
                _ => return Err(Bail),
            }
        }
        if let Some(slot) = content {
            self.constraints.push(SlotConstraint::ForbidElem(slot));
        }
        let empty = self.side_range(|c| c.lits.extend_from_slice(b"null"));
        self.ops.push(JOp::Elem {
            attrs: parts.into_boxed_slice(),
            content,
            empty,
        });
        Ok(())
    }

    /// Static-layout object: literal element children (each a member), plus
    /// optional trailing placeholder children (`SlotChild`).
    fn compile_object_body(
        &mut self,
        element: &'t Element<'a>,
        container: bool,
        ph_attrs: bool,
    ) -> std::result::Result<(), Bail> {
        if ph_attrs {
            return Err(Bail); // object with placeholder attrs: fallback
        }
        self.lits.push(b'{');
        let mut wrote_any = false;

        // `#attributes` for literal attrs (static decision + static bytes).
        if !element.attrs.is_empty() && self.attrs_object_plain(&element.attrs).map_err(|_| Bail)? {
            wrote_any = true;
        }

        // Flattening / positional decisions are compile-time: literal `Data`
        // children only (placeholder `Data` shapes bail via classify above).
        let mut flatten_named = false;
        if container {
            for node in &element.children {
                if let Node::Element(id) = node {
                    let child = self.element_ref(*id);
                    if is_data_element_name(child.name.as_str())
                        && let Some(attr) = child.attrs.iter().find(|a| a.name.as_str() == "Name")
                    {
                        if attr.value.iter().any(|n| matches!(n, Node::Placeholder(_))) {
                            return Err(Bail); // dynamic Data names: fallback
                        }
                        if attr.value.iter().any(literal_nonempty) {
                            flatten_named = true;
                            break;
                        }
                    }
                }
            }
        }
        let positional_data: Vec<ElementId> = if container && !flatten_named {
            element
                .children
                .iter()
                .filter_map(|n| match n {
                    Node::Element(id)
                        if is_data_element_name(self.element_ref(*id).name.as_str()) =>
                    {
                        Some(*id)
                    }
                    _ => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut positional_emitted = false;

        // Compile-time `_N` suffix counting for static members.
        let mut static_names: Vec<(Vec<u8>, u16)> = Vec::new();
        let next_suffix = |name: &[u8], names: &mut Vec<(Vec<u8>, u16)>| -> u16 {
            for (n, c) in names.iter_mut() {
                if n.as_slice() == name {
                    let s = *c;
                    *c += 1;
                    return s;
                }
            }
            names.push((name.to_vec(), 1));
            0
        };

        let mut seen_slot_child = false;
        for node in &element.children {
            match node {
                Node::Element(id) => {
                    if seen_slot_child {
                        return Err(Bail); // static member after dynamic: comma hazard
                    }
                    let child = self.element_ref(*id);
                    let cname = child.name.as_str();
                    if container && is_data_element_name(cname) {
                        if flatten_named {
                            let Some(attr) = child.attrs.iter().find(|a| a.name.as_str() == "Name")
                            else {
                                continue; // unnamed Data skipped in named form
                            };
                            if !attr.value.iter().any(literal_nonempty) {
                                continue; // empty literal name: skipped
                            }
                            if wrote_any {
                                self.lits.push(b',');
                            }
                            wrote_any = true;
                            // Key: JSON-escaped literal name text.
                            self.lits.push(b'"');
                            self.text_content_plain(&attr.value, false)
                                .map_err(|_| Bail)?;
                            self.lits.extend_from_slice(b"\":");
                            self.compile_data_value(*id)?;
                        } else if !positional_emitted && !positional_data.is_empty() {
                            positional_emitted = true;
                            if wrote_any {
                                self.lits.push(b',');
                            }
                            wrote_any = true;
                            self.lits.extend_from_slice(b"\"Data\":{\"#text\":");
                            if positional_data.len() == 1 {
                                self.compile_data_value(positional_data[0])?;
                            } else {
                                self.lits.push(b'[');
                                for (i, did) in positional_data.iter().enumerate() {
                                    if i > 0 {
                                        self.lits.push(b',');
                                    }
                                    self.compile_data_value(*did)?;
                                }
                                self.lits.push(b']');
                            }
                            self.lits.push(b'}');
                        }
                        continue;
                    }
                    // Normal member.
                    if wrote_any {
                        self.lits.push(b',');
                    }
                    wrote_any = true;
                    let suffix = next_suffix(cname.as_bytes(), &mut static_names);
                    self.lits.push(b'"');
                    self.lits.extend_from_slice(cname.as_bytes());
                    if suffix > 0 {
                        self.lits.push(b'_');
                        self.lits.extend_from_slice(suffix.to_string().as_bytes());
                    }
                    self.lits.extend_from_slice(b"\":");
                    self.compile_element_value(*id, is_data_container_name(cname))?;
                }
                Node::Placeholder(ph) => {
                    // Dynamic member: nested-instance value (or absent).
                    self.flush_lit_run();
                    let name_ranges: Vec<(LitRange, u16)> = static_names
                        .iter()
                        .map(|(n, c)| {
                            let r = self.side_range(|cc| cc.lits.extend_from_slice(n));
                            (r, *c)
                        })
                        .collect();
                    self.constraints.push(SlotConstraint::ElemOrEmpty(ph.id));
                    self.elem_slots.push((ph.id, 0));
                    self.ops.push(JOp::SlotChild {
                        slot: ph.id,
                        static_names: name_ranges.into_boxed_slice(),
                        lead_comma: wrote_any,
                    });
                    seen_slot_child = true;
                }
                other => {
                    if literal_nonempty(other) {
                        return Err(Bail); // would force #text: fallback
                    }
                }
            }
        }
        self.lits.push(b'}');
        Ok(())
    }

    /// `render_data_element_value` for a literal `<Data ...>` child: leaf
    /// placeholder -> LeafVal with `""` empty form; literal-only -> rendered
    /// at compile time; anything else bails.
    fn compile_data_value(&mut self, id: ElementId) -> std::result::Result<(), Bail> {
        let element = self.element_ref(id);
        if !subtree_has_placeholder(self.tree(), element) {
            return self.data_element_value_plain(element).map_err(|_| Bail);
        }
        if element.children.len() == 1
            && let Node::Placeholder(ph) = &element.children[0]
        {
            self.flush_lit_run();
            let empty = self.side_range(|c| c.lits.extend_from_slice(b"\"\""));
            self.elem_slots.push((ph.id, 0));
            self.ops.push(JOp::LeafVal { slot: ph.id, empty });
            return Ok(());
        }
        Err(Bail)
    }
}

// ---------------------------------------------------------------------------
// JSON executor
// ---------------------------------------------------------------------------

/// Try to render one record's BinXML as JSON via the compiled-template path.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_render_json_compiled<'a>(
    bytes: &[u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    progs: &mut JsonProgramCache,
    pf: &mut Preflight<JsonProgram>,
    settings: &ParserSettings,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> bool {
    let inst_off = match bytes.first() {
        Some(0x0f) if bytes.len() > 5 && bytes[4] == 0x0c => 5,
        Some(0x0c) => 1,
        _ => return false,
    };
    let data_start = chunk.data.as_ptr() as usize;
    let slice_start = bytes.as_ptr() as usize;
    if slice_start < data_start || slice_start + bytes.len() > data_start + chunk.data.len() {
        return false;
    }
    let stream_offset = slice_start - data_start;

    pf.clear();
    let (prog, slot_range, end) = match pf.scan_instance(
        chunk,
        stream_offset + inst_off,
        0,
        cache,
        progs,
        settings,
        0,
        true,
    ) {
        Ok(v) => v,
        Err(PreflightBail) => return false,
    };
    let consumed = end - stream_offset;
    if consumed > bytes.len() || (consumed < bytes.len() && bytes[consumed] != 0x00) {
        return false;
    }

    let start = out.len();
    match exec_json(&prog, slot_range, pf, chunk, vr, out) {
        Ok(()) => true,
        Err(_) => {
            out.truncate(start);
            false
        }
    }
}

/// Whether `ty` renders as a bare JSON number/bool (`write_value_as_number`).
fn json_bare_type(ty: u8) -> bool {
    matches!(ty, 0x03..=0x0a | 0x0d)
}

fn exec_json(
    prog: &JsonProgram,
    slot_range: SlotRange,
    pf: &Preflight<JsonProgram>,
    chunk: &EvtxChunk<'_>,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> Result<()> {
    let lits = &prog.lits;
    let slot_at = |slot: u16| -> Option<RawSlot> {
        if u32::from(slot) < slot_range.1 - slot_range.0 {
            Some(pf.slots[slot_range.0 as usize + slot as usize])
        } else {
            None
        }
    };

    macro_rules! write_lit {
        ($r:expr) => {{
            let (a, b) = $r;
            out.extend_from_slice(&lits[a as usize..b as usize]);
        }};
    }
    macro_rules! write_scalar {
        ($s:expr) => {{
            let s = $s;
            let vb = &chunk.data[s.off as usize..s.off as usize + usize::from(s.len)];
            let ansi = (s.ansi != NO_ANSI).then(|| pf.ansi[s.ansi as usize].as_str());
            if json_bare_type(s.ty) {
                vr.write_raw_value_text(out, s.ty, vb, ansi, StringEscapeMode::Json)?;
            } else {
                out.push(b'"');
                vr.write_raw_value_text(out, s.ty, vb, ansi, StringEscapeMode::Json)?;
                out.push(b'"');
            }
        }};
    }

    for op in &prog.ops {
        match op {
            JOp::Lit(r) => write_lit!(*r),
            JOp::LeafVal { slot, empty } => match slot_at(*slot) {
                None => write_lit!(*empty),
                Some(s) => {
                    if pf.slot_empty(&s, chunk.data) {
                        write_lit!(*empty);
                    } else if s.ty == 0x21 {
                        if s.nested == NO_NESTED {
                            return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                                "unresolved nested instance in compiled JSON",
                            ));
                        }
                        let inst = &pf.nested[s.nested as usize];
                        out.push(b'{');
                        out.push(b'"');
                        out.extend_from_slice(&inst.prog.root_name);
                        out.extend_from_slice(b"\":");
                        exec_json(&inst.prog, inst.slots, pf, chunk, vr, out)?;
                        out.push(b'}');
                    } else {
                        write_scalar!(s);
                    }
                }
            },
            JOp::Elem {
                attrs,
                content,
                empty,
            } => {
                // Evaluate attribute member presence.
                let mut attr_present = false;
                for part in attrs.iter() {
                    if part.slot == u16::MAX {
                        attr_present = true;
                        break;
                    }
                    if let Some(s) = slot_at(part.slot)
                        && !pf.slot_empty(&s, chunk.data)
                    {
                        attr_present = true;
                        break;
                    }
                }
                let content_slot =
                    content.and_then(|c| slot_at(c).filter(|s| !pf.slot_empty(s, chunk.data)));
                match (attr_present, content_slot) {
                    (false, None) => write_lit!(*empty),
                    (false, Some(s)) => write_scalar!(s),
                    (true, content_slot) => {
                        out.extend_from_slice(b"{\"#attributes\":{");
                        let mut first = true;
                        for part in attrs.iter() {
                            if part.slot == u16::MAX {
                                if !first {
                                    out.push(b',');
                                }
                                first = false;
                                write_lit!(part.lit_member);
                            } else if let Some(s) = slot_at(part.slot)
                                && !pf.slot_empty(&s, chunk.data)
                            {
                                if !first {
                                    out.push(b',');
                                }
                                first = false;
                                write_lit!(part.key);
                                write_scalar!(s);
                            }
                        }
                        out.push(b'}');
                        if let Some(s) = content_slot {
                            out.extend_from_slice(b",\"#text\":");
                            write_scalar!(s);
                        }
                        out.push(b'}');
                    }
                }
            }
            JOp::SlotChild {
                slot,
                static_names,
                lead_comma,
            } => {
                let Some(s) = slot_at(*slot) else { continue };
                if pf.slot_empty(&s, chunk.data) || s.ty != 0x21 {
                    continue; // constraint guarantees elem-or-empty
                }
                if s.nested == NO_NESTED {
                    return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                        "unresolved nested instance in compiled JSON",
                    ));
                }
                let inst = &pf.nested[s.nested as usize];
                let name = inst.prog.root_name.as_slice();
                // `_N` suffix: static members first, then prior dynamics.
                let mut count: u16 = 0;
                for (r, c) in static_names.iter() {
                    if &lits[r.0 as usize..r.1 as usize] == name {
                        count = *c;
                        break;
                    }
                }
                // (Prior dynamic same-name members would need scratch state;
                // a second dynamic member with a colliding name is not
                // expressible in compiled shapes today: one SlotChild per
                // object is enforced at compile time via `seen_slot_child`.)
                if *lead_comma {
                    out.push(b',');
                }
                out.push(b'"');
                out.extend_from_slice(name);
                if count > 0 {
                    out.push(b'_');
                    out.extend_from_slice(count.to_string().as_bytes());
                }
                out.extend_from_slice(b"\":");
                exec_json(&inst.prog, inst.slots, pf, chunk, vr, out)?;
            }
        }
    }
    Ok(())
}

/// Benchmark-only entry to the JSON text-content path.
#[cfg(feature = "bench")]
pub(crate) fn bench_json_text_content(out: &mut Vec<u8>, nodes: &[Node<'_>]) -> Result<()> {
    let mut c = JsonCompiler {
        tree: None,
        lits: std::mem::take(out),
        ops: Vec::new(),
        run_start: 0,
        elem_slots: Vec::new(),
        constraints: Vec::new(),
        vr: ValueRenderer::new(),
        formatter: sonic_rs::format::CompactFormatter,
        separate: false,
    };
    let res = c.text_content_plain(nodes, false);
    *out = c.lits;
    res
}

// ---------------------------------------------------------------------------
// JSON materialized layer (the old JsonEmitter semantics over plain nodes)
// ---------------------------------------------------------------------------
//
// These methods are the single implementation of JSON record rendering: the
// slow lane walks fully materialized trees through them, and the compiler
// calls them for placeholder-free pieces (subtrees, literal attributes,
// literal `Data` elements). Placeholder nodes are real errors here, mirroring
// the legacy emitter's behavior on materialized trees.

/// Render a fully materialized record tree to JSON (the single-walker slow
/// lane), honoring `--separate-json-attributes`.
pub(crate) fn render_tree_json(
    tree: &IrTree<'_>,
    settings: &ParserSettings,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut c = JsonCompiler {
        tree: Some(tree),
        lits: std::mem::take(out),
        ops: Vec::new(),
        run_start: 0,
        elem_slots: Vec::new(),
        constraints: Vec::new(),
        vr: ValueRenderer::new(),
        formatter: sonic_rs::format::CompactFormatter,
        separate: settings.should_separate_json_attributes(),
    };
    let res = c.render_root_plain();
    debug_assert!(c.ops.is_empty(), "materialized walk produced ops");
    *out = c.lits;
    res
}

impl<'t, 'a> JsonCompiler<'t, 'a> {
    /// Mirrors `render_json_with_scope` for a materialized tree.
    fn render_root_plain(&mut self) -> Result<()> {
        let root = self.tree().root_element();
        self.lits.push(b'{');
        if self.separate {
            if !root.attrs.is_empty() && self.render_separate_attrs_plain(root, 0)? {
                self.lits.push(b',');
            }
            self.key_with_suffix_plain(root.name.as_str(), 0);
            self.write_element_value_no_attrs_plain(root, false)?;
        } else {
            self.lits.push(b'"');
            self.lits.extend_from_slice(root.name.as_str().as_bytes());
            self.lits.extend_from_slice(b"\":");
            self.write_element_value_plain(root, false)?;
        }
        self.lits.push(b'}');
        Ok(())
    }

    fn element_ref(&self, id: ElementId) -> &'t Element<'a> {
        self.tree().arena().get(id).expect("invalid element id")
    }

    // --- small writers ---

    fn key_with_suffix_plain(&mut self, name: &str, suffix: u16) {
        self.lits.push(b'"');
        self.lits.extend_from_slice(name.as_bytes());
        if suffix > 0 {
            self.lits.push(b'_');
            self.lits.extend_from_slice(suffix.to_string().as_bytes());
        }
        self.lits.extend_from_slice(b"\":");
    }

    fn json_escaped_str_plain(&mut self, s: &str) -> Result<()> {
        use sonic_rs::format::Formatter;
        self.formatter
            .write_string_fast(&mut self.lits, s, false)
            .map_err(crate::err::EvtxError::from)?;
        Ok(())
    }

    fn json_text_plain(&mut self, text: &Text<'_>) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        match text {
            Text::Utf16(value) => {
                let bytes = value.as_bytes();
                let units = bytes.len() / 2;
                if units > 0 {
                    utf16_simd::write_json_utf16le(&mut self.lits, bytes, units, false)
                        .map_err(crate::err::EvtxError::from)?;
                }
                Ok(())
            }
            Text::Utf8(value) => self.json_escaped_str_plain(value),
        }
    }

    fn write_u64_plain(&mut self, v: u64) -> Result<()> {
        use sonic_rs::format::Formatter;
        self.formatter
            .write_u64(&mut self.lits, v)
            .map_err(crate::err::EvtxError::from)?;
        Ok(())
    }

    fn write_i64_plain(&mut self, v: i64) -> Result<()> {
        use sonic_rs::format::Formatter;
        self.formatter
            .write_i64(&mut self.lits, v)
            .map_err(crate::err::EvtxError::from)?;
        Ok(())
    }

    /// Mirrors `write_value_as_number`.
    fn value_as_number_plain(
        &mut self,
        value: &crate::binxml::value_variant::BinXmlValue<'_>,
    ) -> Result<bool> {
        use crate::binxml::value_variant::BinXmlValue;
        match value {
            BinXmlValue::Int8Type(v) => self.write_i64_plain(i64::from(*v)).map(|_| true),
            BinXmlValue::Int16Type(v) => self.write_i64_plain(i64::from(*v)).map(|_| true),
            BinXmlValue::Int32Type(v) => self.write_i64_plain(i64::from(*v)).map(|_| true),
            BinXmlValue::Int64Type(v) => self.write_i64_plain(*v).map(|_| true),
            BinXmlValue::UInt8Type(v) => self.write_u64_plain(u64::from(*v)).map(|_| true),
            BinXmlValue::UInt16Type(v) => self.write_u64_plain(u64::from(*v)).map(|_| true),
            BinXmlValue::UInt32Type(v) => self.write_u64_plain(u64::from(*v)).map(|_| true),
            BinXmlValue::UInt64Type(v) => self.write_u64_plain(*v).map(|_| true),
            BinXmlValue::BoolType(v) => {
                self.lits
                    .extend_from_slice(if *v { b"true" } else { b"false" });
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    // --- content scans (scan_class over plain nodes, ctx-None semantics) ---

    /// Mirrors `scan_class` with a materialized scope: is this node non-empty
    /// "text-like" content?
    fn node_is_content_plain(node: &Node<'_>) -> bool {
        match node {
            Node::Text(t) | Node::CData(t) => !t.is_empty(),
            Node::EntityRef(_) | Node::CharRef(_) => true,
            Node::Value(v) => !crate::model::ir::is_optional_empty(v),
            // Materialized trees should not contain placeholders; classify as
            // content so the emitting pass reports the error.
            Node::Placeholder(_) => true,
            Node::Element(_) | Node::PITarget(_) | Node::PIData(_) => false,
        }
    }

    fn has_text_content_plain(nodes: &[Node<'_>]) -> bool {
        nodes.iter().any(Self::node_is_content_plain)
    }

    /// Mirrors `content_layout`: `(has_text, has_element_child)`.
    fn content_layout_plain(element: &Element<'_>) -> (bool, bool) {
        let mut has_text = false;
        let mut has_element_child = element.has_element_child;
        for node in &element.children {
            if matches!(node, Node::Element(_)) {
                has_element_child = true;
            } else if Self::node_is_content_plain(node) {
                has_text = true;
            }
            if has_text && has_element_child {
                break;
            }
        }
        (has_text, has_element_child)
    }

    /// Mirrors `attr_flags`: `(has_any, has_any_non_empty_value)`.
    fn attr_flags_plain(attrs: &[Attr<'_>]) -> (bool, bool) {
        if attrs.is_empty() {
            return (false, false);
        }
        for attr in attrs {
            if Self::has_text_content_plain(&attr.value) {
                return (true, true);
            }
        }
        (true, false)
    }

    // --- text/value content rendering (mirrors write_json_text_content etc.) ---

    /// Mirrors `write_json_text_content` (no surrounding quotes).
    fn text_content_plain(&mut self, nodes: &[Node<'a>], skip_elements: bool) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(text) | Node::CData(text) => self.json_text_plain(text)?,
                Node::Value(value) => {
                    let mut sink = std::mem::take(&mut self.lits);
                    let res = self.vr.write_json_value_text(&mut sink, value);
                    self.lits = sink;
                    res?;
                }
                Node::CharRef(ch) => {
                    // In JSON, emit the resolved character (not `&#...;`).
                    if let Some(ch) = char::from_u32(u32::from(*ch)) {
                        let mut buf = [0_u8; 4];
                        let s = ch.encode_utf8(&mut buf);
                        self.json_escaped_str_plain(s)?;
                    } else {
                        self.lits.extend_from_slice(b"&#");
                        self.write_u64_plain(u64::from(*ch))?;
                        self.lits.push(b';');
                    }
                }
                Node::EntityRef(name) => {
                    let resolved = match name.as_str() {
                        "quot" => Some("\""),
                        "apos" => Some("'"),
                        "amp" => Some("&"),
                        "lt" => Some("<"),
                        "gt" => Some(">"),
                        _ => None,
                    };
                    match resolved {
                        Some(s) => self.json_escaped_str_plain(s)?,
                        None => {
                            self.lits.push(b'&');
                            self.lits.extend_from_slice(name.as_str().as_bytes());
                            self.lits.push(b';');
                        }
                    }
                }
                Node::PITarget(_) | Node::PIData(_) => {}
                Node::Placeholder(_) => return Err(unresolved_placeholder()),
                Node::Element(_) => {
                    if skip_elements {
                        continue;
                    }
                    return Err(crate::err::EvtxError::FailedToCreateRecordModel(
                        "unexpected element node in text context",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Mirrors `try_write_as_number` / `try_write_as_number_skip_elements`.
    fn try_as_number_plain(&mut self, nodes: &[Node<'a>], skip_elements: bool) -> Result<bool> {
        let mut single: Option<&Node<'a>> = None;
        for node in nodes {
            match node {
                Node::Element(_) => {
                    if skip_elements {
                        continue;
                    }
                    return Ok(false);
                }
                Node::Text(t) | Node::CData(t) => {
                    if skip_elements {
                        if !t.is_empty() {
                            return Ok(false);
                        }
                    } else if single.is_some() || !t.is_empty() {
                        return Ok(false);
                    } else {
                        // Non-skip mode counts every node; an empty text node
                        // still occupies the single slot.
                        single = Some(node);
                    }
                }
                Node::Value(v) => {
                    if skip_elements && crate::model::ir::is_optional_empty(v) {
                        continue;
                    }
                    if single.is_some() {
                        return Ok(false);
                    }
                    single = Some(node);
                }
                Node::CharRef(_) | Node::EntityRef(_) => return Ok(false),
                Node::PITarget(_) | Node::PIData(_) => {
                    if !skip_elements && single.is_some() {
                        return Ok(false);
                    }
                    if !skip_elements {
                        single = Some(node);
                    }
                }
                Node::Placeholder(_) => {
                    if skip_elements {
                        return Err(unresolved_placeholder());
                    }
                    return Ok(false);
                }
            }
        }
        match single {
            Some(Node::Value(value)) => self.value_as_number_plain(value),
            _ => Ok(false),
        }
    }

    /// Mirrors `render_content_as_json_value`.
    fn content_as_json_value_plain(
        &mut self,
        nodes: &[Node<'a>],
        skip_elements: bool,
    ) -> Result<()> {
        if self.try_as_number_plain(nodes, skip_elements)? {
            return Ok(());
        }
        self.lits.push(b'"');
        self.text_content_plain(nodes, skip_elements)?;
        self.lits.push(b'"');
        Ok(())
    }

    // --- attributes ---

    /// Mirrors `render_attributes_object_body`.
    fn attrs_object_body_plain(&mut self, attrs: &[Attr<'a>]) -> Result<bool> {
        let mut wrote_any = false;
        let mut first = true;
        for attr in attrs {
            if !Self::has_text_content_plain(&attr.value) {
                continue;
            }
            if !first {
                self.lits.push(b',');
            }
            first = false;
            wrote_any = true;
            self.lits.push(b'"');
            self.lits.extend_from_slice(attr.name.as_str().as_bytes());
            self.lits.extend_from_slice(b"\":");
            if self.try_as_number_plain(&attr.value, false)? {
                continue;
            }
            self.lits.push(b'"');
            self.text_content_plain(&attr.value, false)?;
            self.lits.push(b'"');
        }
        Ok(wrote_any)
    }

    /// Mirrors `render_attributes_object`.
    fn attrs_object_plain(&mut self, attrs: &[Attr<'a>]) -> Result<bool> {
        let (_has_any, has_text) = Self::attr_flags_plain(attrs);
        if !has_text {
            return Ok(false);
        }
        self.lits.extend_from_slice(b"\"#attributes\":{");
        let wrote_any = self.attrs_object_body_plain(attrs)?;
        self.lits.push(b'}');
        Ok(wrote_any)
    }

    /// Mirrors `render_separate_attributes_for_element`.
    fn render_separate_attrs_plain(&mut self, element: &Element<'a>, suffix: u16) -> Result<bool> {
        if element.attrs.is_empty() {
            return Ok(false);
        }
        let (_has_any, has_text) = Self::attr_flags_plain(&element.attrs);
        if !has_text {
            return Ok(false);
        }
        self.lits.push(b'"');
        self.lits
            .extend_from_slice(element.name.as_str().as_bytes());
        if suffix > 0 {
            self.lits.push(b'_');
            self.write_u64_plain(u64::from(suffix))?;
        }
        self.lits.extend_from_slice(b"_attributes\":{");
        let wrote_any = self.attrs_object_body_plain(&element.attrs)?;
        self.lits.push(b'}');
        Ok(wrote_any)
    }

    // --- element values ---

    /// Mirrors `try_write_leaf_value`.
    fn try_leaf_value_plain(
        &mut self,
        element: &Element<'a>,
        empty_as_string: bool,
    ) -> Result<bool> {
        if element.has_element_child || element.children.len() != 1 {
            return Ok(false);
        }
        let empty: &[u8] = if empty_as_string { b"\"\"" } else { b"null" };
        match &element.children[0] {
            Node::Text(text) => {
                if text.is_empty() {
                    self.lits.extend_from_slice(empty);
                } else {
                    self.lits.push(b'"');
                    self.json_text_plain(text)?;
                    self.lits.push(b'"');
                }
                Ok(true)
            }
            Node::Value(value) => {
                if crate::model::ir::is_optional_empty(value) {
                    self.lits.extend_from_slice(empty);
                    return Ok(true);
                }
                if self.value_as_number_plain(value)? {
                    return Ok(true);
                }
                self.lits.push(b'"');
                let mut sink = std::mem::take(&mut self.lits);
                let res = self.vr.write_json_value_text(&mut sink, value);
                self.lits = sink;
                res?;
                self.lits.push(b'"');
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Mirrors `render_data_element_value` (`""` for the empty case).
    fn data_element_value_plain(&mut self, element: &Element<'a>) -> Result<()> {
        if self.try_leaf_value_plain(element, true)? {
            return Ok(());
        }
        let (has_text, has_element_child) = Self::content_layout_plain(element);
        if !has_text && !has_element_child {
            self.lits.extend_from_slice(b"\"\"");
            return Ok(());
        }
        if has_element_child {
            self.element_body_plain(element, false, true, has_text)
        } else {
            self.content_as_json_value_plain(&element.children, false)
        }
    }

    /// Mirrors `write_element_value_no_attrs` (separate-attrs mode).
    fn write_element_value_no_attrs_plain(
        &mut self,
        element: &Element<'a>,
        child_is_container: bool,
    ) -> Result<()> {
        if self.try_leaf_value_plain(element, false)? {
            return Ok(());
        }
        let (has_text, has_element_child) = Self::content_layout_plain(element);
        if !has_element_child && !has_text {
            self.lits.extend_from_slice(b"null");
            Ok(())
        } else if !has_element_child {
            self.content_as_json_value_plain(&element.children, false)
        } else {
            self.element_body_plain(element, child_is_container, true, has_text)
        }
    }

    /// Mirrors `write_element_value` (default mode).
    fn write_element_value_plain(
        &mut self,
        element: &Element<'a>,
        child_is_container: bool,
    ) -> Result<()> {
        if element.attrs.is_empty() && self.try_leaf_value_plain(element, false)? {
            return Ok(());
        }
        let (has_text, has_element_child) = Self::content_layout_plain(element);
        let (_has_attrs_any, has_attrs_text) = Self::attr_flags_plain(&element.attrs);

        if !has_element_child && !has_text && !has_attrs_text {
            self.lits.extend_from_slice(b"null");
            Ok(())
        } else if !has_element_child && !has_attrs_text {
            self.content_as_json_value_plain(&element.children, false)
        } else {
            self.element_body_plain(element, child_is_container, false, has_text)
        }
    }

    /// Mirrors `write_element_body_json` (materialized: no array expansion).
    fn element_body_plain(
        &mut self,
        element: &Element<'a>,
        in_data_container: bool,
        omit_attributes: bool,
        has_text: bool,
    ) -> Result<()> {
        // Flatten detection: one non-empty `Data[@Name]` selects named form.
        let mut should_flatten_named_data = false;
        if in_data_container {
            for node in &element.children {
                let Node::Element(id) = node else { continue };
                let child = self.element_ref(*id);
                if !is_data_element_name(child.name.as_str()) {
                    continue;
                }
                let Some(name_nodes) = child
                    .attrs
                    .iter()
                    .find(|a| a.name.as_str() == "Name")
                    .map(|a| &a.value)
                else {
                    continue;
                };
                if Self::has_text_content_plain(name_nodes) {
                    should_flatten_named_data = true;
                    break;
                }
            }
        }

        let mut name_counts: Vec<(&'t str, u16)> = Vec::new();

        self.lits.push(b'{');
        let mut wrote_any = false;

        if !omit_attributes
            && !element.attrs.is_empty()
            && self.attrs_object_plain(&element.attrs)?
        {
            wrote_any = true;
        }

        if has_text {
            if wrote_any {
                self.lits.push(b',');
            }
            wrote_any = true;
            self.lits.extend_from_slice(b"\"#text\":");
            self.content_as_json_value_plain(&element.children, true)?;
        }

        let positional_data_count = if in_data_container && !should_flatten_named_data {
            element
                .children
                .iter()
                .filter(|n| {
                    matches!(n, Node::Element(id)
                        if is_data_element_name(self.element_ref(*id).name.as_str()))
                })
                .count()
        } else {
            0
        };
        let mut positional_data_emitted = false;

        for node in &element.children {
            let Node::Element(id) = node else { continue };
            let child = self.element_ref(*id);

            if in_data_container && is_data_element_name(child.name.as_str()) {
                if should_flatten_named_data {
                    let Some(name_nodes) = child
                        .attrs
                        .iter()
                        .find(|a| a.name.as_str() == "Name")
                        .map(|a| &a.value)
                    else {
                        continue;
                    };
                    if !Self::has_text_content_plain(name_nodes) {
                        continue;
                    }
                    if wrote_any {
                        self.lits.push(b',');
                    }
                    wrote_any = true;
                    self.lits.push(b'"');
                    self.text_content_plain(name_nodes, false)?;
                    self.lits.extend_from_slice(b"\":");
                    self.data_element_value_plain(child)?;
                } else if !positional_data_emitted && positional_data_count > 0 {
                    if wrote_any {
                        self.lits.push(b',');
                    }
                    wrote_any = true;
                    positional_data_emitted = true;
                    self.lits.extend_from_slice(b"\"Data\":{\"#text\":");
                    if positional_data_count == 1 {
                        self.data_element_value_plain(child)?;
                    } else {
                        self.lits.push(b'[');
                        let mut first = true;
                        for node2 in &element.children {
                            let Node::Element(id2) = node2 else { continue };
                            let candidate = self.element_ref(*id2);
                            if !is_data_element_name(candidate.name.as_str()) {
                                continue;
                            }
                            if !first {
                                self.lits.push(b',');
                            }
                            first = false;
                            self.data_element_value_plain(candidate)?;
                        }
                        self.lits.push(b']');
                    }
                    self.lits.push(b'}');
                }
                continue;
            }

            // Normal child member with `_N` suffixing.
            let cname = child.name.as_str();
            let suffix = {
                let mut found = None;
                for (n, c) in name_counts.iter_mut() {
                    if *n == cname {
                        let s = *c;
                        *c += 1;
                        found = Some(s);
                        break;
                    }
                }
                match found {
                    Some(s) => s,
                    None => {
                        name_counts.push((cname, 1));
                        0
                    }
                }
            };

            if wrote_any {
                self.lits.push(b',');
            }
            wrote_any = true;

            let child_is_container = is_data_container_name(cname);
            if self.separate {
                let wrote_attrs = self.render_separate_attrs_plain(child, suffix)?;
                let (child_text, child_elem) = Self::content_layout_plain(child);
                let child_has_value = child_elem || child_text;
                let write_value = child_has_value || !wrote_attrs;
                if wrote_attrs && write_value {
                    self.lits.push(b',');
                }
                if write_value {
                    self.key_with_suffix_plain(cname, suffix);
                    self.write_element_value_no_attrs_plain(child, child_is_container)?;
                }
            } else {
                self.key_with_suffix_plain(cname, suffix);
                self.write_element_value_plain(child, child_is_container)?;
            }
        }

        self.lits.push(b'}');
        Ok(())
    }
}
