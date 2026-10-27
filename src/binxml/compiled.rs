//! Compiled templates: the per-record fast path.
//!
//! Each cached definition compiles into a flat program of literal ranges and
//! value operations. The executors accept checked byte spans from the CLI or
//! retained, validated substitutions from the record iterator.
//!
//! Raw input needs descriptor preflight. Retained values only need program
//! compatibility checks: their types, bounds, nested templates and fragments
//! were validated before the record was yielded. Both sources share the same
//! executors and value formatters.
//!
//! Unsupported shapes use the materialized walker. The record API builds that
//! tree from retained values and fragments without parsing the record again;
//! the CLI materializes its raw input. Failed execution rolls output back
//! before falling back.

use crate::ChunkOffset;
use crate::ParserSettings;
use crate::binxml::ir::{
    IrTemplateCache, TEMPLATE_DEFINITION_HEADER_SIZE, TemplateContent, ValidatedValue,
    binxml_slice_offset_in, build_tree_from_binxml_bytes_direct,
    read_template_definition_header_at, template_body_at,
};
use crate::binxml::name::read_name_link;
use crate::binxml::tokens::{Stream, TokenSink, ends_stream, single_instance_offset, walk_tokens};
use crate::binxml::value_render::{StringEscapeMode, ValueRenderer};
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType, decode_ansi_value, value_ty};
use crate::err::{EvtxError, Result};
use crate::evtx_chunk::EvtxChunk;
use crate::model::ir::{Attr, Element, ElementId, IrArena, IrTree, Node, Placeholder, Text};
use crate::mta::{self, RenderingInfo, SystemFields};
use crate::utils::{ByteCursor, Utf16LeSlice};
use ahash::AHashMap;
use std::hash::{BuildHasher, Hasher};
use std::sync::Arc;

const INDENT_WIDTH: u16 = 2;
const XML_DECL: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

/// A `lits` byte range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LitRange(u32, u32);

impl LitRange {
    /// The literal bytes this range denotes.
    fn of(self, lits: &[u8]) -> &[u8] {
        &lits[self.0 as usize..self.1 as usize]
    }
}
/// A `Preflight::slots` index range (one instance's slots).
type SlotRange = (u32, u32);

/// Duplicate-key `_N` suffix counter: 0 the first time `name` is seen (no
/// suffix), then 1, 2, … for repeats. Shared by the JSON compiler (compile
/// time) and the materialized layer (per record) so the rule cannot drift.
fn next_name_suffix<K: PartialEq>(name: K, counts: &mut Vec<(K, u16)>) -> u16 {
    if let Some((_, c)) = counts.iter_mut().find(|(n, _)| *n == name) {
        let suffix = *c;
        *c += 1;
        suffix
    } else {
        counts.push((name, 1));
        0
    }
}

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
    /// An attribute-less `<Data>` element whose entire content is one hole
    /// (MS-EVEN6 array expansion site). Scalar values follow `Body`
    /// semantics with `open` spliced first; a string array repeats the
    /// element per item — single-item arrays render inline like a scalar,
    /// empty items in larger arrays take the two-line empty form. Mirrors
    /// `array_expand` over the materialized tree.
    Expand {
        slot: u16,
        optional: bool,
        indent: u16,
        open: LitRange,
        tail_text: LitRange,
        tail_empty: LitRange,
        tail_elem: LitRange,
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
    /// Slots compiled as `XOp::Expand` (string arrays allowed).
    expand_slots: Vec<u16>,
    root_name: Vec<u8>,
    system_fields: Option<SystemFields>,
}

/// Per-chunk programs have stable indices, so records borrow them without
/// cloning the cross-chunk Arc. `None` caches failed compilations.
pub(crate) struct ProgramCache<P> {
    by_key: AHashMap<(u32, u16, bool), Option<usize>>,
    programs: Vec<Arc<P>>,
}

impl<P> Default for ProgramCache<P> {
    fn default() -> Self {
        Self {
            by_key: AHashMap::default(),
            programs: Vec::new(),
        }
    }
}

impl<P> ProgramCache<P> {
    fn insert(&mut self, key: (u32, u16, bool), program: Option<Arc<P>>) -> Option<usize> {
        let index = program.map(|program| {
            let index = self.programs.len();
            self.programs.push(program);
            index
        });
        self.by_key.insert(key, index);
        index
    }
}

/// Cross-chunk program store: templates are identical across chunks (same
/// GUID + size + definition bytes), so programs compile once per file/parser
/// instead of once per chunk. Shared across worker threads.
#[derive(Default)]
pub(crate) struct ProgramStore {
    /// Name references per template body (`None`: never shared).
    names: std::sync::RwLock<AHashMap<TemplateKey, Option<Arc<NameRefs>>>>,
    xml: Shard<XmlProgram>,
    json: Shard<JsonProgram>,
    hasher: ahash::RandomState,
}

/// Template identity: GUID, data size, body-bytes hash.
type TemplateKey = ([u8; 16], u32, u64);

/// The names a body resolves in one chunk: hash of the name entries the body
/// does not hold, and the definition offset when its parse depends on it.
type NamesKey = (u64, Option<u32>);

/// Content identity of a compiled program: template identity and the names
/// it resolves, plus the compile parameters.
type StoreKey = (TemplateKey, NamesKey, u16, bool);

type Shard<P> = std::sync::RwLock<AHashMap<StoreKey, Option<Arc<P>>>>;

impl ProgramStore {
    /// Content identity of the template at `def_offset` in `chunk`.
    fn key(&self, chunk: &EvtxChunk<'_>, def_offset: u32) -> Option<(TemplateKey, NamesKey)> {
        let data = chunk.data;
        let header = read_template_definition_header_at(data, def_offset).ok()?;
        let body = template_body_at(data, def_offset, &header).ok()?;
        let template = (header.guid, header.data_size, self.hasher.hash_one(body));
        // Read guard dropped before the write below; hashing runs unlocked.
        let known = self
            .names
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&template)
            .cloned();
        let refs = match known {
            Some(refs) => refs,
            None => {
                let refs = NameRefs::of(chunk, def_offset, body.len()).map(Arc::new);
                self.names
                    .write()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(template, refs.clone());
                refs
            }
        }?;
        Some((template, refs.key(data, def_offset, &self.hasher)?))
    }
}

/// The names a template body references. Bodies name elements by chunk
/// offset, often outside the body, so equal bodies in two chunks can resolve
/// different names.
struct NameRefs {
    /// Where the body was walked. Name offsets are absolute and a name at
    /// a body position is parsed inline, so the same bytes elsewhere parse
    /// the same only if no referenced name moves into or out of the body.
    body: std::ops::Range<usize>,
    /// Some name starts inside `body`.
    inside: bool,
    /// Names whose entries the body does not wholly hold.
    outside: Vec<ChunkOffset>,
}

impl NameRefs {
    /// `None` when the body does not parse within its bounds, or embeds
    /// BinXML (whose names and templates are not tracked).
    fn of(chunk: &EvtxChunk<'_>, def_offset: u32, len: usize) -> Option<Self> {
        let start = def_offset as usize + TEMPLATE_DEFINITION_HEADER_SIZE;
        let mut refs = NameRefs {
            body: start..start + len,
            inside: false,
            outside: Vec::new(),
        };
        let bump = bumpalo::Bump::new();
        // Cut at the body end: a token read past it uses bytes the key misses.
        let src = Stream {
            data: chunk.data.get(..start + len)?,
            ..Stream::chunk(chunk, &bump)
        };
        walk_tokens(src, start, len, true, &mut refs).ok()?;
        // Entries wholly inside the body are covered by the body hash.
        let body = &refs.body;
        refs.outside.retain(|&offset| {
            name_entry(chunk.data, offset)
                .is_none_or(|e| (offset as usize) < body.start || e.end > body.end)
        });
        Some(refs)
    }

    /// The names the body at `def_offset` resolves in `data`.
    fn key(&self, data: &[u8], def_offset: u32, hasher: &ahash::RandomState) -> Option<NamesKey> {
        let start = def_offset as usize + TEMPLATE_DEFINITION_HEADER_SIZE;
        let body = start..start + self.body.len();
        if body != self.body
            && (self.inside || self.outside.iter().any(|&o| body.contains(&(o as usize))))
        {
            return None;
        }
        let mut state = hasher.build_hasher();
        for &offset in &self.outside {
            state.write(&data[name_entry(data, offset)?]);
        }
        Some((state.finish(), self.inside.then_some(def_offset)))
    }

    fn name(&mut self, offset: ChunkOffset) -> Result<()> {
        self.inside |= self.body.contains(&(offset as usize));
        if !self.outside.contains(&offset) {
            self.outside.push(offset);
        }
        Ok(())
    }
}

/// The rendered part (length + UTF-16 chars) of the string-table entry at
/// `offset`, after its link.
fn name_entry(data: &[u8], offset: ChunkOffset) -> Option<std::ops::Range<usize>> {
    let mut cursor = ByteCursor::with_pos(data, offset as usize).ok()?;
    read_name_link(&mut cursor).ok()?;
    let start = cursor.pos();
    let len = cursor.u16_named("name_len").ok()?;
    cursor.take_bytes(2 * usize::from(len), "name").ok()?;
    Some(start..cursor.pos())
}

