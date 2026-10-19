//! BinXML IR construction and template instantiation.
//!
//! This module converts the BinXML token stream in an EVTX record (or template definition)
//! into the IR tree defined in [`crate::model::ir`].
//!
//! ## Key data structures (worth skimming before the code)
//!
//! - [`IrArena`]: bump-backed storage for [`Element`] nodes (dense vector).
//! - [`ElementId`]: index into an [`IrArena`].
//! - [`Node`]: child items inside an element (text/value/entity refs, or `Node::Element(id)`).
//! - [`IrVec`]: bump-allocated `Vec` used for attributes and children.
//!
//! ## Two parsing modes: records vs template definitions
//!
//! BinXML separates **structure** (template definition) from **data** (template instance).
//!
//! - **`BuildMode::TemplateDefinition`**:
//!   - `SubstitutionDescriptor` tokens are recorded as `Node::Placeholder` nodes.
//!   - Template instances are rejected (a template definition can't contain instances).
//!   - Exactly one root element is required (hard error otherwise).
//!
//! - **`BuildMode::Record`**:
//!   - Template instances are instantiated immediately and inserted into the tree.
//!   - `SubstitutionDescriptor` tokens are rejected (they only occur in template defs).
//!   - Multiple top-level fragments are tolerated (fail-soft; first root wins).
//!
//! Templates are parsed once and cached per chunk iterator (`IrTemplateCache`) as IR trees
//! containing placeholders.
//!
//! ## Splicing: the “what just happened?” mechanism
//!
//! There is intentionally **no** "template instance node" in the final IR. When we encounter:
//!
//! - a **template instance token** (`0x0c`), or
//! - a **`BinXmlType` value** (an embedded BinXML fragment inside a value/substitution),
//!
//! we immediately parse/instantiate it into a normal IR element tree and then **attach it**
//! to the current parent (or use it as the record root). This is what the file means by
//! “splicing in-place”: the surrounding parse continues as if those elements had appeared
//! directly in the token stream.
//!
//! There are *two* layers of splicing during template instantiation:
//!
//! 1. **Placeholder splicing**: each `Node::Placeholder` expands to **0 or 1** `Node`s in the
//!    output `IrVec` (0 when `optional` and the value is empty, 1 when resolved).
//! 2. **Array substitution splicing** (MS-EVEN6 §3.1.4.7.5): if a resolved substitution is an
//!    **array** value (e.g. `StringArrayType`), the *containing element is repeated* once per
//!    array item. In IR terms, cloning a single `Node::Element` from the template can yield
//!    **N** element IDs, which are then pushed into the parent’s `children` vector.
//!
//! Example (array substitution expands into repeated elements):
//!
//! ```text
//! Template: <Data>%{0}</Data>
//! Values[0]: StringArrayType(["a", "b"])
//!
//! Result under parent:
//!   children += [ Element(Data{ "a" }), Element(Data{ "b" }) ]
//! ```
//!
//! The renderers (`binxml::compiled`) assume materialized trees have all
//! placeholders resolved/expanded; cached-template programs resolve them at
//! execution time.
//!
//! ## Parser implementation notes
//!
//! The production builder parses BinXML bytes directly (cursor-based) to avoid iterator
//! overhead and intermediate token allocations.

use crate::ChunkOffset;
use crate::EvtxChunk;
use crate::binxml::array_expand::{
    expand_array_substitutions_in_element, node_needs_array_expansion,
};
use crate::binxml::name::NAME_LINK_SIZE;
use crate::binxml::tokens::{
    BinXmlTemplateValues, Stream, TemplateHeader, TokenSink, ends_stream, read_template_header,
    read_template_values_cursor, walk_tokens,
};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{
    Attr, Element, ElementId, IrArena, IrTree, IrVec, Name, Node, Placeholder, Text,
    is_optional_empty,
};
use crate::utils::{ByteCursor, Utf16LeSlice};
use ahash::AHashMap;
use bumpalo::Bump;
use std::rc::Rc;

/// Incremental element builder for streaming token parsing.
///
/// Attributes are collected until the element start is closed, then materialized
/// into an `Element`.
struct ElementBuilder<'a> {
    name: Name<'a>,
    attrs: IrVec<'a, Attr<'a>>,
    current_attr_name: Option<Name<'a>>,
    current_attr_value: IrVec<'a, Node<'a>>,
    arena: &'a Bump,
}

impl<'a> ElementBuilder<'a> {
    fn new(name: Name<'a>, arena: &'a Bump) -> Self {
        ElementBuilder {
            name,
            attrs: IrVec::new_in(arena),
            current_attr_name: None,
            current_attr_value: IrVec::new_in(arena),
            arena,
        }
    }

    fn start_attribute(&mut self, name: Name<'a>) {
        self.finish_attr_if_any();
        self.current_attr_name = Some(name);
    }

    fn push_attr_value(&mut self, node: Node<'a>) {
        if self.current_attr_name.is_some() {
            self.current_attr_value.push(node);
        }
    }

