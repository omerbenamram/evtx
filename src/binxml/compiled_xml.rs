//! Compiled XML templates for fast per-record rendering.
//!
//! Instead of building a full IR tree per record (clone + resolve + walk), this module
//! pre-compiles each BinXml template into static XML byte fragments interleaved with
//! substitution slots. Per-record rendering becomes: write `parts[0]`, format
//! `value[slots[0].sub_id]`, write `parts[1]`, ... — no tree building, no tree walking.
//!
//! Handles TemplateInstance records, inline BinXml records (forwarded events),
//! inner BinXmlType fragments (both TemplateInstance-wrapped and inline token streams),
//! array expansion, processing instructions, and multi-token attribute values.

use crate::ParserSettings;
use crate::binxml::name::{BinXmlNameEncoding, BinXmlNameRef};
use crate::binxml::render_common::{
    FragmentStart, TEMPLATE_DEFINITION_HEADER_SIZE, classify_binxml_fragment, push_indent,
    read_template_definition_ref_at, write_part_with_indent,
};
use crate::binxml::tokens::{
    RawSubValue, read_attribute_cursor, read_entity_ref_cursor, read_fragment_header_cursor,
    read_open_start_element_cursor, read_processing_instruction_data_cursor,
    read_processing_instruction_target_cursor, read_substitution_descriptor_cursor,
};
use crate::binxml::value_render::{RawXmlRenderer, ValueRenderer};
use crate::binxml::value_variant::BinXmlValue;
use crate::evtx_chunk::EvtxChunk;
use crate::model::ir::is_optional_empty;
use crate::string_cache::StringCache;
use crate::utils::ByteCursor;
use encoding::EncodingRef;

/// A substitution slot in a compiled XML template.
struct SubSlot {
    /// Index into the record's substitution value array.
    sub_id: u16,
    /// True if this substitution is optional (type 0x0e).
    optional: bool,
    /// True if this slot appears in an attribute context.
    in_attribute: bool,
    /// Bytes to emit before the formatted value (e.g. ` Name="` for attributes).
    /// Empty if the value appears as element text content with no prefix needed.
    attr_prefix: Vec<u8>,
    /// Bytes to emit after the formatted value (e.g. `"` to close an attribute).
    /// Empty if no suffix needed.
    attr_suffix: Vec<u8>,
    /// Indent level at this slot (number of spaces). Used to offset inner BinXml fragments.
    indent_level: usize,
    /// For element text slots: byte offset in the preceding static part where the
    /// containing element's opening tag starts (including indent/newline).
    /// Used for array expansion to repeat the element.
    repeat_prefix_start: usize,
    /// Length of the containing element's name. Used to construct the close tag
    /// during array expansion. 0 if not applicable.
    element_name_len: u8,
}

/// A compiled XML template — static XML parts interleaved with substitution slots.
pub(crate) struct CompiledXmlTemplate {
    /// N+1 static XML byte fragments for N slots.
    pub(crate) parts: Vec<Vec<u8>>,
    /// Substitution slot metadata.
    slots: Vec<SubSlot>,
}

/// Error type used internally during compilation to signal bail conditions.
enum CompileError {
    /// The template cannot be compiled (e.g. nested templates, unsupported tokens).
    Bail,
}

/// Attempt to compile a BinXml template definition into a `CompiledXmlTemplate`.
///
/// Returns `None` if the template cannot be compiled (bail conditions met).
pub(crate) fn compile_xml_template(
    chunk: &EvtxChunk<'_>,
    template_def_offset: u32,
    settings: &ParserSettings,
) -> Option<CompiledXmlTemplate> {
    compile_xml_template_inner(chunk, template_def_offset, settings).ok()
}

fn compile_xml_template_inner(
    chunk: &EvtxChunk<'_>,
    template_def_offset: u32,
    settings: &ParserSettings,
) -> std::result::Result<CompiledXmlTemplate, CompileError> {
    let data = chunk.data;
    let header = read_template_definition_ref_at(data, template_def_offset)
        .map_err(|_| CompileError::Bail)?;

    let data_start = template_def_offset as usize + TEMPLATE_DEFINITION_HEADER_SIZE;
    let data_end = data_start + header.data_size as usize;
    if data_end > data.len() {
        return Err(CompileError::Bail);
    }

    let indent = settings.should_indent();
    let ansi_codec = settings.get_ansi_codec();
    let mut compiler = TemplateCompiler::new(chunk, ansi_codec, indent, true);
    compiler.compile_bytes(data_start, data_end)?;
    Ok(compiler.finish())
}

