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
use crate::err::{DeserializationError, EvtxError, Result, SerializationError};
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::ir::{Attr, Element, Name, Node, Placeholder, Text};
use crate::utils::ByteCursor;
use crate::{EvtxChunk, ParserSettings};
use quick_xml::events::BytesText;
use serde_json::Value as JsonValue;
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;

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
) -> Result<Element<'a>> {
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
        IrTemplateCache {
            templates: HashMap::new(),
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
        let root =
            build_tree_from_iter_with_mode(iter, chunk, self, BuildMode::TemplateDefinition)?;
        let template = Rc::new(Template { root });
        self.templates.insert(header.guid, Rc::clone(&template));
        Ok(template)
    }

    fn instantiate_template(
        &mut self,
        template_ref: BinXmlTemplateRef<'a>,
        chunk: &'a EvtxChunk<'a>,
    ) -> Result<Element<'a>> {
        let template = self.get_or_parse_template(chunk, template_ref.template_def_offset)?;
        let values = template_values_from_ref(template_ref)?;
        template.instantiate(&values, chunk, self)
    }
}

/// Parsed template definition with placeholder nodes.
#[derive(Debug)]
struct Template<'a> {
    root: Element<'a>,
}

impl<'a> Template<'a> {
    fn instantiate(
        &self,
        values: &[BinXmlValue<'a>],
        chunk: &'a EvtxChunk<'a>,
        cache: &mut IrTemplateCache<'a>,
    ) -> Result<Element<'a>> {
        clone_and_resolve(&self.root, values, chunk, cache)
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
struct TreeBuilder<'a, 'cache> {
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    mode: BuildMode,
    stack: Vec<Element<'a>>,
    current_element: Option<ElementBuilder<'a>>,
    root: Option<Element<'a>>,
}

impl<'a, 'cache> TreeBuilder<'a, 'cache> {
    fn new(
        chunk: &'a EvtxChunk<'a>,
        cache: &'cache mut IrTemplateCache<'a>,
        mode: BuildMode,
    ) -> Self {
        TreeBuilder {
            chunk,
            cache,
            mode,
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
                let element = self.cache.instantiate_template(template, self.chunk)?;
                attach_element(&mut self.stack, &mut self.root, element)
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
                let element =
                    self.current_element
                        .take()
                        .ok_or(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ))?;
                self.stack.push(element.finish());
                Ok(())
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                let element =
                    self.current_element
                        .take()
                        .ok_or(EvtxError::FailedToCreateRecordModel(
                            "close empty - Bad parser state",
                        ))?;
                attach_element(&mut self.stack, &mut self.root, element.finish())
            }
            BinXMLDeserializedTokens::CloseElement => {
                let element = self
                    .stack
                    .pop()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close element - Bad parser state",
                    ))?;
                attach_element(&mut self.stack, &mut self.root, element)
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
                let element = build_tree_from_binxml_bytes(bytes, self.chunk, self.cache)?;
                attach_element(&mut self.stack, &mut self.root, element)
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
            push_child(&mut self.stack, node)
        }
    }

    fn finish(self) -> Result<Element<'a>> {
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
) -> Result<Element<'a>>
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
) -> Result<Element<'a>>
where
    I: IntoIterator<Item = std::result::Result<BinXMLDeserializedTokens<'a>, DeserializationError>>,
{
    let mut builder = TreeBuilder::new(chunk, cache, mode);
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
) -> Result<Element<'a>>
where
    I: IntoIterator<Item = BinXMLDeserializedTokens<'a>>,
{
    let mut builder = TreeBuilder::new(chunk, cache, mode);
    for token in tokens {
        builder.process_token(token)?;
    }
    builder.finish()
}