    fn finish_attr_if_any(&mut self) {
        if let Some(name) = self.current_attr_name.take() {
            if !self.current_attr_value.is_empty() {
                let value =
                    std::mem::replace(&mut self.current_attr_value, IrVec::new_in(self.arena));
                self.attrs.push(Attr { name, value });
            } else {
                self.current_attr_value.clear();
            }
        }
    }

    fn finish(mut self) -> Element<'a> {
        self.finish_attr_if_any();
        Element {
            name: self.name,
            attrs: self.attrs,
            children: IrVec::new_in(self.arena),
            has_element_child: false,
        }
    }
}

/// Size (in bytes) of a template definition header (next offset + guid + size).
pub(crate) const TEMPLATE_DEFINITION_HEADER_SIZE: usize = 24;

/// Cache of parsed BinXML templates keyed by template definition offset.
///
/// Templates are stored as IR trees containing placeholders; instantiation
/// clones the tree and resolves all placeholders using substitution values.
#[derive(Debug)]
pub(crate) struct IrTemplateCache<'a> {
    /// Cached definition tree + "contains a literal expandable array" flag
    /// (precomputed once so per-record expansion gating is a lookup).
    templates: AHashMap<u32, (Rc<IrTree<'a>>, bool)>,
    arena: &'a Bump,
}

impl<'a> IrTemplateCache<'a> {
    #[cfg(feature = "bench")]
    pub(crate) fn new(arena: &'a Bump) -> Self {
        IrTemplateCache::with_capacity(0, arena)
    }

    pub(crate) fn with_capacity(capacity: usize, arena: &'a Bump) -> Self {
        IrTemplateCache {
            templates: AHashMap::with_capacity(capacity),
            arena,
        }
    }

    /// Cached template tree + literal-array flag, for the compiled-template
    /// path (`binxml::compiled`).
    pub(crate) fn template_for_compile(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        template_def_offset: u32,
    ) -> Result<(Rc<IrTree<'a>>, bool)> {
        self.get_or_parse_template_direct(chunk, template_def_offset, 0)
    }

    /// Load a template definition from the chunk (or return a cached copy),
    /// with its "contains a literal expandable array" flag.
    ///
    /// The returned [`IrTree`] is stored in the cache and contains `Node::Placeholder` nodes.
    /// It is never rendered directly; instead it is used as the source for
    /// [`materialize_instance`].
    fn get_or_parse_template_direct(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        template_def_offset: u32,
        depth: usize,
    ) -> Result<(Rc<IrTree<'a>>, bool)> {
        if let Some(existing) = self.templates.get(&template_def_offset) {
            return Ok(existing.clone());
        }
        let header = read_template_definition_header_at(chunk.data, template_def_offset)?;

        let parsed = template_body_at(chunk.data, template_def_offset, &header).and_then(|bytes| {
            build_template_in(Stream::chunk(chunk, self.arena), bytes, self, depth)
        });
        // If the embedded chunk template is corrupt/missing, optionally fall back to an
        // offline WEVT cache (provider resources) when configured.
        #[cfg(feature = "wevt_templates")]
        let parsed = parsed.or_else(|err| self.wevt_template(chunk, &header, depth).ok_or(err));

        let template = Rc::new(parsed?);
        let has_literal_array = tree_has_literal_array(&template);
        self.templates.insert(
            template_def_offset,
            (Rc::clone(&template), has_literal_array),
        );
        Ok((template, has_literal_array))
    }

    #[cfg(feature = "wevt_templates")]
    fn wevt_template(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        header: &TemplateHeader,
        depth: usize,
    ) -> Option<IrTree<'a>> {
        let cache = chunk.settings.get_wevt_cache()?;
        let guid = winstructs::guid::Guid::from_buffer(&header.guid).ok()?;
        let binxml = cache
            .load_temp_binxml_fragment_in(&guid.to_string(), self.arena)
            .ok()?;
        let src = Stream::wevt(binxml, chunk.settings.get_ansi_codec(), self.arena);
        build_template_in(src, binxml, self, depth).ok()
    }
}

/// Parsing mode for the streaming builder.
///
/// See the module-level docs for the "record vs template definition" behavior matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildMode {
    Record,
    TemplateDefinition,
}

/// Hard cap on BinXML fragment / template-instance nesting per record. Real
/// records nest one or two levels; the cap exists to bound recursion depth on
/// crafted input (a 64 KiB chunk could otherwise chain thousands of levels).
const MAX_BINXML_NESTING: usize = 64;

/// Hard cap on open-element depth within one BinXML stream, bounding tree
/// depth (and the walkers' recursion over it) on crafted input.
const MAX_ELEMENT_DEPTH: usize = 512;

