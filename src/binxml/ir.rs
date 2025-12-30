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
//! The JSON/XML renderers (`binxml::ir_json`, `binxml::ir_xml`) assume templates have already
//! been instantiated and all placeholders have been resolved/expanded.
//!
//! ## Parser implementation notes
//!
//! The production builder parses BinXML bytes directly (cursor-based) to avoid iterator
//! overhead and intermediate token allocations.

use crate::EvtxChunk;
use crate::binxml::array_expand::{
    expand_array_substitutions_in_element, node_needs_array_expansion,
};
use crate::binxml::name::{BinXmlNameEncoding, BinXmlNameRef};
use crate::binxml::tokens::{
    BinXMLAttribute, BinXMLOpenStartElement, BinXMLProcessingInstructionTarget,
    BinXmlEntityReference, BinXmlTemplateValues, TemplateSubstitutionDescriptor,
    read_attribute_cursor, read_entity_ref_cursor, read_fragment_header_cursor,
    read_open_start_element_cursor, read_processing_instruction_data_cursor,
    read_processing_instruction_target_cursor, read_substitution_descriptor_cursor,
    read_template_values_cursor,
};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{DeserializationError, EvtxError, Result};
use crate::model::ir::{
    Attr, Element, ElementId, IrArena, IrTree, IrVec, Name, Node, Placeholder, TemplateValue, Text,
    is_optional_empty_template_value,
};
use crate::utils::{ByteCursor, Utf16LeSlice};
use ahash::AHashMap;
use bumpalo::Bump;
use encoding::EncodingRef;
use std::rc::Rc;

/// Size (in bytes) of the "name link" header that precedes an inline string table entry.
///
/// This is used by [`expand_string_ref`] when falling back to decode a missing string-cache
/// entry directly from the chunk.
const BINXML_NAME_LINK_SIZE: u32 = 6;

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
const TEMPLATE_DEFINITION_HEADER_SIZE: usize = 24;

/// Cache of parsed BinXML templates keyed by template GUID.
///
/// Templates are stored as IR trees containing placeholders; instantiation
/// clones the tree and resolves all placeholders using substitution values.
#[derive(Debug)]
pub(crate) struct IrTemplateCache<'a> {
    templates: AHashMap<[u8; 16], Rc<IrTree<'a>>>,
    arena: &'a Bump,
}

impl<'a> IrTemplateCache<'a> {
    #[cfg(feature = "bench")]
    pub fn new(arena: &'a Bump) -> Self {
        IrTemplateCache::with_capacity(0, arena)
    }

    pub fn with_capacity(capacity: usize, arena: &'a Bump) -> Self {
        IrTemplateCache {
            templates: AHashMap::with_capacity(capacity),
            arena,
        }
    }

    /// Instantiate a template instance and return the root `ElementId` of the instantiated tree.
    ///
    /// This is where template expansion stops being abstract:
    /// - The cached template definition is an IR tree that may contain `Node::Placeholder`.
    /// - The provided substitution values are converted to [`TemplateValue`]s (including parsing
    ///   embedded `BinXmlType` fragments into `TemplateValue::BinXmlElement`).
    /// - The template tree is deep-cloned into the record arena and all placeholders are resolved.
    /// - Any array substitutions are expanded into repeated elements (see `resolve_node_into`).
    ///
    /// The returned `ElementId` is then *spliced* into the record's tree at the template
    /// instance token location.
    fn instantiate_template_direct_values(
        &mut self,
        template_ref: BinXmlTemplateValues<'a>,
        chunk: &'a EvtxChunk<'a>,
        arena: &mut IrArena<'a>,
        bump: &'a Bump,
    ) -> Result<ElementId> {
        let template =
            self.get_or_parse_template_direct(chunk, template_ref.template_def_offset)?;
        arena.reserve(template.arena().count());
        let values = template_values_from_values(template_ref.values, chunk, self, arena, bump)?;
        let (root, _needs_array_expansion) = clone_and_resolve(
            template.arena(),
            template.root(),
            values.as_slice(),
            self.arena,
            arena,
        )?;
        Ok(root)
    }