fn build_tree_from_binxml_bytes<'a, 'cache>(
    bytes: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
) -> Result<Element<'a>> {
    let offset = binxml_slice_offset(chunk, bytes)?;
    let deserializer = crate::binxml::deserializer::BinXmlDeserializer::init(
        chunk.data,
        offset,
        Some(chunk),
        false,
        chunk.settings.get_ansi_codec(),
    );
    let iter = deserializer.iter_tokens(Some(bytes.len() as u32))?;
    build_tree_from_iter(iter, chunk, cache)
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
    element: &Element<'a>,
    values: &[BinXmlValue<'a>],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
) -> Result<Element<'a>> {
    let mut resolved = Element {
        name: element.name.clone(),
        attrs: Vec::new(),
        children: Vec::new(),
        has_element_child: element.has_element_child,
    };

    for attr in &element.attrs {
        let mut new_attr = Attr {
            name: attr.name.clone(),
            value: Vec::new(),
        };
        for node in &attr.value {
            resolve_node_into(node, values, chunk, cache, &mut new_attr.value)?;
        }
        if !new_attr.value.is_empty() {
            resolved.attrs.push(new_attr);
        }
    }

    for node in &element.children {
        let before = resolved.children.len();
        resolve_node_into(node, values, chunk, cache, &mut resolved.children)?;
        if !resolved.has_element_child {
            if resolved.children[before..]
                .iter()
                .any(|n| matches!(n, Node::Element(_)))
            {
                resolved.has_element_child = true;
            }
        }
    }

    Ok(resolved)
}