/// Streaming token consumer that builds the IR tree.
///
/// This builder maintains a typical XML stack of "open elements" (`stack`) and emits
/// `Node`s either into the current attribute being built (`current_element`) or into the
/// current element's `children`.
///
/// Splicing-related behavior:
/// - In `TemplateDefinition` mode, `SubstitutionDescriptor` tokens become `Node::Placeholder`.
/// - In `Record` mode, template instances are instantiated immediately and attached to the
///   current parent/root (no special node remains in the IR).
/// - Array substitutions can cause a single template element to expand into multiple elements;
///   this happens during cloning/instantiation, not during streaming token consumption.
struct TreeBuilder<'a, 'cache, 'arena> {
    src: Stream<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    arena: &'arena mut IrArena<'a>,
    /// Fragment/instance nesting depth of this stream (see [`MAX_BINXML_NESTING`]).
    depth: usize,
    stack: IrVec<'a, ElementId>,
    current_element: Option<ElementBuilder<'a>>,
    root: Option<ElementId>,
}

impl<'a> TokenSink<'a> for TreeBuilder<'a, '_, '_> {
    fn open_start(&mut self, name: ChunkOffset) -> Result<()> {
        if self.current_element.is_some() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "open start - Bad parser state",
            ));
        }
        let name = self.name(name)?;
        self.current_element = Some(ElementBuilder::new(name, self.src.bump));
        Ok(())
    }

    fn attribute(&mut self, name: ChunkOffset) -> Result<()> {
        // Compute name before borrowing `current_element` mutably (avoid borrow conflict).
        let name = self.name(name)?;
        let builder = self
            .current_element
            .as_mut()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("attribute - Bad parser state"))?;
        builder.start_attribute(name);
        Ok(())
    }

    fn close_start(&mut self) -> Result<()> {
        if self.stack.len() >= MAX_ELEMENT_DEPTH {
            return Err(EvtxError::FailedToCreateRecordModel(
                "BinXML elements nested too deeply",
            ));
        }
        let element = self
            .current_element
            .take()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("close start - Bad parser state"))?
            .finish();
        let element_id = self.arena.new_node(element);
        self.stack.push(element_id);
        Ok(())
    }

    fn close_empty(&mut self) -> Result<()> {
        let element = self
            .current_element
            .take()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("close empty - Bad parser state"))?
            .finish();
        let element_id = self.arena.new_node(element);
        self.attach(element_id)
    }

    fn close_element(&mut self) -> Result<()> {
        let element_id = self.stack.pop().ok_or_else(|| {
            EvtxError::FailedToCreateRecordModel("close element - Bad parser state")
        })?;
        self.attach(element_id)
    }

    fn substitution(&mut self, id: u16, optional: bool) -> Result<()> {
        if self.mode != BuildMode::TemplateDefinition {
            return Err(EvtxError::FailedToCreateRecordModel(
                "substitution outside template definition",
            ));
        }

        // In template definitions, substitutions are *structural*: we can't resolve them yet,
        // so we store a placeholder node that records:
        // - the substitution index into the instance value array
        // - whether the substitution is optional (may be omitted if empty)
        self.push_node(Node::Placeholder(Placeholder { id, optional }))
    }

    fn value(&mut self, value: BinXmlValue<'a>) -> Result<()> {
        match value {
            BinXmlValue::BinXmlType(bytes) => {
                if self.current_element.is_some() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "nested BinXML inside attribute value",
                    ));
                }
                // A `BinXmlType` value is an embedded BinXML fragment. We parse it into a normal
                // IR subtree and then attach the resulting element at the current location.
                //
                // This is another form of "splicing": the fragment becomes regular child elements
                // rather than being kept as a typed value node.
                if bytes.is_empty() {
                    return Ok(());
                }
                let element_id = build_in(
                    self.src,
                    bytes,
                    self.cache,
                    self.arena,
                    BuildMode::Record,
                    self.depth + 1,
                )?;
                self.attach(element_id)
            }
            BinXmlValue::EvtXml => Err(EvtxError::FailedToCreateRecordModel(
                "Unimplemented - EvtXml",
            )),
            other => {
                let node = value_to_node(other)?;
                self.push_node(node)
            }
        }
    }

    fn template_instance(&mut self, template: BinXmlTemplateValues<'a>) -> Result<()> {
        if self.mode != BuildMode::Record {
            return Err(EvtxError::FailedToCreateRecordModel(
                "template instance inside template definition",
            ));
        }
        if self.current_element.is_some() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "template instance inside attribute value",
            ));
        }
        // Template instances are *not* represented as a distinct IR node. We expand them into a
        // regular element tree and attach that tree at the current parse location.
        let chunk = self.src.chunk.ok_or_else(|| {
            EvtxError::FailedToCreateRecordModel("template instance requires an EVTX chunk context")
        })?;
        // No nested-instance list: nested BinXML is built in stream order (as
        // `build_in` does), so errors surface in the same order as a full walk.
        let instance =
            validate_tpl_instance(template, chunk, self.cache, self.arena, None, self.depth)?;
        let element_id = materialize_instance(&instance, &[], self.src.bump, self.arena)?;
        self.attach(element_id)
    }

    fn entity_ref(&mut self, name: ChunkOffset) -> Result<()> {
        let name = self.name(name)?;
        self.push_node(Node::EntityRef(name))
    }

    fn pi_target(&mut self, name: ChunkOffset) -> Result<()> {
        let name = self.name(name)?;
        self.push_node(Node::PITarget(name))
    }

    fn pi_data(&mut self, data: Utf16LeSlice<'a>) -> Result<()> {
        self.push_node(Node::PIData(Text::utf16(data)))
    }
}

