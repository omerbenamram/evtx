use crate::err::{EvtxError, Result};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::xml::{XmlElementBuilder, XmlModel, XmlPIBuilder};
use crate::utils::ByteCursor;
use crate::xml_output::{BinXmlOutput, XmlOutput};
use crate::{ChunkOffset, JsonStreamOutput, template_cache::CompiledTemplateOp};
use log::{debug, trace, warn};
use std::borrow::Cow;
use std::io::Write;

use crate::EvtxChunk;
use crate::binxml::name::{BinXmlName, BinXmlNameRef};
use crate::binxml::tokens::read_template_definition_cursor;

pub fn parse_tokens<'a, T: BinXmlOutput>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
) -> Result<()> {
    let expanded_tokens = expand_templates(tokens, chunk)?;
    let record_model = create_record_model(expanded_tokens, chunk)?;

    visitor.visit_start_of_stream()?;

    let mut stack = vec![];

    for owned_token in record_model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                visitor.visit_open_start_element(stack.last().ok_or_else(|| {
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?)?;
            }
            XmlModel::CloseElement => {
                let close_element = stack.pop().ok_or_else(|| {
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?;
                visitor.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => visitor.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => visitor.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => visitor.visit_entity_reference(&entity)?,
        };
    }

    visitor.visit_end_of_stream()?;

    Ok(())
}

pub fn create_record_model<'a>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
) -> Result<Vec<XmlModel<'a>>> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            BinXMLDeserializedTokens::FragmentHeader(_) => {}
            BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            BinXMLDeserializedTokens::AttributeList => {}
            BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                trace!("BinXMLDeserializedTokens::CloseStartElement");
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(expand_string_ref(&entity.name, chunk)?))
            }
            BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                if current_pi.is_some() {
                    warn!("PITarget without following PIData, previous target will be ignored.")
                }
                builder.name(expand_string_ref(&name.name, chunk)?);
                current_pi = Some(builder);
            }
            BinXMLDeserializedTokens::PIData(data) => match current_pi.take() {
                None => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "PI Data without PI target - Bad parser state",
                    ));
                }
                Some(mut builder) => {
                    builder.data(Cow::Owned(data));
                    model.push(builder.finish());
                }
            },
            BinXMLDeserializedTokens::Substitution(_) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            BinXMLDeserializedTokens::EndOfStream => model.push(XmlModel::EndOfStream),
            BinXMLDeserializedTokens::StartOfStream => model.push(XmlModel::StartOfStream),
            BinXMLDeserializedTokens::CloseEmptyElement => {
                trace!("BinXMLDeserializedTokens::CloseEmptyElement");
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close empty - Bad parser state",
                        ));
                    }
                    Some(builder) => {
                        model.push(XmlModel::OpenElement(builder.finish()?));
                        model.push(XmlModel::CloseElement);
                    }
                };
            }
            BinXMLDeserializedTokens::Attribute(ref attr) => {
                trace!("BinXMLDeserializedTokens::Attribute(attr) - {:?}", attr);
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(expand_string_ref(&attr.name, chunk)?)
                }
            }
            BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                trace!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let mut builder = XmlElementBuilder::new();
                builder.name(expand_string_ref(&elem.name, chunk)?);
                current_element = Some(builder);
            }
            BinXMLDeserializedTokens::Value(value) => {
                trace!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Call `expand_templates` before calling this function",
                            ));
                        }
                        _ => {
                            model.push(XmlModel::Value(Cow::Owned(value)));
                        }
                    },
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
        }
    }

    Ok(model)
}

const BINXML_NAME_LINK_SIZE: u32 = 6;

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

fn expand_token_substitution<'a>(
    template: &BinXmlTemplateRef,
    substitution_descriptor: &TemplateSubstitutionDescriptor,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
    decoded: &mut [Option<BinXMLDeserializedTokens<'a>>],
    remaining_uses: &mut [u32],
) -> Result<()> {
    if substitution_descriptor.ignore {
        return Ok(());
    }
    // NOTE: BinXML substitution indices can be referenced multiple times within a template.
    // We can only move the decoded substitution value on its *last* use; otherwise we must clone.
    let value = take_or_clone_decoded_substitution_value(
        template,
        substitution_descriptor.substitution_index,
        chunk,
        decoded,
        remaining_uses,
    )?;

    _expand_templates(value, chunk, stack)?;

    Ok(())
}

