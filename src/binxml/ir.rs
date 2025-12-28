//! BinXML IR construction and JSON rendering.
//!
//! This module provides:
//! - A streaming builder that converts BinXML tokens into the IR tree.
//! - A per-iterator template cache that stores parsed templates with placeholders.
//! - Template instantiation that resolves placeholders into concrete nodes.
//! - A JSON renderer that streams output directly from the IR tree.
//!
//! The builder consumes `BinXmlDeserializer` output directly to avoid building
//! a flat token vector for each record. Template definitions are parsed once
//! and reused across records in a chunk via `IrTemplateCache`.

use crate::binxml::name::{BinXmlName, BinXmlNameRef};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{DeserializationError, EvtxError, Result};
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::ir::{Attr, Element, ElementId, IrTree, Name, Node, Placeholder, Text};
use crate::utils::{ByteCursor, write_utf16le_json_escaped};
use crate::{EvtxChunk, ParserSettings};
use indextree::Arena;
use jiff::{Timestamp, tz::Offset};
use sonic_rs::format::{CompactFormatter, Formatter};
use sonic_rs::writer::WriteExt;
use std::borrow::Cow;
use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;
use zmij::Buffer as ZmijBuffer;

const BINXML_NAME_LINK_SIZE: u32 = 6;

/// Incremental element builder for streaming token parsing.
///
/// Attributes are collected until the element start is closed, then materialized
/// into an `Element`.
struct ElementBuilder<'a> {
    name: Name<'a>,
    attrs: Vec<Attr<'a>>,
    current_attr_name: Option<Name<'a>>,
    current_attr_value: Vec<Node<'a>>,
}

impl<'a> ElementBuilder<'a> {
    fn new(name: Name<'a>) -> Self {
        ElementBuilder {
            name,
            attrs: Vec::new(),
            current_attr_name: None,
            current_attr_value: Vec::new(),
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
                let value = std::mem::take(&mut self.current_attr_value);
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
            children: Vec::new(),
            has_element_child: false,
        }
    }
}

#[allow(dead_code)]
pub(crate) fn build_tree<'a>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
) -> Result<IrTree<'a>> {
    let mut cache = IrTemplateCache::new();
    build_tree_from_tokens(tokens, chunk, &mut cache, BuildMode::Record)
}

const TEMPLATE_DEFINITION_HEADER_SIZE: usize = 24;

/// Cache of parsed BinXML templates keyed by template GUID.
///
/// Templates are stored as IR trees containing placeholders; instantiation
/// clones the tree and resolves all placeholders using substitution values.
#[derive(Debug)]
pub(crate) struct IrTemplateCache<'a> {
    templates: HashMap<[u8; 16], Rc<Template<'a>>>,
}

impl<'a> IrTemplateCache<'a> {
    pub fn new() -> Self {
        IrTemplateCache::with_capacity(0)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        IrTemplateCache {
            templates: HashMap::with_capacity(capacity),
        }
    }

    fn get_or_parse_template(
        &mut self,
        chunk: &'a EvtxChunk<'a>,
        template_def_offset: u32,
    ) -> Result<Rc<Template<'a>>> {
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

        let deserializer = crate::binxml::deserializer::BinXmlDeserializer::init(
            chunk.data,
            data_start as u64,
            Some(chunk),
            true,
            chunk.settings.get_ansi_codec(),
        );

        let iter = deserializer.iter_tokens(Some(header.data_size))?;
        let mut arena = Arena::with_capacity(estimate_node_capacity(header.data_size));
        let root = build_tree_from_iter_with_mode_into(
            iter,
            chunk,
            self,
            BuildMode::TemplateDefinition,
            &mut arena,
        )?;
        let template = Rc::new(Template {
            tree: IrTree::new(arena, root),
        });
        self.templates.insert(header.guid, Rc::clone(&template));
        Ok(template)
    }

    fn instantiate_template(
        &mut self,
        template_ref: BinXmlTemplateRef<'a>,
        chunk: &'a EvtxChunk<'a>,
        arena: &mut Arena<Element<'a>>,
    ) -> Result<ElementId> {
        let template = self.get_or_parse_template(chunk, template_ref.template_def_offset)?;
        arena.reserve(template.tree.arena().count());
        let values = template_values_from_ref(template_ref)?;
        template.instantiate(&values, chunk, self, arena)
    }
}

/// Parsed template definition with placeholder nodes.
#[derive(Debug)]
struct Template<'a> {
    tree: IrTree<'a>,
}