impl<'a, 'cache, 'arena> TreeBuilder<'a, 'cache, 'arena> {
    /// Attach an element to the current parent (or set it as root).
    ///
    /// Used by element closure and by splicing (template instances, nested `BinXmlType`
    /// fragments). With no open parent the first root wins; additional roots are ignored in
    /// records (fail-soft for corrupted records) and rejected in template definitions.
    fn attach(&mut self, element_id: ElementId) -> Result<()> {
        if let Some(&parent_id) = self.stack.last() {
            let parent = self
                .arena
                .get_mut(parent_id)
                .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid parent element id"))?;
            parent.push_child(Node::Element(element_id));
        } else if self.root.is_none() {
            self.root = Some(element_id);
        } else if self.mode == BuildMode::TemplateDefinition {
            return Err(EvtxError::FailedToCreateRecordModel(
                "multiple root elements",
            ));
        }
        Ok(())
    }

    fn name(&self, offset: ChunkOffset) -> Result<Name<'a>> {
        // Chunk streams reference the string table; WEVT templates inline names.
        let name = match self.src.chunk {
            Some(chunk) => expand_string_ref(offset, chunk, self.src.bump),
            None => expand_wevt_inline_name_ref(self.src.data, offset, self.src.bump),
        }?;
        Ok(Name::new(name))
    }

    fn push_node(&mut self, node: Node<'a>) -> Result<()> {
        if let Some(builder) = self.current_element.as_mut() {
            if matches!(node, Node::Element(_)) {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "element inside attribute value",
                ));
            }
            // Attribute context: append to the attribute value node list (no elements allowed).
            builder.push_attr_value(node);
            Ok(())
        } else {
            // Element context: append to the current open element's children.
            push_child(self.arena, &self.stack, node)
        }
    }

    fn finish(self) -> Result<ElementId> {
        if self.current_element.is_some() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "unfinished element start",
            ));
        }

        if !self.stack.is_empty() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "unbalanced element stack",
            ));
        }

        if let Some(root) = self.root {
            return Ok(root);
        }

        match self.mode {
            BuildMode::Record => {
                // Some corrupted records can contain an empty/invalid BinXML fragment (e.g. only
                // an EOF token). Keep iteration fail-soft by synthesizing an empty root element.
                let name = Name::new("Event");
                let element = Element::new_in(name, self.src.bump);
                let root = self.arena.new_node(element);
                Ok(root)
            }
            BuildMode::TemplateDefinition => {
                Err(EvtxError::FailedToCreateRecordModel("missing root element"))
            }
        }
    }
}

/// Parse a BinXML byte slice (a sub-slice of `src.data`) into an IR element tree.
///
/// [`walk_tokens`] parses the tokens and [`TreeBuilder`] builds the tree. Splicing shows up
/// via token types:
/// - `0x0c` (TemplateInstance): instantiate the template and attach it immediately
///   (`TreeBuilder::template_instance`).
/// - `Value(BinXmlType(..))`: `TreeBuilder::value` builds a nested IR subtree and attaches it
///   as child elements.
///
/// `mode` controls whether substitutions become placeholders (template definitions) or are
/// rejected (records); template definitions also carry dependency IDs on element start tokens.
/// `depth` is the fragment/instance nesting depth of this stream (recursion guard).
fn build_in<'a>(
    src: Stream<'a>,
    bytes: &'a [u8],
    cache: &mut IrTemplateCache<'a>,
    arena: &mut IrArena<'a>,
    mode: BuildMode,
    depth: usize,
) -> Result<ElementId> {
    // Only template definitions carry dependency IDs on element start tokens.
    let has_dep_id = mode == BuildMode::TemplateDefinition;
    if depth > MAX_BINXML_NESTING {
        return Err(EvtxError::FailedToCreateRecordModel(
            "BinXML fragments nested too deeply",
        ));
    }
    let offset = binxml_slice_offset_in(src.data, bytes)?;
    let mut builder = TreeBuilder {
        src,
        cache,
        mode,
        arena,
        depth,
        stack: IrVec::new_in(src.bump),
        current_element: None,
        root: None,
    };
    walk_tokens(src, offset as usize, bytes.len(), has_dep_id, &mut builder)?;
    builder.finish()
}

/// Build an IR tree directly from BinXML bytes without an iterator.
pub(crate) fn build_tree_from_binxml_bytes_direct<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
) -> Result<IrTree<'a>> {
    let mut arena =
        IrArena::with_capacity_in(estimate_node_capacity(bytes.len() as u32), &chunk.arena);
    let src = Stream::chunk(chunk, &chunk.arena);
    let root = build_in(src, bytes, cache, &mut arena, BuildMode::Record, 0)?;
    Ok(IrTree::new(arena, root))
}