impl<'a> TokenSink<'a> for NameRefs {
    fn open_start(&mut self, name: ChunkOffset) -> Result<()> {
        self.name(name)
    }
    fn attribute(&mut self, name: ChunkOffset) -> Result<()> {
        self.name(name)
    }
    fn entity_ref(&mut self, name: ChunkOffset) -> Result<()> {
        self.name(name)
    }
    fn pi_target(&mut self, name: ChunkOffset) -> Result<()> {
        self.name(name)
    }
    fn value(&mut self, value: BinXmlValue<'a>) -> Result<()> {
        match value {
            BinXmlValue::BinXmlType(_) => Err(EvtxError::FailedToCreateRecordModel(
                "embedded BinXML in template body",
            )),
            _ => Ok(()),
        }
    }
}

impl std::fmt::Debug for ProgramStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProgramStore").finish_non_exhaustive()
    }
}

/// Per-format shard access used by `get_or_compile`.
pub(crate) trait StoredProgram: TemplateProgram {
    fn shard(store: &ProgramStore) -> &Shard<Self>;
    /// Tag the record rendered at `out[start..]` as carved (CLI output).
    fn mark_recovered(out: &mut Vec<u8>, start: usize);
    /// Render `fragment` as the last child of the `Event` element rendered at `out[start..]`,
    /// as `render_tree` renders it in the tree. `false` when `out` does not end with that
    /// element or the fragment fails to render, and the caller discards `out[start..]`.
    fn append_to_event(
        fragment: &IrTree<'_>,
        settings: &ParserSettings,
        start: usize,
        out: &mut Vec<u8>,
    ) -> bool;
}

impl StoredProgram for XmlProgram {
    fn shard(store: &ProgramStore) -> &Shard<Self> {
        &store.xml
    }
    fn mark_recovered(out: &mut Vec<u8>, start: usize) {
        if out[start..].starts_with(XML_DECL) {
            let at = start + XML_DECL.len();
            out.splice(at..at, b"<!-- recovered -->\n".iter().copied());
        }
    }
    fn append_to_event(
        fragment: &IrTree<'_>,
        settings: &ParserSettings,
        start: usize,
        out: &mut Vec<u8>,
    ) -> bool {
        let indent = settings.should_indent();
        let close: &[u8] = if indent { b"</Event>\n" } else { b"</Event>" };
        if !out[start..].ends_with(close) {
            return false;
        }
        // The fragment goes before the closing tag.
        out.truncate(out.len() - close.len());
        let rendered =
            render_fragment_xml(fragment.arena(), fragment.root(), INDENT_WIDTH, indent, out);
        out.extend_from_slice(close);
        rendered.is_ok()
    }
}

impl StoredProgram for JsonProgram {
    fn mark_recovered(out: &mut Vec<u8>, start: usize) {
        if out.get(start) == Some(&b'{') {
            let mark: &[u8] = if out.get(start + 1) == Some(&b'\n') {
                b"\n  \"recovered\": true,"
            } else {
                b"\"recovered\":true,"
            };
            out.splice(start + 1..start + 1, mark.iter().copied());
        }
    }
    fn append_to_event(
        fragment: &IrTree<'_>,
        _settings: &ParserSettings,
        start: usize,
        out: &mut Vec<u8>,
    ) -> bool {
        // `{"Event":{...}}`: the fragment becomes the last member of `Event`.
        if !out[start..].ends_with(b"}}") {
            return false;
        }
        out.truncate(out.len() - 2);
        let mut c = JsonCompiler::new(Some(fragment), std::mem::take(out), false);
        let rendered = c.render_member_plain();
        *out = c.lits;
        out.extend_from_slice(b"}}");
        rendered.is_ok()
    }
    fn shard(store: &ProgramStore) -> &Shard<Self> {
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
    // Reusable plan contains only indices; records retain their own values.
    nested: Vec<Option<Instance<usize>>>,
}

impl std::fmt::Debug for RenderCaches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderCaches")
            .field("xml_programs", &self.xml.by_key.len())
            .field("json_programs", &self.json.by_key.len())
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

impl SlotConstraint {
    fn slot(self) -> u16 {
        match self {
            Self::ForbidElem(slot) | Self::ElemOrEmpty(slot) => slot,
        }
    }

    fn violated(self, is_elem: bool, empty: bool) -> bool {
        match self {
            Self::ForbidElem(_) => is_elem,
            Self::ElemOrEmpty(_) => !is_elem && !empty,
        }
    }
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
    /// Slots whose ops handle string-array values (element repetition /
    /// positional-Data aggregation). Arrays anywhere else bail.
    fn expand_slots(&self) -> &[u16] {
        &[]
    }
    /// The template's root element name.
    fn root_name(&self) -> &[u8];
    /// Where the program's records keep their `System` fields, when an `.MTA` file is set.
    /// `None` otherwise, and for templates whose records need their tree built for them.
    fn system_fields(&self) -> Option<&SystemFields>;
    fn compile(
        tree: &IrTree<'_>,
        has_literal_array: bool,
        base_indent: u16,
        is_root: bool,
        settings: &ParserSettings,
    ) -> Option<Self>;
    /// Run a prepared instance. Errors may leave partial output.
    fn exec<'a, V: ValueSource<'a>>(
        instance: Instance<V::Scope>,
        progs: &ProgramCache<Self>,
        values: &V,
        chunk: &'a EvtxChunk<'a>,
        cache: Option<&mut IrTemplateCache<'a>>,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
    ) -> Result<()>;
    /// The materialized walker for this format (the slow lane).
    fn render_tree(tree: &IrTree<'_>, settings: &ParserSettings, out: &mut Vec<u8>) -> Result<()>;
}

impl TemplateProgram for XmlProgram {
    const ALLOW_GENERIC_FRAGS: bool = true;
    fn elem_slots(&self) -> &[(u16, u16)] {
        &self.elem_slots
    }
    fn expand_slots(&self) -> &[u16] {
        &self.expand_slots
    }
    fn root_name(&self) -> &[u8] {
        &self.root_name
    }
    fn system_fields(&self) -> Option<&SystemFields> {
        self.system_fields.as_ref()
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
    fn exec<'a, V: ValueSource<'a>>(
        instance: Instance<V::Scope>,
        progs: &ProgramCache<Self>,
        values: &V,
        chunk: &'a EvtxChunk<'a>,
        cache: Option<&mut IrTemplateCache<'a>>,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        exec(instance, progs, values, chunk, cache, vr, out)
    }
    /// The single-walker slow lane (irregular records, after
    /// materialization), writing straight into `out`. Byte-compatible with
    /// the cached-program lane by construction: it IS the same walk, with
    /// zero placeholder sites.
    fn render_tree(tree: &IrTree<'_>, settings: &ParserSettings, out: &mut Vec<u8>) -> Result<()> {
        out.extend_from_slice(XML_DECL);
        render_fragment_xml(tree.arena(), tree.root(), 0, settings.should_indent(), out)
    }
}

// ---------------------------------------------------------------------------
// Compilation (template definition IR -> program)
// ---------------------------------------------------------------------------

/// `flush_lit_run`/`side_range` for a compiler with `lits`, `ops: Vec<$op>`
/// and `run_start` fields.
macro_rules! lit_run_methods {
    ($op:ident) => {
        fn flush_lit_run(&mut self) {
            let end = self.lits.len();
            if end > self.run_start {
                self.ops
                    .push($op::Lit(LitRange(self.run_start as u32, end as u32)));
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
            LitRange(start, self.lits.len() as u32)
        }
    };
}

struct Bail;

struct XmlCompiler<'t, 'a> {
    arena: &'t IrArena<'a>,
    lits: Vec<u8>,
    ops: Vec<XOp>,
    /// Start of the not-yet-flushed literal run (`lits[run_start..]`).
    run_start: usize,
    elem_slots: Vec<(u16, u16)>,
    expand_slots: Vec<u16>,
    indent_on: bool,
    vr: ValueRenderer,
    /// Walking a fully materialized tree (slow lane / fragments): placeholder
    /// sites are errors instead of ops, and any error is a real record error.
    /// When false (template compilation), any error just means "not cacheable".
    materialized: bool,
}

impl<'t, 'a> XmlCompiler<'t, 'a> {
    fn new(arena: &'t IrArena<'a>, lits: Vec<u8>, indent_on: bool, materialized: bool) -> Self {
        XmlCompiler {
            arena,
            lits,
            ops: Vec::new(),
            run_start: 0,
            elem_slots: Vec::new(),
            expand_slots: Vec::new(),
            indent_on,
            vr: ValueRenderer::new(),
            materialized,
        }
    }
}

/// Compile-lane bail sentinel (mapped to `None` by `compile_xml_template`).
fn bail_err() -> EvtxError {
    model_err("compiled-template bail")
}

fn unresolved_placeholder() -> EvtxError {
    model_err("unresolved placeholder in tree")
}

fn element_in_attr() -> EvtxError {
    model_err("element node inside attribute value")
}

fn element_in_text() -> EvtxError {
    model_err("unexpected element node in text context")
}

fn unresolved_nested_json() -> EvtxError {
    model_err("unresolved nested instance in compiled JSON")
}

fn model_err(msg: &'static str) -> EvtxError {
    EvtxError::FailedToCreateRecordModel(msg)
}

/// Compile a cached template definition into an XML program. Returns `None`
/// for shapes the op set doesn't model.
fn compile_xml_template(
    tree: &IrTree<'_>,
    has_literal_array: bool,
    base_indent: u16,
    is_root: bool,
    settings: &ParserSettings,
) -> Option<XmlProgram> {
    if has_literal_array {
        return None;
    }
    let mut c = XmlCompiler::new(
        tree.arena(),
        Vec::with_capacity(512),
        settings.should_indent(),
        false,
    );
    c.ops.reserve(32);
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
                expand_slots: c.expand_slots,
                root_name: tree.root_element().name.as_str().as_bytes().to_vec(),
                system_fields: settings.get_mta().and_then(|_| SystemFields::of(tree)),
            })
        }
        Err(_) => None,
    }
}

