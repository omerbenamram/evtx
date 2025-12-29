//! Rendering helpers for template BinXML.
//!
//! Offline template caching needs deterministic, human-readable XML output. These helpers bridge
//! from the WEVT inline-name BinXML token stream to IR trees and then render XML strings either as
//! a template skeleton (with `{sub:N}` placeholders) or as a fully rendered fragment once
//! substitution values are available.
//!
//! References:
//! - `docs/wevt_templates.md` (project notes + curated links)
//! - MS-EVEN6 (BinXml token grammar + inline names)

use encoding::EncodingRef;

use super::binxml::{TEMP_BINXML_OFFSET, parse_temp_binxml_fragment, parse_wevt_binxml_fragment};
use crate::ParserSettings;
use crate::binxml::ir_xml::render_xml_record;
use crate::binxml::name::BinXmlNameRef;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::deserialized::{BinXMLDeserializedTokens, TemplateSubstitutionDescriptor};
use crate::model::ir::{Attr, Element, ElementId, IrArena, IrTree, IrVec, Name, Node, Text};
use bumpalo::Bump;
use crate::utils::ByteCursor;

/// Render a `TEMP` entry to an XML string (with `{sub:N}` placeholders for substitutions).
///
/// `TEMP` is the raw template definition as shipped in provider resources. Rendering it as an
/// XML *skeleton* is useful for building caches and for debugging, even when you don’t have an
/// EVTX record’s substitution values.
pub fn render_temp_to_xml(temp_bytes: &[u8], ansi_codec: EncodingRef) -> Result<String> {
    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let (tokens, _bytes_consumed) = parse_temp_binxml_fragment(temp_bytes, ansi_codec)?;

    let mode = SubstitutionMode::Placeholders { names: None };
    let arena = Bump::new();
    let tree = build_wevt_tree(binxml, tokens, mode, &arena)?;
    render_ir_xml(&tree, ansi_codec)
}

/// Render a `TEMP` entry to an XML string, applying substitution values.
///
/// This is the "last mile" helper for offline rendering: given a raw `TEMP` blob (as extracted
/// from `WEVT_TEMPLATE`) and the corresponding substitution values array from an EVTX record's
/// `TemplateInstance`, emit the fully rendered XML fragment.
///
/// Substitution values are provided as strings and will be inserted as text/attribute values.
pub fn render_temp_to_xml_with_substitution_values(
    temp_bytes: &[u8],
    substitution_values: &[String],
    ansi_codec: EncodingRef,
) -> Result<String> {
    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let (tokens, _bytes_consumed) = parse_temp_binxml_fragment(temp_bytes, ansi_codec)?;

    let mode = SubstitutionMode::Values(substitution_values);
    let arena = Bump::new();
    let tree = build_wevt_tree(binxml, tokens, mode, &arena)?;
    render_ir_xml(&tree, ansi_codec)
}

/// Render a parsed template definition to XML.
///
/// Compared to `render_temp_to_xml`, this variant can annotate substitutions using the parsed
/// template item descriptors/names (from the CRIM blob).
///
/// Caches and diagnostics benefit from stable, readable placeholders (`{sub:idx:name}`) instead
/// of only positional ones.
pub fn render_template_definition_to_xml(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    ansi_codec: EncodingRef,
) -> Result<String> {
    let binxml = template.binxml;
    let (tokens, _bytes_consumed) = parse_wevt_binxml_fragment(binxml, ansi_codec)?;

    let names = template
        .items
        .iter()
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();

    let mode = SubstitutionMode::Placeholders {
        names: Some(&names),
    };
    let arena = Bump::new();
    let tree = build_wevt_tree(binxml, tokens, mode, &arena)?;
    render_ir_xml(&tree, ansi_codec)
}