/// Parsed content retains the values and fragments checked before yielding a
/// record, so both compiled rendering and materialization can reuse them.
#[derive(Debug, Clone)]
pub(crate) enum RecordContent<'a> {
    Tree(IrTree<'a>),
    Template(TemplateContent<'a>),
}

#[derive(Debug, Clone)]
pub(crate) enum ValidatedValue<'a> {
    Scalar(BinXmlValue<'a>),
    Template(usize),
    Fragment(ElementId),
}

#[derive(Debug, Clone)]
pub(crate) struct ValidatedInstance<'a> {
    pub(crate) offset: u32,
    pub(crate) template: Rc<IrTree<'a>>,
    pub(crate) has_literal_array: bool,
    pub(crate) has_arrays: bool,
    pub(crate) values: Vec<ValidatedValue<'a>>,
}

/// Nested templates are stored deepest-first; child IDs remain valid when a
/// record is retained, cloned, or rendered after another record.
#[derive(Debug, Clone)]
pub(crate) struct TemplateContent<'a> {
    pub(crate) root: ValidatedInstance<'a>,
    pub(crate) nested: Vec<ValidatedInstance<'a>>,
    pub(crate) frags: std::mem::ManuallyDrop<IrArena<'a>>,
}

impl<'a> TemplateContent<'a> {
    pub(crate) fn instance(&self, id: usize) -> &ValidatedInstance<'a> {
        if id == usize::MAX {
            &self.root
        } else {
            &self.nested[id]
        }
    }

    pub(crate) fn materialize(&self, bump: &'a Bump) -> Result<IrTree<'a>> {
        // Preserve fragment element IDs. No fragment bytes are parsed again.
        let mut arena = (*self.frags).clone();
        let root = materialize_instance(&self.root, &self.nested, bump, &mut arena)?;
        Ok(IrTree::new(arena, root))
    }
}

/// Instantiate a validated template instance into `arena` and return its root.
///
/// Nested instances are materialized first so every placeholder resolves to a scalar or an
/// element; the template tree is then deep-cloned with all placeholders resolved and array
/// substitutions expanded (see `resolve_node_into`). The returned `ElementId` is *spliced* into
/// the record tree at the template instance location.
fn materialize_instance<'a>(
    instance: &ValidatedInstance<'a>,
    nested: &[ValidatedInstance<'a>],
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
) -> Result<ElementId> {
    let has_templates = instance
        .values
        .iter()
        .any(|v| matches!(v, ValidatedValue::Template(_)));
    let resolved;
    let values: &[ValidatedValue<'a>] = if has_templates {
        let mut copy = IrVec::with_capacity_in(instance.values.len(), bump);
        for value in &instance.values {
            copy.push(match value {
                ValidatedValue::Template(id) => ValidatedValue::Fragment(materialize_instance(
                    &nested[*id],
                    nested,
                    bump,
                    arena,
                )?),
                other => other.clone(),
            });
        }
        resolved = copy;
        &resolved
    } else {
        // Nothing to swap in: resolve against the validated values directly.
        &instance.values
    };
    arena.reserve(instance.template.arena().count());
    clone_and_resolve(
        instance.template.arena(),
        instance.template.root(),
        values,
        bump,
        arena,
    )
    .map(|(root, _)| root)
}

pub(crate) fn build_record_content<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
) -> Result<RecordContent<'a>> {
    if let Some(template_ref) = read_single_instance_stream(bytes, chunk)? {
        let mut frags = IrArena::new_in(&chunk.arena);
        let mut nested = Vec::new();
        let root =
            validate_tpl_instance(template_ref, chunk, cache, &mut frags, Some(&mut nested), 0)?;
        return Ok(RecordContent::Template(TemplateContent {
            root,
            nested,
            frags: std::mem::ManuallyDrop::new(frags),
        }));
    }
    build_tree_from_binxml_bytes_direct(bytes, chunk, cache).map(RecordContent::Tree)
}

/// Validate a template instance's substitution values. With `nested`, values that are
/// themselves single-instance streams are validated recursively and pushed there (as
/// `ValidatedValue::Template`); without it, every non-empty BinXML value is built in place.
fn validate_tpl_instance<'a>(
    template_ref: BinXmlTemplateValues<'a>,
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    frags: &mut IrArena<'a>,
    mut nested: Option<&mut Vec<ValidatedInstance<'a>>>,
    depth: usize,
) -> Result<ValidatedInstance<'a>> {
    let (template, has_literal_array) =
        cache.get_or_parse_template_direct(chunk, template_ref.template_def_offset, depth)?;
    let mut has_arrays = false;
    let values = template_ref
        .values
        .into_iter()
        .map(|scalar| {
            has_arrays |= scalar.expandable_array_len().is_some();
            let BinXmlValue::BinXmlType(bytes) = scalar else {
                return Ok(ValidatedValue::Scalar(scalar));
            };
            if bytes.is_empty() {
                return Ok(ValidatedValue::Scalar(BinXmlValue::NullType));
            }
            if let Some(nested) = nested.as_deref_mut()
                && depth < MAX_BINXML_NESTING
                && let Some(inner) = read_single_instance_stream(bytes, chunk)?
            {
                let child =
                    validate_tpl_instance(inner, chunk, cache, frags, Some(nested), depth + 1)?;
                nested.push(child);
                return Ok(ValidatedValue::Template(nested.len() - 1));
            }
            let src = Stream::chunk(chunk, &chunk.arena);
            build_in(src, bytes, cache, frags, BuildMode::Record, depth + 1)
                .map(ValidatedValue::Fragment)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(ValidatedInstance {
        offset: template_ref.template_def_offset,
        template,
        has_literal_array,
        has_arrays,
        values,
    })
}

/// Parse `bytes` as a `[0x0f fragment header] 0x0c TemplateInstance (EOF | end)`
/// stream. Returns `Ok(None)` for any other shape.
fn read_single_instance_stream<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
) -> Result<Option<BinXmlTemplateValues<'a>>> {
    let Some(instance_offset) = crate::binxml::tokens::single_instance_offset(bytes) else {
        return Ok(None);
    };

    let stream_offset = binxml_slice_offset_in(chunk.data, bytes)? as usize;
    let mut cursor = ByteCursor::with_pos(chunk.data, stream_offset + instance_offset)?;
    let template_ref =
        read_template_values_cursor(&mut cursor, chunk.settings.get_ansi_codec(), &chunk.arena)?;

    // Anything after the instance other than EOF is a shape we don't handle
    // here. An instance that read past the record is accepted.
    let consumed = cursor.position() as usize - stream_offset;
    if ends_stream(bytes, consumed) == Some(false) {
        return Ok(None);
    }
    Ok(Some(template_ref))
}