/// Render a materialized fragment subtree at `indent` (executor cold path).
fn render_fragment_xml(
    arena: &IrArena<'_>,
    root: ElementId,
    indent: u16,
    indent_on: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    let mut c = XmlCompiler::new(arena, std::mem::take(out), indent_on, true);
    let res = c.compile_element(root, indent);
    debug_assert!(c.ops.is_empty(), "materialized walk produced ops");
    *out = c.lits;
    res
}

impl<'t, 'a> XmlCompiler<'t, 'a> {
    fn element_ref(&self, id: ElementId) -> &'t Element<'a> {
        self.arena.get(id).expect("invalid element id")
    }

    lit_run_methods!(XOp);

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

        // Attribute-less single-hole `<Data>` elements are the array
        // expansion sites: compile them as self-contained `Expand` ops (the
        // open tag must be able to repeat per array item).
        if !self.materialized
            && element.attrs.is_empty()
            && is_data_element_name(element.name.as_str())
            && let ChildrenKind::SinglePlaceholder(ph) = classify_children(element)
        {
            return self.compile_expand(element, ph, indent);
        }

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
                let name = element.name.as_str().as_bytes();
                let is_binary = element.name.as_str() == "Binary";
                self.lits.push(b'>');
                self.flush_lit_run();
                let tail_text = self.side_range(|c| {
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(name);
                    c.lits.push(b'>');
                    c.newline();
                });
                let tail_empty = self.side_range(|c| {
                    if !is_binary {
                        c.newline();
                        c.indent_str(indent);
                    }
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(name);
                    c.lits.push(b'>');
                    c.newline();
                });
                let tail_elem = self.side_range(|c| {
                    c.indent_str(indent);
                    c.lits.extend_from_slice(b"</");
                    c.lits.extend_from_slice(name);
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
                            return Err(model_err("PIData without PITarget"));
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
                        return Err(model_err("unresolved placeholder in attribute value"));
                    }
                    n_placeholders += 1;
                }
                Node::Element(_) => {
                    return Err(element_in_attr());
                }
                Node::PITarget(_) | Node::PIData(_) => {
                    return Err(EvtxError::Unimplemented {
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
        let name = attr.name.as_str().as_bytes();
        self.flush_lit_run();
        let pre = self.side_range(|c| {
            c.lits.push(b' ');
            c.lits.extend_from_slice(name);
            c.lits.extend_from_slice(b"=\"");
        });
        self.ops.push(XOp::AttrVal { slot: ph.id, pre });
        Ok(())
    }

    /// Compile an array-expansion site (`<Data>%n</Data>`, no attributes)
    /// into a self-contained `XOp::Expand`: the open tag lives in the op so
    /// the executor can repeat the whole element per array item.
    fn compile_expand(
        &mut self,
        element: &'t Element<'a>,
        ph: &Placeholder,
        indent: u16,
    ) -> Result<()> {
        let name = element.name.as_str().as_bytes();
        self.flush_lit_run();
        let open = self.side_range(|c| {
            c.indent_str(indent);
            c.lits.push(b'<');
            c.lits.extend_from_slice(name);
            c.lits.push(b'>');
        });
        let tail_text = self.side_range(|c| {
            c.lits.extend_from_slice(b"</");
            c.lits.extend_from_slice(name);
            c.lits.push(b'>');
            c.newline();
        });
        let tail_empty = self.side_range(|c| {
            c.newline();
            c.indent_str(indent);
            c.lits.extend_from_slice(b"</");
            c.lits.extend_from_slice(name);
            c.lits.push(b'>');
            c.newline();
        });
        let tail_elem = self.side_range(|c| {
            c.indent_str(indent);
            c.lits.extend_from_slice(b"</");
            c.lits.extend_from_slice(name);
            c.lits.push(b'>');
            c.newline();
        });
        self.elem_slots.push((ph.id, indent + INDENT_WIDTH));
        self.expand_slots.push(ph.id);
        self.ops.push(XOp::Expand {
            slot: ph.id,
            optional: ph.optional,
            indent,
            open,
            tail_text,
            tail_empty,
            tail_elem,
        });
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
            Node::Element(_) => Err(element_in_text()),
            Node::Placeholder(_) => Err(unresolved_placeholder()),
        }
    }

    fn compile_literal_text(&mut self, text: &Text<'a>, in_attribute: bool) -> Result<()> {
        let mode = StringEscapeMode::Xml { in_attribute };
        match text {
            Text::Utf16(value) => self.vr.write_utf16_escaped(&mut self.lits, *value, mode),
            Text::Utf8(value) => self.vr.write_str_escaped(&mut self.lits, value, mode),
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
    /// Type-discriminated side-table index: `Preflight::ansi` for ANSI
    /// strings, `Preflight::arrays` for string arrays.
    aux: u32,
}

const NO_NESTED: u16 = u16::MAX;
const NO_AUX: u32 = u32::MAX;

impl RawSlot {
    /// The value's byte window in the chunk data.
    fn bytes<'d>(&self, data: &'d [u8]) -> &'d [u8] {
        &data[self.off as usize..self.off as usize + usize::from(self.len)]
    }

    /// Index into `Preflight::nested` for resolved nested instances.
    fn nested_idx(&self) -> Option<usize> {
        (self.nested != NO_NESTED).then_some(usize::from(self.nested))
    }

    /// Index into `Preflight::ansi` for pre-decoded ANSI payloads.
    fn ansi_idx(&self) -> Option<usize> {
        (self.ty == value_ty::ANSI_STRING && self.aux != NO_AUX).then_some(self.aux as usize)
    }

    /// A non-empty embedded BinXML value (renders as an element).
    fn is_binxml_payload(&self) -> bool {
        self.ty == value_ty::BIN_XML && self.len > 0
    }
}

/// A cached program and the substitution slots for one validated instance.
#[derive(Clone, Copy)]
pub(crate) struct Instance<S = SlotRange> {
    prog: usize,
    slots: S,
}

/// Reusable per-chunk pre-flight scratch.
#[derive(Default)]
pub(crate) struct Preflight {
    slots: Vec<RawSlot>,
    nested: Vec<Instance>,
    /// Decoded ANSI values, concatenated; `ansi_spans[i]` is value `i`.
    ansi: String,
    ansi_spans: Vec<(u32, u32)>,
    /// NUL-filter scratch for ANSI decoding, reset before every value.
    ansi_scratch: bumpalo::Bump,
    /// Per string-array slot: `(start, count)` into `array_items`.
    arrays: Vec<(u32, u32)>,
    /// Flat `(offset, byte len)` spans of string-array items (NUL-free).
    array_items: Vec<(u32, u16)>,
}

/// How the pre-flight validates a descriptor of a given type.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TyClass {
    /// Arrays, exotics, unknowns: materialize lane.
    Reject,
    /// Exact wire width.
    Fixed(u8),
    /// Even-length UTF-16LE payload.
    Utf16,
    /// Any-length payload (NULL skips it, BINARY/BIN_XML slice it).
    Sized,
    /// ANSI string: any length, pre-decoded at pre-flight.
    Ansi,
    /// Pointer-width integer: 4 (32-bit) or 8 (64-bit) bytes.
    SizeT,
    /// `8 + 4 * sub_authority_count` bytes.
    Sid,
    /// UTF-16 string array: even-length payload of NUL-terminated strings;
    /// admitted only on slots the program compiled as expandable.
    StrArray,
}

/// The declarative wire-facts table the descriptor scan runs on.
const TY_CLASS: [TyClass; 256] = {
    use TyClass::*;
    use value_ty::*;
    let mut t = [Reject; 256];
    t[NULL as usize] = Sized;
    t[UTF16_STRING as usize] = Utf16;
    t[ANSI_STRING as usize] = Ansi;
    t[INT8 as usize] = Fixed(1);
    t[UINT8 as usize] = Fixed(1);
    t[INT16 as usize] = Fixed(2);
    t[UINT16 as usize] = Fixed(2);
    t[INT32 as usize] = Fixed(4);
    t[UINT32 as usize] = Fixed(4);
    t[INT64 as usize] = Fixed(8);
    t[UINT64 as usize] = Fixed(8);
    t[REAL32 as usize] = Fixed(4);
    t[REAL64 as usize] = Fixed(8);
    t[BOOL as usize] = Fixed(4); // a 4-byte i32 on the wire
    t[BINARY as usize] = Sized;
    t[GUID as usize] = Fixed(16);
    t[SIZE_T as usize] = SizeT;
    t[FILETIME as usize] = Fixed(8);
    t[SYSTIME as usize] = Fixed(16);
    t[SID as usize] = Sid;
    t[HEX_INT32 as usize] = Fixed(4);
    t[HEX_INT64 as usize] = Fixed(8);
    t[BIN_XML as usize] = Sized;
    t[STR_ARRAY as usize] = StrArray;
    t
};

