//! BinXML IR construction and template instantiation.
//!
//! This module provides:
//! - A streaming builder that converts BinXML tokens into the IR tree.
//! - A per-iterator template cache that stores parsed templates with placeholders.
//! - Template instantiation that resolves placeholders into concrete nodes.
//!
//! The production builder parses BinXML bytes directly (cursor-based) to avoid
//! iterator overhead and intermediate token allocations. For benchmarks we keep
//! a token-iterator path that consumes `BinXmlDeserializer` output.
//!
//! Template definitions are parsed once and reused across records in a chunk via
//! `IrTemplateCache`.
//!
//! The JSON and XML streaming renderers live in `binxml::ir_json` and
//! `binxml::ir_xml`.

use crate::EvtxChunk;
use crate::binxml::name::{BinXmlNameEncoding, BinXmlNameRef};
use crate::binxml::tokens::{
    read_attribute_cursor, read_entity_ref_cursor, read_fragment_header_cursor,
    read_open_start_element_cursor, read_processing_instruction_data_cursor,
    read_processing_instruction_target_cursor, read_substitution_descriptor_cursor,
    read_template_values_cursor,
};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{DeserializationError, EvtxError, Result};
#[cfg(feature = "bench")]
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::model::deserialized::{
    BinXMLAttribute, BinXMLOpenStartElement, BinXMLProcessingInstructionTarget,
    BinXmlEntityReference, BinXmlTemplateValues, TemplateSubstitutionDescriptor,
};
use crate::model::ir::{
    Attr, Element, ElementId, IrArena, IrTree, IrVec, Name, Node, Placeholder, TemplateValue, Text,
    is_optional_empty_template_value,
};
use crate::utils::{ByteCursor, Utf16LeSlice};
use ahash::AHashMap;
use bumpalo::Bump;
use std::rc::Rc;

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
        clone_and_resolve(
            template.arena(),
            template.root(),
            values.as_slice(),
            self.arena,
            arena,
        )
    }

    fn get_or_parse_template_direct(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        template_def_offset: u32,
    ) -> Result<Rc<IrTree<'a>>> {
        let header = read_template_definition_header_at(chunk.data, template_def_offset)?;
        if let Some(existing) = self.templates.get(&header.guid) {
            return Ok(Rc::clone(existing));
        }

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
        let root = build_tree_from_binxml_bytes_direct_with_mode(
            bytes,
            chunk,
            self,
            self.arena,
            &mut arena,
            BuildMode::TemplateDefinition,
            true,
        )?;
        let template = Rc::new(IrTree::new(arena, root));
        self.templates.insert(header.guid, Rc::clone(&template));
        Ok(template)
    }
}

/// Parsing mode for the streaming builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildMode {
    Record,
    TemplateDefinition,
}

/// Streaming token consumer that builds the IR tree.
///
/// In `TemplateDefinition` mode, substitutions are captured as placeholders.
/// In `Record` mode, template instances are instantiated and spliced in-place.
struct TreeBuilder<'a, 'cache, 'arena> {
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    bump: &'a Bump,
    arena: &'arena mut IrArena<'a>,
    stack: Vec<ElementId>,
    current_element: Option<ElementBuilder<'a>>,
    root: Option<ElementId>,
}

impl<'a, 'cache, 'arena> TreeBuilder<'a, 'cache, 'arena> {
    fn new(
        chunk: &'a EvtxChunk<'a>,
        cache: &'cache mut IrTemplateCache<'a>,
        mode: BuildMode,
        bump: &'a Bump,
        arena: &'arena mut IrArena<'a>,
    ) -> Self {
        TreeBuilder {
            chunk,
            cache,
            mode,
            bump,
            arena,
            stack: Vec::new(),
            current_element: None,
            root: None,
        }
    }

    #[cfg(feature = "bench")]
    fn process_token(&mut self, token: BinXMLDeserializedTokens<'a>) -> Result<()> {
        match token {
            BinXMLDeserializedTokens::FragmentHeader(_)
            | BinXMLDeserializedTokens::AttributeList
            | BinXMLDeserializedTokens::StartOfStream
            | BinXMLDeserializedTokens::EndOfStream => Ok(()),
            BinXMLDeserializedTokens::TemplateInstance(template) => {
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
                let mut values = Vec::with_capacity(template.substitution_array.len());
                for token in template.substitution_array {
                    match token {
                        BinXMLDeserializedTokens::Value(v) => values.push(v),
                        _ => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "template substitution value was not a value token",
                            ));
                        }
                    }
                }