/// Build an IR tree from a WEVT_TEMPLATE BinXML fragment (inline-name encoding).
///
/// The input `binxml` should start at the BinXML fragment header (token 0x0f).
/// This parses in `TemplateDefinition` mode, producing a tree that may contain `Node::Placeholder`.
#[cfg(feature = "wevt_templates")]
pub(crate) fn build_wevt_template_definition_ir<'a>(
    binxml: &'a [u8],
    ansi_codec: crate::AnsiCodec,
    bump: &'a Bump,
) -> Result<IrTree<'a>> {
    let mut cache = IrTemplateCache::with_capacity(0, bump);
    build_template_in(
        Stream::wevt(binxml, ansi_codec, bump),
        binxml,
        &mut cache,
        0,
    )
}

/// Parse a template definition stream into a placeholder tree.
fn build_template_in<'a>(
    src: Stream<'a>,
    bytes: &'a [u8],
    cache: &mut IrTemplateCache<'a>,
    depth: usize,
) -> Result<IrTree<'a>> {
    let mut arena = IrArena::with_capacity_in(estimate_node_capacity(bytes.len() as u32), src.bump);
    let root = build_in(
        src,
        bytes,
        cache,
        &mut arena,
        BuildMode::TemplateDefinition,
        depth,
    )?;
    Ok(IrTree::new(arena, root))
}

/// Instantiate a template-definition IR tree by resolving all placeholders.
///
/// This returns a fully-resolved IR tree that is ready for XML/JSON rendering.
#[cfg(feature = "wevt_templates")]
pub(crate) fn instantiate_template_definition_ir<'a>(
    template: &IrTree<'a>,
    values: &[ValidatedValue<'a>],
    bump: &'a Bump,
) -> Result<IrTree<'a>> {
    let mut arena = IrArena::with_capacity_in(template.arena().count(), bump);
    let (root, _needs_array_expansion) =
        clone_and_resolve(template.arena(), template.root(), values, bump, &mut arena)?;
    Ok(IrTree::new(arena, root))
}

/// Whether the template definition tree contains a literal `Node::Value` array
/// that would trigger element repetition at instantiation time.
fn tree_has_literal_array(tree: &IrTree<'_>) -> bool {
    let arena = tree.arena();
    for id in 0..arena.count() {
        let Some(element) = arena.get(id) else {
            continue;
        };
        if element.children.iter().any(node_needs_array_expansion) {
            return true;
        }
        for attr in &element.attrs {
            if attr.value.iter().any(node_needs_array_expansion) {
                return true;
            }
        }
    }
    false
}

pub(crate) fn binxml_slice_offset_in(data: &[u8], bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() {
        return Err(EvtxError::FailedToCreateRecordModel("empty BinXML slice"));
    }
    let data_start = data.as_ptr() as usize;
    let slice_start = bytes.as_ptr() as usize;
    let slice_end = slice_start.saturating_add(bytes.len());
    let data_end = data_start.saturating_add(data.len());

    if slice_start < data_start || slice_end > data_end {
        return Err(EvtxError::FailedToCreateRecordModel(
            "BinXML slice is outside base data buffer",
        ));
    }

    Ok((slice_start - data_start) as u64)
}

fn estimate_node_capacity(data_size: u32) -> usize {
    let bytes = data_size as usize;
    let estimate = bytes / 12;
    estimate.max(16)
}

pub(crate) fn read_template_definition_header_at(
    data: &[u8],
    offset: u32,
) -> Result<TemplateHeader> {
    Ok(read_template_header(&mut ByteCursor::with_pos(
        data,
        offset as usize,
    )?)?)
}