/// Fast-lane bail depth for nested instances (deeper chains use the slow
/// lane, which is itself capped — see `ir::MAX_BINXML_NESTING`).
const MAX_FAST_LANE_NESTING: usize = 8;

/// Sanity cap on a template instance's substitution count (the wire could
/// claim up to `u32::MAX`; nothing legitimate comes close).
const MAX_SUBSTITUTIONS: usize = 4096;

struct PreflightBail;

impl From<crate::err::DeserializationError> for PreflightBail {
    fn from(_: crate::err::DeserializationError) -> Self {
        PreflightBail
    }
}

impl From<EvtxError> for PreflightBail {
    fn from(_: EvtxError) -> Self {
        PreflightBail
    }
}

impl Preflight {
    fn ansi(&self, i: usize) -> &str {
        let (start, end) = self.ansi_spans[i];
        &self.ansi[start as usize..end as usize]
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.nested.clear();
        self.ansi.clear();
        self.ansi_spans.clear();
        self.arrays.clear();
        self.array_items.clear();
    }

    /// Item spans of a string-array slot (validated by the pre-flight).
    fn str_array_items(&self, s: &RawSlot) -> &[(u32, u16)] {
        debug_assert_eq!(s.ty, value_ty::STR_ARRAY);
        let (start, count) = self.arrays[s.aux as usize];
        &self.array_items[start as usize..(start + count) as usize]
    }

    /// Emptiness of a slot, mirroring `is_optional_empty` over the value the
    /// regular path would have decoded (string NUL-truncation included).
    #[inline(always)]
    fn slot_empty(&self, s: &RawSlot, data: &[u8]) -> bool {
        match s.ty {
            value_ty::NULL => true,
            // The decoded string is empty when sized 0 or NUL-led (mirrors
            // `utf16_by_char_count` truncation).
            value_ty::UTF16_STRING => s.len == 0 || s.bytes(data).starts_with(&[0, 0]),
            value_ty::ANSI_STRING => s.ansi_idx().is_none_or(|i| self.ansi(i).is_empty()),
            value_ty::BINARY | value_ty::BIN_XML => s.len == 0,
            _ => false,
        }
    }

    /// Scan a `TemplateInstance` whose header starts at absolute `pos` (the
    /// byte after the 0x0c token). Appends slots/nested entries and returns
    /// `(instance, end_pos)`.
    #[allow(clippy::too_many_arguments)]
    fn scan_instance<'a, P: StoredProgram>(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        pos: usize,
        depth: usize,
        cache: &mut IrTemplateCache<'a>,
        progs: &mut ProgramCache<P>,
        base_indent: u16,
        is_root: bool,
    ) -> std::result::Result<(Instance, usize), PreflightBail> {
        if depth > MAX_FAST_LANE_NESTING {
            return Err(PreflightBail);
        }
        let data = chunk.data;

        // Instance header (mirrors `read_template_values_cursor`).
        let mut cur = ByteCursor::with_pos(data, pos)?;
        cur.u8()?; // unknown byte
        let _template_id = cur.u32()?;
        let def_offset = cur.u32()?;
        if cur.position() as u32 == def_offset {
            // Inline definition: skip its header + payload.
            let header = read_template_definition_header_at(data, def_offset)?;
            cur.set_pos_u64(
                cur.position()
                    + (TEMPLATE_DEFINITION_HEADER_SIZE as u64)
                    + u64::from(header.data_size),
                "skip inline template definition",
            )?;
        }
        let n = cur.u32()? as usize;
        if n > MAX_SUBSTITUTIONS {
            return Err(PreflightBail);
        }

        let prog = get_or_compile(
            chunk,
            (def_offset, base_indent, is_root),
            progs,
            TemplateSource::Bytes(cache),
        )
        .ok_or(PreflightBail)?;

        // Descriptor table: n x (u16 size, u8 type, u8 pad), then the values.
        let descriptors = cur.take_bytes(n * 4, "descriptor table")?;
        let mut off = cur.pos();
        let slot_start = self.slots.len() as u32;
        for (i, desc) in descriptors.chunks_exact(4).enumerate() {
            let &[len_lo, len_hi, ty, _pad] = desc else {
                unreachable!("chunks_exact(4)");
            };
            let len = u16::from_le_bytes([len_lo, len_hi]);
            let end = off + usize::from(len);
            if end > data.len() {
                return Err(PreflightBail);
            }
            match TY_CLASS[usize::from(ty)] {
                TyClass::Reject => return Err(PreflightBail),
                TyClass::Fixed(width) => {
                    if len != u16::from(width) {
                        return Err(PreflightBail);
                    }
                }
                TyClass::Utf16 => {
                    if len % 2 != 0 {
                        return Err(PreflightBail);
                    }
                }
                TyClass::SizeT => {
                    if !(len == 4 || len == 8) {
                        return Err(PreflightBail);
                    }
                }
                TyClass::Sid => {
                    // SID payload: revision, sub-authority count, 48-bit
                    // authority, then 4-byte sub-authorities.
                    let &[_revision, sub_count, ..] = &data[off..end] else {
                        return Err(PreflightBail);
                    };
                    if usize::from(len) != 8 + 4 * usize::from(sub_count) {
                        return Err(PreflightBail);
                    }
                }
                TyClass::StrArray => {
                    // Admitted only on slots the program compiled as
                    // expandable; everything else keeps the slow lane.
                    if len % 2 != 0 || !progs.programs[prog].expand_slots().contains(&(i as u16)) {
                        return Err(PreflightBail);
                    }
                }
                TyClass::Sized | TyClass::Ansi => {}
            }
            let mut slot = RawSlot {
                off: off as u32,
                len,
                ty,
                nested: NO_NESTED,
                aux: NO_AUX,
            };
            if ty == value_ty::ANSI_STRING && len > 0 {
                // Decode ANSI now so the executor stays infallible.
                let codec = chunk.settings.get_ansi_codec();
                self.ansi_scratch.reset();
                let decoded = decode_ansi_value(codec, &data[off..end], &self.ansi_scratch)?;
                slot.aux = self.ansi_spans.len() as u32;
                let start = self.ansi.len() as u32;
                self.ansi.push_str(&decoded);
                self.ansi_spans.push((start, self.ansi.len() as u32));
            } else if ty == value_ty::STR_ARRAY {
                // Split the payload into NUL-terminated item spans now so the
                // executor stays infallible. Mirrors the deserializer: every
                // NUL ends an item; the payload must end exactly on one (an
                // unterminated tail keeps the legacy lane's read-past quirk).
                let items_start = self.array_items.len() as u32;
                let mut item_off = off;
                for (pi, pair) in data[off..end].chunks_exact(2).enumerate() {
                    if let &[0, 0] = pair {
                        let nul_at = off + pi * 2;
                        self.array_items
                            .push((item_off as u32, (nul_at - item_off) as u16));
                        item_off = nul_at + 2;
                    }
                }
                let count = self.array_items.len() as u32 - items_start;
                if item_off != end || count == 0 {
                    return Err(PreflightBail);
                }
                slot.aux = self.arrays.len() as u32;
                self.arrays.push((items_start, count));
            }
            self.slots.push(slot);
            off = end;
        }
        let slot_range = (slot_start, self.slots.len() as u32);

        // Resolve nested instances for slots this program renders as elements.
        // Recursive scans may append programs; keep indices, not a borrow of the cache.
        for element in 0..progs.programs[prog].elem_slots().len() {
            let (slot_id, child_indent) = progs.programs[prog].elem_slots()[element];
            if u32::from(slot_id) >= slot_range.1 - slot_range.0 {
                continue; // out-of-range -> Skip at exec
            }
            let idx = slot_start as usize + slot_id as usize;
            let s = self.slots[idx];
            if !s.is_binxml_payload() {
                continue;
            }
            let frag = s.bytes(data);
            let Some(inst_off) = single_instance_offset(frag) else {
                // Generic fragment: rendered via the materialized fallback
                // where the executor supports it; otherwise fall back.
                if P::ALLOW_GENERIC_FRAGS {
                    continue;
                }
                return Err(PreflightBail);
            };
            let (instance, nend) = self.scan_instance(
                chunk,
                s.off as usize + inst_off,
                depth + 1,
                cache,
                progs,
                child_indent,
                false,
            )?;
            // The nested instance must span its whole payload (allow EOF pad).
            if ends_stream(frag, nend - s.off as usize) != Some(true) {
                return Err(PreflightBail);
            }
            if self.nested.len() >= usize::from(u16::MAX) {
                return Err(PreflightBail);
            }
            self.slots[idx].nested = self.nested.len() as u16;
            self.nested.push(instance);
        }

        // Per-slot constraints (kept rare; violations route to the fallback).
        for &c in progs.programs[prog].constraints() {
            if u32::from(c.slot()) >= slot_range.1 - slot_range.0 {
                continue; // out-of-range resolves to Skip everywhere
            }
            let s = &self.slots[slot_start as usize + c.slot() as usize];
            if c.violated(s.is_binxml_payload(), self.slot_empty(s, data)) {
                return Err(PreflightBail);
            }
        }

        Ok((
            Instance {
                prog,
                slots: slot_range,
            },
            off,
        ))
    }
}

enum TemplateSource<'s, 'a> {
    Bytes(&'s mut IrTemplateCache<'a>),
    Validated(&'s IrTree<'a>, bool),
}