fn take_or_clone_decoded_substitution_value<'a>(
    template: &BinXmlTemplateRef,
    substitution_index: u16,
    chunk: &'a EvtxChunk<'a>,
    decoded: &mut [Option<BinXMLDeserializedTokens<'a>>],
    remaining_uses: &mut [u32],
) -> Result<BinXMLDeserializedTokens<'a>> {
    let idx = substitution_index as usize;

    if idx >= template.substitutions.len() {
        return Ok(BinXMLDeserializedTokens::Value(BinXmlValue::NullType));
    }
    debug_assert!(
        idx < remaining_uses.len() && idx < decoded.len(),
        "remaining_uses/decoded must be sized to substitutions"
    );

    let remaining = remaining_uses[idx];
    debug_assert!(
        remaining > 0,
        "remaining_uses for idx {idx} should be > 0 when expanding a substitution"
    );

    remaining_uses[idx] = remaining.saturating_sub(1);

    // Decode once per substitution index, then move/clone the decoded token as needed.
    if decoded[idx].is_none() {
        let span = &template.substitutions[idx];
        let v = span.decode(chunk)?;
        decoded[idx] = Some(BinXMLDeserializedTokens::Value(v));
    }

    if remaining == 1 {
        Ok(decoded[idx]
            .take()
            .expect("decoded must be populated for an in-bounds substitution"))
    } else {
        Ok(decoded[idx]
            .as_ref()
            .expect("decoded must be populated for an in-bounds substitution")
            .clone())
    }
}

fn expand_template<'a>(
    template: BinXmlTemplateRef,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
) -> Result<()> {
    if let Some(template_def) = chunk
        .template_table
        .get_template(template.template_def_offset)
    {
        let mut remaining_uses = vec![0u32; template.substitutions.len()];
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(desc) = token {
                if desc.ignore {
                    continue;
                }
                let idx = desc.substitution_index as usize;
                if idx < remaining_uses.len() {
                    remaining_uses[idx] += 1;
                }
            }
        }

        // Cache decoded substitutions by index; move on last use, clone otherwise.
        let mut decoded: Vec<Option<BinXMLDeserializedTokens<'a>>> =
            vec![None; template.substitutions.len()];

        // We expect to find all the templates in the template cache.
        // Clone from cache since the cache owns the tokens.
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(
                    &template,
                    substitution_descriptor,
                    chunk,
                    stack,
                    &mut decoded,
                    &mut remaining_uses,
                )?;
            } else {
                _expand_templates(token.clone(), chunk, stack)?;
            }
        }
    } else {
        // If the file was not closed correctly, there can be a template which was not found in the header.
        // In that case, we will try to read it directly from the chunk.
        debug!(
            "Template in offset {} was not found in cache",
            template.template_def_offset
        );

        let mut cursor = ByteCursor::with_pos(chunk.data, template.template_def_offset as usize)?;
        let template_def = read_template_definition_cursor(
            &mut cursor,
            Some(chunk),
            chunk.arena,
            chunk.settings.get_ansi_codec(),
        )?;

        let mut remaining_uses = vec![0u32; template.substitutions.len()];
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(desc) = token {
                if desc.ignore {
                    continue;
                }
                let idx = desc.substitution_index as usize;
                if idx < remaining_uses.len() {
                    remaining_uses[idx] += 1;
                }
            }
        }

        let mut decoded: Vec<Option<BinXMLDeserializedTokens<'a>>> =
            vec![None; template.substitutions.len()];

        for token in template_def.tokens {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(
                    &template,
                    &substitution_descriptor,
                    chunk,
                    stack,
                    &mut decoded,
                    &mut remaining_uses,
                )?;
            } else {
                _expand_templates(token, chunk, stack)?;
            }
        }
    };

    Ok(())
}

fn _expand_templates<'a>(
    token: BinXMLDeserializedTokens<'a>,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
) -> Result<()> {
    match token {
        BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens)) => {
            for token in tokens.into_iter() {
                _expand_templates(token, chunk, stack)?;
            }
        }
        BinXMLDeserializedTokens::TemplateInstance(template) => {
            expand_template(template, chunk, stack)?;
        }
        _ => stack.push(token),
    }

    Ok(())
}