/// Render a parsed template definition to XML, applying substitution values.
///
/// This is the "last mile" for offline rendering: given a template definition (from
/// `WEVT_TEMPLATE`) and the corresponding substitution values array (from an EVTX record's
/// `TemplateInstance`), emit a fully-rendered XML event fragment.
///
/// The `substitution_values` are provided as strings and are inserted as text/attribute values.
pub fn render_template_definition_to_xml_with_substitution_values(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    substitution_values: &[String],
    ansi_codec: EncodingRef,
) -> Result<String> {
    let binxml = template.binxml;
    let (tokens, _bytes_consumed) = parse_wevt_binxml_fragment(binxml, ansi_codec)?;

    let mode = SubstitutionMode::Values(substitution_values);
    let arena = Bump::new();
    let tree = build_wevt_tree(binxml, tokens, mode, &arena)?;
    render_ir_xml(&tree, ansi_codec)
}

/// How substitutions should be represented while building a WEVT IR tree.
///
/// In `Placeholders` mode, substitutions are rendered as `{sub:N}` markers
/// (optionally annotated with template item names). In `Values` mode, provided
/// substitution strings are inserted directly into the tree.
enum SubstitutionMode<'a> {
    Placeholders { names: Option<&'a [Option<String>]> },
    Values(&'a [String]),
}

fn render_ir_xml(tree: &IrTree<'_>, ansi_codec: EncodingRef) -> Result<String> {
    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut out = Vec::new();
    render_xml_record(tree, &settings, &mut out)?;
    String::from_utf8(out).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

fn build_wevt_tree<'a>(
    binxml: &'a [u8],
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    mode: SubstitutionMode<'_>,
    bump: &'a Bump,
) -> Result<IrTree<'a>> {
    let mut arena = IrArena::new_in(bump);
    let mut stack: Vec<ElementId> = Vec::new();
    let mut current_element: Option<WevtElementBuilder<'a>> = None;
    let mut root: Option<ElementId> = None;

    for token in tokens {
        match token {
            BinXMLDeserializedTokens::FragmentHeader(_)
            | BinXMLDeserializedTokens::AttributeList
            | BinXMLDeserializedTokens::StartOfStream
            | BinXMLDeserializedTokens::EndOfStream => {}
            BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT template BinXML".to_string(),
                });
            }
            BinXMLDeserializedTokens::OpenStartElement(elem) => {
                if current_element.is_some() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "open start - Bad parser state",
                    ));
                }
                let name = resolve_name(binxml, &elem.name, bump)?;
                current_element = Some(WevtElementBuilder::new(name, bump));
            }
            BinXMLDeserializedTokens::Attribute(attr) => {
                let builder =
                    current_element
                        .as_mut()
                        .ok_or(EvtxError::FailedToCreateRecordModel(
                            "attribute - Bad parser state",
                        ))?;
                let name = resolve_name(binxml, &attr.name, bump)?;
                builder.start_attribute(name);
            }
            BinXMLDeserializedTokens::Value(value) => match value {
                BinXmlValue::EvtXml => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unexpected EvtXml in WEVT template BinXML",
                    ));
                }
                BinXmlValue::BinXmlType(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unexpected BinXmlType in WEVT template BinXML",
                    ));
                }
                BinXmlValue::StringType(s) => {
                    let node = Node::Text(Text::utf16(s));
                    push_node(&mut arena, &stack, &mut current_element, node)?;
                }
                BinXmlValue::AnsiStringType(s) => {
                    let node = Node::Text(Text::utf8(s));
                    push_node(&mut arena, &stack, &mut current_element, node)?;
                }
                other => {
                    let node = Node::Value(other);
                    push_node(&mut arena, &stack, &mut current_element, node)?;
                }
            },
            BinXMLDeserializedTokens::EntityRef(entity) => {
                let name = resolve_name(binxml, &entity.name, bump)?;
                let node = Node::EntityRef(name);
                push_node(&mut arena, &stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::PITarget(target) => {
                let name = resolve_name(binxml, &target.name, bump)?;
                let node = Node::PITarget(name);
                push_node(&mut arena, &stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::PIData(data) => {
                let node = Node::PIData(Text::utf16(data));
                push_node(&mut arena, &stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::Substitution(sub) => {
                if let Some(text) = substitution_text(&mode, &sub) {
                    let text = bump.alloc_str(&text);
                    let node = Node::Text(Text::utf8(text));
                    push_node(&mut arena, &stack, &mut current_element, node)?;
                }
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close start - Bad parser state",
                    ))?
                    .finish();
                let element_id = arena.new_node(element);
                stack.push(element_id);
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close empty - Bad parser state",
                    ))?
                    .finish();
                let element_id = arena.new_node(element);
                attach_element(&mut arena, &stack, &mut root, element_id)?;
            }
            BinXMLDeserializedTokens::CloseElement => {
                let element_id = stack.pop().ok_or(EvtxError::FailedToCreateRecordModel(
                    "close element - Bad parser state",
                ))?;
                attach_element(&mut arena, &stack, &mut root, element_id)?;
            }
            BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA/CharRef",
                ));
            }
        }
    }

    if current_element.is_some() {
        return Err(EvtxError::FailedToCreateRecordModel(
            "unfinished element start",
        ));
    }

    if !stack.is_empty() {
        return Err(EvtxError::FailedToCreateRecordModel(
            "unbalanced element stack",
        ));
    }

    let root_id = root.ok_or(EvtxError::FailedToCreateRecordModel("missing root element"))?;
    Ok(IrTree::new(arena, root_id))
}