                let element_id = self.cache.instantiate_template_direct_values(
                    BinXmlTemplateValues {
                        template_id: template.template_id,
                        template_def_offset: template.template_def_offset,
                        template_guid: template.template_guid,
                        values,
                    },
                    self.chunk,
                    self.arena,
                    self.bump,
                )?;
                attach_element(
                    self.arena,
                    &self.stack,
                    &mut self.root,
                    element_id,
                    self.mode,
                )
            }
            BinXMLDeserializedTokens::Substitution(substitution) => {
                self.process_substitution(substitution)
            }
            BinXMLDeserializedTokens::OpenStartElement(elem) => {
                self.process_open_start_element(elem)
            }
            BinXMLDeserializedTokens::Attribute(attr) => self.process_attribute(attr),
            BinXMLDeserializedTokens::Value(value) => self.process_value(value),
            BinXMLDeserializedTokens::EntityRef(entity) => self.process_entity_ref(entity),
            BinXMLDeserializedTokens::PITarget(name) => self.process_pi_target(name),
            BinXMLDeserializedTokens::PIData(data) => self.process_pi_data(data),
            BinXMLDeserializedTokens::CloseStartElement => self.process_close_start_element(),
            BinXMLDeserializedTokens::CloseEmptyElement => self.process_close_empty_element(),
            BinXMLDeserializedTokens::CloseElement => self.process_close_element(),
            BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => Err(
                EvtxError::FailedToCreateRecordModel("Unimplemented - CDATA/CharRef"),
            ),
        }
    }

    fn process_open_start_element(&mut self, elem: BinXMLOpenStartElement) -> Result<()> {
        if self.current_element.is_some() {
            return Err(EvtxError::FailedToCreateRecordModel(
                "open start - Bad parser state",
            ));
        }
        let name = Name::new(expand_string_ref(&elem.name, self.chunk, self.bump)?);
        self.current_element = Some(ElementBuilder::new(name, self.bump));
        Ok(())
    }

    fn process_attribute(&mut self, attr: BinXMLAttribute) -> Result<()> {
        let builder = self
            .current_element
            .as_mut()
            .ok_or_else(|| EvtxError::FailedToCreateRecordModel("attribute - Bad parser state"))?;
        let name = Name::new(expand_string_ref(&attr.name, self.chunk, self.bump)?);
        builder.start_attribute(name);
        Ok(())
    }

    fn process_entity_ref(&mut self, entity: BinXmlEntityReference) -> Result<()> {
        let name = Name::new(expand_string_ref(&entity.name, self.chunk, self.bump)?);
        self.push_node(Node::EntityRef(name))
    }

    fn process_pi_target(&mut self, name: BinXMLProcessingInstructionTarget) -> Result<()> {
        let target = Name::new(expand_string_ref(&name.name, self.chunk, self.bump)?);
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
                if bytes.is_empty() {
                    return Ok(());
                }
                let element_id = build_tree_from_binxml_bytes_direct_with_mode(
                    bytes,
                    self.chunk,
                    self.cache,
                    self.bump,
                    self.arena,
                    BuildMode::Record,
                    false,
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
        let element_id = self
            .cache
            .instantiate_template_direct_values(template, self.chunk, self.arena, bump)?;
        attach_element(
            self.arena,
            &self.stack,
            &mut self.root,
            element_id,
            self.mode,
        )
    }

    fn push_node(&mut self, node: Node<'a>) -> Result<()> {
        if let Some(builder) = self.current_element.as_mut() {
            if matches!(node, Node::Element(_)) {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "element inside attribute value",
                ));
            }
            builder.push_attr_value(node);
            Ok(())
        } else {
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

fn build_tree_from_binxml_bytes_direct_with_mode<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
    mode: BuildMode,
    has_dep_id: bool,
) -> Result<ElementId> {
    let offset = binxml_slice_offset(chunk, bytes)?;
    let mut cursor = ByteCursor::with_pos(chunk.data, offset as usize)?;
    let mut data_read: u32 = 0;
    let data_size = bytes.len() as u32;
    let mut eof = false;

    let ansi_codec = chunk.settings.get_ansi_codec();
    let name_encoding = BinXmlNameEncoding::Offset;

    let mut builder = TreeBuilder::new(chunk, cache, mode, bump, arena);

    while !eof && data_read < data_size {
        let start = cursor.position();
        let token_byte = cursor.u8()?;

        match token_byte {
            0x00 => {
                eof = true;
            }
            0x0c => {
                let template =
                    read_template_values_cursor(&mut cursor, Some(chunk), ansi_codec, bump)?;
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
                let value = BinXmlValue::from_binxml_cursor_in(
                    &mut cursor,
                    Some(chunk),
                    None,
                    ansi_codec,
                    bump,
                )?;
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
    build_tree_from_binxml_bytes_direct_with_mode(
        bytes,
        chunk,
        cache,
        bump,
        arena,
        BuildMode::Record,
        false,
    )
}

/// Build an IR tree directly from BinXML bytes without an iterator.
pub(crate) fn build_tree_from_binxml_bytes_direct<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
) -> Result<IrTree<'a>> {
    let mut arena =
        IrArena::with_capacity_in(estimate_node_capacity(bytes.len() as u32), &chunk.arena);
    let root = build_tree_from_binxml_bytes_direct_with_mode(
        bytes,
        chunk,
        cache,
        &chunk.arena,
        &mut arena,
        BuildMode::Record,
        false,
    )?;
    Ok(IrTree::new(arena, root))
}

fn binxml_slice_offset(chunk: &EvtxChunk<'_>, bytes: &[u8]) -> Result<u64> {
    if bytes.is_empty() {
        return Err(EvtxError::FailedToCreateRecordModel("empty BinXML slice"));
    }
    let chunk_start = chunk.data.as_ptr() as usize;
    let slice_start = bytes.as_ptr() as usize;
    let slice_end = slice_start.saturating_add(bytes.len());
    let chunk_end = chunk_start.saturating_add(chunk.data.len());

    if slice_start < chunk_start || slice_end > chunk_end {
        return Err(EvtxError::FailedToCreateRecordModel(
            "BinXML slice is outside chunk data",
        ));
    }

    Ok((slice_start - chunk_start) as u64)
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
                        bytes,
                        chunk,
                        cache,
                        bump,
                        arena,
                        BuildMode::Record,
                        false,
                    )?;
                    values.push(TemplateValue::BinXmlElement(element_id));
                }
            }
            other => values.push(TemplateValue::Value(other)),
        }
    }
    Ok(values)
}

