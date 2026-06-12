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
use crate::binxml::ir_xml::render_xml_element_materialized;
use crate::binxml::value_render::{StringEscapeMode, ValueRenderer};
use crate::err::Result;
use crate::evtx_chunk::EvtxChunk;
use crate::model::ir::{Attr, Element, ElementId, IrTree, Node, Placeholder, Text};
use ahash::AHashMap;
use std::rc::Rc;

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
pub(crate) type XmlProgramCache = AHashMap<(u32, u16, bool), Option<Rc<XmlProgram>>>;

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
        Err(Bail) => None,
    }
}

impl<'t, 'a> XmlCompiler<'t, 'a> {
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

    fn element(&self, id: ElementId) -> &'t Element<'a> {
        self.tree.arena().get(id).expect("invalid element id")
    }

    fn compile_element(&mut self, id: ElementId, indent: u16) -> std::result::Result<(), Bail> {
        let element = self.element(id);

        // Placeholder-free subtree: render with the real emitter for exact
        // byte parity (covers names, attrs, layout, Binary special-casing).
        if !subtree_has_placeholder(self.tree, element) {
            render_xml_element_materialized(
                self.tree.arena(),
                id,
                indent as usize,
                self.indent_on,
                &mut self.lits,
            )
            .map_err(|_| Bail)?;
            return Ok(());
        }

        self.indent_str(indent);
        self.lits.push(b'<');
        self.lits
            .extend_from_slice(element.name.as_str().as_bytes());

        for attr in &element.attrs {
            self.compile_attr(attr)?;
        }

        match classify_children(element) {
            ChildrenKind::SinglePlaceholder(ph) => {
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
                for node in &element.children {
                    self.compile_literal_content_node(node, false)?;
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
            ChildrenKind::Bail => return Err(Bail),
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

    fn compile_attr(&mut self, attr: &Attr<'a>) -> std::result::Result<(), Bail> {
        // Placeholders are dynamic; everything else is compile-time constant.
        // Mirrors `attribute_value_is_empty` + `render_nodes`.
        let mut has_nonempty_const = false;
        let mut n_placeholders = 0usize;
        for node in attr.value.iter() {
            match node {
                Node::Placeholder(_) => n_placeholders += 1,
                Node::Element(_) | Node::PITarget(_) | Node::PIData(_) => return Err(Bail),
                Node::Text(t) | Node::CData(t) => {
                    if !t.is_empty() {
                        has_nonempty_const = true;
                    }
                }
                Node::EntityRef(_) | Node::CharRef(_) => has_nonempty_const = true,
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
            return Err(Bail);
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
    fn compile_literal_content_node(
        &mut self,
        node: &Node<'a>,
        in_attribute: bool,
    ) -> std::result::Result<(), Bail> {
        match node {
            Node::Text(text) => self.compile_literal_text(text, in_attribute),
            Node::Value(value) => {
                let mut sink = std::mem::take(&mut self.lits);
                let res = self.vr.write_xml_value_text(&mut sink, value, in_attribute);
                self.lits = sink;
                res.map_err(|_| Bail)
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
            Node::Element(_) | Node::Placeholder(_) => Err(Bail),
        }
    }

    fn compile_literal_text(
        &mut self,
        text: &Text<'a>,
        in_attribute: bool,
    ) -> std::result::Result<(), Bail> {
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
                res.map_err(|_| Bail)
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
            Node::PITarget(_) | Node::PIData(_) => return ChildrenKind::Bail,
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
        if has_literal_content {
            return ChildrenKind::StaticInline;
        }
        return ChildrenKind::Empty;
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

struct NestedInst {
    prog: Rc<XmlProgram>,
    slots: SlotRange,
}

/// Reusable per-chunk pre-flight scratch.
#[derive(Default)]
pub(crate) struct Preflight {
    slots: Vec<RawSlot>,
    nested: Vec<NestedInst>,
    ansi: Vec<String>,
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

impl Preflight {
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
        progs: &mut XmlProgramCache,
        settings: &ParserSettings,
        base_indent: u16,
        is_root: bool,
    ) -> std::result::Result<(Rc<XmlProgram>, SlotRange, usize), PreflightBail> {
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
        for &(slot_id, child_indent) in &prog.elem_slots {
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
                // Generic fragment: rendered via the materialized fallback.
                _ => continue,
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

fn get_or_compile<'a>(
    chunk: &'a EvtxChunk<'a>,
    def_offset: u32,
    base_indent: u16,
    is_root: bool,
    cache: &mut IrTemplateCache<'a>,
    progs: &mut XmlProgramCache,
    settings: &ParserSettings,
) -> Option<Rc<XmlProgram>> {
    let key = (def_offset, base_indent, is_root);
    if let Some(entry) = progs.get(&key) {
        return entry.clone();
    }
    let compiled =
        cache
            .template_for_compile(chunk, def_offset)
            .ok()
            .and_then(|(tree, has_literal_array)| {
                compile_xml_template(&tree, has_literal_array, base_indent, is_root, settings)
                    .map(Rc::new)
            });
    progs.insert(key, compiled.clone());
    compiled
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
    pf: &mut Preflight,
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
    pf: &Preflight,
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
    pf: &Preflight,
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
    render_xml_element_materialized(tree.arena(), tree.root(), indent as usize, indent_on, out)
}