pub fn expand_templates<'a>(
    token_tree: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
) -> Result<Vec<BinXMLDeserializedTokens<'a>>> {
    // We can assume the new tree will be at least as big as the old one.
    let mut stack = Vec::with_capacity(token_tree.len());

    for token in token_tree {
        _expand_templates(token, chunk, &mut stack)?
    }

    Ok(stack)
}

/// JSON-only streaming path with compiled template ops.
///
/// - expand templates without rescanning tokens for substitution counts,
/// - resolve names by offset (no HashMap hashing in `StringCache`),
/// - avoid building `XmlElementBuilder` / `XmlElement` for JSON emission.
pub fn parse_tokens_streaming_json<'a, W: Write>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut JsonStreamOutput<W>,
) -> Result<()> {
    visitor.visit_start_of_stream()?;

    struct Asm<'a, W: Write> {
        chunk: &'a EvtxChunk<'a>,
        out: &'a mut JsonStreamOutput<W>,
        /// Currently open (but not yet closed-start) element tag name offset.
        current_tag: Option<ChunkOffset>,
        /// Current attribute name offset awaiting a value.
        current_attr: Option<ChunkOffset>,
        /// Collected attributes for the current element (name offset + Cow value).
        /// Using `Cow` allows us to store either owned values (from substitutions)
        /// or borrowed values (from template definitions) without cloning.
        attrs: Vec<(ChunkOffset, Cow<'a, BinXmlValue<'a>>)>,
        /// Stack of open element tag name offsets for `CloseElement`.
        tag_stack: Vec<ChunkOffset>,
    }

    impl<'a, W: Write> Asm<'a, W> {
        fn new(chunk: &'a EvtxChunk<'a>, out: &'a mut JsonStreamOutput<W>) -> Self {
            Asm {
                chunk,
                out,
                current_tag: None,
                current_attr: None,
                attrs: Vec::new(),
                tag_stack: Vec::new(),
            }
        }

        #[inline]
        fn open_start_element(&mut self, tag_name_offset: ChunkOffset) {
            self.current_tag = Some(tag_name_offset);
            self.current_attr = None;
            self.attrs.clear();
        }

        #[inline]
        fn attribute_name(&mut self, name_offset: ChunkOffset) {
            self.current_attr = Some(name_offset);
        }

        fn close_start_element(&mut self) -> Result<()> {
            let tag = self.current_tag.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close start - Bad parser state")
            })?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.tag_stack.push(tag);
            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_empty_element(&mut self) -> Result<()> {
            let tag = self.current_tag.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close empty - Bad parser state")
            })?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.out.visit_close_element_offset(self.chunk, tag)?;

            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_element(&mut self) -> Result<()> {
            let tag = self.tag_stack.pop().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close element - Bad parser state")
            })?;
            self.out.visit_close_element_offset(self.chunk, tag)?;
            Ok(())
        }

        fn value_owned(&mut self, value: BinXmlValue<'a>) -> Result<()> {
            // Nested BinXML expands inline - consume tokens by value.
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens.into_iter() {
                    self.token_owned(nested)?;
                }
                return Ok(());
            }

            if self.current_tag.is_some() {
                // Attribute value (only if we have a pending attribute name).
                if let Some(attr_name) = self.current_attr.take() {
                    self.attrs.push((attr_name, Cow::Owned(value)));
                }
                return Ok(());
            }

            // Text node.
            self.out.visit_characters(Cow::Owned(value))?;
            Ok(())
        }

        fn value_ref(&mut self, value: &'a BinXmlValue<'a>) -> Result<()> {
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens.iter() {
                    self.token_ref(nested)?;
                }
                return Ok(());
            }

            if self.current_tag.is_some() {
                if let Some(attr_name) = self.current_attr.take() {
                    self.attrs.push((attr_name, Cow::Borrowed(value)));
                }
                return Ok(());
            }

            self.out.visit_characters(Cow::Borrowed(value))?;
            Ok(())
        }

        fn entity_ref_offset(&mut self, name_offset: ChunkOffset) -> Result<()> {
            if let Some(name) = self.chunk.string_cache.get_cached_string(name_offset) {
                self.out.visit_entity_reference(name)?;
                return Ok(());
            }

            // Fallback: parse the name from the string table entry.
            let name_off = name_offset.checked_add(BINXML_NAME_LINK_SIZE).ok_or(
                EvtxError::FailedToCreateRecordModel("string table offset overflow"),
            )?;
            let mut cursor = ByteCursor::with_pos(self.chunk.data, name_off as usize)?;
            let name = BinXmlName::from_cursor(&mut cursor)?;
            self.out.visit_entity_reference(&name)?;
            Ok(())
        }

        fn emit_substitution(
            &mut self,
            template: &BinXmlTemplateRef,
            cache: &mut [Option<&'a BinXmlValue<'a>>],
            substitution_index: u16,
        ) -> Result<()> {
            let idx = substitution_index as usize;
            if idx >= template.substitutions.len() {
                return self.value_owned(BinXmlValue::NullType);
            }

            let v_ref: &'a BinXmlValue<'a> = if let Some(v) = cache[idx] {
                v
            } else {
                let span = &template.substitutions[idx];
                let v = span.decode(self.chunk)?;
                let v_ref: &'a BinXmlValue<'a> = self.chunk.arena.alloc(v);
                cache[idx] = Some(v_ref);
                v_ref
            };

            self.value_ref(v_ref)
        }

        fn expand_template(&mut self, template: &BinXmlTemplateRef) -> Result<()> {
            if let Some(entry) = self
                .chunk
                .template_table
                .get_entry(template.template_def_offset)
            {
                // Decode substitutions on-demand and cache decoded values by index.
                let mut sub_cache: Vec<Option<&'a BinXmlValue<'a>>> =
                    vec![None; template.substitutions.len()];

                for op in entry.compiled.ops.iter() {
                    match *op {
                        CompiledTemplateOp::FragmentHeader
                        | CompiledTemplateOp::AttributeList
                        | CompiledTemplateOp::StartOfStream
                        | CompiledTemplateOp::EndOfStream => {}
                        CompiledTemplateOp::OpenStartElement { name_offset } => {
                            self.open_start_element(name_offset)
                        }
                        CompiledTemplateOp::Attribute { name_offset } => {
                            self.attribute_name(name_offset)
                        }
                        CompiledTemplateOp::CloseStartElement => self.close_start_element()?,
                        CompiledTemplateOp::CloseEmptyElement => self.close_empty_element()?,
                        CompiledTemplateOp::CloseElement => self.close_element()?,
                        CompiledTemplateOp::EntityRef { name_offset } => {
                            self.entity_ref_offset(name_offset)?
                        }
                        CompiledTemplateOp::PITarget { .. } | CompiledTemplateOp::PIData { .. } => {
                            // JSON streaming doesn't support PI; match existing behavior (errors when encountered).
                            return Err(EvtxError::Unimplemented {
                                name: "processing instructions in JSON streaming".to_string(),
                            });
                        }
                        CompiledTemplateOp::Value { token_index } => {
                            // Values in template definitions are borrowed from the cache (lifetime 'a).
                            let idx = token_index as usize;
                            if let Some(t) = entry.definition.tokens.get(idx) {
                                self.token_ref(t)?;
                            }
                        }
                        CompiledTemplateOp::Substitution {
                            substitution_index,
                            ignore,
                        } => {
                            if ignore {
                                continue;
                            }
                            self.emit_substitution(template, &mut sub_cache, substitution_index)?;
                        }
                        CompiledTemplateOp::Unsupported { token_index } => {
                            // Fallback to original tokens from cache (lifetime 'a).
                            let idx = token_index as usize;
                            if let Some(t) = entry.definition.tokens.get(idx) {
                                self.token_ref(t)?;
                            }
                        }
                    }
                }

                Ok(())
            } else {
                // Template not in cache - read directly from chunk (rare).
                debug!(
                    "Template in offset {} was not found in cache (json fast path)",
                    template.template_def_offset
                );
                let mut cursor =
                    ByteCursor::with_pos(self.chunk.data, template.template_def_offset as usize)?;
                let template_def = read_template_definition_cursor(
                    &mut cursor,
                    Some(self.chunk),
                    self.chunk.arena,
                    self.chunk.settings.get_ansi_codec(),
                )?;

                let mut sub_cache: Vec<Option<&'a BinXmlValue<'a>>> =
                    vec![None; template.substitutions.len()];

                for t in template_def.tokens {
                    match t {
                        BinXMLDeserializedTokens::Substitution(desc) => {
                            if desc.ignore {
                                continue;
                            }
                            self.emit_substitution(template, &mut sub_cache, desc.substitution_index)?;
                        }
                        other => self.token_owned(other)?,
                    }
                }

                Ok(())
            }
        }

        fn token_owned(&mut self, token: BinXMLDeserializedTokens<'a>) -> Result<()> {
            match token {
                BinXMLDeserializedTokens::FragmentHeader(_)
                | BinXMLDeserializedTokens::AttributeList
                | BinXMLDeserializedTokens::StartOfStream
                | BinXMLDeserializedTokens::EndOfStream => {}

                BinXMLDeserializedTokens::OpenStartElement(elem) => {
                    self.open_start_element(elem.name.offset)
                }
                BinXMLDeserializedTokens::Attribute(attr) => self.attribute_name(attr.name.offset),
                BinXMLDeserializedTokens::Value(value) => self.value_owned(value)?,

                BinXMLDeserializedTokens::CloseStartElement => self.close_start_element()?,
                BinXMLDeserializedTokens::CloseEmptyElement => self.close_empty_element()?,
                BinXMLDeserializedTokens::CloseElement => self.close_element()?,

                BinXMLDeserializedTokens::EntityRef(entity) => {
                    self.entity_ref_offset(entity.name.offset)?
                }

                BinXMLDeserializedTokens::TemplateInstance(template) => {
                    self.expand_template(&template)?
                }

                BinXMLDeserializedTokens::PITarget(_) | BinXMLDeserializedTokens::PIData(_) => {
                    return Err(EvtxError::Unimplemented {
                        name: "processing instructions in JSON streaming".to_string(),
                    });
                }
                BinXMLDeserializedTokens::Substitution(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Substitution token should not appear in input stream",
                    ));
                }
                BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unimplemented CDATA/CharRef",
                    ));
                }
            }
            Ok(())
        }

        fn token_ref(&mut self, token: &'a BinXMLDeserializedTokens<'a>) -> Result<()> {
            match token {
                BinXMLDeserializedTokens::FragmentHeader(_)
                | BinXMLDeserializedTokens::AttributeList
                | BinXMLDeserializedTokens::StartOfStream
                | BinXMLDeserializedTokens::EndOfStream => {}

                BinXMLDeserializedTokens::OpenStartElement(elem) => {
                    self.open_start_element(elem.name.offset)
                }
                BinXMLDeserializedTokens::Attribute(attr) => self.attribute_name(attr.name.offset),
                BinXMLDeserializedTokens::Value(value) => self.value_ref(value)?,

                BinXMLDeserializedTokens::CloseStartElement => self.close_start_element()?,
                BinXMLDeserializedTokens::CloseEmptyElement => self.close_empty_element()?,
                BinXMLDeserializedTokens::CloseElement => self.close_element()?,

                BinXMLDeserializedTokens::EntityRef(entity) => {
                    self.entity_ref_offset(entity.name.offset)?
                }

                BinXMLDeserializedTokens::TemplateInstance(template) => {
                    // Template definitions shouldn't contain nested TemplateInstance tokens,
                    // but handle defensively by cloning (rare path).
                    let owned = template.clone();
                    self.expand_template(&owned)?
                }

                BinXMLDeserializedTokens::PITarget(_) | BinXMLDeserializedTokens::PIData(_) => {
                    return Err(EvtxError::Unimplemented {
                        name: "processing instructions in JSON streaming".to_string(),
                    });
                }
                BinXMLDeserializedTokens::Substitution(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Substitution token should not appear in input stream",
                    ));
                }
                BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unimplemented CDATA/CharRef",
                    ));
                }
            }
            Ok(())
        }
    }

    let mut asm = Asm::new(chunk, visitor);
    for token in tokens {
        asm.token_owned(token)?;
    }

    asm.out.visit_end_of_stream()?;
    Ok(())
}