/// Stateful compiler that walks template BinXml bytes and builds the compiled template.
struct TemplateCompiler<'a> {
    data: &'a [u8],
    chunk: &'a EvtxChunk<'a>,
    string_cache: &'a StringCache,
    ansi_codec: EncodingRef,
    indent: bool,
    /// True if OpenStartElement tokens include a dependency identifier (u16).
    /// Template definitions have this; inline BinXml fragments (type 0x21 values) do not.
    has_dependency_identifier: bool,
    /// Current accumulating static XML bytes (the "current part").
    current_part: Vec<u8>,
    /// Completed parts so far.
    parts: Vec<Vec<u8>>,
    /// Slots collected so far.
    slots: Vec<SubSlot>,
    /// Element stack for indentation tracking.
    /// Each entry is (element_name_str, has_children_flag).
    element_stack: Vec<StackEntry>,
    /// True if we are currently inside an attribute value.
    in_attribute: bool,
    /// Pending attribute name prefix (` AttrName="`) to emit only if the value is non-empty.
    /// For substitution slots in attributes, we defer the prefix to the slot.
    pending_attr_prefix: Option<Vec<u8>>,
}

struct StackEntry {
    name: String,
    indent_level: usize,
    has_element_child: bool,
    has_any_child: bool,
    children_started: bool,
    /// Byte offset in `current_part` where this element's opening tag starts
    /// (including indent/newline prefix). Used for array expansion.
    open_byte_start: usize,
}

impl<'a> TemplateCompiler<'a> {
    fn new(
        chunk: &'a EvtxChunk<'a>,
        ansi_codec: EncodingRef,
        indent: bool,
        has_dependency_identifier: bool,
    ) -> Self {
        TemplateCompiler {
            data: chunk.data,
            chunk,
            string_cache: &chunk.string_cache,
            ansi_codec,
            indent,
            has_dependency_identifier,
            current_part: Vec::with_capacity(512),
            parts: Vec::new(),
            slots: Vec::new(),
            element_stack: Vec::new(),
            in_attribute: false,
            pending_attr_prefix: None,
        }
    }

    fn compile_bytes(
        &mut self,
        data_start: usize,
        data_end: usize,
    ) -> std::result::Result<(), CompileError> {
        let mut cursor =
            ByteCursor::with_pos(self.data, data_start).map_err(|_| CompileError::Bail)?;
        let data_size = (data_end - data_start) as u32;
        let mut data_read: u32 = 0;
        let mut eof = false;

        while !eof && data_read < data_size {
            let start = cursor.position();
            let token_byte = cursor.u8().map_err(|_| CompileError::Bail)?;

            match token_byte {
                0x00 => {
                    eof = true;
                }
                0x0c => {
                    // Nested TemplateInstance — bail
                    return Err(CompileError::Bail);
                }
                0x0f => {
                    let _ =
                        read_fragment_header_cursor(&mut cursor).map_err(|_| CompileError::Bail)?;
                }
                0x01 => {
                    // OpenStartElement (no attributes)
                    self.handle_open_start_element(&mut cursor, false)?;
                }
                0x41 => {
                    // OpenStartElement (has attributes)
                    self.handle_open_start_element(&mut cursor, true)?;
                }
                0x02 => {
                    // CloseStartElement — finalize element start tag
                    self.handle_close_start_element()?;
                }
                0x03 => {
                    // CloseEmptyElement — self-closing element
                    self.handle_close_empty_element()?;
                }
                0x04 => {
                    // CloseElement — write closing tag
                    self.handle_close_element()?;
                }
                0x06 | 0x46 => {
                    // Attribute
                    self.handle_attribute(&mut cursor)?;
                }
                0x05 | 0x45 => {
                    // Value — inline text value in template definition (rare but possible)
                    self.handle_value(&mut cursor)?;
                }
                0x09 | 0x49 => {
                    // EntityReference
                    self.handle_entity_ref(&mut cursor)?;
                }
                0x0d => {
                    // NormalSubstitution
                    self.handle_substitution(&mut cursor, false)?;
                }
                0x0e => {
                    // OptionalSubstitution
                    self.handle_substitution(&mut cursor, true)?;
                }
                0x0a => {
                    // ProcessingInstructionTarget
                    self.handle_pi_target(&mut cursor)?;
                }
                0x0b => {
                    // ProcessingInstructionData
                    self.handle_pi_data(&mut cursor)?;
                }
                0x07 | 0x47 => {
                    // CDataSection — bail
                    return Err(CompileError::Bail);
                }
                0x08 | 0x48 => {
                    // CharReference — bail
                    return Err(CompileError::Bail);
                }
                _ => {
                    return Err(CompileError::Bail);
                }
            }

            let total_read = cursor.position() - start;
            data_read = data_read.saturating_add(total_read as u32);
        }

        Ok(())
    }