fn resolve_node_into<'a, 'cache>(
    node: &Node<'a>,
    values: &[BinXmlValue<'a>],
    chunk: &'a EvtxChunk<'a>,
    cache: &'cache mut IrTemplateCache<'a>,
    out: &mut Vec<Node<'a>>,
) -> Result<()> {
    match node {
        Node::Placeholder(ph) => resolve_placeholder_into(ph, values, chunk, cache, out),
        Node::Element(el) => {
            let cloned = clone_and_resolve(el.as_ref(), values, chunk, cache)?;
            out.push(Node::Element(Box::new(cloned)));
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
            let element = build_tree_from_binxml_bytes(bytes, chunk, cache)?;
            out.push(Node::Element(Box::new(element)));
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
    stack: &mut Vec<Element<'a>>,
    root: &mut Option<Element<'a>>,
    element: Element<'a>,
) -> Result<()> {
    if let Some(parent) = stack.last_mut() {
        parent.push_child(Node::Element(Box::new(element)));
        Ok(())
    } else if root.is_none() {
        *root = Some(element);
        Ok(())
    } else {
        Err(EvtxError::FailedToCreateRecordModel(
            "multiple root elements",
        ))
    }
}

fn push_child<'a>(stack: &mut Vec<Element<'a>>, node: Node<'a>) -> Result<()> {
    let parent = stack
        .last_mut()
        .ok_or(EvtxError::FailedToCreateRecordModel(
            "value outside of element",
        ))?;
    parent.push_child(node);
    Ok(())
}

fn value_to_node<'a>(value: BinXmlValue<'a>) -> Result<Node<'a>> {
    match value {
        BinXmlValue::StringType(s) => Ok(Node::Text(Text::new(Cow::Owned(s)))),
        BinXmlValue::AnsiStringType(s) => Ok(Node::Text(Text::new(s))),
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

pub(crate) fn render_json_record<W: Write>(
    root: &Element<'_>,
    settings: &ParserSettings,
    writer: &mut W,
) -> Result<()> {
    let mut emitter = JsonEmitter::new(writer, settings);
    emitter.write_object_start()?;
    emitter.write_element_field(root, false)?;
    emitter.write_object_end()?;
    Ok(())
}

/// Per-object JSON rendering frame used to de-duplicate keys.
#[derive(Debug)]
struct ObjectFrame {
    first_field: bool,
    used_keys: HashSet<String>,
}

/// Streaming JSON renderer for IR trees.
struct JsonEmitter<'w, W: Write> {
    writer: &'w mut W,
    frames: Vec<ObjectFrame>,
    separate_json_attributes: bool,
}

impl<'w, W: Write> JsonEmitter<'w, W> {
    fn new(writer: &'w mut W, settings: &ParserSettings) -> Self {
        JsonEmitter {
            writer,
            frames: Vec::new(),
            separate_json_attributes: settings.should_separate_json_attributes(),
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        Ok(())
    }

    fn write_object_start(&mut self) -> Result<()> {
        self.write_bytes(b"{")?;
        self.frames.push(ObjectFrame {
            first_field: true,
            used_keys: HashSet::new(),
        });
        Ok(())
    }

    fn write_object_end(&mut self) -> Result<()> {
        self.write_bytes(b"}")?;
        self.frames.pop();
        Ok(())
    }

    fn current_frame_mut(&mut self) -> &mut ObjectFrame {
        self.frames
            .last_mut()
            .expect("no current JSON object frame")
    }

    fn write_comma_if_needed(&mut self) -> Result<()> {
        let frame = self.current_frame_mut();
        if frame.first_field {
            frame.first_field = false;
            Ok(())
        } else {
            self.write_bytes(b",")
        }
    }

    fn reserve_unique_key(&mut self, key: &str) -> String {
        let frame = self
            .frames
            .last_mut()
            .expect("no current JSON object frame");
        if frame.used_keys.contains(key) {
            let mut suffix = 1;
            loop {
                let candidate = format!("{}_{}", key, suffix);
                if !frame.used_keys.contains(&candidate) {
                    frame.used_keys.insert(candidate.clone());
                    return candidate;
                }
                suffix += 1;
            }
        } else {
            frame.used_keys.insert(key.to_owned());
            key.to_owned()
        }
    }

    fn write_reserved_key(&mut self, key: &str) -> Result<()> {
        self.write_comma_if_needed()?;
        serde_json::to_writer(&mut self.writer, key).map_err(SerializationError::from)?;
        self.write_bytes(b":")
    }

    fn write_key(&mut self, key: &str) -> Result<String> {
        let unique_key = self.reserve_unique_key(key);
        self.write_reserved_key(&unique_key)?;
        Ok(unique_key)
    }

    fn write_element_field(
        &mut self,
        element: &Element<'_>,
        in_data_container: bool,
    ) -> Result<()> {
        let is_data = is_data_element(element.name.as_str());
        let data_name_attr = if is_data {
            self.get_name_attr_value(element)?
        } else {
            None
        };

        if is_data && data_name_attr.is_none() && in_data_container {
            return Ok(());
        }

        let key = data_name_attr
            .as_deref()
            .unwrap_or_else(|| element.name.as_str());

        let has_attrs = if is_data {
            false
        } else {
            self.has_non_null_attributes(element)?
        };

        let write_attrs_separately = has_attrs && self.separate_json_attributes;

        let element_key = if write_attrs_separately {
            let unique_key = self.reserve_unique_key(key);
            self.write_attributes_sibling(&unique_key, element)?;
            unique_key
        } else {
            String::new()
        };

        let has_text_nodes = has_text_nodes(element);
        let has_element_child = element.has_element_child;

        if write_attrs_separately && !has_text_nodes && !has_element_child {
            return Ok(());
        }

        if write_attrs_separately {
            self.write_reserved_key(&element_key)?;
            self.write_element_value(element, false)?;
        } else {
            self.write_key(key)?;
            self.write_element_value(element, has_attrs)?;
        }

        Ok(())
    }

    fn write_element_value(
        &mut self,
        element: &Element<'_>,
        include_attributes: bool,
    ) -> Result<()> {
        let has_element_child = element.has_element_child;
        let needs_object = has_element_child || include_attributes;
        if !needs_object {
            self.write_nodes_as_scalar(&element.children)?;
            return Ok(());
        }

        self.write_object_start()?;
        if include_attributes {
            self.write_attributes_inline(element)?;
        }

        let child_is_container = is_data_container(element.name.as_str());
        self.write_children(element, child_is_container)?;

        if !self.separate_json_attributes {
            self.write_text_field(element)?;
        }

        self.write_object_end()?;
        Ok(())
    }

    fn write_attributes_inline(&mut self, element: &Element<'_>) -> Result<()> {
        let mut attrs: Vec<&Attr<'_>> = Vec::new();
        for attr in &element.attrs {
            if let Some(_) = self.attr_value_to_json(&attr.value)? {
                attrs.push(attr);
            }
        }

        if attrs.is_empty() {
            return Ok(());
        }

        self.write_key("#attributes")?;
        self.write_object_start()?;
        for attr in attrs {
            self.write_key(attr.name.as_str())?;
            let value = self.attr_value_to_json(&attr.value)?.ok_or(
                EvtxError::FailedToCreateRecordModel("attribute value vanished"),
            )?;
            serde_json::to_writer(&mut self.writer, &value).map_err(SerializationError::from)?;
        }
        self.write_object_end()?;
        Ok(())
    }

    fn write_attributes_sibling(&mut self, element_key: &str, element: &Element<'_>) -> Result<()> {
        let attr_key = format!("{}_attributes", element_key);
        self.write_reserved_key(&attr_key)?;
        self.write_object_start()?;
        for attr in &element.attrs {
            let Some(value) = self.attr_value_to_json(&attr.value)? else {
                continue;
            };
            self.write_key(attr.name.as_str())?;
            serde_json::to_writer(&mut self.writer, &value).map_err(SerializationError::from)?;
        }
        self.write_object_end()?;
        Ok(())
    }

    fn write_children(&mut self, element: &Element<'_>, in_data_container: bool) -> Result<()> {
        let mut data_values: Vec<JsonValue> = Vec::new();

        for node in &element.children {
            let Node::Element(child) = node else {
                continue;
            };
            let child = child.as_ref();
            let child_name = child.name.as_str();
            let child_is_container = is_data_container(child_name);

            if in_data_container && is_data_element(child_name) {
                if let Some(name_attr) = self.get_name_attr_value(child)? {
                    self.write_key(&name_attr)?;
                    self.write_element_value(child, false)?;
                } else {
                    self.collect_data_values(child, &mut data_values)?;
                }
                continue;
            }

            self.write_element_field(child, child_is_container)?;
        }

        if in_data_container && !data_values.is_empty() {
            self.write_aggregated_data(&data_values)?;
        }

        Ok(())
    }

    fn write_text_field(&mut self, element: &Element<'_>) -> Result<()> {
        let mut values: Vec<JsonValue> = Vec::new();
        for node in element
            .children
            .iter()
            .filter(|n| !matches!(n, Node::Element(_)))
        {
            values.push(self.node_to_json_value(node)?);
        }
        if values.is_empty() {
            return Ok(());
        }

        self.write_key("#text")?;
        if values.len() == 1 {
            serde_json::to_writer(&mut self.writer, &values[0])
                .map_err(SerializationError::from)?;
        } else {
            self.write_bytes(b"[")?;
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 {
                    self.write_bytes(b",")?;
                }
                serde_json::to_writer(&mut self.writer, value).map_err(SerializationError::from)?;
            }
            self.write_bytes(b"]")?;
        }
        Ok(())
    }

    fn write_nodes_as_scalar(&mut self, nodes: &[Node<'_>]) -> Result<()> {
        let non_elements: Vec<&Node<'_>> = nodes
            .iter()
            .filter(|n| !matches!(n, Node::Element(_)))
            .collect();

        if non_elements.is_empty() {
            self.write_bytes(b"null")?;
            return Ok(());
        }

        if non_elements.len() == 1 {
            let value = self.node_to_json_value(non_elements[0])?;
            serde_json::to_writer(&mut self.writer, &value).map_err(SerializationError::from)?;
            return Ok(());
        }

        let mut concat = String::new();
        for node in non_elements {
            let value = self.node_to_json_value(node)?;
            match value {
                JsonValue::String(s) => concat.push_str(&s),
                JsonValue::Number(n) => concat.push_str(&n.to_string()),
                JsonValue::Bool(b) => concat.push_str(if b { "true" } else { "false" }),
                JsonValue::Null => concat.push_str("null"),
                other => concat.push_str(&other.to_string()),
            }
        }

        serde_json::to_writer(&mut self.writer, &concat).map_err(SerializationError::from)?;
        Ok(())
    }

    fn has_non_null_attributes(&self, element: &Element<'_>) -> Result<bool> {
        for attr in &element.attrs {
            if let Some(value) = self.attr_value_to_json(&attr.value)? {
                if !value.is_null() {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn attr_value_to_json(&self, nodes: &[Node<'_>]) -> Result<Option<JsonValue>> {
        let non_elements: Vec<&Node<'_>> = nodes
            .iter()
            .filter(|n| !matches!(n, Node::Element(_)))
            .collect();

        if non_elements.is_empty() {
            return Ok(None);
        }

        if non_elements.len() == 1 {
            let value = self.node_to_json_value(non_elements[0])?;
            if value.is_null() {
                return Ok(None);
            }
            return Ok(Some(value));
        }

        let mut concat = String::new();
        for node in non_elements {
            let value = self.node_to_json_value(node)?;
            match value {
                JsonValue::String(s) => concat.push_str(&s),
                JsonValue::Number(n) => concat.push_str(&n.to_string()),
                JsonValue::Bool(b) => concat.push_str(if b { "true" } else { "false" }),
                JsonValue::Null => concat.push_str("null"),
                other => concat.push_str(&other.to_string()),
            }
        }
        Ok(Some(JsonValue::String(concat)))
    }

    fn node_to_json_value(&self, node: &Node<'_>) -> Result<JsonValue> {
        match node {
            Node::Text(text) | Node::CData(text) => Ok(JsonValue::String(text.value.to_string())),
            Node::Value(value) => binxml_value_to_json(value),
            Node::EntityRef(name) => {
                let resolved = resolve_entity_ref(name.as_binxml_name())?;
                Ok(JsonValue::String(resolved))
            }
            Node::CharRef(_) => Err(EvtxError::Unimplemented {
                name: "character reference".to_string(),
            }),
            Node::PITarget(_) | Node::PIData(_) => Err(EvtxError::Unimplemented {
                name: "processing instruction".to_string(),
            }),
            Node::Placeholder(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unresolved placeholder in tree",
            )),
            Node::Element(_) => Err(EvtxError::FailedToCreateRecordModel(
                "unexpected element node in value context",
            )),
        }
    }

    fn collect_data_values(&self, element: &Element<'_>, out: &mut Vec<JsonValue>) -> Result<()> {
        for node in element
            .children
            .iter()
            .filter(|n| !matches!(n, Node::Element(_)))
        {
            out.push(self.node_to_json_value(node)?);
        }
        Ok(())
    }

    fn write_aggregated_data(&mut self, values: &[JsonValue]) -> Result<()> {
        if self.separate_json_attributes {
            let mut concat = String::new();
            for value in values {
                match value {
                    JsonValue::String(s) => concat.push_str(s),
                    JsonValue::Number(n) => concat.push_str(&n.to_string()),
                    JsonValue::Bool(b) => concat.push_str(if *b { "true" } else { "false" }),
                    JsonValue::Null => {}
                    other => concat.push_str(&other.to_string()),
                }
            }

            self.write_key("Data")?;
            serde_json::to_writer(&mut self.writer, &concat).map_err(SerializationError::from)?;
            return Ok(());
        }

        self.write_key("Data")?;
        self.write_object_start()?;
        self.write_key("#text")?;

        if values.len() == 1 {
            serde_json::to_writer(&mut self.writer, &values[0])
                .map_err(SerializationError::from)?;
        } else {
            self.write_bytes(b"[")?;
            for (idx, value) in values.iter().enumerate() {
                if idx > 0 {
                    self.write_bytes(b",")?;
                }
                serde_json::to_writer(&mut self.writer, value).map_err(SerializationError::from)?;
            }
            self.write_bytes(b"]")?;
        }

        self.write_object_end()?;
        Ok(())
    }

    fn get_name_attr_value(&self, element: &Element<'_>) -> Result<Option<String>> {
        for attr in &element.attrs {
            if attr.name.as_str() == "Name" {
                if attr.value.is_empty() {
                    return Ok(None);
                }
                let mut s = String::new();
                for node in attr.value.iter().filter(|n| !matches!(n, Node::Element(_))) {
                    let value = self.node_to_json_value(node)?;
                    match value {
                        JsonValue::String(text) => s.push_str(&text),
                        JsonValue::Number(num) => s.push_str(&num.to_string()),
                        JsonValue::Bool(b) => s.push_str(if b { "true" } else { "false" }),
                        JsonValue::Null => {}
                        other => s.push_str(&other.to_string()),
                    }
                }
                return Ok(Some(s));
            }
        }
        Ok(None)
    }
}

fn binxml_value_to_json(value: &BinXmlValue<'_>) -> Result<JsonValue> {
    match value {
        BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => Err(
            EvtxError::FailedToCreateRecordModel("unsupported BinXML value in tree"),
        ),
        _ => Ok(JsonValue::from(value)),
    }
}

fn resolve_entity_ref(name: &BinXmlName) -> Result<String> {
    let entity_ref = format!("&{};", name.as_str());
    let xml_event = BytesText::from_escaped(&entity_ref);
    match xml_event.unescape() {
        Ok(escaped) => Ok(escaped.to_string()),
        Err(_) => Err(EvtxError::SerializationError(
            SerializationError::JsonStructureError {
                message: format!("Unterminated XML Entity {}", entity_ref),
            },
        )),
    }
}

fn has_text_nodes(element: &Element<'_>) -> bool {
    element
        .children
        .iter()
        .any(|n| !matches!(n, Node::Element(_)))
}

fn is_data_container(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn is_data_element(name: &str) -> bool {
    name == "Data"
}