    /// Load a template definition from the chunk (or return a cached copy).
    ///
    /// The returned [`IrTree`] is stored in the cache and contains `Node::Placeholder` nodes.
    /// It is never rendered directly; instead it is used as the source for
    /// [`instantiate_template_direct_values`].
    fn get_or_parse_template_direct(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        template_def_offset: u32,
    ) -> Result<Rc<IrTree<'a>>> {
        let header = read_template_definition_header_at(chunk.data, template_def_offset)?;
        if let Some(existing) = self.templates.get(&header.guid) {
            return Ok(Rc::clone(existing));
        }

        let parse_from_chunk = (|| -> Result<Rc<IrTree<'a>>> {
            let data_start = template_def_offset as usize + TEMPLATE_DEFINITION_HEADER_SIZE;
            let data_end = data_start.checked_add(header.data_size as usize).ok_or(
                EvtxError::FailedToCreateRecordModel("template data size overflow"),
            )?;
            if data_end > chunk.data.len() {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "template data out of bounds",
                ));
            }

            let mut arena =
                IrArena::with_capacity_in(estimate_node_capacity(header.data_size), self.arena);
            let bytes = &chunk.data[data_start..data_end];
            let bump = self.arena;
            let ansi_codec = chunk.settings.get_ansi_codec();
            let root = build_tree_from_binxml_bytes_direct_with_mode(
                BuildTreeFromBinXmlBytesDirectArgs {
                    bytes,
                    data: chunk.data,
                    chunk: Some(chunk),
                    cache: self,
                    ansi_codec,
                    bump,
                    arena: &mut arena,
                    mode: BuildMode::TemplateDefinition,
                    has_dep_id: true,
                    name_encoding: BinXmlNameEncoding::Offset,
                },
            )?;
            let template = Rc::new(IrTree::new(arena, root));
            self.templates.insert(header.guid, Rc::clone(&template));
            Ok(template)
        })();

        match parse_from_chunk {
            Ok(t) => Ok(t),
            Err(parse_err) => {
                // If the embedded chunk template is corrupt/missing, optionally fall back to an
                // offline WEVT cache (provider resources) when configured.
                #[cfg(feature = "wevt_templates")]
                {
                    if let Some(cache) = chunk.settings.get_wevt_cache() {
                        let guid = match winstructs::guid::Guid::from_buffer(&header.guid) {
                            Ok(g) => g,
                            Err(_) => return Err(parse_err),
                        };

                        let binxml = match cache
                            .load_temp_binxml_fragment_in(&guid.to_string(), self.arena)
                        {
                            Ok(b) => b,
                            Err(_) => return Err(parse_err),
                        };

                        let mut arena = IrArena::with_capacity_in(
                            estimate_node_capacity(binxml.len() as u32),
                            self.arena,
                        );

                        let bump = self.arena;
                        let ansi_codec = chunk.settings.get_ansi_codec();
                        let root = match build_tree_from_binxml_bytes_direct_with_mode(
                            BuildTreeFromBinXmlBytesDirectArgs {
                                bytes: binxml,
                                data: binxml,
                                chunk: None,
                                cache: self,
                                ansi_codec,
                                bump,
                                arena: &mut arena,
                                mode: BuildMode::TemplateDefinition,
                                has_dep_id: true,
                                name_encoding: BinXmlNameEncoding::WevtInline,
                            },
                        ) {
                            Ok(r) => r,
                            Err(_) => return Err(parse_err),
                        };

                        let template = Rc::new(IrTree::new(arena, root));
                        self.templates.insert(header.guid, Rc::clone(&template));
                        return Ok(template);
                    }
                }

                Err(parse_err)
            }
        }
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
    /// Optional EVTX chunk context (needed for Offset-name resolution and some value parsing).
    chunk: Option<&'a EvtxChunk<'a>>,
    /// Base buffer that BinXML offsets are relative to for this parse.
    ///
    /// - For normal record parsing this is `chunk.data`.
    /// - For WEVT_TEMPLATE parsing this is the BinXML fragment slice (starts at offset 0).
    data: &'a [u8],
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    bump: &'a Bump,
    arena: &'arena mut IrArena<'a>,
    ansi_codec: EncodingRef,
    name_encoding: BinXmlNameEncoding,
    stack: Vec<ElementId>,
    current_element: Option<ElementBuilder<'a>>,
    root: Option<ElementId>,
}