fn get_or_compile<'a, P: StoredProgram>(
    chunk: &'a EvtxChunk<'a>,
    key: (u32, u16, bool),
    progs: &mut ProgramCache<P>,
    source: TemplateSource<'_, 'a>,
) -> Option<usize> {
    let settings = &chunk.settings;
    let (def_offset, base_indent, is_root) = key;
    if let Some(entry) = progs.by_key.get(&key) {
        return *entry;
    }

    // Cross-chunk store, keyed by template content identity.
    let store = &*chunk.program_store;
    let store_key = store
        .key(chunk, def_offset)
        .map(|(template, names)| (template, names, base_indent, is_root));
    // Lock sections are bare map reads/inserts (compilation happens outside),
    // so a poisoned lock cannot hold inconsistent data: recover and continue.
    if let Some(sk) = store_key.as_ref()
        && let Some(entry) = P::shard(store)
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(sk)
    {
        return progs.insert(key, entry.clone());
    }

    let compiled = match source {
        TemplateSource::Validated(tree, has_literal_array) => {
            P::compile(tree, has_literal_array, base_indent, is_root, settings).map(Arc::new)
        }
        TemplateSource::Bytes(cache) => cache
            .template_for_compile(chunk, def_offset)
            .ok()
            .and_then(|(tree, has_literal_array)| {
                P::compile(&tree, has_literal_array, base_indent, is_root, settings).map(Arc::new)
            }),
    };
    if let Some(sk) = store_key {
        P::shard(store)
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(sk, compiled.clone());
    }
    progs.insert(key, compiled)
}

// ---------------------------------------------------------------------------
// Per-record entry + executor
// ---------------------------------------------------------------------------

/// Try to render one record's BinXML via the compiled-template path.
///
/// Returns `false` with `out` untouched when the record isn't covered (the
/// caller then uses the regular path). On `true` the record was rendered
/// byte-identically to `P::render_tree` over the materialized record, with
/// `mta::add_rendering_info` applied when an `.MTA` file is set.
#[allow(clippy::too_many_arguments)]
pub(crate) fn try_render_compiled<'a, P: StoredProgram>(
    bytes: &[u8],
    record_number: u64,
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    progs: &mut ProgramCache<P>,
    pf: &mut Preflight,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> bool {
    // Single-instance stream shape (mirrors `read_single_instance_stream`).
    let Some(inst_off) = single_instance_offset(bytes) else {
        return false;
    };
    let Ok(stream_offset) = binxml_slice_offset_in(chunk.data, bytes) else {
        return false;
    };
    let stream_offset = stream_offset as usize;
    pf.clear();
    let Ok((instance, end)) =
        pf.scan_instance(chunk, stream_offset + inst_off, 0, cache, progs, 0, true)
    else {
        return false;
    };
    // Anything after the instance other than EOF is unhandled here, and so
    // is an instance that read past the record (the regular path accepts it).
    if ends_stream(bytes, end - stream_offset) != Some(true) {
        return false;
    }
    let values = RawValues {
        pf,
        data: chunk.data,
    };
    let rendering_info = match chunk.settings.get_mta() {
        None => None,
        Some(mta) => {
            let Some(fields) = progs.programs[instance.prog].system_fields() else {
                return false;
            };
            let slots = values.slots(instance.slots);
            let value = |id| values.mta_value(slots, id, progs, chunk);
            if fields.needs_tree(value) {
                return false;
            }
            fields.rendering_info(mta, record_number, value)
        }
    };
    let start = out.len();
    if !exec_or_rollback(instance, progs, &values, chunk, Some(cache), vr, out) {
        return false;
    }
    if let Some(info) = rendering_info {
        if !append_rendering_info::<P>(&info, chunk, start, out) {
            out.truncate(start);
            return false;
        }
        #[cfg(test)]
        MTA_SPLICES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    }
    true
}

/// How many records got their `RenderingInfo` from `try_render_compiled`, for the tests in
/// `mta` that check it renders them as the record's tree does.
#[cfg(test)]
pub(crate) static MTA_SPLICES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Adds `info` to the record rendered at `out[start..]`, where `mta::add_rendering_info` puts
/// it in a record's tree.
fn append_rendering_info<'a, P: StoredProgram>(
    info: &RenderingInfo<'a>,
    chunk: &'a EvtxChunk<'a>,
    start: usize,
    out: &mut Vec<u8>,
) -> bool {
    let bump = &chunk.arena;
    let mut arena = IrArena::new_in(bump);
    let root = mta::rendering_info_element(info, &mut arena, bump);
    P::append_to_event(&IrTree::new(arena, root), &chunk.settings, start, out)
}

/// `P::exec`, truncating `out` back on error. Errors are unreachable after a
/// successful preflight except for attribute-position BinXml elements, which
/// the regular path also rejects per record.
fn exec_or_rollback<'a, P: TemplateProgram, V: ValueSource<'a>>(
    instance: Instance<V::Scope>,
    progs: &ProgramCache<P>,
    values: &V,
    chunk: &'a EvtxChunk<'a>,
    cache: Option<&mut IrTemplateCache<'a>>,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> bool {
    let start = out.len();
    let ok = P::exec(instance, progs, values, chunk, cache, vr, out).is_ok();
    if !ok {
        out.truncate(start);
    }
    ok
}

/// Runtime slot class for `Body`/`ChildSlot` branching.
enum SlotClass {
    Skip,
    TextLike,
    Element,
}

