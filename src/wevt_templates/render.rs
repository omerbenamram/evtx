//! Rendering helpers for template BinXML.
//!
//! These helpers are used for *offline template extraction/debugging*.
//! Production EVTX parsing/rendering should go through the main BinXML→IR pipeline, which now also supports
//! WEVT_TEMPLATE fallback during parsing.

use bumpalo::Bump;

use crate::AnsiCodec;
use crate::ParserSettings;
use crate::binxml::ir::{build_wevt_template_definition_ir, instantiate_template_definition_ir};
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{IrTree, TemplateValue};
use crate::wevt_templates::TEMP_HEADER_SIZE;
use crate::wevt_templates::manifest::TemplateDefinition;

/// Render a `TEMP` entry to an XML string (with `{sub:N}` placeholders for substitutions).
///
/// This is the byte-oriented counterpart of [`render_template_definition_to_xml`] for callers
/// that hold raw `TEMP` blobs (e.g. from [`extract_temp_templates_from_wevt_blob`]).
///
/// [`extract_temp_templates_from_wevt_blob`]: crate::wevt_templates::extract_temp_templates_from_wevt_blob
pub fn render_temp_to_xml(temp_bytes: &[u8], ansi_codec: AnsiCodec) -> Result<String> {
    render_binxml_placeholders(temp_binxml_fragment(temp_bytes)?, None, ansi_codec)
}

/// Render a `TEMP` entry to an XML string, applying typed substitution values.
///
/// The `bump` arena is used for any allocations needed while building and instantiating the
/// template IR. Callers that already have an EVTX record/chunk context can pass
/// `&record.chunk.arena` to avoid copying substitution values.
pub fn render_temp_to_xml_with_values<'a>(
    temp_bytes: &[u8],
    substitution_values: &[BinXmlValue<'a>],
    ansi_codec: AnsiCodec,
    bump: &'a Bump,
) -> Result<String> {
    let binxml = temp_binxml_fragment(temp_bytes)?;
    render_binxml_with_values(binxml, substitution_values, ansi_codec, bump)
}

fn temp_binxml_fragment(temp_bytes: &[u8]) -> Result<&[u8]> {
    temp_bytes.get(TEMP_HEADER_SIZE..).ok_or_else(|| {
        EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_HEADER_SIZE
        ))
    })
}

/// Render a parsed template definition to XML (with `{sub:idx[:name]}` placeholders).
pub fn render_template_definition_to_xml(
    template: &TemplateDefinition<'_>,
    ansi_codec: AnsiCodec,
) -> Result<String> {
    let names = template
        .items
        .iter()
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    render_binxml_placeholders(template.binxml, Some(&names), ansi_codec)
}

/// Render a parsed template definition to XML, applying typed substitution values.
///
/// The `bump` arena is used for any allocations needed while building and instantiating the
/// template IR. Callers that already have an EVTX record/chunk context can pass
/// `&record.chunk.arena` to avoid copying substitution values.
pub fn render_template_definition_to_xml_with_values<'a>(
    template: &TemplateDefinition<'_>,
    substitution_values: &[BinXmlValue<'a>],
    ansi_codec: AnsiCodec,
    bump: &'a Bump,
) -> Result<String> {
    render_binxml_with_values(template.binxml, substitution_values, ansi_codec, bump)
}

fn render_binxml_placeholders(
    binxml: &[u8],
    names: Option<&[Option<String>]>,
    ansi_codec: AnsiCodec,
) -> Result<String> {
    let bump = Bump::new();
    let ir = build_wevt_template_definition_ir(binxml, ansi_codec, &bump)?;
    let values = placeholder_values(&ir, names, &bump);
    render_instantiated(&ir, &values, ansi_codec, &bump)
}

fn render_binxml_with_values<'a>(
    binxml: &[u8],
    substitution_values: &[BinXmlValue<'a>],
    ansi_codec: AnsiCodec,
    bump: &'a Bump,
) -> Result<String> {
    let ir = build_wevt_template_definition_ir(binxml, ansi_codec, bump)?;
    let values = template_values_from_binxml_values(substitution_values);
    render_instantiated(&ir, &values, ansi_codec, bump)
}

fn render_instantiated<'a>(
    ir: &IrTree<'a>,
    values: &[TemplateValue<'a>],
    ansi_codec: AnsiCodec,
    bump: &'a Bump,
) -> Result<String> {
    let instantiated = instantiate_template_definition_ir(ir, values, bump)?;
    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut out = Vec::new();
    crate::binxml::compiled::render_tree_xml(&instantiated, &settings, &mut out)?;
    String::from_utf8(out).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

fn max_placeholder_id(tree: &IrTree<'_>) -> usize {
    let mut max: Option<u16> = None;
    let arena = tree.arena();
    for id in 0..arena.count() {
        let element = arena.get(id).expect("element id in range");
        for attr in &element.attrs {
            for node in &attr.value {
                if let crate::model::ir::Node::Placeholder(ph) = node {
                    max = Some(max.map_or(ph.id, |m| m.max(ph.id)));
                }
            }
        }
        for node in &element.children {
            if let crate::model::ir::Node::Placeholder(ph) = node {
                max = Some(max.map_or(ph.id, |m| m.max(ph.id)));
            }
        }
    }
    max.map(|v| v as usize).unwrap_or(0)
}

fn placeholder_values<'a>(
    template: &IrTree<'_>,
    names: Option<&[Option<String>]>,
    bump: &'a Bump,
) -> Vec<TemplateValue<'a>> {
    let max_id = max_placeholder_id(template);
    let mut out = Vec::with_capacity(max_id + 1);

    for idx in 0..=max_id {
        let mut s = format!("{{sub:{idx}}}");
        if let Some(names) = names
            && let Some(name) = names.get(idx).and_then(|n| n.as_deref())
        {
            s = format!("{{sub:{idx}:{name}}}");
        }
        let s = bump.alloc_str(&s);
        out.push(TemplateValue::Value(BinXmlValue::AnsiStringType(s)));
    }
    out
}

fn template_values_from_binxml_values<'a>(values: &[BinXmlValue<'a>]) -> Vec<TemplateValue<'a>> {
    values
        .iter()
        .map(|v| match v {
            // Offline WEVT rendering does not currently splice nested BinXML substitutions.
            // Treat these as empty so rendering can proceed.
            BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => {
                TemplateValue::Value(BinXmlValue::NullType)
            }
            other => TemplateValue::Value(other.clone()),
        })
        .collect()
}