struct TreeBuilderInit<'a, 'cache, 'arena> {
    chunk: Option<&'a EvtxChunk<'a>>,
    data: &'a [u8],
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    ansi_codec: EncodingRef,
    name_encoding: BinXmlNameEncoding,
    bump: &'a Bump,
    arena: &'arena mut IrArena<'a>,
}

impl<'a, 'cache, 'arena> TreeBuilder<'a, 'cache, 'arena> {
    fn new(init: TreeBuilderInit<'a, 'cache, 'arena>) -> Self {
        TreeBuilder {
            chunk: init.chunk,
            data: init.data,
            cache: init.cache,
            mode: init.mode,
            bump: init.bump,
            arena: init.arena,
            ansi_codec: init.ansi_codec,
            name_encoding: init.name_encoding,
            stack: Vec::new(),
            current_element: None,
            root: None,
        }
    }

    fn process_open_start_element(&mut self, elem: BinXMLOpenStartElement) -> Result<()> {
        if self.current_element.is_some() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "open start - Bad parser state",
            ));
        }
        let name = Name::new(self.expand_name_ref(&elem.name)?);
        self.current_element = Some(ElementBuilder::new(name, self.bump));
        Ok(())
    }

    fn process_attribute(&mut self, attr: BinXMLAttribute) -> Result<()> {
        // Compute name before borrowing `current_element` mutably (avoid borrow conflict).
        let name = Name::new(self.expand_name_ref(&attr.name)?);
        let builder = self
            .current_element
            .as_mut()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("attribute - Bad parser state"))?;
        builder.start_attribute(name);
        Ok(())
    }

    fn process_entity_ref(&mut self, entity: BinXmlEntityReference) -> Result<()> {
        let name = Name::new(self.expand_name_ref(&entity.name)?);
        self.push_node(Node::EntityRef(name))
    }

    fn process_pi_target(&mut self, name: BinXMLProcessingInstructionTarget) -> Result<()> {
        let target = Name::new(self.expand_name_ref(&name.name)?);
        self.push_node(Node::PITarget(target))
    }

    fn process_pi_data(&mut self, data: Utf16LeSlice<'a>) -> Result<()> {
        let node = Node::PIData(Text::utf16(data));
        self.push_node(node)
    }

    fn process_close_start_element(&mut self) -> Result<()> {
        let element = self
            .current_element
            .take()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("close start - Bad parser state"))?
            .finish();
        let element_id = self.arena.new_node(element);
        self.stack.push(element_id);
        Ok(())
    }

    fn process_close_empty_element(&mut self) -> Result<()> {
        let element = self
            .current_element
            .take()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("close empty - Bad parser state"))?
            .finish();
        let element_id = self.arena.new_node(element);
        attach_element(
            self.arena,
            &self.stack,
            &mut self.root,
            element_id,
            self.mode,
        )
    }

    fn process_close_element(&mut self) -> Result<()> {
        let element_id = self.stack.pop().ok_or_else(|| {
            EvtxError::FailedToCreateRecordModel("close element - Bad parser state")
        })?;
        attach_element(
            self.arena,
            &self.stack,
            &mut self.root,
            element_id,
            self.mode,
        )
    }

    fn process_substitution(&mut self, substitution: TemplateSubstitutionDescriptor) -> Result<()> {
        if self.mode != BuildMode::TemplateDefinition {
            return Err(EvtxError::FailedToCreateRecordModel(
                "substitution outside template definition",
            ));
        }

        // In template definitions, substitutions are *structural*: we can't resolve them yet,
        // so we store a placeholder node that records:
        // - the substitution index into the instance value array
        // - the declared value type
        // - whether the substitution is optional (may be omitted if empty)
        let placeholder = Node::Placeholder(Placeholder {
            id: substitution.substitution_index,
            value_type: substitution.value_type,
            optional: substitution.optional,
        });
        self.push_node(placeholder)
    }

    fn process_value(&mut self, value: BinXmlValue<'a>) -> Result<()> {
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
                let element_id = build_tree_from_binxml_bytes_direct_with_mode(
                    BuildTreeFromBinXmlBytesDirectArgs {
                        bytes,
                        data: self.data,
                        chunk: self.chunk,
                        cache: &mut *self.cache,
                        ansi_codec: self.ansi_codec,
                        bump: self.bump,
                        arena: &mut *self.arena,
                        mode: BuildMode::Record,
                        has_dep_id: false,
                        name_encoding: self.name_encoding,
                    },
                )?;
                attach_element(
                    self.arena,
                    &self.stack,
                    &mut self.root,
                    element_id,
                    self.mode,
                )
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

    fn process_template_instance_values(
        &mut self,
        template: BinXmlTemplateValues<'a>,
        bump: &'a Bump,
    ) -> Result<()> {
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
        let chunk = self.chunk.ok_or_else(|| {
            EvtxError::FailedToCreateRecordModel("template instance requires an EVTX chunk context")
        })?;
        let element_id = self
            .cache
            .instantiate_template_direct_values(template, chunk, self.arena, bump)?;
        attach_element(
            self.arena,
            &self.stack,
            &mut self.root,
            element_id,
            self.mode,
        )
    }

    fn expand_name_ref(&self, name_ref: &BinXmlNameRef) -> Result<&'a str> {
        match self.name_encoding {
            BinXmlNameEncoding::Offset => {
                let chunk = self.chunk.ok_or_else(|| {
                    EvtxError::FailedToCreateRecordModel(
                        "Offset name encoding requires an EVTX chunk context",
                    )
                })?;
                expand_string_ref(name_ref, chunk, self.bump)
            }
            BinXmlNameEncoding::WevtInline => {
                expand_wevt_inline_name_ref(self.data, name_ref, self.bump)
            }
        }
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
                let element = Element::new_in(name, self.bump);
                let root = self.arena.new_node(element);
                Ok(root)
            }
            BuildMode::TemplateDefinition => {
                Err(EvtxError::FailedToCreateRecordModel("missing root element"))
            }
        }
    }
}