impl<'a> Template<'a> {
    fn instantiate(
        &self,
        values: &[BinXmlValue<'a>],
        chunk: &'a EvtxChunk<'a>,
        cache: &mut IrTemplateCache<'a>,
        arena: &mut Arena<Element<'a>>,
    ) -> Result<ElementId> {
        clone_and_resolve(
            self.tree.arena(),
            self.tree.root(),
            values,
            chunk,
            cache,
            arena,
        )
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
    arena: &'arena mut Arena<Element<'a>>,
    stack: Vec<ElementId>,
    current_element: Option<ElementBuilder<'a>>,
    root: Option<ElementId>,
}

impl<'a, 'cache, 'arena> TreeBuilder<'a, 'cache, 'arena> {
    fn new(
        chunk: &'a EvtxChunk<'a>,
        cache: &'cache mut IrTemplateCache<'a>,
        mode: BuildMode,
        arena: &'arena mut Arena<Element<'a>>,
    ) -> Self {
        TreeBuilder {
            chunk,
            cache,
            mode,
            arena,
            stack: Vec::new(),
            current_element: None,
            root: None,
        }
    }

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
                let element_id = self
                    .cache
                    .instantiate_template(template, self.chunk, self.arena)?;
                attach_element(self.arena, &mut self.stack, &mut self.root, element_id)
            }
            BinXMLDeserializedTokens::Substitution(substitution) => {
                self.process_substitution(substitution)
            }
            BinXMLDeserializedTokens::OpenStartElement(elem) => {
                if self.current_element.is_some() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "open start - Bad parser state",
                    ));
                }
                let name = Name::new(expand_string_ref(&elem.name, self.chunk)?);
                self.current_element = Some(ElementBuilder::new(name));
                Ok(())
            }
            BinXMLDeserializedTokens::Attribute(attr) => {
                let builder =
                    self.current_element
                        .as_mut()
                        .ok_or(EvtxError::FailedToCreateRecordModel(
                            "attribute - Bad parser state",
                        ))?;
                let name = Name::new(expand_string_ref(&attr.name, self.chunk)?);
                builder.start_attribute(name);
                Ok(())
            }
            BinXMLDeserializedTokens::Value(value) => self.process_value(value),
            BinXMLDeserializedTokens::EntityRef(entity) => {
                let name = Name::new(expand_string_ref(&entity.name, self.chunk)?);
                self.push_node(Node::EntityRef(name))
            }
            BinXMLDeserializedTokens::PITarget(name) => {
                let target = Name::new(expand_string_ref(&name.name, self.chunk)?);
                self.push_node(Node::PITarget(target))
            }
            BinXMLDeserializedTokens::PIData(data) => {
                let node = Node::PIData(Text::new(Cow::Owned(data)));
                self.push_node(node)
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                let element = self
                    .current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close start - Bad parser state",
                    ))?
                    .finish();
                let element_id = self.arena.new_node(element);
                self.stack.push(element_id);
                Ok(())
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                let element = self
                    .current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close empty - Bad parser state",
                    ))?
                    .finish();
                let element_id = self.arena.new_node(element);
                attach_element(self.arena, &mut self.stack, &mut self.root, element_id)
            }
            BinXMLDeserializedTokens::CloseElement => {
                let element_id = self
                    .stack
                    .pop()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close element - Bad parser state",
                    ))?;
                attach_element(self.arena, &mut self.stack, &mut self.root, element_id)
            }
            BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => Err(
                EvtxError::FailedToCreateRecordModel("Unimplemented - CDATA/CharRef"),
            ),
        }
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
                let element_id =
                    build_tree_from_binxml_bytes(bytes, self.chunk, self.cache, self.arena)?;
                attach_element(self.arena, &mut self.stack, &mut self.root, element_id)
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
            push_child(self.arena, &mut self.stack, node)
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

        self.root
            .ok_or(EvtxError::FailedToCreateRecordModel("missing root element"))
    }
}

pub(crate) fn build_tree_from_iter<'a, 'cache, I>(
    iter: I,
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
) -> Result<IrTree<'a>>
where
    I: IntoIterator<Item = std::result::Result<BinXMLDeserializedTokens<'a>, DeserializationError>>,
{
    build_tree_from_iter_with_mode(iter, chunk, cache, BuildMode::Record)
}