    fn handle_open_start_element(
        &mut self,
        cursor: &mut ByteCursor<'a>,
        has_attributes: bool,
    ) -> std::result::Result<(), CompileError> {
        // Template definitions include a dependency identifier (u16);
        // inline BinXml fragments (type 0x21 values) omit it.
        let open = read_open_start_element_cursor(
            cursor,
            has_attributes,
            self.has_dependency_identifier,
            BinXmlNameEncoding::Offset,
        )
        .map_err(|_| CompileError::Bail)?;

        let name_str = self.resolve_name(&open.name)?;
        let indent_level = self.current_indent_level();

        // Mark parent as having an element child
        if let Some(parent) = self.element_stack.last_mut() {
            if !parent.children_started {
                // First child of parent — write newline after parent's ">"
                if self.indent {
                    self.current_part.push(b'\n');
                }
                parent.children_started = true;
            }
            parent.has_element_child = true;
            parent.has_any_child = true;
        }

        // Record where this element's opening tag begins (including indent)
        let open_byte_start = self.current_part.len();

        // Write indentation
        self.write_indent(indent_level);

        // Write `<Name`
        self.current_part.push(b'<');
        self.current_part.extend_from_slice(name_str.as_bytes());

        self.element_stack.push(StackEntry {
            name: name_str,
            indent_level,
            has_element_child: false,
            has_any_child: false,
            children_started: false,
            open_byte_start,
        });

        Ok(())
    }

    fn handle_attribute(
        &mut self,
        cursor: &mut ByteCursor<'a>,
    ) -> std::result::Result<(), CompileError> {
        // Close any previous attribute that's still open.
        self.close_attribute_if_open();

        let attr = read_attribute_cursor(cursor, BinXmlNameEncoding::Offset)
            .map_err(|_| CompileError::Bail)?;
        let name = self.resolve_name(&attr.name)?;

        // We accumulate the attribute prefix (` Name="`) as pending.
        // If the next token is a substitution, the prefix goes into the slot.
        // Otherwise it goes into current_part directly.
        let mut prefix = Vec::with_capacity(name.len() + 3);
        prefix.push(b' ');
        prefix.extend_from_slice(name.as_bytes());
        prefix.extend_from_slice(b"=\"");
        self.pending_attr_prefix = Some(prefix);
        self.in_attribute = true;

        Ok(())
    }

    fn handle_close_start_element(&mut self) -> std::result::Result<(), CompileError> {
        // Close any open attribute value with its closing quote.
        self.close_attribute_if_open();

        // Write `>`
        self.current_part.push(b'>');

        Ok(())
    }

    fn handle_close_empty_element(&mut self) -> std::result::Result<(), CompileError> {
        self.close_attribute_if_open();

        // Pop the element from stack — it was pushed at open_start
        let entry = self.element_stack.pop().ok_or(CompileError::Bail)?;

        // Write `>` then close tag on same/next line (matching ir_xml behavior)
        // For empty elements, ir_xml writes:
        //   <Tag>\n  </Tag>\n    (for most tags)
        //   <Tag></Tag>\n        (for Binary)
        self.current_part.push(b'>');

        if entry.name == "Binary" {
            self.current_part.extend_from_slice(b"</");
            self.current_part.extend_from_slice(entry.name.as_bytes());
            self.current_part.push(b'>');
            if self.indent {
                self.current_part.push(b'\n');
            }
        } else {
            if self.indent {
                self.current_part.push(b'\n');
            }
            self.write_indent(entry.indent_level);
            self.current_part.extend_from_slice(b"</");
            self.current_part.extend_from_slice(entry.name.as_bytes());
            self.current_part.push(b'>');
            if self.indent {
                self.current_part.push(b'\n');
            }
        }

        Ok(())
    }