/// Parse a BinXML byte slice into an IR element tree using the cursor-based fast path.
///
/// This is the "token loop" for the production parser. It:
/// - computes the slice's offset relative to the chunk (`binxml_slice_offset`)
/// - iterates token bytes using [`ByteCursor`]
/// - delegates stateful construction to [`TreeBuilder`]
///
/// Splicing behavior shows up here via token types:
/// - `0x0c` (TemplateInstance): instantiate the template and attach it immediately
///   (`TreeBuilder::process_template_instance_values`).
/// - `Value(BinXmlType(..))`: parsed inside `TreeBuilder::process_value`, which builds a
///   nested IR subtree and attaches it as child elements.
///
/// `mode` controls whether substitutions become placeholders (template definitions) or are
/// rejected (records). `has_dep_id` controls parsing of element start tokens for streams that
/// include dependency IDs.
struct BuildTreeFromBinXmlBytesDirectArgs<'a, 'cache, 'arena> {
    bytes: &'a [u8],
    data: &'a [u8],
    chunk: Option<&'a EvtxChunk<'a>>,
    cache: &'cache mut IrTemplateCache<'a>,
    ansi_codec: EncodingRef,
    bump: &'a Bump,
    arena: &'arena mut IrArena<'a>,
    mode: BuildMode,
    has_dep_id: bool,
    name_encoding: BinXmlNameEncoding,
}

