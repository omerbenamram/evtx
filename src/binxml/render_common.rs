//! Shared helpers for BinXML XML rendering paths.
//!
//! The IR-based XML renderer and the compiled/raw XML fast path intentionally
//! produce the same document envelope and indentation behavior. This module
//! centralizes the small pieces of that contract that are otherwise easy to
//! duplicate inconsistently.

use crate::err::{EvtxError, Result};
use crate::utils::ByteCursor;
use sonic_rs::writer::WriteExt;

/// XML declaration emitted for every rendered record.
pub(crate) const XML_DECLARATION: &[u8] = b"<?xml version=\"1.0\" encoding=\"utf-8\"?>\n";

/// Size (in bytes) of a template definition header (`next_offset + guid + size`).
pub(crate) const TEMPLATE_DEFINITION_HEADER_SIZE: usize = 24;

/// Minimal template definition metadata shared by the IR and compiled XML paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TemplateDefinitionRef {
    pub guid: [u8; 16],
    pub data_size: u32,
}

/// Starting point for a BinXML fragment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FragmentStart {
    /// A template-instance token (`0x0c`) begins at `token_offset`.
    TemplateInstance { token_offset: usize },
    /// The bytes are a direct token stream and should be compiled inline from `token_offset`.
    InlineTokens { token_offset: usize },
}

/// Parse template metadata directly from the chunk/template table.
pub(crate) fn read_template_definition_ref_at(
    data: &[u8],
    offset: u32,
) -> Result<TemplateDefinitionRef> {
    let mut cursor = ByteCursor::with_pos(data, offset as usize)?;
    let _next_template_offset = cursor.u32_named("next_template_offset")?;
    let guid_bytes = cursor.take_bytes(16, "template_guid")?;
    let data_size = cursor.u32_named("template_data_size")?;

    let guid = <[u8; 16]>::try_from(guid_bytes)
        .map_err(|_| EvtxError::FailedToCreateRecordModel("template guid size mismatch"))?;

    Ok(TemplateDefinitionRef { guid, data_size })
}

/// Classify a BinXML fragment so callers can share fragment-header handling.
pub(crate) fn classify_binxml_fragment(bytes: &[u8]) -> Option<FragmentStart> {
    match bytes.first().copied() {
        None => None,
        Some(0x0f) => {
            if bytes.len() < 5 {
                None
            } else if bytes[4] == 0x0c {
                Some(FragmentStart::TemplateInstance { token_offset: 4 })
            } else {
                Some(FragmentStart::InlineTokens { token_offset: 4 })
            }
        }
        Some(0x0c) => Some(FragmentStart::TemplateInstance { token_offset: 0 }),
        Some(_) => Some(FragmentStart::InlineTokens { token_offset: 0 }),
    }
}

/// Emit the stable XML document preamble used by both rendering paths.
pub(crate) fn write_xml_declaration<W: WriteExt>(writer: &mut W) -> Result<()> {
    writer.write_all(XML_DECLARATION).map_err(EvtxError::from)
}

/// Append `indent` spaces to an in-memory XML buffer.
pub(crate) fn push_indent(buf: &mut Vec<u8>, indent: usize) {
    for _ in 0..indent {
        buf.push(b' ');
    }
}

/// Append a static template fragment while offsetting indentation for nested renders.
pub(crate) fn write_part_with_indent(buf: &mut Vec<u8>, part: &[u8], indent_offset: usize) {
    if indent_offset == 0 || part.is_empty() {
        buf.extend_from_slice(part);
        return;
    }

    for (pos, &byte) in part.iter().enumerate() {
        buf.push(byte);
        if byte == b'\n' && pos + 1 < part.len() {
            push_indent(buf, indent_offset);
        }
    }
}