fn build_tree_from_iter_with_mode<'a, 'cache, I>(
    iter: I,
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
) -> Result<IrTree<'a>>
where
    I: IntoIterator<Item = std::result::Result<BinXMLDeserializedTokens<'a>, DeserializationError>>,
{
    let mut arena = Arena::new();
    let root = build_tree_from_iter_with_mode_into(iter, chunk, cache, mode, &mut arena)?;
    Ok(IrTree::new(arena, root))
}

fn build_tree_from_iter_with_mode_into<'a, 'cache, I>(
    iter: I,
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    arena: &mut Arena<Element<'a>>,
) -> Result<ElementId>
where
    I: IntoIterator<Item = std::result::Result<BinXMLDeserializedTokens<'a>, DeserializationError>>,
{
    let mut builder = TreeBuilder::new(chunk, cache, mode, arena);
    for token in iter {
        let token = token.map_err(EvtxError::from)?;
        builder.process_token(token)?;
    }
    builder.finish()
}

fn build_tree_from_tokens<'a, 'cache, I>(
    tokens: I,
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
) -> Result<IrTree<'a>>
where
    I: IntoIterator<Item = BinXMLDeserializedTokens<'a>>,
{
    let mut arena = Arena::new();
    let root = build_tree_from_tokens_into(tokens, chunk, cache, mode, &mut arena)?;
    Ok(IrTree::new(arena, root))
}

fn build_tree_from_tokens_into<'a, 'cache, I>(
    tokens: I,
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    arena: &mut Arena<Element<'a>>,
) -> Result<ElementId>
where
    I: IntoIterator<Item = BinXMLDeserializedTokens<'a>>,
{
    let mut builder = TreeBuilder::new(chunk, cache, mode, arena);
    for token in tokens {
        builder.process_token(token)?;
    }
    builder.finish()
}

fn build_tree_from_binxml_bytes<'a, 'cache>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    arena: &mut Arena<Element<'a>>,
) -> Result<ElementId> {
    let offset = binxml_slice_offset(chunk, bytes)?;
    let deserializer = crate::binxml::deserializer::BinXmlDeserializer::init(
        chunk.data,
        offset,
        Some(chunk),
        false,
        chunk.settings.get_ansi_codec(),
    );
    let iter = deserializer.iter_tokens(Some(bytes.len() as u32))?;
    build_tree_from_iter_with_mode_into(iter, chunk, cache, BuildMode::Record, arena)
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

fn template_values_from_ref<'a>(template: BinXmlTemplateRef<'a>) -> Result<Vec<BinXmlValue<'a>>> {
    let mut values = Vec::with_capacity(template.substitution_array.len());
    for token in template.substitution_array {
        match token {
            BinXMLDeserializedTokens::Value(value) => values.push(value),
            _ => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "template substitution value was not a value token",
                ));
            }
        }
    }
    Ok(values)
}