fn exec<'a, V: ValueSource<'a>>(
    instance: Instance<V::Scope>,
    progs: &XmlProgramCache,
    pf: &V,
    chunk: &'a EvtxChunk<'a>,
    mut cache: Option<&mut IrTemplateCache<'a>>,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> Result<()> {
    let prog = &progs.programs[instance.prog];
    let lits = &prog.lits;
    let slots = pf.slots(instance.slots);
    let slot_at = |id: u16| slots.get(usize::from(id));
    let classify = |slot: u16, optional: bool| -> SlotClass {
        match slot_at(slot) {
            None => SlotClass::Skip,
            Some(s) => {
                if pf.empty(s) {
                    if optional {
                        SlotClass::Skip
                    } else {
                        SlotClass::TextLike
                    }
                } else if pf.element(s) {
                    SlotClass::Element
                } else {
                    SlotClass::TextLike
                }
            }
        }
    };

    macro_rules! write_lit {
        ($r:expr) => {
            out.extend_from_slice($r.of(lits))
        };
    }
    macro_rules! write_val {
        ($s:expr, $in_attr:expr) => {{
            let s = $s;
            pf.write(
                s,
                vr,
                out,
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
                    if pf.element(s) {
                        return Err(element_in_attr());
                    }
                    write_val!(s, *in_attr);
                }
            }
            XOp::AttrVal { slot, pre } => {
                if let Some(s) = slot_at(*slot)
                    && !pf.empty(s)
                {
                    if pf.element(s) {
                        return Err(element_in_attr());
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
                            s,
                            progs,
                            pf,
                            chunk,
                            cache.as_deref_mut(),
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
                    render_element_slot(
                        s,
                        progs,
                        pf,
                        chunk,
                        cache.as_deref_mut(),
                        vr,
                        *indent,
                        prog.indent_on,
                        out,
                    )?;
                }
            },
            XOp::Expand {
                slot,
                optional,
                indent,
                open,
                tail_text,
                tail_empty,
                tail_elem,
            } => {
                if let Some(items) = slot_at(*slot).and_then(|s| pf.array(s)) {
                    // Array: the element repeats per item. Single-item arrays
                    // are not expanded by the materialize lane (the value node
                    // stays inline, empty or not); empty items in larger
                    // arrays drop their text node (two-line empty form).
                    let multi = items.len() > 1;
                    for item in items {
                        write_lit!(*open);
                        if multi && item.is_empty() {
                            write_lit!(*tail_empty);
                        } else {
                            vr.write_value_text(
                                out,
                                &BinXmlValue::StringType(item),
                                StringEscapeMode::Xml {
                                    in_attribute: false,
                                },
                            )?;
                            write_lit!(*tail_text);
                        }
                    }
                } else {
                    // Scalar: `Body` semantics with the open tag spliced in.
                    write_lit!(*open);
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
                                s,
                                progs,
                                pf,
                                chunk,
                                cache.as_deref_mut(),
                                vr,
                                *indent + INDENT_WIDTH,
                                prog.indent_on,
                                out,
                            )?;
                            write_lit!(*tail_elem);
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// Render an element-class slot: a nested compiled instance, or a generic
/// BinXml fragment via the materialized fallback renderer.
#[allow(clippy::too_many_arguments)]
fn render_element_slot<'a, V: ValueSource<'a>>(
    s: &V::Value,
    progs: &XmlProgramCache,
    pf: &V,
    chunk: &'a EvtxChunk<'a>,
    cache: Option<&mut IrTemplateCache<'a>>,
    vr: &mut ValueRenderer,
    indent: u16,
    indent_on: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    if let Some(inst) = pf.nested(s) {
        return exec(inst, progs, pf, chunk, cache, vr, out);
    }
    if let Some((arena, root)) = pf.fragment(s) {
        return render_fragment_xml(arena, root, indent, indent_on, out);
    }
    // Generic (non-instance) fragment: materialize and render. Cold path.
    let Some(cache) = cache else {
        return Err(unresolved_placeholder());
    };
    let frag = pf.bytes(s).ok_or_else(unresolved_placeholder)?;
    let tree = build_tree_from_binxml_bytes_direct(frag, chunk, cache)?;
    render_fragment_xml(tree.arena(), tree.root(), indent, indent_on, out)
}

// ---------------------------------------------------------------------------
// JSON programs
// ---------------------------------------------------------------------------

/// One attribute member inside a `JOp::Elem` `#attributes` object.
#[derive(Debug, Clone)]
enum JAttrPart {
    /// Pre-rendered `"name":value` member; always emitted.
    Literal(LitRange),
    /// Conditional member: `"name":` key bytes plus the value from `slot`,
    /// omitted when the slot is empty.
    Placeholder { key: LitRange, slot: u16 },
}

#[derive(Debug, Clone)]
enum JOp {
    /// Emit `lits[range]`.
    Lit(LitRange),
    /// A leaf element value: `null`/`""` when empty, bare number for
    /// int/bool-typed slots, nested-instance object for BinXml slots,
    /// quoted escaped string otherwise.
    LeafVal {
        prefix: LitRange,
        slot: u16,
        empty: LitRange,
    },
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
    /// Slots whose `LeafVal` sits at a positional-`Data` aggregation site
    /// (string arrays allowed: they render as a JSON array `#text`).
    expand_slots: Vec<u16>,
    /// Raw root element name bytes (member key for nested-instance values).
    root_name: Vec<u8>,
    system_fields: Option<SystemFields>,
}

impl TemplateProgram for JsonProgram {
    const ALLOW_GENERIC_FRAGS: bool = false;
    fn elem_slots(&self) -> &[(u16, u16)] {
        &self.elem_slots
    }
    fn constraints(&self) -> &[SlotConstraint] {
        &self.constraints
    }
    fn expand_slots(&self) -> &[u16] {
        &self.expand_slots
    }
    fn root_name(&self) -> &[u8] {
        &self.root_name
    }
    fn system_fields(&self) -> Option<&SystemFields> {
        self.system_fields.as_ref()
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
    fn exec<'a, V: ValueSource<'a>>(
        instance: Instance<V::Scope>,
        progs: &ProgramCache<Self>,
        values: &V,
        _chunk: &'a EvtxChunk<'a>,
        _cache: Option<&mut IrTemplateCache<'a>>,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        exec_json(instance, progs, values, vr, out)
    }
    /// The single-walker slow lane, honoring `--separate-json-attributes`.
    fn render_tree(tree: &IrTree<'_>, settings: &ParserSettings, out: &mut Vec<u8>) -> Result<()> {
        let mut c = JsonCompiler::new(
            Some(tree),
            std::mem::take(out),
            settings.should_separate_json_attributes(),
        );
        let res = c.render_root_plain();
        debug_assert!(c.ops.is_empty(), "materialized walk produced ops");
        *out = c.lits;
        res
    }
}

struct JsonCompiler<'t, 'a> {
    tree: Option<&'t IrTree<'a>>,
    lits: Vec<u8>,
    ops: Vec<JOp>,
    run_start: usize,
    elem_slots: Vec<(u16, u16)>,
    constraints: Vec<SlotConstraint>,
    expand_slots: Vec<u16>,
    vr: ValueRenderer,
    /// `--separate-json-attributes` (slow lane only; such templates bail).
    separate: bool,
}

impl<'t, 'a> JsonCompiler<'t, 'a> {
    fn new(tree: Option<&'t IrTree<'a>>, lits: Vec<u8>, separate: bool) -> Self {
        JsonCompiler {
            tree,
            lits,
            ops: Vec::new(),
            run_start: 0,
            elem_slots: Vec::new(),
            constraints: Vec::new(),
            expand_slots: Vec::new(),
            vr: ValueRenderer::new(),
            separate,
        }
    }
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
    let mut c = JsonCompiler::new(Some(tree), Vec::with_capacity(512), false);
    c.ops.reserve(32);
    if is_root {
        c.lits.push(b'{');
        c.key_with_suffix_plain(root.name.as_str(), 0);
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
                expand_slots: c.expand_slots,
                root_name: root.name.as_str().as_bytes().to_vec(),
                system_fields: settings.get_mta().and_then(|_| SystemFields::of(tree)),
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

    lit_run_methods!(JOp);

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
                    self.compile_elem_op(element, Some(ph.id))
                }
            }
            ChildrenKind::Empty => {
                if !ph_attrs {
                    // Statically resolvable: `null` or attrs-only object.
                    // (subtree_has_placeholder was true, so this can't happen.)
                    Err(Bail)
                } else {
                    self.compile_elem_op(element, None)
                }
            }
            ChildrenKind::StaticInline => Err(Bail), // literal text + ph attrs: rare
            ChildrenKind::StaticLines => self.compile_object_body(element, container, ph_attrs),
            ChildrenKind::Bail => Err(Bail),
        }
    }

    // Fuse the literal prefix with its value to avoid a second dispatch per field.
    fn push_leaf_val(&mut self, slot: u16, empty: LitRange) {
        let prefix = if let Some(JOp::Lit(prefix)) = self.ops.last() {
            let prefix = *prefix;
            self.ops.pop();
            prefix
        } else {
            LitRange(0, 0)
        };
        self.ops.push(JOp::LeafVal {
            prefix,
            slot,
            empty,
        });
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
        self.push_leaf_val(slot, empty);
        Ok(())
    }

    /// `JOp::Elem` for `<Tag attr=%a ...>%c?</Tag>` shapes.
    ///
    /// The caller decides leaf-vs-object form; literal attribute members are
    /// emitted unconditionally here, which is what makes static attribute
    /// text force the object form at run time.
    fn compile_elem_op(
        &mut self,
        element: &'t Element<'a>,
        content: Option<u16>,
    ) -> std::result::Result<(), Bail> {
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
                    let lit_member = LitRange(start, self.lits.len() as u32);
                    self.run_start = self.lits.len();
                    parts.push(JAttrPart::Literal(lit_member));
                }
                1 if attr.value.len() == 1 => {
                    let Node::Placeholder(ph) = &attr.value[0] else {
                        return Err(Bail);
                    };
                    let name = attr.name.as_str().as_bytes();
                    let key = self.side_range(|c| {
                        c.lits.push(b'"');
                        c.lits.extend_from_slice(name);
                        c.lits.extend_from_slice(b"\":");
                    });
                    self.constraints.push(SlotConstraint::ForbidElem(ph.id));
                    parts.push(JAttrPart::Placeholder { key, slot: ph.id });
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
        let mut static_names: Vec<(&[u8], u16)> = Vec::new();

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
                            self.compile_data_value(*id, false)?;
                        } else if !positional_emitted && !positional_data.is_empty() {
                            positional_emitted = true;
                            if wrote_any {
                                self.lits.push(b',');
                            }
                            wrote_any = true;
                            self.lits.extend_from_slice(b"\"Data\":{\"#text\":");
                            if positional_data.len() == 1 {
                                // Sole positional Data: a string array here
                                // aggregates into a JSON array value, so the
                                // slot is expandable.
                                self.compile_data_value(positional_data[0], true)?;
                            } else {
                                self.lits.push(b'[');
                                for (i, did) in positional_data.iter().enumerate() {
                                    if i > 0 {
                                        self.lits.push(b',');
                                    }
                                    self.compile_data_value(*did, false)?;
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
                    let suffix = next_name_suffix(cname.as_bytes(), &mut static_names);
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
                    if seen_slot_child {
                        return Err(Bail); // dynamic same-name suffixing needs runtime state
                    }
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
    fn compile_data_value(
        &mut self,
        id: ElementId,
        expandable: bool,
    ) -> std::result::Result<(), Bail> {
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
            if expandable {
                self.expand_slots.push(ph.id);
            }
            self.push_leaf_val(ph.id, empty);
            return Ok(());
        }
        Err(Bail)
    }
}

// ---------------------------------------------------------------------------
// JSON executor
// ---------------------------------------------------------------------------

/// The same executors consume either the CLI's checked byte spans or values
/// already validated by the record iterator. Implementations are specialized
/// at compile time; the decoded path never reads substitution bytes again.
pub(crate) trait ValueSource<'a> {
    type Scope: Copy;
    type Value;
    fn slots(&self, scope: Self::Scope) -> &[Self::Value];
    fn empty(&self, slot: &Self::Value) -> bool;
    fn element(&self, slot: &Self::Value) -> bool;
    fn array(&self, slot: &Self::Value) -> Option<impl ExactSizeIterator<Item = Utf16LeSlice<'a>>>;
    fn bare(&self, slot: &Self::Value) -> bool;
    fn write(
        &self,
        slot: &Self::Value,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
        mode: StringEscapeMode,
    ) -> Result<()>;
    fn nested(&self, slot: &Self::Value) -> Option<Instance<Self::Scope>>;
    fn fragment(&self, _slot: &Self::Value) -> Option<(&IrArena<'a>, ElementId)> {
        None
    }
    fn bytes(&self, _slot: &Self::Value) -> Option<&'a [u8]> {
        None
    }
}

struct RawValues<'s, 'a> {
    pf: &'s Preflight,
    data: &'a [u8],
}

impl<'a> RawValues<'_, 'a> {
    /// Substitution `id` of an instance, as `mta::SystemFields` reads it: decoded the way the
    /// record's tree holds it.
    fn mta_value<'s, P: TemplateProgram>(
        &'s self,
        slots: &[RawSlot],
        id: u16,
        progs: &'s ProgramCache<P>,
        chunk: &'a EvtxChunk<'a>,
    ) -> mta::Value<'s> {
        let Some(s) = slots.get(usize::from(id)) else {
            return mta::Value::None;
        };
        if self.pf.slot_empty(s, self.data) {
            return mta::Value::None;
        }
        if s.ty == value_ty::BIN_XML {
            let nested = s.nested_idx().map(|i| self.pf.nested[i].prog);
            return mta::Value::Element(nested.map(|prog| progs.programs[prog].root_name()));
        }
        let value = BinXmlValueType::from_u8(s.ty).and_then(|ty| {
            let mut cursor = ByteCursor::with_pos(s.bytes(self.data), 0).ok()?;
            let ansi_codec = chunk.settings.get_ansi_codec();
            let size = Some(s.len);
            BinXmlValue::deserialize_value_type_cursor_in(
                &ty,
                &mut cursor,
                size,
                ansi_codec,
                &chunk.arena,
            )
            .ok()
        });
        value.map_or(mta::Value::None, mta::Value::Scalar)
    }
}