fn clone_and_resolve<'a>(
    template_arena: &IrArena<'a>,
    element_id: ElementId,
    values: &[TemplateValue<'a>],
    bump: &'a Bump,
    arena: &mut IrArena<'a>,
) -> Result<ElementId> {
    let element = template_arena
        .get(element_id)
        .ok_or_else(|| EvtxError::FailedToCreateRecordModel("invalid template element id"))?;

    let mut resolved = Element {
        name: element.name.clone(),
        attrs: IrVec::with_capacity_in(element.attrs.len(), bump),
        children: IrVec::with_capacity_in(element.children.len(), bump),
        has_element_child: false,
    };

    for attr in &element.attrs {
        let mut new_attr = Attr {
            name: attr.name.clone(),
            value: IrVec::with_capacity_in(attr.value.len(), bump),
        };
        for node in &attr.value {
            resolve_node_into(
                template_arena,
                node,
                values,
                bump,
                arena,
                &mut new_attr.value,
            )?;
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
    }

    Ok(arena.new_node(resolved))
}

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
            let cloned = clone_and_resolve(template_arena, *element_id, values, bump, arena)?;
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
            out.push(Node::EntityRef(name.clone()));
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
            out.push(Node::PITarget(name.clone()));
            Ok(())
        }
        Node::PIData(text) => {
            out.push(Node::PIData(text.clone()));
            Ok(())
        }
    }
}

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

/// Benchmark-only helper to build an IR tree from BinXML bytes using a caller-provided bump.
#[cfg(feature = "bench")]
pub(crate) fn bench_build_tree_from_binxml_bytes<'a>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &mut IrTemplateCache<'a>,
    bump: &'a Bump,
) -> Result<ElementId> {
    let offset = binxml_slice_offset(chunk, bytes)?;
    let deserializer = crate::binxml::deserializer::BinXmlDeserializer::init(
        chunk.data,
        offset,
        Some(chunk),
        false,
        chunk.settings.get_ansi_codec(),
    );
    let iter = deserializer.iter_tokens_in(Some(bytes.len() as u32), bump)?;
    let mut arena = IrArena::new_in(bump);
    let mut builder = TreeBuilder::new(chunk, cache, BuildMode::Record, bump, &mut arena);
    for token in iter {
        let token = token.map_err(EvtxError::from)?;
        builder.process_token(token)?;
    }
    builder.finish()
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