fn substitution_text(
    mode: &SubstitutionMode<'_>,
    sub: &TemplateSubstitutionDescriptor,
) -> Option<String> {
    match mode {
        SubstitutionMode::Values(values) => {
            if sub.ignore {
                return None;
            }
            let idx = sub.substitution_index as usize;
            Some(values.get(idx).cloned().unwrap_or_default())
        }
        SubstitutionMode::Placeholders { names } => {
            let idx = sub.substitution_index as usize;
            let mut placeholder = format!("{{sub:{idx}}}");
            if let Some(names) = names
                && let Some(name) = names.get(idx).and_then(|n| n.as_deref())
            {
                placeholder = format!("{{sub:{idx}:{name}}}");
            }
            Some(placeholder)
        }
    }
}

fn resolve_name<'a>(binxml: &'a [u8], name_ref: &BinXmlNameRef, bump: &'a Bump) -> Result<Name<'a>> {
    // Inline WEVT name structure: u16 hash + u16 char_count + UTF-16LE chars + u16 NUL.
    // NameRef parsing already validates the hash; we just decode here.
    let mut cursor = ByteCursor::with_pos(binxml, name_ref.offset as usize)?;
    let _ = cursor.u16_named("wevt_inline_name_hash")?;
    let name = cursor
        .len_prefixed_utf16_string_bump(true, "wevt_inline_name", bump)?
        .unwrap_or("");
    Ok(Name::new(name))
}

fn attach_element<'a>(
    arena: &mut IrArena<'a>,
    stack: &[ElementId],
    root: &mut Option<ElementId>,
    element_id: ElementId,
) -> Result<()> {
    if let Some(parent_id) = stack.last().copied() {
        let parent = arena
            .get_mut(parent_id)
            .ok_or(EvtxError::FailedToCreateRecordModel(
                "invalid parent element id",
            ))?;
        parent.push_child(Node::Element(element_id));
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

fn push_node<'a>(
    arena: &mut IrArena<'a>,
    stack: &[ElementId],
    current_element: &mut Option<WevtElementBuilder<'a>>,
    node: Node<'a>,
) -> Result<()> {
    if let Some(builder) = current_element.as_mut() {
        if matches!(node, Node::Element(_)) {
            return Err(EvtxError::FailedToCreateRecordModel(
                "element inside attribute value",
            ));
        }
        builder.push_attr_value(node);
        Ok(())
    } else {
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
        parent.push_child(node);
        Ok(())
    }
}

/// Incremental element builder for WEVT BinXML tokens.
///
/// Attributes are accumulated until the start tag is closed, then materialized
/// into a concrete `Element` with name/attribute separation.
struct WevtElementBuilder<'a> {
    name: Name<'a>,
    attrs: IrVec<'a, Attr<'a>>,
    current_attr_name: Option<Name<'a>>,
    current_attr_value: IrVec<'a, Node<'a>>,
    arena: &'a Bump,
}

impl<'a> WevtElementBuilder<'a> {
    fn new(name: Name<'a>, arena: &'a Bump) -> Self {
        WevtElementBuilder {
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