fn clone_and_resolve<'a, 'cache>(
    template_arena: &Arena<Element<'a>>,
    element_id: ElementId,
    values: &[BinXmlValue<'a>],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    arena: &mut Arena<Element<'a>>,
) -> Result<ElementId> {
    let element = template_arena
        .get(element_id)
        .ok_or(EvtxError::FailedToCreateRecordModel(
            "invalid template element id",
        ))?
        .get();

    let mut resolved = Element {
        name: element.name.clone(),
        attrs: Vec::with_capacity(element.attrs.len()),
        children: Vec::with_capacity(element.children.len()),
        has_element_child: false,
    };

    for attr in &element.attrs {
        let mut new_attr = Attr {
            name: attr.name.clone(),
            value: Vec::with_capacity(attr.value.len()),
        };
        for node in &attr.value {
            resolve_node_into(
                template_arena,
                node,
                values,
                chunk,
                cache,
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
            chunk,
            cache,
            arena,
            &mut resolved.children,
        )?;
        if !resolved.has_element_child {
            if resolved.children[before..]
                .iter()
                .any(|n| matches!(n, Node::Element(_)))
            {
                resolved.has_element_child = true;
            }
        }
    }

    Ok(arena.new_node(resolved))
}

fn resolve_node_into<'a, 'cache>(
    template_arena: &Arena<Element<'a>>,
    node: &Node<'a>,
    values: &[BinXmlValue<'a>],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    arena: &mut Arena<Element<'a>>,
    out: &mut Vec<Node<'a>>,
) -> Result<()> {
    match node {
        Node::Placeholder(ph) => resolve_placeholder_into(ph, values, chunk, cache, arena, out),
        Node::Element(element_id) => {
            let cloned =
                clone_and_resolve(template_arena, *element_id, values, chunk, cache, arena)?;
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

fn resolve_placeholder_into<'a, 'cache>(
    placeholder: &Placeholder,
    values: &[BinXmlValue<'a>],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    arena: &mut Arena<Element<'a>>,
    out: &mut Vec<Node<'a>>,
) -> Result<()> {
    let index = placeholder.id as usize;
    if index >= values.len() {
        return Ok(());
    }

    let value = &values[index];
    if placeholder.optional && is_optional_empty(value) {
        return Ok(());
    }

    match value {
        BinXmlValue::BinXmlType(bytes) => {
            if bytes.is_empty() {
                return Ok(());
            }
            let element_id = build_tree_from_binxml_bytes(bytes, chunk, cache, arena)?;
            out.push(Node::Element(element_id));
            Ok(())
        }
        BinXmlValue::EvtXml => Err(EvtxError::FailedToCreateRecordModel(
            "Unimplemented - EvtXml",
        )),
        other => {
            let node = value_to_node(other.clone())?;
            out.push(node);
            Ok(())
        }
    }
}

fn is_optional_empty(value: &BinXmlValue<'_>) -> bool {
    match value {
        BinXmlValue::NullType => true,
        BinXmlValue::StringType(s) => s.is_empty(),
        BinXmlValue::AnsiStringType(s) => s.is_empty(),
        BinXmlValue::BinaryType(bytes) => bytes.is_empty(),
        BinXmlValue::BinXmlType(bytes) => bytes.is_empty(),
        BinXmlValue::StringArrayType(v) => v.is_empty(),
        BinXmlValue::Int8ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt8ArrayType(v) => v.is_empty(),
        BinXmlValue::Int16ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt16ArrayType(v) => v.is_empty(),
        BinXmlValue::Int32ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt32ArrayType(v) => v.is_empty(),
        BinXmlValue::Int64ArrayType(v) => v.is_empty(),
        BinXmlValue::UInt64ArrayType(v) => v.is_empty(),
        BinXmlValue::Real32ArrayType(v) => v.is_empty(),
        BinXmlValue::Real64ArrayType(v) => v.is_empty(),
        BinXmlValue::BoolArrayType(v) => v.is_empty(),
        BinXmlValue::GuidArrayType(v) => v.is_empty(),
        BinXmlValue::FileTimeArrayType(v) => v.is_empty(),
        BinXmlValue::SysTimeArrayType(v) => v.is_empty(),
        BinXmlValue::SidArrayType(v) => v.is_empty(),
        BinXmlValue::HexInt32ArrayType(v) => v.is_empty(),
        BinXmlValue::HexInt64ArrayType(v) => v.is_empty(),
        _ => false,
    }
}

fn attach_element<'a>(
    arena: &mut Arena<Element<'a>>,
    stack: &mut Vec<ElementId>,
    root: &mut Option<ElementId>,
    element_id: ElementId,
) -> Result<()> {
    if let Some(parent_id) = stack.last().copied() {
        let parent = arena
            .get_mut(parent_id)
            .ok_or(EvtxError::FailedToCreateRecordModel(
                "invalid parent element id",
            ))?;
        parent.get_mut().push_child(Node::Element(element_id));
        Ok(())
    } else if root.is_none() {
        *root = Some(element_id);
        Ok(())
    } else {
        Err(EvtxError::FailedToCreateRecordModel(
            "multiple root elements",
        ))
    }
}

fn push_child<'a>(
    arena: &mut Arena<Element<'a>>,
    stack: &mut Vec<ElementId>,
    node: Node<'a>,
) -> Result<()> {
    let parent_id = stack
        .last()
        .copied()
        .ok_or(EvtxError::FailedToCreateRecordModel(
            "value outside of element",
        ))?;
    let parent = arena
        .get_mut(parent_id)
        .ok_or(EvtxError::FailedToCreateRecordModel(
            "invalid parent element id",
        ))?;
    parent.get_mut().push_child(node);
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
) -> Result<Cow<'a, BinXmlName>> {
    match chunk.string_cache.get_cached_string(string_ref.offset) {
        Some(s) => Ok(Cow::Borrowed(s)),
        None => {
            let name_off = string_ref.offset.checked_add(BINXML_NAME_LINK_SIZE).ok_or(
                EvtxError::FailedToCreateRecordModel("string table offset overflow"),
            )?;
            let mut cursor = ByteCursor::with_pos(chunk.data, name_off as usize)?;
            let string = BinXmlName::from_cursor(&mut cursor)?;
            Ok(Cow::Owned(string))
        }
    }
}

pub(crate) fn render_json_record<W: WriteExt>(
    tree: &IrTree<'_>,
    _settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(writer, tree.arena());
    let root = tree.root_element();
    emitter.write_bytes(b"{\"")?;
    emitter.write_name(root.name.as_str())?;
    emitter.write_bytes(b"\":")?;
    emitter.write_element_value(root, false)?;
    emitter.write_bytes(b"}")?;
    Ok(())
}

const MAX_UNIQUE_NAMES: usize = 64;

/// Key for comparing element names without allocating.
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

/// Entry for counting unique child element names.
struct NameCount<'a> {
    key: NameKey<'a>,
    count: u16,
    emitted: bool,
}

/// Streaming JSON renderer for IR trees.
struct JsonEmitter<'w, 'a, W: WriteExt> {
    writer: &'w mut W,
    arena: &'a Arena<Element<'a>>,
    float_buf: ZmijBuffer,
    formatter: CompactFormatter,
}

impl<'w, 'a, W: WriteExt> JsonEmitter<'w, 'a, W> {
    fn new(writer: &'w mut W, arena: &'a Arena<Element<'a>>) -> Self {
        JsonEmitter {
            writer,
            arena,
            float_buf: ZmijBuffer::new(),
            formatter: CompactFormatter,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        Ok(())
    }

    fn write_byte(&mut self, byte: u8) -> Result<()> {
        self.writer.write_all(&[byte])?;
        Ok(())
    }

    fn write_name(&mut self, name: &str) -> Result<()> {
        self.write_bytes(name.as_bytes())
    }

    fn write_json_key_from_name(&mut self, name: &Name<'_>) -> Result<()> {
        self.write_byte(b'"')?;
        self.write_name(name.as_str())?;
        self.write_bytes(b"\":")
    }

    fn write_json_key_from_nodes(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'"')?;
        self.write_json_text_content(nodes)?;
        self.write_bytes(b"\":")
    }

    fn write_json_escaped(&mut self, value: &str) -> Result<()> {
        self.formatter
            .write_string_fast(&mut self.writer, value, false)
            .map_err(EvtxError::from)
    }

    fn write_json_text_content(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        for node in nodes {
            match node {
                Node::Text(text) | Node::CData(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    match text {
                        Text::Utf8(value) => {
                            self.write_json_escaped(value.as_ref())?;
                        }
                        Text::Utf16(value) => {
                            write_utf16le_json_escaped(&mut self.writer, *value)
                                .map_err(EvtxError::from)?;
                        }
                    }
                }
                Node::Value(value) => {
                    self.write_value_text(value)?;
                }
                Node::CharRef(ch) => {
                    self.write_bytes(b"&#")?;
                    self.formatter
                        .write_u64(&mut self.writer, u64::from(*ch))
                        .map_err(EvtxError::from)?;
                    self.write_bytes(b";")?;
                }
                Node::EntityRef(name) => {
                    self.write_bytes(b"&")?;
                    self.write_name(name.as_str())?;
                    self.write_bytes(b";")?;
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

    fn write_value_text(&mut self, value: &BinXmlValue<'_>) -> Result<()> {
        match value {
            BinXmlValue::NullType => Ok(()),
            BinXmlValue::StringType(s) => {
                write_utf16le_json_escaped(&mut self.writer, *s).map_err(EvtxError::from)
            }
            BinXmlValue::AnsiStringType(s) => self.write_json_escaped(s.as_ref()),
            BinXmlValue::Int8Type(v) => self
                .formatter
                .write_i8(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt8Type(v) => self
                .formatter
                .write_u8(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Int16Type(v) => self
                .formatter
                .write_i16(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt16Type(v) => self
                .formatter
                .write_u16(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Int32Type(v) => self
                .formatter
                .write_i32(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt32Type(v) => self
                .formatter
                .write_u32(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Int64Type(v) => self
                .formatter
                .write_i64(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt64Type(v) => self
                .formatter
                .write_u64(&mut self.writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Real32Type(v) => self.write_float(*v),
            BinXmlValue::Real64Type(v) => self.write_float(*v),
            BinXmlValue::BoolType(v) => self.write_bytes(if *v { b"true" } else { b"false" }),
            BinXmlValue::BinaryType(bytes) => self.write_hex_bytes(bytes),
            BinXmlValue::GuidType(guid) => write!(self.writer, "{}", guid).map_err(EvtxError::from),
            BinXmlValue::SizeTType(v) => self
                .formatter
                .write_u64(&mut self.writer, *v as u64)
                .map_err(EvtxError::from),
            BinXmlValue::FileTimeType(tm) => self.write_datetime(tm),
            BinXmlValue::SysTimeType(tm) => self.write_datetime(tm),
            BinXmlValue::SidType(sid) => write!(self.writer, "{}", sid).map_err(EvtxError::from),
            BinXmlValue::HexInt32Type(s) => self.write_json_escaped(s.as_ref()),
            BinXmlValue::HexInt64Type(s) => self.write_json_escaped(s.as_ref()),
            BinXmlValue::StringArrayType(items) => {
                let mut first = true;
                for item in items {
                    if !first {
                        self.write_byte(b',')?;
                    }
                    first = false;
                    write_utf16le_json_escaped(&mut self.writer, *item).map_err(EvtxError::from)?;
                }
                Ok(())
            }
            BinXmlValue::Int8ArrayType(items) => self.write_delimited(items),
            BinXmlValue::UInt8ArrayType(items) => self.write_delimited(items),
            BinXmlValue::Int16ArrayType(items) => self.write_delimited(items),
            BinXmlValue::UInt16ArrayType(items) => self.write_delimited(items),
            BinXmlValue::Int32ArrayType(items) => self.write_delimited(items),
            BinXmlValue::UInt32ArrayType(items) => self.write_delimited(items),
            BinXmlValue::Int64ArrayType(items) => self.write_delimited(items),
            BinXmlValue::UInt64ArrayType(items) => self.write_delimited(items),
            BinXmlValue::Real32ArrayType(items) => self.write_float_list(items),
            BinXmlValue::Real64ArrayType(items) => self.write_float_list(items),
            BinXmlValue::BoolArrayType(items) => self.write_delimited(items),
            BinXmlValue::GuidArrayType(items) => self.write_delimited(items),
            BinXmlValue::FileTimeArrayType(items) => self.write_datetime_list(items),
            BinXmlValue::SysTimeArrayType(items) => self.write_datetime_list(items),
            BinXmlValue::SidArrayType(items) => self.write_delimited(items),
            BinXmlValue::HexInt32ArrayType(items) => {
                let mut first = true;
                for item in items {
                    if !first {
                        self.write_byte(b',')?;
                    }
                    first = false;
                    self.write_json_escaped(item.as_ref())?;
                }
                Ok(())
            }
            BinXmlValue::HexInt64ArrayType(items) => {
                let mut first = true;
                for item in items {
                    if !first {
                        self.write_byte(b',')?;
                    }
                    first = false;
                    self.write_json_escaped(item.as_ref())?;
                }
                Ok(())
            }
            BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => Err(
                EvtxError::FailedToCreateRecordModel("unsupported BinXML value in JSON"),
            ),
            _ => Err(EvtxError::Unimplemented {
                name: format!("JSON formatting for {:?}", value),
            }),
        }
    }

    fn write_hex_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        for &b in bytes {
            let hi = (b >> 4) & 0x0f;
            let lo = b & 0x0f;
            self.write_byte(to_hex_digit(hi))?;
            self.write_byte(to_hex_digit(lo))?;
        }
        Ok(())
    }

    fn write_float<F: zmij::Float>(&mut self, value: F) -> Result<()> {
        let (buf, writer) = (&mut self.float_buf, &mut self.writer);
        let s = buf.format(value);
        writer.write_all(s.as_bytes())?;
        Ok(())
    }

    fn write_float_list<F: zmij::Float>(&mut self, items: &[F]) -> Result<()> {
        let (buf, writer) = (&mut self.float_buf, &mut self.writer);
        let mut first = true;
        for item in items {
            if !first {
                writer.write_all(b",")?;
            }
            first = false;
            let s = buf.format(*item);
            writer.write_all(s.as_bytes())?;
        }
        Ok(())
    }

    fn write_datetime(&mut self, tm: &Timestamp) -> Result<()> {
        let dt = Offset::UTC.to_datetime(*tm);
        let year = dt.year() as i32;

        self.write_4_digits(year as u32)?;
        self.write_byte(b'-')?;
        self.write_2_digits(u32::from(dt.month() as u8))?;
        self.write_byte(b'-')?;
        self.write_2_digits(u32::from(dt.day() as u8))?;
        self.write_byte(b'T')?;
        self.write_2_digits(u32::from(dt.hour() as u8))?;
        self.write_byte(b':')?;
        self.write_2_digits(u32::from(dt.minute() as u8))?;
        self.write_byte(b':')?;
        self.write_2_digits(u32::from(dt.second() as u8))?;
        self.write_byte(b'.')?;
        let micros = (dt.subsec_nanosecond() / 1_000) as u32;
        self.write_6_digits(micros)?;
        self.write_byte(b'Z')?;
        Ok(())
    }

    fn write_datetime_list(&mut self, items: &[Timestamp]) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(b',')?;
            }
            first = false;
            self.write_datetime(item)?;
        }
        Ok(())
    }

    fn write_2_digits(&mut self, value: u32) -> Result<()> {
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(b'0' + tens as u8)?;
        self.write_byte(b'0' + ones as u8)?;
        Ok(())
    }

    fn write_4_digits(&mut self, value: u32) -> Result<()> {
        let thousands = (value / 1000) % 10;
        let hundreds = (value / 100) % 10;
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(b'0' + thousands as u8)?;
        self.write_byte(b'0' + hundreds as u8)?;
        self.write_byte(b'0' + tens as u8)?;
        self.write_byte(b'0' + ones as u8)?;
        Ok(())
    }

    fn write_6_digits(&mut self, value: u32) -> Result<()> {
        let hundreds_thousands = (value / 100000) % 10;
        let tens_thousands = (value / 10000) % 10;
        let thousands = (value / 1000) % 10;
        let hundreds = (value / 100) % 10;
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(b'0' + hundreds_thousands as u8)?;
        self.write_byte(b'0' + tens_thousands as u8)?;
        self.write_byte(b'0' + thousands as u8)?;
        self.write_byte(b'0' + hundreds as u8)?;
        self.write_byte(b'0' + tens as u8)?;
        self.write_byte(b'0' + ones as u8)?;
        Ok(())
    }

    fn write_delimited<T: std::fmt::Display>(&mut self, items: &[T]) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(b',')?;
            }
            first = false;
            write!(self.writer, "{}", item)?;
        }
        Ok(())
    }

    fn try_write_as_number(&mut self, nodes: &[Node<'_>]) -> Result<bool> {
        if nodes.len() != 1 {
            return Ok(false);
        }
        let Node::Value(value) = &nodes[0] else {
            return Ok(false);
        };
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

    fn write_signed_number(&mut self, value: i64) -> Result<bool> {
        self.formatter
            .write_i64(&mut self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    fn write_unsigned_number(&mut self, value: u64) -> Result<bool> {
        self.formatter
            .write_u64(&mut self.writer, value)
            .map_err(EvtxError::from)?;
        Ok(true)
    }

    fn render_text_to_json_string(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        self.write_byte(b'"')?;
        self.write_json_text_content(nodes)?;
        self.write_byte(b'"')
    }

    fn render_content_as_json_value(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        if self.try_write_as_number(nodes)? {
            return Ok(());
        }
        self.render_text_to_json_string(nodes)
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

    fn has_non_empty_attributes(&self, element: &Element<'_>) -> bool {
        for attr in &element.attrs {
            if self.has_non_empty_text_content(&attr.value) {
                return true;
            }
        }
        false
    }

    fn render_attributes_object(&mut self, attrs: &[Attr<'_>]) -> Result<bool> {
        let mut has_any = false;
        for attr in attrs {
            if self.has_non_empty_text_content(&attr.value) {
                has_any = true;
                break;
            }
        }
        if !has_any {
            return Ok(false);
        }

        self.write_bytes(b"\"#attributes\":{")?;
        let mut first = true;
        for attr in attrs {
            if !self.has_non_empty_text_content(&attr.value) {
                continue;
            }
            if !first {
                self.write_byte(b',')?;
            }
            first = false;
            self.write_byte(b'"')?;
            self.write_name(attr.name.as_str())?;
            self.write_bytes(b"\":")?;
            if self.try_write_as_number(&attr.value)? {
                continue;
            }
            self.render_text_to_json_string(&attr.value)?;
        }
        self.write_byte(b'}')?;
        Ok(true)
    }

    fn should_render_as_null(&self, element: &Element<'_>) -> bool {
        if element.has_element_child {
            return false;
        }
        if self.has_non_empty_text_content(&element.children) {
            return false;
        }
        if self.has_non_empty_attributes(element) {
            return false;
        }
        true
    }

    fn can_render_as_simple_value(&self, element: &Element<'_>) -> bool {
        if element.has_element_child {
            return false;
        }
        if self.has_non_empty_attributes(element) {
            return false;
        }
        self.has_non_empty_text_content(&element.children)
    }

    fn is_leaf_string(&self, element: &Element<'_>) -> bool {
        element.attrs.is_empty() && !element.has_element_child
    }

    fn render_data_element_value(&mut self, element: &Element<'_>) -> Result<()> {
        if !self.has_non_empty_text_content(&element.children) && !element.has_element_child {
            return self.write_bytes(b"\"\"");
        }

        if element.has_element_child {
            self.write_element_body_json(element, false)
        } else {
            self.render_content_as_json_value(&element.children)
        }
    }

    fn write_element_value(
        &mut self,
        element: &Element<'_>,
        child_is_container: bool,
    ) -> Result<()> {
        if self.should_render_as_null(element) {
            self.write_bytes(b"null")
        } else if self.can_render_as_simple_value(element) {
            self.render_content_as_json_value(&element.children)
        } else if self.is_leaf_string(element) {
            self.render_content_as_json_value(&element.children)
        } else {
            self.write_element_body_json(element, child_is_container)
        }
    }

    fn write_element_body_json(
        &mut self,
        element: &Element<'_>,
        in_data_container: bool,
    ) -> Result<()> {
        let arena = self.arena;
        let mut name_counts: [Option<NameCount<'_>>; MAX_UNIQUE_NAMES] =
            std::array::from_fn(|_| None);
        let mut num_unique = 0usize;

        for node in &element.children {
            let Node::Element(child_id) = node else {
                continue;
            };
            let child = arena.get(*child_id).expect("invalid element id").get();
            let key = NameKey::from_name(&child.name);
            let mut found = false;
            for idx in 0..num_unique {
                let Some(nc) = name_counts[idx].as_mut() else {
                    continue;
                };
                if nc.key.eql(key) {
                    nc.count = nc.count.saturating_add(1);
                    found = true;
                    break;
                }
            }
            if !found && num_unique < MAX_UNIQUE_NAMES {
                name_counts[num_unique] = Some(NameCount {
                    key,
                    count: 1,
                    emitted: false,
                });
                num_unique += 1;
            }
        }

        self.write_byte(b'{')?;
        let mut wrote_any = false;

        if !element.attrs.is_empty() {
            if self.render_attributes_object(&element.attrs)? {
                wrote_any = true;
            }
        }

        let should_flatten = in_data_container;

        for node in &element.children {
            let Node::Element(child_id) = node else {
                continue;
            };

            let child = arena.get(*child_id).expect("invalid element id").get();
            let key = NameKey::from_name(&child.name);
            let mut count = 1u16;
            let mut found = false;

            for idx in 0..num_unique {
                let Some(nc) = name_counts[idx].as_mut() else {
                    continue;
                };
                if nc.key.eql(key) {
                    if nc.emitted {
                        found = true;
                        break;
                    }
                    nc.emitted = true;
                    count = nc.count;
                    found = true;
                    break;
                }
            }

            if !found {
                continue;
            }

            if should_flatten && is_data_element(child.name.as_str()) {
                for node2 in &element.children {
                    let Node::Element(candidate_id) = node2 else {
                        continue;
                    };
                    let candidate = arena.get(*candidate_id).expect("invalid element id").get();
                    if !is_data_element(candidate.name.as_str()) {
                        continue;
                    }
                    let Some(name_nodes) = Self::get_name_attr_nodes(candidate) else {
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
                    self.render_data_element_value(candidate)?;
                }
                continue;
            }

            if wrote_any {
                self.write_byte(b',')?;
            }

            self.write_json_key_from_name(&child.name)?;
            let child_is_container = is_data_container(child.name.as_str());

            if count == 1 {
                self.write_element_value(child, child_is_container)?;
            } else {
                self.write_byte(b'[')?;
                let mut first = true;
                for node2 in &element.children {
                    let Node::Element(candidate_id) = node2 else {
                        continue;
                    };
                    let candidate = arena.get(*candidate_id).expect("invalid element id").get();
                    if !NameKey::from_name(&candidate.name).eql(key) {
                        continue;
                    }
                    if !first {
                        self.write_byte(b',')?;
                    }
                    first = false;
                    self.write_element_value(candidate, child_is_container)?;
                }
                self.write_byte(b']')?;
            }

            wrote_any = true;
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

fn to_hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'A' + (value - 10),
    }
}

fn is_data_container(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn is_data_element(name: &str) -> bool {
    name == "Data"
}
