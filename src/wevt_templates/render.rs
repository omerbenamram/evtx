//! Rendering helpers for template BinXML.
//!
//! These helpers are used for *offline template extraction/debugging* (e.g. `extract-wevt-templates --dump-temp-xml`).
//! Production EVTX parsing/rendering should go through the main BinXML→IR pipeline, which now also supports
//! WEVT_TEMPLATE fallback during parsing.

use bumpalo::Bump;
use encoding::EncodingRef;

use crate::ParserSettings;
use crate::binxml::ir::{build_wevt_template_definition_ir, instantiate_template_definition_ir};
use crate::binxml::ir_xml::render_xml_record;
use crate::binxml::value_variant::BinXmlValue;
use crate::err::{EvtxError, Result};
use crate::model::ir::{IrTree, TemplateValue};

// TEMP layout: BinXML fragment starts at offset 40.
const TEMP_BINXML_OFFSET: usize = 40;

/// Render a `TEMP` entry to an XML string (with `{sub:N}` placeholders for substitutions).
pub fn render_temp_to_xml(temp_bytes: &[u8], ansi_codec: EncodingRef) -> Result<String> {
    let binxml = temp_bytes.get(TEMP_BINXML_OFFSET..).ok_or_else(|| {
        EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        ))
    })?;

    let bump = Bump::new();
    let template = build_wevt_template_definition_ir(binxml, ansi_codec, &bump)?;
    let values = placeholder_values(&template, None, &bump);
    let instantiated = instantiate_template_definition_ir(&template, &values, &bump)?;
    render_ir_xml(&instantiated, ansi_codec)
}

/// Render a `TEMP` entry to an XML string, applying typed substitution values.
///
/// The `bump` arena is used for any allocations needed while building and instantiating the
/// template IR. Callers that already have an EVTX record/chunk context can pass
/// `&record.chunk.arena` to avoid copying substitution values.
pub fn render_temp_to_xml_with_values<'a>(
    temp_bytes: &[u8],
    substitution_values: &[BinXmlValue<'a>],
    ansi_codec: EncodingRef,
    bump: &'a Bump,
) -> Result<String> {
    let binxml = temp_bytes.get(TEMP_BINXML_OFFSET..).ok_or_else(|| {
        EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        ))
    })?;

    let template = build_wevt_template_definition_ir(binxml, ansi_codec, bump)?;
    let values = template_values_from_binxml_values(substitution_values);
    let instantiated = instantiate_template_definition_ir(&template, &values, bump)?;
    render_ir_xml(&instantiated, ansi_codec)
}

/// Render a parsed template definition to XML (with `{sub:idx[:name]}` placeholders).
pub fn render_template_definition_to_xml(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    ansi_codec: EncodingRef,
) -> Result<String> {
    let bump = Bump::new();
    let ir = build_wevt_template_definition_ir(template.binxml, ansi_codec, &bump)?;
    let names = template
        .items
        .iter()
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    let values = placeholder_values(&ir, Some(&names), &bump);
    let instantiated = instantiate_template_definition_ir(&ir, &values, &bump)?;
    render_ir_xml(&instantiated, ansi_codec)
}

/// Render a parsed template definition to XML, applying typed substitution values.
///
/// The `bump` arena is used for any allocations needed while building and instantiating the
/// template IR. Callers that already have an EVTX record/chunk context can pass
/// `&record.chunk.arena` to avoid copying substitution values.
pub fn render_template_definition_to_xml_with_values<'a>(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    substitution_values: &[BinXmlValue<'a>],
    ansi_codec: EncodingRef,
    bump: &'a Bump,
) -> Result<String> {
    let ir = build_wevt_template_definition_ir(template.binxml, ansi_codec, bump)?;
    let values = template_values_from_binxml_values(substitution_values);
    let instantiated = instantiate_template_definition_ir(&ir, &values, bump)?;
    render_ir_xml(&instantiated, ansi_codec)
}

fn render_ir_xml(tree: &IrTree<'_>, ansi_codec: EncodingRef) -> Result<String> {
    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut out = Vec::new();
    render_xml_record(tree, &settings, &mut out)?;
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

fn template_values_from_binxml_values<'a>(
    values: &[BinXmlValue<'a>],
) -> Vec<TemplateValue<'a>> {
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