    fn handle_close_element(&mut self) -> std::result::Result<(), CompileError> {
        let entry = self.element_stack.pop().ok_or(CompileError::Bail)?;

        if entry.has_element_child {
            // Children were on separate lines — write indent + close tag
            self.write_indent(entry.indent_level);
        } else if !entry.has_any_child && self.indent && entry.name != "Binary" {
            // Empty element using CloseElement (not CloseEmptyElement).
            // Match IR renderer: `>\n  </Tag>\n`
            // Binary is exempted — it renders inline: `<Binary></Binary>`
            self.current_part.push(b'\n');
            self.write_indent(entry.indent_level);
        }
        // Otherwise: text-only children — close tag follows inline.

        self.current_part.extend_from_slice(b"</");
        self.current_part.extend_from_slice(entry.name.as_bytes());
        self.current_part.push(b'>');
        if self.indent {
            self.current_part.push(b'\n');
        }

        Ok(())
    }

    fn handle_entity_ref(
        &mut self,
        cursor: &mut ByteCursor<'a>,
    ) -> std::result::Result<(), CompileError> {
        let entity = read_entity_ref_cursor(cursor, BinXmlNameEncoding::Offset)
            .map_err(|_| CompileError::Bail)?;
        let name = self.resolve_name(&entity.name)?;

        // In attribute context, just emit the pending prefix (without closing quote).
        // The entity ref is part of the attribute value.
        if self.in_attribute {
            if let Some(prefix) = self.pending_attr_prefix.take() {
                self.current_part.extend_from_slice(&prefix);
            }
        } else {
            self.flush_pending_attr_prefix();
        }

        // Mark parent as having child content (for element context)
        if !self.in_attribute
            && let Some(parent) = self.element_stack.last_mut()
        {
            parent.has_any_child = true;
        }

        self.current_part.push(b'&');
        self.current_part.extend_from_slice(name.as_bytes());
        self.current_part.push(b';');

        Ok(())
    }

    fn handle_pi_target(
        &mut self,
        cursor: &mut ByteCursor<'a>,
    ) -> std::result::Result<(), CompileError> {
        let target = read_processing_instruction_target_cursor(cursor, BinXmlNameEncoding::Offset)
            .map_err(|_| CompileError::Bail)?;
        let name = self.resolve_name(&target.name)?;

        // Mark parent as having child content
        if let Some(parent) = self.element_stack.last_mut() {
            parent.has_any_child = true;
        }

        self.current_part.extend_from_slice(b"<?");
        self.current_part.extend_from_slice(name.as_bytes());

        Ok(())
    }

    fn handle_pi_data(
        &mut self,
        cursor: &mut ByteCursor<'a>,
    ) -> std::result::Result<(), CompileError> {
        let data = read_processing_instruction_data_cursor(cursor)
            .map_err(|_| CompileError::Bail)?
            .to_string()
            .map_err(|_| CompileError::Bail)?;

        if !data.is_empty() {
            self.current_part.push(b' ');
            self.current_part.extend_from_slice(data.as_bytes());
        }
        self.current_part.extend_from_slice(b"?>");

        Ok(())
    }

    fn handle_value(
        &mut self,
        cursor: &mut ByteCursor<'a>,
    ) -> std::result::Result<(), CompileError> {
        // Inline values in template definitions — parse the value and render it as static XML.
        let in_attr = self.in_attribute;
        let value = BinXmlValue::from_binxml_cursor_in(
            cursor,
            Some(self.chunk),
            None,
            self.ansi_codec,
            &self.chunk.arena,
        )
        .map_err(|_| CompileError::Bail)?;

        if in_attr {
            // Attribute context: emit prefix (if pending), then render value.
            // Do NOT close the attribute quote here — the attribute value may be a
            // sequence of Value/EntityRef/CharRef tokens. The closing `"` is handled
            // when the attribute ends (at CloseStartElement, CloseEmptyElement, or
            // the next Attribute token).
            if is_optional_empty(&value) {
                // Empty value in attribute — skip (don't emit prefix).
                // The attribute close is handled by flush_pending_attr_prefix later.
            } else {
                if let Some(prefix) = self.pending_attr_prefix.take() {
                    self.current_part.extend_from_slice(&prefix);
                }
                let mut renderer = ValueRenderer::new();
                renderer
                    .write_xml_value_text(&mut self.current_part, &value, true)
                    .map_err(|_| CompileError::Bail)?;
            }
        } else {
            self.flush_pending_attr_prefix();

            // Mark parent as having child content
            if let Some(parent) = self.element_stack.last_mut() {
                parent.has_any_child = true;
            }

            // Render the value directly into the static part buffer
            let mut renderer = ValueRenderer::new();
            renderer
                .write_xml_value_text(&mut self.current_part, &value, false)
                .map_err(|_| CompileError::Bail)?;
        }

        Ok(())
    }