impl<'a> ValueSource<'a> for RawValues<'_, 'a> {
    type Scope = SlotRange;
    type Value = RawSlot;
    #[inline(always)]
    fn slots(&self, scope: SlotRange) -> &[RawSlot] {
        &self.pf.slots[scope.0 as usize..scope.1 as usize]
    }
    #[inline(always)]
    fn empty(&self, s: &RawSlot) -> bool {
        self.pf.slot_empty(s, self.data)
    }
    #[inline(always)]
    fn element(&self, s: &RawSlot) -> bool {
        s.is_binxml_payload()
    }
    #[inline(always)]
    fn array(&self, s: &RawSlot) -> Option<impl ExactSizeIterator<Item = Utf16LeSlice<'a>>> {
        (s.ty == value_ty::STR_ARRAY).then(|| {
            self.pf.str_array_items(s).iter().map(|&(off, len)| {
                Utf16LeSlice::new(
                    &self.data[off as usize..off as usize + usize::from(len)],
                    usize::from(len) / 2,
                )
            })
        })
    }
    #[inline(always)]
    fn bare(&self, s: &RawSlot) -> bool {
        json_bare_type(s.ty)
    }
    #[inline(always)]
    fn write(
        &self,
        s: &RawSlot,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
        mode: StringEscapeMode,
    ) -> Result<()> {
        vr.write_raw_value_text(
            out,
            s.ty,
            s.bytes(self.data),
            s.ansi_idx().map(|i| self.pf.ansi(i)),
            mode,
        )
    }
    #[inline(always)]
    fn nested(&self, s: &RawSlot) -> Option<Instance> {
        s.nested_idx().map(|i| self.pf.nested[i])
    }
    #[inline(always)]
    fn bytes(&self, s: &RawSlot) -> Option<&'a [u8]> {
        Some(s.bytes(self.data))
    }
}

struct DecodedValues<'s, 'a> {
    content: &'s TemplateContent<'a>,
    nested: &'s mut Vec<Option<Instance<usize>>>,
}

impl<'a> ValueSource<'a> for DecodedValues<'_, 'a> {
    type Scope = usize;
    type Value = ValidatedValue<'a>;
    #[inline(always)]
    fn slots(&self, scope: usize) -> &[ValidatedValue<'a>] {
        &self.content.instance(scope).values
    }
    #[inline(always)]
    fn empty(&self, s: &Self::Value) -> bool {
        match s {
            ValidatedValue::Scalar(v) => crate::model::ir::is_optional_empty(v),
            _ => false,
        }
    }
    #[inline(always)]
    fn element(&self, s: &Self::Value) -> bool {
        !matches!(s, ValidatedValue::Scalar(_))
    }
    #[inline(always)]
    fn array(&self, s: &Self::Value) -> Option<impl ExactSizeIterator<Item = Utf16LeSlice<'a>>> {
        match s {
            ValidatedValue::Scalar(BinXmlValue::StringArrayType(items)) => {
                Some(items.iter().copied())
            }
            _ => None,
        }
    }
    #[inline(always)]
    fn bare(&self, s: &Self::Value) -> bool {
        matches!(s, ValidatedValue::Scalar(value) if json_bare_value(value))
    }
    #[inline(always)]
    fn write(
        &self,
        s: &Self::Value,
        vr: &mut ValueRenderer,
        out: &mut Vec<u8>,
        mode: StringEscapeMode,
    ) -> Result<()> {
        let ValidatedValue::Scalar(value) = s else {
            return Err(unresolved_placeholder());
        };
        if matches!(value, BinXmlValue::StringArrayType(_)) {
            // Reused array slots must expand at every occurrence. Scalar ops
            // fall back to materialization, matching the raw source.
            return Err(model_err("array substitution outside its expand op"));
        }
        vr.write_value_text(out, value, mode)
    }
    #[inline(always)]
    fn nested(&self, s: &Self::Value) -> Option<Instance<usize>> {
        match s {
            ValidatedValue::Template(id) => self.nested[*id],
            _ => None,
        }
    }
    #[inline(always)]
    fn fragment(&self, s: &Self::Value) -> Option<(&IrArena<'a>, ElementId)> {
        match s {
            ValidatedValue::Fragment(id) => Some((&self.content.frags, *id)),
            _ => None,
        }
    }
}

impl<'a> DecodedValues<'_, 'a> {
    fn render<P: StoredProgram>(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        progs: &mut ProgramCache<P>,
        out: &mut Vec<u8>,
    ) -> bool {
        let Some(instance) = self.prepare(usize::MAX, 0, 0, true, chunk, progs) else {
            return false;
        };
        exec_or_rollback(
            instance,
            progs,
            self,
            chunk,
            None,
            &mut ValueRenderer::new(),
            out,
        )
    }

    /// Check program compatibility only. All values, nested templates and
    /// fragments have already been decoded and validated by the iterator.
    fn prepare<P: StoredProgram>(
        &mut self,
        scope: usize,
        depth: usize,
        indent: u16,
        root: bool,
        chunk: &'a EvtxChunk<'a>,
        progs: &mut ProgramCache<P>,
    ) -> Option<Instance<usize>> {
        use BinXmlValue::*;
        if depth > MAX_FAST_LANE_NESTING {
            return None;
        }
        let input = self.content.instance(scope);
        let prog = get_or_compile(
            chunk,
            (input.offset, indent, root),
            progs,
            TemplateSource::Validated(&input.template, input.has_literal_array),
        )?;
        if input.has_arrays {
            for (i, value) in input.values.iter().enumerate() {
                if let ValidatedValue::Scalar(value) = value
                    && value.expandable_array_len().is_some()
                {
                    match value {
                        StringArrayType(items)
                            if !items.is_empty()
                                && u16::try_from(i).ok().is_some_and(|i| {
                                    progs.programs[prog].expand_slots().contains(&i)
                                }) => {}
                        _ => return None,
                    }
                }
            }
        }
        for item in 0..progs.programs[prog].elem_slots().len() {
            let (id, child_indent) = progs.programs[prog].elem_slots()[item];
            match input.values.get(usize::from(id)) {
                Some(ValidatedValue::Template(id)) => {
                    let child = self.prepare(*id, depth + 1, child_indent, false, chunk, progs)?;
                    self.nested[*id] = Some(child);
                }
                Some(ValidatedValue::Fragment(_)) if !P::ALLOW_GENERIC_FRAGS => return None,
                _ => {}
            }
        }
        for &constraint in progs.programs[prog].constraints() {
            if let Some(slot) = input.values.get(usize::from(constraint.slot()))
                && constraint.violated(self.element(slot), self.empty(slot))
            {
                return None;
            }
        }
        Some(Instance { prog, slots: scope })
    }
}

pub(crate) fn try_render_validated<'a>(
    content: &TemplateContent<'a>,
    chunk: &'a EvtxChunk<'a>,
    caches: &mut RenderCaches,
    json: bool,
    out: &mut Vec<u8>,
) -> bool {
    caches.nested.clear();
    caches.nested.resize(content.nested.len(), None);
    let mut values = DecodedValues {
        content,
        nested: &mut caches.nested,
    };
    if json {
        values.render(chunk, &mut caches.json, out)
    } else {
        values.render(chunk, &mut caches.xml, out)
    }
}

/// Whether `ty` renders as a bare JSON number/bool (`write_value_as_number`).
fn json_bare_type(ty: u8) -> bool {
    matches!(ty, value_ty::INT8..=value_ty::UINT64 | value_ty::BOOL)
}

fn json_bare_value(value: &BinXmlValue<'_>) -> bool {
    use BinXmlValue::*;
    matches!(
        value,
        Int8Type(_)
            | UInt8Type(_)
            | Int16Type(_)
            | UInt16Type(_)
            | Int32Type(_)
            | UInt32Type(_)
            | Int64Type(_)
            | UInt64Type(_)
            | BoolType(_)
    )
}

