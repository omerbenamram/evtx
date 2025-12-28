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
use crate::binxml::name::{BinXmlNameRef, read_wevt_inline_name_at};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::deserialized::{BinXMLDeserializedTokens, TemplateSubstitutionDescriptor};
use crate::model::ir::{Attr, Element, Name, Node, Text};
use std::borrow::Cow;

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
    let root = build_wevt_tree(binxml, tokens, mode)?;
    render_ir_xml(&root, ansi_codec)
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
    let root = build_wevt_tree(binxml, tokens, mode)?;
    render_ir_xml(&root, ansi_codec)
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
    let root = build_wevt_tree(binxml, tokens, mode)?;
    render_ir_xml(&root, ansi_codec)
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
    let root = build_wevt_tree(binxml, tokens, mode)?;
    render_ir_xml(&root, ansi_codec)
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

fn render_ir_xml(root: &Element<'_>, ansi_codec: EncodingRef) -> Result<String> {
    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut out = Vec::new();
    render_xml_record(root, &settings, &mut out)?;
    String::from_utf8(out).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

fn build_wevt_tree<'a>(
    binxml: &'a [u8],
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    mode: SubstitutionMode<'_>,
) -> Result<Element<'a>> {
    let mut stack: Vec<Element<'a>> = Vec::new();
    let mut current_element: Option<WevtElementBuilder<'a>> = None;
    let mut root: Option<Element<'a>> = None;

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
                let name = resolve_name(binxml, &elem.name)?;
                current_element = Some(WevtElementBuilder::new(name));
            }
            BinXMLDeserializedTokens::Attribute(attr) => {
                let builder =
                    current_element
                        .as_mut()
                        .ok_or(EvtxError::FailedToCreateRecordModel(
                            "attribute - Bad parser state",
                        ))?;
                let name = resolve_name(binxml, &attr.name)?;
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
                _ => {
                    let node = Node::Value(value);
                    push_node(&mut stack, &mut current_element, node)?;
                }
            },
            BinXMLDeserializedTokens::EntityRef(entity) => {
                let name = resolve_name(binxml, &entity.name)?;
                let node = Node::EntityRef(name);
                push_node(&mut stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::PITarget(target) => {
                let name = resolve_name(binxml, &target.name)?;
                let node = Node::PITarget(name);
                push_node(&mut stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::PIData(data) => {
                let node = Node::PIData(Text::new(Cow::Owned(data)));
                push_node(&mut stack, &mut current_element, node)?;
            }
            BinXMLDeserializedTokens::Substitution(sub) => {
                if let Some(text) = substitution_text(&mode, &sub) {
                    let node = Node::Text(Text::new(Cow::Owned(text)));
                    push_node(&mut stack, &mut current_element, node)?;
                }
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close start - Bad parser state",
                    ))?
                    .finish();
                stack.push(element);
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close empty - Bad parser state",
                    ))?
                    .finish();
                attach_element(&mut stack, &mut root, element)?;
            }
            BinXMLDeserializedTokens::CloseElement => {
                let element = stack.pop().ok_or(EvtxError::FailedToCreateRecordModel(
                    "close element - Bad parser state",
                ))?;
                attach_element(&mut stack, &mut root, element)?;
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

    root.ok_or(EvtxError::FailedToCreateRecordModel("missing root element"))
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
            if let Some(names) = names {
                if let Some(name) = names.get(idx).and_then(|n| n.as_ref().map(|s| s.as_str())) {
                    placeholder = format!("{{sub:{idx}:{name}}}");
                }
            }
            Some(placeholder)
        }
    }
}

fn resolve_name<'a>(binxml: &'a [u8], name_ref: &BinXmlNameRef) -> Result<Name<'a>> {
    let name = read_wevt_inline_name_at(binxml, name_ref.offset)?;
    Ok(Name::new(Cow::Owned(name)))
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

fn push_node<'a>(
    stack: &mut Vec<Element<'a>>,
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
        let parent = stack
            .last_mut()
            .ok_or(EvtxError::FailedToCreateRecordModel(
                "value outside of element",
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
    attrs: Vec<Attr<'a>>,
    current_attr_name: Option<Name<'a>>,
    current_attr_value: Vec<Node<'a>>,
}

impl<'a> WevtElementBuilder<'a> {
    fn new(name: Name<'a>) -> Self {
        WevtElementBuilder {
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