    fn handle_substitution(
        &mut self,
        cursor: &mut ByteCursor<'a>,
        optional: bool,
    ) -> std::result::Result<(), CompileError> {
        let descriptor = read_substitution_descriptor_cursor(cursor, optional)
            .map_err(|_| CompileError::Bail)?;
        let sub_index = descriptor.substitution_index;

        if self.in_attribute {
            // Attribute context: the pending_attr_prefix becomes the slot's prefix,
            // and `"` becomes the slot's suffix. The attribute is only emitted if the
            // value is non-empty (for optional subs).
            let attr_prefix = self.pending_attr_prefix.take().unwrap_or_default();
            let attr_suffix = b"\"".to_vec();

            // End current part, create slot
            let finished_part = std::mem::replace(&mut self.current_part, Vec::with_capacity(256));
            self.parts.push(finished_part);
            let indent_level = self.current_indent_level();
            self.slots.push(SubSlot {
                sub_id: sub_index,
                optional,
                in_attribute: true,
                attr_prefix,
                attr_suffix,
                indent_level,
                repeat_prefix_start: 0,
                element_name_len: 0,
            });

            // The attribute is now "consumed" — the close quote is in the slot suffix
            self.in_attribute = false;
        } else {
            // Element text context
            self.flush_pending_attr_prefix();

            // Mark parent as having child content
            if let Some(parent) = self.element_stack.last_mut() {
                parent.has_any_child = true;
            }

            // Record element repeat info for array expansion.
            let (repeat_prefix_start, element_name_len) =
                if let Some(entry) = self.element_stack.last() {
                    (entry.open_byte_start, entry.name.len().min(255) as u8)
                } else {
                    (0, 0)
                };

            let indent_level = self.current_indent_level();
            let finished_part = std::mem::replace(&mut self.current_part, Vec::with_capacity(256));
            self.parts.push(finished_part);
            self.slots.push(SubSlot {
                sub_id: sub_index,
                optional,
                in_attribute: false,
                attr_prefix: Vec::new(),
                attr_suffix: Vec::new(),
                indent_level,
                repeat_prefix_start,
                element_name_len,
            });
        }

        Ok(())
    }

    /// Close an open attribute: flush any pending prefix (empty attribute) or
    /// just add the closing `"` if content was already written.
    fn close_attribute_if_open(&mut self) {
        if self.in_attribute {
            if let Some(prefix) = self.pending_attr_prefix.take() {
                // No content was written for this attribute — emit prefix + closing quote.
                self.current_part.extend_from_slice(&prefix);
            }
            self.current_part.push(b'"');
            self.in_attribute = false;
        }
    }

    fn flush_pending_attr_prefix(&mut self) {
        if let Some(prefix) = self.pending_attr_prefix.take() {
            self.current_part.extend_from_slice(&prefix);
            // Close the attribute value quote
            self.current_part.push(b'"');
            self.in_attribute = false;
        }
    }

    fn resolve_name(&self, name_ref: &BinXmlNameRef) -> std::result::Result<String, CompileError> {
        if let Some(s) = self.string_cache.get_cached_string(name_ref.offset) {
            return Ok(s.as_str().to_string());
        }
        // Fail-soft fallback: read the name directly from chunk data.
        // The name starts 6 bytes after the offset (past the BinXmlNameLink).
        let name_off = name_ref.offset.checked_add(6).ok_or(CompileError::Bail)?;
        let mut cursor =
            ByteCursor::with_pos(self.data, name_off as usize).map_err(|_| CompileError::Bail)?;
        cursor
            .len_prefixed_utf16_string_utf8(true, "name")
            .map_err(|_| CompileError::Bail)?
            .ok_or(CompileError::Bail)
    }

    fn current_indent_level(&self) -> usize {
        self.element_stack.len() * 2
    }

    fn write_indent(&mut self, level: usize) {
        if !self.indent {
            return;
        }
        push_indent(&mut self.current_part, level);
    }