/// XML streaming path with compiled template ops.
///
/// This mirrors `parse_tokens_streaming_json` but emits XML via `XmlOutput` without building an
/// intermediate `XmlModel` / `XmlElementBuilder` per record.
pub fn parse_tokens_streaming_xml<'a, W: Write>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut XmlOutput<W>,
) -> Result<()> {
    visitor.visit_start_of_stream()?;

    struct Asm<'a, W: Write> {
        chunk: &'a EvtxChunk<'a>,
        out: &'a mut XmlOutput<W>,
        /// Currently open (but not yet closed-start) element tag name offset.
        current_tag: Option<ChunkOffset>,
        /// Current attribute name offset awaiting a value.
        current_attr: Option<ChunkOffset>,
        /// Collected attributes for the current element (name offset + Cow value).
        attrs: Vec<(ChunkOffset, Cow<'a, BinXmlValue<'a>>)>,
        /// Stack of open element tag name offsets for `CloseElement`.
        tag_stack: Vec<ChunkOffset>,
        /// Processing-instruction builder (PITarget + PIData).
        current_pi: Option<XmlPIBuilder<'a>>,
    }

    impl<'a, W: Write> Asm<'a, W> {
        fn new(chunk: &'a EvtxChunk<'a>, out: &'a mut XmlOutput<W>) -> Self {
            Asm {
                chunk,
                out,
                current_tag: None,
                current_attr: None,
                attrs: Vec::new(),
                tag_stack: Vec::new(),
                current_pi: None,
            }
        }

        #[inline]
        fn open_start_element(&mut self, tag_name_offset: ChunkOffset) {
            self.current_tag = Some(tag_name_offset);
            self.current_attr = None;
            self.attrs.clear();
        }

        #[inline]
        fn attribute_name(&mut self, name_offset: ChunkOffset) {
            self.current_attr = Some(name_offset);
        }

        fn close_start_element(&mut self) -> Result<()> {
            let tag = self.current_tag.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close start - Bad parser state")
            })?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.tag_stack.push(tag);
            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_empty_element(&mut self) -> Result<()> {
            let tag = self.current_tag.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close empty - Bad parser state")
            })?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.out.visit_close_element_offset(self.chunk, tag)?;

            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_element(&mut self) -> Result<()> {
            let tag = self.tag_stack.pop().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("close element - Bad parser state")
            })?;
            self.out.visit_close_element_offset(self.chunk, tag)?;
            Ok(())
        }

        fn value_owned(&mut self, value: BinXmlValue<'a>) -> Result<()> {
            // Nested BinXML expands inline - consume tokens by value.
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens.into_iter() {
                    self.token_owned(nested)?;
                }
                return Ok(());
            }

            if self.current_tag.is_some() {
                // Attribute value (only if we have a pending attribute name).
                if let Some(attr_name) = self.current_attr.take() {
                    self.attrs.push((attr_name, Cow::Owned(value)));
                }
                return Ok(());
            }

            // Text node.
            self.out.visit_characters(Cow::Owned(value))?;
            Ok(())
        }

        fn value_ref(&mut self, value: &'a BinXmlValue<'a>) -> Result<()> {
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens.iter() {
                    self.token_ref(nested)?;
                }
                return Ok(());
            }

            if self.current_tag.is_some() {
                if let Some(attr_name) = self.current_attr.take() {
                    self.attrs.push((attr_name, Cow::Borrowed(value)));
                }
                return Ok(());
            }

            self.out.visit_characters(Cow::Borrowed(value))?;
            Ok(())
        }

        fn entity_ref_offset(&mut self, name_offset: ChunkOffset) -> Result<()> {
            if let Some(name) = self.chunk.string_cache.get_cached_string(name_offset) {
                self.out.visit_entity_reference(name)?;
                return Ok(());
            }

            // Fallback: parse the name from the string table entry.
            let name_off = name_offset.checked_add(BINXML_NAME_LINK_SIZE).ok_or(
                EvtxError::FailedToCreateRecordModel("string table offset overflow"),
            )?;
            let mut cursor = ByteCursor::with_pos(self.chunk.data, name_off as usize)?;
            let name = BinXmlName::from_cursor(&mut cursor)?;
            self.out.visit_entity_reference(&name)?;
            Ok(())
        }

        fn pi_target_offset(&mut self, name_offset: ChunkOffset) -> Result<()> {
            if self.current_pi.is_some() {
                warn!("PITarget without following PIData, previous target will be ignored.")
            }
            let mut builder = XmlPIBuilder::new();
            // PITarget names are string-table names.
            let name = if let Some(n) = self.chunk.string_cache.get_cached_string(name_offset) {
                Cow::Borrowed(n)
            } else {
                let parsed_off = name_offset.checked_add(BINXML_NAME_LINK_SIZE).ok_or(
                    EvtxError::FailedToCreateRecordModel("string table offset overflow"),
                )?;
                let mut cursor = ByteCursor::with_pos(self.chunk.data, parsed_off as usize)?;
                Cow::Owned(BinXmlName::from_cursor(&mut cursor)?)
            };
            builder.name(name);
            self.current_pi = Some(builder);
            Ok(())
        }

        fn pi_data_owned(&mut self, data: String) -> Result<()> {
            let builder = self.current_pi.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("PI Data without PI target - Bad parser state")
            })?;
            let mut b = builder;
            b.data(Cow::Owned(data));
            match b.finish() {
                XmlModel::PI(pi) => self.out.visit_processing_instruction(&pi).map_err(Into::into),
                _ => Ok(()),
            }
        }

        fn pi_data_ref(&mut self, data: &'a str) -> Result<()> {
            let builder = self.current_pi.take().ok_or_else(|| {
                EvtxError::FailedToCreateRecordModel("PI Data without PI target - Bad parser state")
            })?;
            let mut b = builder;
            b.data(Cow::Borrowed(data));
            match b.finish() {
                XmlModel::PI(pi) => self.out.visit_processing_instruction(&pi).map_err(Into::into),
                _ => Ok(()),
            }
        }

        fn emit_substitution(
            &mut self,
            template: &BinXmlTemplateRef,
            cache: &mut [Option<&'a BinXmlValue<'a>>],
            substitution_index: u16,
        ) -> Result<()> {
            let idx = substitution_index as usize;
            if idx >= template.substitutions.len() {
                return self.value_owned(BinXmlValue::NullType);
            }

            let v_ref: &'a BinXmlValue<'a> = if let Some(v) = cache[idx] {
                v
            } else {
                let span = &template.substitutions[idx];
                let v = span.decode(self.chunk)?;
                let v_ref: &'a BinXmlValue<'a> = self.chunk.arena.alloc(v);
                cache[idx] = Some(v_ref);
                v_ref
            };

            self.value_ref(v_ref)
        }

        fn expand_template(&mut self, template: &BinXmlTemplateRef) -> Result<()> {
            if let Some(entry) = self
                .chunk
                .template_table
                .get_entry(template.template_def_offset)
            {
                let mut sub_cache: Vec<Option<&'a BinXmlValue<'a>>> =
                    vec![None; template.substitutions.len()];

                for op in entry.compiled.ops.iter() {
                    match *op {
                        CompiledTemplateOp::FragmentHeader
                        | CompiledTemplateOp::AttributeList
                        | CompiledTemplateOp::StartOfStream
                        | CompiledTemplateOp::EndOfStream => {}
                        CompiledTemplateOp::OpenStartElement { name_offset } => {
                            self.open_start_element(name_offset)
                        }
                        CompiledTemplateOp::Attribute { name_offset } => {
                            self.attribute_name(name_offset)
                        }
                        CompiledTemplateOp::CloseStartElement => self.close_start_element()?,
                        CompiledTemplateOp::CloseEmptyElement => self.close_empty_element()?,
                        CompiledTemplateOp::CloseElement => self.close_element()?,
                        CompiledTemplateOp::EntityRef { name_offset } => {
                            self.entity_ref_offset(name_offset)?
                        }
                        CompiledTemplateOp::PITarget { name_offset } => {
                            self.pi_target_offset(name_offset)?
                        }
                        CompiledTemplateOp::PIData { token_index } => {
                            let idx = token_index as usize;
                            if let Some(t) = entry.definition.tokens.get(idx) {
                                self.token_ref(t)?;
                            }
                        }
                        CompiledTemplateOp::Value { token_index } => {
                            let idx = token_index as usize;
                            if let Some(t) = entry.definition.tokens.get(idx) {
                                self.token_ref(t)?;
                            }
                        }
                        CompiledTemplateOp::Substitution {
                            substitution_index,
                            ignore,
                        } => {
                            if ignore {
                                continue;
                            }
                            self.emit_substitution(template, &mut sub_cache, substitution_index)?;
                        }
                        CompiledTemplateOp::Unsupported { token_index } => {
                            let idx = token_index as usize;
                            if let Some(t) = entry.definition.tokens.get(idx) {
                                self.token_ref(t)?;
                            }
                        }
                    }
                }

                Ok(())
            } else {
                // Template not in cache - read directly from chunk (rare).
                debug!(
                    "Template in offset {} was not found in cache (xml fast path)",
                    template.template_def_offset
                );
                let mut cursor =
                    ByteCursor::with_pos(self.chunk.data, template.template_def_offset as usize)?;
                let template_def = read_template_definition_cursor(
                    &mut cursor,
                    Some(self.chunk),
                    self.chunk.arena,
                    self.chunk.settings.get_ansi_codec(),
                )?;

                let mut sub_cache: Vec<Option<&'a BinXmlValue<'a>>> =
                    vec![None; template.substitutions.len()];

                for t in template_def.tokens {
                    match t {
                        BinXMLDeserializedTokens::Substitution(desc) => {
                            if desc.ignore {
                                continue;
                            }
                            self.emit_substitution(template, &mut sub_cache, desc.substitution_index)?;
                        }
                        other => self.token_owned(other)?,
                    }
                }

                Ok(())
            }
        }

        fn token_owned(&mut self, token: BinXMLDeserializedTokens<'a>) -> Result<()> {
            match token {
                BinXMLDeserializedTokens::FragmentHeader(_)
                | BinXMLDeserializedTokens::AttributeList
                | BinXMLDeserializedTokens::StartOfStream
                | BinXMLDeserializedTokens::EndOfStream => {}

                BinXMLDeserializedTokens::OpenStartElement(elem) => {
                    self.open_start_element(elem.name.offset)
                }
                BinXMLDeserializedTokens::Attribute(attr) => self.attribute_name(attr.name.offset),
                BinXMLDeserializedTokens::Value(value) => self.value_owned(value)?,

                BinXMLDeserializedTokens::CloseStartElement => self.close_start_element()?,
                BinXMLDeserializedTokens::CloseEmptyElement => self.close_empty_element()?,
                BinXMLDeserializedTokens::CloseElement => self.close_element()?,

                BinXMLDeserializedTokens::EntityRef(entity) => {
                    self.entity_ref_offset(entity.name.offset)?
                }

                BinXMLDeserializedTokens::PITarget(target) => {
                    self.pi_target_offset(target.name.offset)?
                }
                BinXMLDeserializedTokens::PIData(data) => self.pi_data_owned(data)?,

                BinXMLDeserializedTokens::TemplateInstance(template) => {
                    self.expand_template(&template)?
                }

                BinXMLDeserializedTokens::Substitution(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Substitution token should not appear in input stream",
                    ));
                }
                BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unimplemented CDATA/CharRef",
                    ));
                }
            }
            Ok(())
        }

        fn token_ref(&mut self, token: &'a BinXMLDeserializedTokens<'a>) -> Result<()> {
            match token {
                BinXMLDeserializedTokens::FragmentHeader(_)
                | BinXMLDeserializedTokens::AttributeList
                | BinXMLDeserializedTokens::StartOfStream
                | BinXMLDeserializedTokens::EndOfStream => {}

                BinXMLDeserializedTokens::OpenStartElement(elem) => {
                    self.open_start_element(elem.name.offset)
                }
                BinXMLDeserializedTokens::Attribute(attr) => self.attribute_name(attr.name.offset),
                BinXMLDeserializedTokens::Value(value) => self.value_ref(value)?,

                BinXMLDeserializedTokens::CloseStartElement => self.close_start_element()?,
                BinXMLDeserializedTokens::CloseEmptyElement => self.close_empty_element()?,
                BinXMLDeserializedTokens::CloseElement => self.close_element()?,

                BinXMLDeserializedTokens::EntityRef(entity) => {
                    self.entity_ref_offset(entity.name.offset)?
                }

                BinXMLDeserializedTokens::PITarget(target) => {
                    self.pi_target_offset(target.name.offset)?
                }
                BinXMLDeserializedTokens::PIData(data) => self.pi_data_ref(data.as_str())?,

                BinXMLDeserializedTokens::TemplateInstance(template) => {
                    // Template definitions shouldn't contain nested TemplateInstance tokens,
                    // but handle defensively by cloning (rare path).
                    let owned = template.clone();
                    self.expand_template(&owned)?
                }

                BinXMLDeserializedTokens::Substitution(_) => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Substitution token should not appear in input stream",
                    ));
                }
                BinXMLDeserializedTokens::CDATASection | BinXMLDeserializedTokens::CharRef => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "Unimplemented CDATA/CharRef",
                    ));
                }
            }
            Ok(())
        }
    }

    let mut asm = Asm::new(chunk, visitor);
    for token in tokens {
        asm.token_owned(token)?;
    }

    asm.out.visit_end_of_stream()?;
    Ok(())
}

// (unit tests live in `tests/`; this module intentionally has no local tests)