/// The body (the bytes after the header) of the template definition at
/// `offset` in `data`.
pub(crate) fn template_body_at<'d>(
    data: &'d [u8],
    offset: u32,
    header: &TemplateHeader,
) -> Result<&'d [u8]> {
    let start = offset as usize + TEMPLATE_DEFINITION_HEADER_SIZE;
    let end = start.checked_add(header.data_size as usize).ok_or(
        EvtxError::FailedToCreateRecordModel("template data size overflow"),
    )?;
    data.get(start..end)
        .ok_or(EvtxError::FailedToCreateRecordModel(
            "template data out of bounds",
        ))
}

/// Deep-clone a template element into the record arena while resolving placeholders.
///
/// This is the core template-instantiation routine. It walks the template element's
/// attribute values and child nodes in order, and appends the resolved representation
/// into new bump-allocated vectors.
///
/// The key design choice is that resolution is done via `*_into(.., out: &mut IrVec<Node>)`
/// helpers. This enables **splicing**:
/// - optional placeholders can disappear (0 output nodes),
/// - array substitutions can repeat an element (1 input child element → N output elements).
///
/// `has_element_child` is recomputed based on what was actually appended after resolution.
///
/// Example (conceptual):
///
/// ```text
/// Template children: [ Text("hello "), Placeholder(0), Element(B), Placeholder(1 optional) ]
/// Values[0] = StringType("world")
/// Values[1] = NullType (optional)  => removed
///
/// Resolved children: [ Text("hello "), Text("world"), Element(B) ]
/// ```
fn clone_and_resolve<'a>(
    template_arena: &IrArena<'a>,
    element_id: ElementId,
    values: &[ValidatedValue<'a>],
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
) -> Result<(ElementId, bool)> {
    let element = template_arena
        .get(element_id)
        .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid template element id"))?;

    let mut resolved = Element {
        name: element.name,
        attrs: IrVec::with_capacity_in(element.attrs.len(), bump),
        children: IrVec::with_capacity_in(element.children.len(), bump),
        has_element_child: false,
    };

    // Fast-path hint for array expansion:
    // If we never append an expandable array value (`Node::Value(<ArrayType>)` with len > 1)
    // into this element's attrs/children, we can skip the array-expansion scan entirely.
    let mut needs_array_expansion = false;

    for attr in &element.attrs {
        let mut new_attr = Attr {
            name: attr.name,
            value: IrVec::with_capacity_in(attr.value.len(), bump),
        };
        for node in &attr.value {
            let before = new_attr.value.len();
            resolve_node_into(
                template_arena,
                node,
                values,
                bump,
                arena,
                &mut new_attr.value,
            )?;
            if !needs_array_expansion
                && new_attr.value[before..]
                    .iter()
                    .any(node_needs_array_expansion)
            {
                needs_array_expansion = true;
            }
        }
        if !new_attr.value.is_empty() {
            resolved.attrs.push(new_attr);
        }
    }

    for node in &element.children {
        let before = resolved.children.len();
        resolve_node_into(
            template_arena,
            node,
            values,
            bump,
            arena,
            &mut resolved.children,
        )?;
        if !resolved.has_element_child
            && resolved.children[before..]
                .iter()
                .any(|n| matches!(n, Node::Element(_)))
        {
            resolved.has_element_child = true;
        }
        if !needs_array_expansion
            && resolved.children[before..]
                .iter()
                .any(node_needs_array_expansion)
        {
            needs_array_expansion = true;
        }
    }

    Ok((arena.new_node(resolved), needs_array_expansion))
}

/// Resolve one template [`Node`] and append the result into `out`.
///
/// This function is intentionally "append-only" because some input nodes expand to multiple
/// output nodes:
/// - `Node::Placeholder(..)` → 0 or 1 nodes (optional empties disappear).
/// - `Node::Element(..)` → 1 or many nodes (array substitution expansion may repeat the element).
/// - Everything else → exactly 1 node (shallow clone).
fn resolve_node_into<'a>(
    template_arena: &IrArena<'a>,
    node: &Node<'a>,
    values: &[ValidatedValue<'a>],
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
    out: &mut IrVec<'a, Node<'a>>,
) -> Result<()> {
    match node {
        Node::Placeholder(ph) => resolve_placeholder_into(ph, values, out),
        Node::Element(element_id) => {
            let (cloned, needs_array_expansion) =
                clone_and_resolve(template_arena, *element_id, values, bump, arena)?;
            // MS-EVEN6 §3.1.4.7.5: array substitutions expand by repeating the containing element.
            // After placeholder resolution, array substitutions appear as `Node::Value(<ArrayType>)`
            // inside an element's content/attributes. Expand them here so renderers see the proper
            // repeated-element structure (matching common tools like libevtx).
            if needs_array_expansion
                && let Some(expanded) = expand_array_substitutions_in_element(arena, bump, cloned)?
            {
                for id in expanded {
                    out.push(Node::Element(id));
                }
                return Ok(());
            }
            out.push(Node::Element(cloned));
            Ok(())
        }
        other => {
            out.push(other.clone());
            Ok(())
        }
    }
}