    fn finish(mut self) -> CompiledXmlTemplate {
        // Push the final part
        self.parts.push(self.current_part);
        CompiledXmlTemplate {
            parts: self.parts,
            slots: self.slots,
        }
    }
}

// ---------------------------------------------------------------------------
// Raw value rendering — format substitution values directly from chunk bytes
// without constructing intermediate BinXmlValue enums.
// ---------------------------------------------------------------------------

/// Context for raw rendering, needed for recursive BinXmlType fragment handling.
pub(crate) struct RawRenderContext<'a, 'c> {
    pub chunk: &'a EvtxChunk<'a>,
    pub cache: &'c mut crate::binxml::ir::IrTemplateCache<'a>,
    pub settings: &'c ParserSettings,
}

/// Render a compiled XML template using raw (unparsed) substitution descriptors.
///
/// Same structure as `render_compiled_xml` but reads value bytes directly from
/// `chunk_data` via `RawSubValue` offsets instead of pre-parsed `BinXmlValue` enums.
///
/// Returns `true` on success, `false` if a bail condition is encountered
/// (EvtHandle, EvtXml) — the caller should report an error.
pub(crate) fn render_compiled_xml_raw(
    template: &CompiledXmlTemplate,
    raw_values: &[RawSubValue],
    chunk_data: &[u8],
    buf: &mut Vec<u8>,
    ctx: &mut RawRenderContext<'_, '_>,
    indent: bool,
    indent_offset: usize,
) -> bool {
    let mut raw_renderer = RawXmlRenderer::new();

    // After rendering a BinXmlType value, the close tag of the parent element
    // needs indentation. Track this across iterations.
    let mut binxml_close_indent: Option<usize> = None;

    // After array expansion, skip this many bytes at the start of the next static part
    // (the original close tag that was replaced by the repeated element close tags).
    let mut skip_next_part_prefix: usize = 0;

    for (i, slot) in template.slots.iter().enumerate() {
        // If previous slot was BinXmlType, add indent before the close tag
        if let Some(amt) = binxml_close_indent.take() {
            push_indent(buf, amt);
        }

        // Write static part before this slot (skipping bytes consumed by previous array expansion)
        let part = &template.parts[i];
        let skip = skip_next_part_prefix.min(part.len());
        skip_next_part_prefix = 0;
        write_part_with_indent(buf, &part[skip..], indent_offset);

        let rv = match raw_values.get(slot.sub_id as usize) {
            Some(rv) => rv,
            None => {
                // Missing substitution — treat as empty optional
                handle_raw_optional_empty(template, buf, slot, i, indent, indent_offset);
                continue;
            }
        };

        // Bail on types we can't render from raw bytes.
        match rv.value_type {
            0x20 | 0x23 => return false,
            _ => {}
        }

        // Bounds check and extract raw bytes early so we can check actual content.
        let end = rv.offset + rv.size as usize;
        if end > chunk_data.len() {
            return false;
        }
        let raw = &chunk_data[rv.offset..end];

        // Content-aware empty check: NullType is always empty; strings check
        // for NUL-only content (matching the IR parser's trimming behavior).
        // For array types, use the original type (non-empty raw = non-empty value),
        // matching the IR path where StringArrayType is never considered empty.
        let is_empty = !RawXmlRenderer::value_has_content(raw, rv.value_type);

        // For optional empty substitutions in indented mode, add `\n` + indent
        // before the close tag to match the IR renderer's empty element formatting.
        // For non-optional empty, just skip (close tag follows inline).
        if slot.optional && is_empty {
            handle_raw_optional_empty(template, buf, slot, i, indent, indent_offset);
            continue;
        }
        if is_empty {
            continue;
        }

        // Handle BinXmlType (0x21) — recursively compile and render inner fragment
        if rv.value_type == 0x21 {
            // BinXmlType expands to element children. Add `\n` before the
            // inner content if the preceding static part didn't end with one.
            if indent && !slot.in_attribute && !template.parts[i].ends_with(b"\n") {
                buf.push(b'\n');
            }

            let fragment_indent = if indent {
                indent_offset + slot.indent_level
            } else {
                0
            };
            if !render_binxml_fragment_raw(raw, buf, ctx, fragment_indent) {
                return false;
            }

            // After the fragment, the close tag in the next static part needs indent.
            if indent && !slot.in_attribute {
                binxml_close_indent = Some(indent_offset + slot.indent_level.saturating_sub(2));
            }
            continue;
        }

        // Handle array types (0x80+) — element repetition per MS-EVEN6 §3.1.4.7.5
        if rv.value_type >= 0x80 {
            let base_type = rv.value_type & 0x7F;
            let items = RawXmlRenderer::split_array_items(raw, base_type);

            if items.len() <= 1 {
                // Single-item array: render as scalar value
                if let Some(&item) = items.first()
                    && raw_renderer
                        .write_xml_value(buf, item, base_type, item.len() as u16, slot.in_attribute)
                        .is_err()
                {
                    return false;
                }
                continue;
            }

            // Multi-item array: repeat the containing element for each item.
            // The element opening tag is at the end of parts[i].
            let raw_opening = &part[slot.repeat_prefix_start..];

            // Undo the element opening that was written as part of parts[i].
            // Since raw_opening has no internal \n, rendered length == raw length.
            buf.truncate(buf.len() - raw_opening.len());

            // Render each item as a repeated element
            for (j, &item) in items.iter().enumerate() {
                if j > 0 && indent {
                    buf.push(b'\n');
                    push_indent(buf, indent_offset);
                }
                buf.extend_from_slice(raw_opening);

                let name_bytes = extract_element_name(raw_opening);
                let item_has_content = RawXmlRenderer::value_has_content(item, base_type);

                if item_has_content {
                    if raw_renderer
                        .write_xml_value(buf, item, base_type, item.len() as u16, slot.in_attribute)
                        .is_err()
                    {
                        return false;
                    }
                } else if indent {
                    // Empty item: IR path uses Omit which produces empty element
                    // formatting with \n + indent before the close tag.
                    buf.push(b'\n');
                    let close_indent = indent_offset + slot.indent_level.saturating_sub(2);
                    push_indent(buf, close_indent);
                }

                // Write close tag </Name>
                buf.extend_from_slice(b"</");
                buf.extend_from_slice(name_bytes);
                buf.push(b'>');
            }

            // Write \n + indent_offset after last element (replaces the skipped \n in parts[i+1])
            if indent {
                buf.push(b'\n');
                push_indent(buf, indent_offset);
            }

            // Skip the original close tag + \n in parts[i+1]
            let name_len = slot.element_name_len as usize;
            skip_next_part_prefix = 2 + name_len + 1 + if indent { 1 } else { 0 }; // </Name>\n
            continue;
        }

        // Write prefix (attribute opening like ` Name="`)
        if !slot.attr_prefix.is_empty() {
            buf.extend_from_slice(&slot.attr_prefix);
        }

        // Format the value from raw bytes
        if raw_renderer
            .write_xml_value(buf, raw, rv.value_type, rv.size, slot.in_attribute)
            .is_err()
        {
            return false;
        }

        // Write suffix (attribute closing like `"`)
        if !slot.attr_suffix.is_empty() {
            buf.extend_from_slice(&slot.attr_suffix);
        }
    }

    // Write the final static part
    if let Some(amt) = binxml_close_indent.take() {
        push_indent(buf, amt);
    }
    if let Some(last_part) = template.parts.last() {
        let skip = skip_next_part_prefix.min(last_part.len());
        write_part_with_indent(buf, &last_part[skip..], indent_offset);
    }

    true
}

