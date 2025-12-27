use crate::err::{EvtxError, Result};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::xml::{XmlElementBuilder, XmlModel, XmlPIBuilder};
use crate::utils::ByteCursor;
use crate::xml_output::BinXmlOutput;
use crate::{ChunkOffset, JsonStreamOutput, template_cache::CompiledTemplateOp};
use log::{debug, trace, warn};
use std::borrow::Cow;
use std::io::Write;

use std::mem;

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
                visitor.visit_open_start_element(stack.last().ok_or({
                    EvtxError::FailedToCreateRecordModel(
                        "Invalid parser state - expected stack to be non-empty",
                    )
                })?)?;
            }
            XmlModel::CloseElement => {
                let close_element = stack.pop().ok_or({
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
    template: &mut BinXmlTemplateRef<'a>,
    substitution_descriptor: &TemplateSubstitutionDescriptor,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
    remaining_uses: &mut [u32],
) -> Result<()> {
    if substitution_descriptor.ignore {
        return Ok(());
    }
    // NOTE: BinXML substitution indices can be referenced multiple times within a template.
    // We can only move the substitution value on its *last* use; otherwise we must clone.
    let value = take_or_clone_substitution_value(
        template,
        substitution_descriptor.substitution_index,
        remaining_uses,
    );

    _expand_templates(value, chunk, stack)?;

    Ok(())
}

fn take_or_clone_substitution_value<'a>(
    template: &mut BinXmlTemplateRef<'a>,
    substitution_index: u16,
    remaining_uses: &mut [u32],
) -> BinXMLDeserializedTokens<'a> {
    let idx = substitution_index as usize;

    if idx >= template.substitution_array.len() {
        return BinXMLDeserializedTokens::Value(BinXmlValue::NullType);
    }
    debug_assert!(
        idx < remaining_uses.len(),
        "remaining_uses must be sized to substitution_array"
    );

    let remaining = remaining_uses[idx];
    debug_assert!(
        remaining > 0,
        "remaining_uses for idx {idx} should be > 0 when expanding a substitution"
    );

    remaining_uses[idx] = remaining.saturating_sub(1);

    if remaining == 1 {
        mem::replace(
            &mut template.substitution_array[idx],
            BinXMLDeserializedTokens::Value(BinXmlValue::NullType),
        )
    } else {
        template.substitution_array[idx].clone()
    }
}

fn expand_template<'a>(
    mut template: BinXmlTemplateRef<'a>,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
) -> Result<()> {
    if let Some(template_def) = chunk
        .template_table
        .get_template(template.template_def_offset)
    {
        let mut remaining_uses = vec![0u32; template.substitution_array.len()];
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

        // We expect to find all the templates in the template cache.
        // Clone from cache since the cache owns the tokens.
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(
                    &mut template,
                    substitution_descriptor,
                    chunk,
                    stack,
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

        let mut remaining_uses = vec![0u32; template.substitution_array.len()];
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

        for token in template_def.tokens {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(
                    &mut template,
                    &substitution_descriptor,
                    chunk,
                    stack,
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
            let tag = self
                .current_tag
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close start - Bad parser state",
                ))?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.tag_stack.push(tag);
            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_empty_element(&mut self) -> Result<()> {
            let tag = self
                .current_tag
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close empty - Bad parser state",
                ))?;

            self.out
                .visit_open_start_element_offsets(self.chunk, tag, &self.attrs)?;
            self.out.visit_close_element_offset(self.chunk, tag)?;

            self.current_attr = None;
            self.attrs.clear();
            Ok(())
        }

        fn close_element(&mut self) -> Result<()> {
            let tag = self
                .tag_stack
                .pop()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close element - Bad parser state",
                ))?;
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

        fn expand_template(&mut self, template: &mut BinXmlTemplateRef<'a>) -> Result<()> {
            if let Some(entry) = self
                .chunk
                .template_table
                .get_entry(template.template_def_offset)
            {
                // Copy precomputed substitution use-counts (avoid rescanning template tokens).
                let mut remaining_uses = vec![0u32; template.substitution_array.len()];
                let counts = &entry.compiled.substitution_use_counts;
                let n = remaining_uses.len().min(counts.len());
                remaining_uses[..n].copy_from_slice(&counts[..n]);

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
                            // Substitutions are moved/cloned from the owned substitution array.
                            let token = take_or_clone_substitution_value(
                                template,
                                substitution_index,
                                &mut remaining_uses,
                            );
                            self.token_owned(token)?;
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

                // Count substitution uses for take_or_clone.
                let mut remaining_uses = vec![0u32; template.substitution_array.len()];
                for t in &template_def.tokens {
                    if let BinXMLDeserializedTokens::Substitution(desc) = t {
                        if desc.ignore {
                            continue;
                        }
                        let idx = desc.substitution_index as usize;
                        if idx < remaining_uses.len() {
                            remaining_uses[idx] += 1;
                        }
                    }
                }

                for t in template_def.tokens {
                    match t {
                        BinXMLDeserializedTokens::Substitution(desc) => {
                            if desc.ignore {
                                continue;
                            }
                            let token = take_or_clone_substitution_value(
                                template,
                                desc.substitution_index,
                                &mut remaining_uses,
                            );
                            self.token_owned(token)?;
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

                BinXMLDeserializedTokens::TemplateInstance(mut template) => {
                    self.expand_template(&mut template)?
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
                    let mut owned = template.clone();
                    self.expand_template(&mut owned)?
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

#[cfg(test)]
mod tests {
    use super::*;
    use bumpalo::Bump;
    use bumpalo::collections::String as BumpString;

    #[test]
    fn repeated_template_substitution_index_preserves_value() {
        let arena = Bump::new();
        let s = BumpString::from_str_in("hello", &arena);

        let mut template = BinXmlTemplateRef {
            template_id: 0,
            template_def_offset: 0,
            template_guid: None,
            substitution_array: vec![BinXMLDeserializedTokens::Value(BinXmlValue::StringType(s))],
        };

        // Simulate a template definition that references substitution index 0 twice.
        let mut remaining_uses = vec![2u32];

        let first = take_or_clone_substitution_value(&mut template, 0u16, &mut remaining_uses);
        let second = take_or_clone_substitution_value(&mut template, 0u16, &mut remaining_uses);

        assert_eq!(remaining_uses[0], 0);

        match first {
            BinXMLDeserializedTokens::Value(BinXmlValue::StringType(s)) => {
                assert_eq!(s.as_str(), "hello")
            }
            other => panic!("expected StringType, got {other:?}"),
        }

        match second {
            BinXMLDeserializedTokens::Value(BinXmlValue::StringType(s)) => {
                assert_eq!(s.as_str(), "hello")
            }
            other => panic!("expected StringType, got {other:?}"),
        }

        // The last use moves the value out; leaving NullType behind is fine (no further uses).
        assert!(matches!(
            template.substitution_array[0],
            BinXMLDeserializedTokens::Value(BinXmlValue::NullType)
        ));
    }
}