fn build_tree_from_binxml_bytes_direct_with_mode<'a, 'cache, 'arena>(
    args: BuildTreeFromBinXmlBytesDirectArgs<'a, 'cache, 'arena>,
) -> Result<ElementId> {
    let BuildTreeFromBinXmlBytesDirectArgs {
        bytes,
        data,
        chunk,
        cache,
        ansi_codec,
        bump,
        arena,
        mode,
        has_dep_id,
        name_encoding,
    } = args;
    let offset = binxml_slice_offset_in(data, bytes)?;
    let mut cursor = ByteCursor::with_pos(data, offset as usize)?;
    let mut data_read: u32 = 0;
    let data_size = bytes.len() as u32;
    let mut eof = false;

    let mut builder = TreeBuilder::new(TreeBuilderInit {
        chunk,
        data,
        cache,
        mode,
        ansi_codec,
        name_encoding,
        bump,
        arena,
    });

    while !eof && data_read < data_size {
        let start = cursor.position();
        let token_byte = cursor.u8()?;

        match token_byte {
            0x00 => {
                eof = true;
            }
            0x0c => {
                let template = read_template_values_cursor(&mut cursor, chunk, ansi_codec, bump)?;
                builder.process_template_instance_values(template, bump)?;
            }
            0x01 => {
                let elem =
                    read_open_start_element_cursor(&mut cursor, false, has_dep_id, name_encoding)?;
                builder.process_open_start_element(elem)?;
            }
            0x41 => {
                let elem =
                    read_open_start_element_cursor(&mut cursor, true, has_dep_id, name_encoding)?;
                builder.process_open_start_element(elem)?;
            }
            0x02 => {
                builder.process_close_start_element()?;
            }
            0x03 => {
                builder.process_close_empty_element()?;
            }
            0x04 => {
                builder.process_close_element()?;
            }
            0x05 | 0x45 => {
                let value =
                    BinXmlValue::from_binxml_cursor_in(&mut cursor, chunk, None, ansi_codec, bump)?;
                builder.process_value(value)?;
            }
            0x06 | 0x46 => {
                let attr = read_attribute_cursor(&mut cursor, name_encoding)?;
                builder.process_attribute(attr)?;
            }
            0x09 | 0x49 => {
                let entity = read_entity_ref_cursor(&mut cursor, name_encoding)?;
                builder.process_entity_ref(entity)?;
            }
            0x0a => {
                let target = read_processing_instruction_target_cursor(&mut cursor, name_encoding)?;
                builder.process_pi_target(target)?;
            }
            0x0b => {
                let data = read_processing_instruction_data_cursor(&mut cursor)?;
                builder.process_pi_data(data)?;
            }
            0x0d => {
                let substitution = read_substitution_descriptor_cursor(&mut cursor, false)?;
                builder.process_substitution(substitution)?;
            }
            0x0e => {
                let substitution = read_substitution_descriptor_cursor(&mut cursor, true)?;
                builder.process_substitution(substitution)?;
            }
            0x0f => {
                let _ = read_fragment_header_cursor(&mut cursor)?;
            }
            0x07 | 0x47 => {
                return Err(DeserializationError::UnimplementedToken {
                    name: "CDataSection",
                    offset: cursor.position(),
                }
                .into());
            }
            0x08 | 0x48 => {
                return Err(DeserializationError::UnimplementedToken {
                    name: "CharReference",
                    offset: cursor.position(),
                }
                .into());
            }
            _ => {
                return Err(DeserializationError::InvalidToken {
                    value: token_byte,
                    offset: cursor.position(),
                }
                .into());
            }
        }

        let total_read = cursor.position() - start;
        data_read = data_read.saturating_add(total_read as u32);
    }

    builder.finish()
}

#[cfg(feature = "bench")]
fn build_tree_from_binxml_bytes_direct_root<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
) -> Result<ElementId> {
    build_tree_from_binxml_bytes_direct_with_mode(BuildTreeFromBinXmlBytesDirectArgs {
        bytes,
        data: chunk.data,
        chunk: Some(chunk),
        cache,
        ansi_codec: chunk.settings.get_ansi_codec(),
        bump,
        arena,
        mode: BuildMode::Record,
        has_dep_id: false,
        name_encoding: BinXmlNameEncoding::Offset,
    })
}

/// Build an IR tree directly from BinXML bytes without an iterator.
pub(crate) fn build_tree_from_binxml_bytes_direct<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
) -> Result<IrTree<'a>> {
    let mut arena =
        IrArena::with_capacity_in(estimate_node_capacity(bytes.len() as u32), &chunk.arena);
    let root = build_tree_from_binxml_bytes_direct_with_mode(BuildTreeFromBinXmlBytesDirectArgs {
        bytes,
        data: chunk.data,
        chunk: Some(chunk),
        cache,
        ansi_codec: chunk.settings.get_ansi_codec(),
        bump: &chunk.arena,
        arena: &mut arena,
        mode: BuildMode::Record,
        has_dep_id: false,
        name_encoding: BinXmlNameEncoding::Offset,
    })?;
    Ok(IrTree::new(arena, root))
}