/// Render an embedded BinXml fragment using the raw path.
///
/// The fragment bytes are the inner BinXml content from a BinXmlType substitution.
/// Returns `true` on success, `false` on bail.
fn render_binxml_fragment_raw(
    inner_bytes: &[u8],
    buf: &mut Vec<u8>,
    ctx: &mut RawRenderContext<'_, '_>,
    indent_offset: usize,
) -> bool {
    let template_token_offset = match classify_binxml_fragment(inner_bytes) {
        Some(FragmentStart::TemplateInstance { token_offset }) => token_offset,
        Some(FragmentStart::InlineTokens { token_offset }) => {
            return render_inline_tokens_compiled(
                &inner_bytes[token_offset..],
                buf,
                ctx.chunk,
                ctx.settings,
                indent_offset,
                true,
            );
        }
        None => return false,
    };

    // Compute absolute offset of inner_bytes within chunk data
    let inner_abs_offset = {
        let data_start = ctx.chunk.data.as_ptr() as usize;
        let inner_start = inner_bytes.as_ptr() as usize;
        inner_start - data_start
    };

    // Cursor positioned after the 0x0c token byte
    let mut cursor =
        match ByteCursor::with_pos(ctx.chunk.data, inner_abs_offset + template_token_offset + 1) {
            Ok(c) => c,
            Err(_) => return false,
        };

    // Read raw value descriptors for the inner template
    let mut inner_raw_values = Vec::with_capacity(16);
    let template_def_offset =
        match crate::binxml::tokens::read_template_raw_values(&mut cursor, &mut inner_raw_values) {
            Ok(off) => off,
            Err(_) => return false,
        };

    // Look up or compile the inner template
    let compiled =
        match ctx
            .cache
            .get_or_compile_xml_template(ctx.chunk, template_def_offset, ctx.settings)
        {
            Some(c) => c,
            None => return false,
        };

    let indent = ctx.settings.should_indent();

    // Add indent at the start of the fragment (first line)
    push_indent(buf, indent_offset);

    // Render recursively
    render_compiled_xml_raw(
        &compiled,
        &inner_raw_values,
        ctx.chunk.data,
        buf,
        ctx,
        indent,
        indent_offset,
    )
}