/// Resolve a template `Placeholder` and append its concrete representation into `out`.
///
/// Placeholders are emitted only while parsing a template definition. During instantiation
/// (cloning), we look up `placeholder.id` in the instance value array and:
///
/// - **out of bounds**: treat as missing → emit nothing (fail-soft)
/// - **optional + empty**: emit nothing (this is the common "omitted" path)
/// - **embedded BinXML**: emit `Node::Element(id)` (subtree already parsed in record arena)
/// - **scalar value**: convert via `value_to_node` and emit `Text`/`Value`
///
/// Note that placeholder resolution itself does **not** perform array-substitution expansion.
/// If the resolved value is an array type, it will be emitted as `Node::Value(<ArrayType>)`,
/// and the *containing element* will later be expanded/repeated by
/// [`expand_array_substitutions_in_element`] when that element is cloned.
///
/// Example (conceptual):
///
/// ```text
/// Template: <Data>%{0}</Data>
/// Placeholder(0) resolves to Value(StringArrayType(["a","b"]))
/// => out += [ Value(StringArrayType(["a","b"])) ]
/// => Data element is then repeated into two <Data> nodes.
/// ```
fn resolve_placeholder_into<'a>(
    placeholder: &Placeholder,
    values: &[ValidatedValue<'a>],
    out: &mut IrVec<'a, Node<'a>>,
) -> Result<()> {
    let Some(value) = values.get(placeholder.id as usize) else {
        return Ok(());
    };
    match value {
        ValidatedValue::Fragment(element_id) => out.push(Node::Element(*element_id)),
        ValidatedValue::Scalar(value) if placeholder.optional && is_optional_empty(value) => {}
        ValidatedValue::Scalar(BinXmlValue::EvtXml) => {
            return Err(EvtxError::FailedToCreateRecordModel(
                "Unimplemented - EvtXml",
            ));
        }
        // Nested templates are materialized into fragments before cloning.
        ValidatedValue::Scalar(BinXmlValue::BinXmlType(_)) | ValidatedValue::Template(_) => {
            return Err(EvtxError::FailedToCreateRecordModel(
                "unsupported BinXML value in template substitution",
            ));
        }
        ValidatedValue::Scalar(other) => out.push(value_to_node(other.clone())?),
    }
    Ok(())
}

/// Append `node` into the current open element's `children`.
///
/// This is the common "emit" primitive for streaming token consumption.
/// Callers are expected to enforce context rules (e.g. elements are not allowed in attributes).
fn push_child<'a>(arena: &mut IrArena<'a>, stack: &[ElementId], node: Node<'a>) -> Result<()> {
    let parent_id = stack
        .last()
        .copied()
        .ok_or_else(|| EvtxError::FailedToCreateRecordModel("value outside of element"))?;
    let parent = arena
        .get_mut(parent_id)
        .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid parent element id"))?;
    parent.push_child(node);
    Ok(())
}

fn value_to_node<'a>(value: BinXmlValue<'a>) -> Result<Node<'a>> {
    match value {
        BinXmlValue::StringType(s) => Ok(Node::Text(Text::utf16(s))),
        BinXmlValue::AnsiStringType(s) => Ok(Node::Text(Text::utf8(s))),
        BinXmlValue::EvtXml | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtHandle => Err(
            EvtxError::FailedToCreateRecordModel("unsupported BinXML value in tree"),
        ),
        other => Ok(Node::Value(other)),
    }
}

fn expand_string_ref<'a>(
    offset: ChunkOffset,
    chunk: &'a EvtxChunk<'a>,
    bump: &'a Bump,
) -> Result<&'a str> {
    match chunk.string_cache.get_cached_string(offset) {
        Some(s) => Ok(s.as_str()),
        None => {
            // Fail-soft fallback for corrupted chunks / missing string cache entries:
            // decode the name into the caller-provided bump arena.
            let name_off =
                offset
                    .checked_add(NAME_LINK_SIZE)
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "string table offset overflow",
                    ))?;
            let mut cursor = ByteCursor::with_pos(chunk.data, name_off as usize)?;
            let s = cursor
                .len_prefixed_utf16_string_bump(true, "name", bump)?
                .unwrap_or("");
            Ok(s)
        }
    }
}

fn expand_wevt_inline_name_ref<'a>(
    data: &'a [u8],
    offset: ChunkOffset,
    bump: &'a Bump,
) -> Result<&'a str> {
    let mut cursor = ByteCursor::with_pos(data, offset as usize)?;
    let _ = cursor.u16_named("wevt_inline_name_hash")?;
    let s = cursor
        .len_prefixed_utf16_string_bump(true, "wevt_inline_name", bump)?
        .unwrap_or("");
    Ok(s)
}

/// Benchmark-only helper to build an IR tree directly from BinXML bytes without an iterator.
#[cfg(feature = "bench")]
pub(crate) fn bench_build_tree_from_binxml_bytes_direct<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    bump: &'a Bump,
) -> Result<ElementId> {
    let mut arena = IrArena::new_in(bump);
    build_in(
        Stream::chunk(chunk, bump),
        bytes,
        cache,
        &mut arena,
        BuildMode::Record,
        0,
    )
}