/// Build an IR tree from a WEVT_TEMPLATE BinXML fragment (inline-name encoding).
///
/// The input `binxml` should start at the BinXML fragment header (token 0x0f).
/// This parses in `TemplateDefinition` mode, producing a tree that may contain `Node::Placeholder`.
#[cfg(feature = "wevt_templates")]
pub(crate) fn build_wevt_template_definition_ir<'a>(
    binxml: &'a [u8],
    ansi_codec: EncodingRef,
    bump: &'a Bump,
) -> Result<IrTree<'a>> {
    let mut cache = IrTemplateCache::with_capacity(0, bump);
    let mut arena = IrArena::with_capacity_in(estimate_node_capacity(binxml.len() as u32), bump);
    let root = build_tree_from_binxml_bytes_direct_with_mode(BuildTreeFromBinXmlBytesDirectArgs {
        bytes: binxml,
        data: binxml,
        chunk: None,
        cache: &mut cache,
        ansi_codec,
        bump,
        arena: &mut arena,
        mode: BuildMode::TemplateDefinition,
        has_dep_id: true,
        name_encoding: BinXmlNameEncoding::WevtInline,
    })?;
    Ok(IrTree::new(arena, root))
}

/// Instantiate a template-definition IR tree by resolving all placeholders.
///
/// This returns a fully-resolved IR tree that is ready for XML/JSON rendering.
#[cfg(feature = "wevt_templates")]
pub(crate) fn instantiate_template_definition_ir<'a>(
    template: &IrTree<'a>,
    values: &[TemplateValue<'a>],
    bump: &'a Bump,
) -> Result<IrTree<'a>> {
    let mut arena = IrArena::with_capacity_in(template.arena().count(), bump);
    let (root, _needs_array_expansion) =
        clone_and_resolve(template.arena(), template.root(), values, bump, &mut arena)?;
    Ok(IrTree::new(arena, root))
}