fn exec_json<'a, V: ValueSource<'a>>(
    instance: Instance<V::Scope>,
    progs: &JsonProgramCache,
    pf: &V,
    vr: &mut ValueRenderer,
    out: &mut Vec<u8>,
) -> Result<()> {
    let prog = &progs.programs[instance.prog];
    let lits = &prog.lits;
    let slots = pf.slots(instance.slots);
    let slot_at = |id: u16| slots.get(usize::from(id));

    macro_rules! write_lit {
        ($r:expr) => {
            out.extend_from_slice($r.of(lits))
        };
    }
    macro_rules! write_scalar {
        ($s:expr) => {{
            let s = $s;
            if pf.bare(s) {
                pf.write(s, vr, out, StringEscapeMode::Json)?;
            } else {
                out.push(b'"');
                pf.write(s, vr, out, StringEscapeMode::Json)?;
                out.push(b'"');
            }
        }};
    }

    for op in &prog.ops {
        match op {
            JOp::Lit(r) => write_lit!(*r),
            JOp::LeafVal {
                prefix,
                slot,
                empty,
            } => {
                write_lit!(*prefix);
                match slot_at(*slot) {
                    None => write_lit!(*empty),
                    Some(s) => {
                        if let Some(items) = pf.array(s) {
                            // Positional-Data aggregation: one item renders bare
                            // (like an unexpanded scalar), more become a JSON array.
                            // Items are strings, so they are always quoted.
                            let multi = items.len() > 1;
                            if multi {
                                out.push(b'[');
                            }
                            for (i, item) in items.enumerate() {
                                if i > 0 {
                                    out.push(b',');
                                }
                                out.push(b'"');
                                vr.write_value_text(
                                    out,
                                    &BinXmlValue::StringType(item),
                                    StringEscapeMode::Json,
                                )?;
                                out.push(b'"');
                            }
                            if multi {
                                out.push(b']');
                            }
                        } else if pf.empty(s) {
                            write_lit!(*empty);
                        } else if pf.element(s) {
                            let Some(inst) = pf.nested(s) else {
                                return Err(unresolved_nested_json());
                            };
                            out.push(b'{');
                            out.push(b'"');
                            out.extend_from_slice(&progs.programs[inst.prog].root_name);
                            out.extend_from_slice(b"\":");
                            exec_json(inst, progs, pf, vr, out)?;
                            out.push(b'}');
                        } else {
                            write_scalar!(s);
                        }
                    }
                }
            }
            JOp::Elem {
                attrs,
                content,
                empty,
            } => {
                // Evaluate attribute member presence.
                let attr_present = attrs.iter().any(|part| match *part {
                    JAttrPart::Literal(_) => true,
                    JAttrPart::Placeholder { slot, .. } => {
                        slot_at(slot).is_some_and(|s| !pf.empty(s))
                    }
                });
                let content_slot = content.and_then(|c| slot_at(c).filter(|s| !pf.empty(*s)));
                match (attr_present, content_slot) {
                    (false, None) => write_lit!(*empty),
                    (false, Some(s)) => write_scalar!(s),
                    (true, content_slot) => {
                        out.extend_from_slice(b"{\"#attributes\":{");
                        let mut first = true;
                        for part in attrs.iter() {
                            match *part {
                                JAttrPart::Literal(member) => {
                                    if !first {
                                        out.push(b',');
                                    }
                                    first = false;
                                    write_lit!(member);
                                }
                                JAttrPart::Placeholder { key, slot } => {
                                    if let Some(s) = slot_at(slot)
                                        && !pf.empty(s)
                                    {
                                        if !first {
                                            out.push(b',');
                                        }
                                        first = false;
                                        write_lit!(key);
                                        write_scalar!(s);
                                    }
                                }
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
                if pf.empty(s) || !pf.element(s) {
                    continue; // constraint guarantees elem-or-empty
                }
                let Some(inst) = pf.nested(s) else {
                    return Err(unresolved_nested_json());
                };
                let name = progs.programs[inst.prog].root_name.as_slice();
                // `_N` suffix: static members first, then prior dynamics.
                let mut count: u16 = 0;
                for (r, c) in static_names.iter() {
                    if &lits[r.0 as usize..r.1 as usize] == name {
                        count = *c;
                        break;
                    }
                }
                // Prior dynamic same-name members would need scratch state; the
                // compiler admits at most one SlotChild per object.
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
                exec_json(inst, progs, pf, vr, out)?;
            }
        }
    }
    Ok(())
}

/// Benchmark-only entry to the JSON text-content path.
#[cfg(feature = "bench")]
pub(crate) fn bench_json_text_content(out: &mut Vec<u8>, nodes: &[Node<'_>]) -> Result<()> {
    let mut c = JsonCompiler::new(None, std::mem::take(out), false);
    let res = c.text_content_plain(nodes, false);
    *out = c.lits;
    res
}

// ---------------------------------------------------------------------------
// JSON materialized layer (the old JsonEmitter semantics over plain nodes).
//
// This layer is the sole behavioral source of truth for JSON text: both the
// slow lane (`JsonProgram::render_tree`) and the compiler's pre-rendering of literal
// content go through these `*_plain` methods, so the two lanes cannot drift.
// ---------------------------------------------------------------------------
//
// These methods are the single implementation of JSON record rendering: the
// slow lane walks fully materialized trees through them, and the compiler
// calls them for placeholder-free pieces (subtrees, literal attributes,
// literal `Data` elements). Placeholder nodes are real errors here, mirroring
// the legacy emitter's behavior on materialized trees.

impl<'t, 'a> JsonCompiler<'t, 'a> {
    /// `,"<root>":<value>`: the root as one more member of an enclosing object.
    fn render_member_plain(&mut self) -> Result<()> {
        let root = self.tree().root_element();
        self.lits.push(b',');
        self.key_with_suffix_plain(root.name.as_str(), 0);
        self.write_element_value_plain(root, false)
    }

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
            self.key_with_suffix_plain(root.name.as_str(), 0);
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
        self.vr
            .write_str_escaped(&mut self.lits, s, StringEscapeMode::Json)
    }

    fn json_text_plain(&mut self, text: &Text<'_>) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        match text {
            Text::Utf16(value) => {
                self.vr
                    .write_utf16_escaped(&mut self.lits, *value, StringEscapeMode::Json)
            }
            Text::Utf8(value) => self.json_escaped_str_plain(value),
        }
    }

    /// Pre-render a bare JSON number/bool (see `json_bare_value`).
    fn value_as_number_plain(&mut self, value: &BinXmlValue<'_>) -> Result<bool> {
        if !json_bare_value(value) {
            return Ok(false);
        }
        self.vr.write_json_value_text(&mut self.lits, value)?;
        Ok(true)
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
                        self.vr.write_u64(&mut self.lits, u64::from(*ch))?;
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
                    return Err(element_in_text());
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
            self.vr.write_u64(&mut self.lits, u64::from(suffix))?;
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
            let suffix = next_name_suffix(cname, &mut name_counts);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ir::Name;
    use bumpalo::Bump;

    #[test]
    fn chunk_remains_send() {
        fn assert_send<T: Send>() {}
        assert_send::<EvtxChunk<'static>>();
    }

    fn binxml_placeholder(id: u16) -> Node<'static> {
        Node::Placeholder(Placeholder { id, optional: true })
    }

    #[test]
    fn json_compiler_bails_on_multiple_dynamic_child_slots() {
        let bump = Bump::new();
        let mut arena = IrArena::new_in(&bump);

        let static_child = arena.new_node(Element::new_in(Name::new("Static"), &bump));

        let mut root = Element::new_in(Name::new("Parent"), &bump);
        root.push_child(Node::Element(static_child));
        root.push_child(binxml_placeholder(0));
        root.push_child(binxml_placeholder(1));
        let root = arena.new_node(root);

        let tree = IrTree::new(arena, root);

        assert!(
            compile_json_template(&tree, false, true, &ParserSettings::default()).is_none(),
            "multiple dynamic child slots need runtime name-suffix state; use the materialized path"
        );
    }

    /// Embedded BinXML names elements the body's name walk does not see, so
    /// such a body must not be shared across chunks.
    #[test]
    fn name_refs_reject_embedded_binxml() {
        // Value token, BinXml type, 1-byte payload (EOF); then EOF.
        let body = [0x05, 0x21, 0x01, 0x00, 0x00, 0x00];
        let bump = bumpalo::Bump::new();
        let src = Stream {
            data: &body,
            chunk: None,
            ansi_codec: encoding_rs::WINDOWS_1252,
            bump: &bump,
        };
        let mut refs = NameRefs {
            body: 0..body.len(),
            inside: false,
            outside: Vec::new(),
        };
        assert!(walk_tokens(src, 0, body.len(), true, &mut refs).is_err());
    }

    /// Name offsets are absolute, so the same body bytes at another offset
    /// share a key only if no name moves into or out of the body.
    #[test]
    fn name_refs_key_rejects_moved_bodies() {
        // One string-table entry at 30: link, length 1, "A".
        let mut data = [0u8; 40];
        data[36] = 1;
        data[38] = b'A';
        let hasher = ahash::RandomState::new();
        let start = 100 + TEMPLATE_DEFINITION_HEADER_SIZE;
        let refs = |inside| NameRefs {
            body: start..start + 16,
            inside,
            outside: vec![30],
        };
        let at_100 = refs(false).key(&data, 100, &hasher);
        assert!(matches!(at_100, Some((_, None))));
        assert!(matches!(
            refs(true).key(&data, 100, &hasher),
            Some((_, Some(100)))
        ));
        // Moved, nothing crosses the body boundary: same key.
        assert_eq!(refs(false).key(&data, 200, &hasher), at_100);
        // A name was inside the old body.
        assert_eq!(refs(true).key(&data, 200, &hasher), None);
        // The outside name lands inside the moved body (24..40).
        assert_eq!(refs(false).key(&data, 0, &hasher), None);
    }
}