/// Render a record whose BinXml content is not wrapped in a TemplateInstance.
///
/// Some EVTX files (notably forwarded events) have records where the FragmentHeader
/// is followed directly by OpenStartElement tokens instead of a TemplateInstance.
/// These records contain fully-expanded BinXml with no substitution slots.
pub(crate) fn render_record_inline_tokens<'a>(
    inner_bytes: &[u8],
    buf: &mut Vec<u8>,
    chunk: &'a EvtxChunk<'a>,
    _ir_cache: &mut crate::binxml::ir::IrTemplateCache<'a>,
    settings: &ParserSettings,
) -> bool {
    render_inline_tokens_compiled(inner_bytes, buf, chunk, settings, 0, false)
}

/// Compile and render inline BinXml tokens that are not wrapped in a TemplateInstance.
fn render_inline_tokens_compiled<'a>(
    inner_bytes: &[u8],
    buf: &mut Vec<u8>,
    chunk: &'a EvtxChunk<'a>,
    settings: &ParserSettings,
    indent_offset: usize,
    add_initial_indent: bool,
) -> bool {
    let inner_abs_offset = {
        let data_start = chunk.data.as_ptr() as usize;
        let inner_start = inner_bytes.as_ptr() as usize;
        inner_start - data_start
    };

    let data_start = inner_abs_offset;
    let data_end = inner_abs_offset + inner_bytes.len();

    let indent = settings.should_indent();
    let ansi_codec = settings.get_ansi_codec();
    let mut compiler = TemplateCompiler::new(chunk, ansi_codec, indent, false);
    if compiler.compile_bytes(data_start, data_end).is_err() {
        return false;
    }
    let compiled = compiler.finish();

    // Inline token streams should not carry substitution slots in this path.
    if !compiled.slots.is_empty() {
        return false;
    }

    if add_initial_indent {
        push_indent(buf, indent_offset);
    }

    for part in &compiled.parts {
        write_part_with_indent(buf, part, indent_offset);
    }
    true
}

/// Handle the empty-optional indentation logic for raw rendering.
#[inline]
fn handle_raw_optional_empty(
    template: &CompiledXmlTemplate,
    buf: &mut Vec<u8>,
    slot: &SubSlot,
    slot_index: usize,
    indent: bool,
    indent_offset: usize,
) {
    if indent
        && !slot.in_attribute
        && buf.ends_with(b">")
        && let Some(next_part) = template.parts.get(slot_index + 1)
        && next_part.starts_with(b"</")
        && !next_part.starts_with(b"</Binary>")
    {
        buf.push(b'\n');
        let close_indent = indent_offset + slot.indent_level.saturating_sub(2);
        push_indent(buf, close_indent);
    }
}

/// Extract the element name from a raw opening tag like `  <Data Name="Foo">`.
/// Returns the name bytes between the first `<` and the first ` ` or `>`.
fn extract_element_name(raw_opening: &[u8]) -> &[u8] {
    let start = raw_opening
        .iter()
        .position(|&b| b == b'<')
        .map(|p| p + 1)
        .unwrap_or(0);
    let end = raw_opening[start..]
        .iter()
        .position(|&b| b == b' ' || b == b'>')
        .map(|p| start + p)
        .unwrap_or(raw_opening.len());
    &raw_opening[start..end]
}