fn binxml_slice_offset_in(data: &[u8], bytes: &[u8]) -> Result<u64> {
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

/// Minimal template header used for cache lookups.
#[derive(Debug)]
struct TemplateHeader {
    guid: [u8; 16],
    data_size: u32,
}

fn estimate_node_capacity(data_size: u32) -> usize {
    let bytes = data_size as usize;
    let estimate = bytes / 12;
    estimate.max(16)
}

fn read_template_definition_header_at(data: &[u8], offset: u32) -> Result<TemplateHeader> {
    let mut cursor = ByteCursor::with_pos(data, offset as usize)?;
    let _next_template_offset = cursor.u32_named("next_template_offset")?;
    let guid_bytes = cursor.take_bytes(16, "template_guid")?;
    let data_size = cursor.u32_named("template_data_size")?;

    let guid = <[u8; 16]>::try_from(guid_bytes)
        .map_err(|_| EvtxError::FailedToCreateRecordModel("template guid size mismatch"))?;

    Ok(TemplateHeader { guid, data_size })
}

/// Convert raw template-instance substitution values into [`TemplateValue`]s.
///
/// Most values are kept as-is (`TemplateValue::Value(..)`), but `BinXmlType` is special:
/// in a substitution array, `BinXmlType` means "this substitution is itself a BinXML fragment".
/// We parse that fragment into a normal IR subtree and store its root as
/// `TemplateValue::BinXmlElement(id)`.
///
/// This is one of the main "splicing" optimizations in this module: placeholder resolution can
/// later insert an embedded fragment by simply pushing `Node::Element(id)` into the parent.
///
/// Example:
///
/// ```text
/// values_raw = [ StringType("x"), BinXmlType(<C>y</C>) ]
/// values     = [ Value(StringType("x")), BinXmlElement(id_of_C) ]
/// ```
fn template_values_from_values<'a>(
    values_raw: Vec<BinXmlValue<'a>>,
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    arena: &mut IrArena<'a>,
    bump: &'a Bump,
) -> Result<IrVec<'a, TemplateValue<'a>>> {
    let mut values = IrVec::with_capacity_in(values_raw.len(), cache.arena);
    for value in values_raw {
        match value {
            BinXmlValue::BinXmlType(bytes) => {
                if bytes.is_empty() {
                    values.push(TemplateValue::Value(BinXmlValue::NullType));
                } else {
                    let element_id = build_tree_from_binxml_bytes_direct_with_mode(
                        BuildTreeFromBinXmlBytesDirectArgs {
                            bytes,
                            data: chunk.data,
                            chunk: Some(chunk),
                            cache,
                            ansi_codec: chunk.settings.get_ansi_codec(),
                            bump,
                            arena,
                            mode: BuildMode::Record,
                            has_dep_id: false,
                            name_encoding: BinXmlNameEncoding::Offset,
                        },
                    )?;
                    values.push(TemplateValue::BinXmlElement(element_id));
                }
            }
            other => values.push(TemplateValue::Value(other)),
        }
    }
    Ok(values)
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
    values: &[TemplateValue<'a>],
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
    values: &[TemplateValue<'a>],
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
    out: &mut IrVec<'a, Node<'a>>,
) -> Result<()> {
    match node {
        Node::Placeholder(ph) => resolve_placeholder_into(ph, values, bump, arena, out),
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
        Node::Text(text) => {
            out.push(Node::Text(text.clone()));
            Ok(())
        }
        Node::Value(value) => {
            out.push(Node::Value(value.clone()));
            Ok(())
        }
        Node::EntityRef(name) => {
            out.push(Node::EntityRef(*name));
            Ok(())
        }
        Node::CharRef(ch) => {
            out.push(Node::CharRef(*ch));
            Ok(())
        }
        Node::CData(text) => {
            out.push(Node::CData(text.clone()));
            Ok(())
        }
        Node::PITarget(name) => {
            out.push(Node::PITarget(*name));
            Ok(())
        }
        Node::PIData(text) => {
            out.push(Node::PIData(text.clone()));
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
    values: &[TemplateValue<'a>],
    _bump: &'a Bump,
    _arena: &mut IrArena<'a>,
    out: &mut IrVec<'a, Node<'a>>,
) -> Result<()> {
    let index = placeholder.id as usize;
    if index >= values.len() {
        return Ok(());
    }

    let value = &values[index];
    if placeholder.optional && is_optional_empty_template_value(value) {
        return Ok(());
    }

    match value {
        TemplateValue::BinXmlElement(element_id) => {
            out.push(Node::Element(*element_id));
            Ok(())
        }
        TemplateValue::Value(value) => match value {
            BinXmlValue::EvtXml => Err(EvtxError::FailedToCreateRecordModel(
                "Unimplemented - EvtXml",
            )),
            BinXmlValue::BinXmlType(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unsupported BinXML value in template substitution",
            )),
            other => {
                let node = value_to_node(other.clone())?;
                out.push(node);
                Ok(())
            }
        },
    }
}

/// Attach an element to the current parent (or set it as root).
///
/// This is used by both "normal" element closure and by splicing operations:
/// - template instance expansion
/// - nested `BinXmlType` fragment expansion
///
/// Behavior:
/// - if there is an open parent on `stack`, append as a child element
/// - else, if `root` is unset, set it
/// - else, if `mode == Record`, ignore additional roots (fail-soft for corrupted records)
/// - else, error (template definitions must have exactly one root)
fn attach_element<'a>(
    arena: &mut IrArena<'a>,
    stack: &[ElementId],
    root: &mut Option<ElementId>,
    element_id: ElementId,
    mode: BuildMode,
) -> Result<()> {
    if let Some(parent_id) = stack.last().copied() {
        let parent = arena
            .get_mut(parent_id)
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid parent element id"))?;
        parent.push_child(Node::Element(element_id));
        Ok(())
    } else if root.is_none() {
        *root = Some(element_id);
        Ok(())
    } else {
        match mode {
            BuildMode::Record => {
                // Corrupted records can contain multiple top-level fragments. Keep iteration
                // fail-soft by ignoring additional root elements (the first root wins).
                Ok(())
            }
            BuildMode::TemplateDefinition => Err(EvtxError::FailedToCreateRecordModel(
                "multiple root elements",
            )),
        }
    }
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
    string_ref: &BinXmlNameRef,
    chunk: &'a EvtxChunk<'a>,
    bump: &'a Bump,
) -> Result<&'a str> {
    match chunk.string_cache.get_cached_string(string_ref.offset) {
        Some(s) => Ok(s.as_str()),
        None => {
            // Fail-soft fallback for corrupted chunks / missing string cache entries:
            // decode the name into the caller-provided bump arena.
            let name_off = string_ref.offset.checked_add(BINXML_NAME_LINK_SIZE).ok_or(
                EvtxError::FailedToCreateRecordModel("string table offset overflow"),
            )?;
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
    string_ref: &BinXmlNameRef,
    bump: &'a Bump,
) -> Result<&'a str> {
    let mut cursor = ByteCursor::with_pos(data, string_ref.offset as usize)?;
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
    build_tree_from_binxml_bytes_direct_root(bytes, chunk, cache, bump, &mut arena)
}
