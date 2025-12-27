use crate::err::{EvtxError, Result};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
use crate::utils::ByteCursor;
use crate::xml_output::BinXmlOutput;
use log::{debug, trace, warn};
use std::borrow::Cow;

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

fn stream_expand_token<'a, T: BinXmlOutput>(
    token: BinXMLDeserializedTokens<'a>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
    element_stack: &mut Vec<XmlElement<'a>>,
    current_element: &mut Option<XmlElementBuilder<'a>>,
    current_pi: &mut Option<XmlPIBuilder<'a>>,
) -> Result<()> {
    match token {
        BinXMLDeserializedTokens::FragmentHeader(_) | BinXMLDeserializedTokens::AttributeList => {}
        BinXMLDeserializedTokens::OpenStartElement(elem) => {
            let mut builder = XmlElementBuilder::new();
            builder.name(expand_string_ref(&elem.name, chunk)?);
            *current_element = Some(builder);
        }
        BinXMLDeserializedTokens::Attribute(attr) => {
            if let Some(b) = current_element.as_mut() {
                b.attribute_name(expand_string_ref(&attr.name, chunk)?);
            } else {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "attribute - Bad parser state",
                ));
            }
        }
        BinXMLDeserializedTokens::Value(value) => {
            // Handle BinXmlType by expanding nested tokens inline
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens {
                    stream_expand_token(
                        nested,
                        chunk,
                        visitor,
                        element_stack,
                        current_element,
                        current_pi,
                    )?;
                }
            } else if let Some(b) = current_element.as_mut() {
                b.attribute_value(Cow::Owned(value))?;
            } else {
                visitor.visit_characters(Cow::Owned(value))?;
            }
        }
        BinXMLDeserializedTokens::CloseStartElement => {
            let element = current_element
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close start - Bad parser state",
                ))?
                .finish()?;
            visitor.visit_open_start_element(&element)?;
            element_stack.push(element);
        }
        BinXMLDeserializedTokens::CloseEmptyElement => {
            let element = current_element
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close empty - Bad parser state",
                ))?
                .finish()?;
            visitor.visit_open_start_element(&element)?;
            visitor.visit_close_element(&element)?;
        }
        BinXMLDeserializedTokens::CloseElement => {
            let element = element_stack
                .pop()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close element - Bad parser state",
                ))?;
            visitor.visit_close_element(&element)?;
        }
        BinXMLDeserializedTokens::EntityRef(entity) => {
            match expand_string_ref(&entity.name, chunk)? {
                Cow::Borrowed(s) => visitor.visit_entity_reference(s)?,
                Cow::Owned(s) => {
                    let tmp = s;
                    visitor.visit_entity_reference(&tmp)?;
                }
            }
        }
        BinXMLDeserializedTokens::PITarget(name) => {
            let mut b = XmlPIBuilder::new();
            b.name(expand_string_ref(&name.name, chunk)?);
            *current_pi = Some(b);
        }
        BinXMLDeserializedTokens::PIData(data) => {
            let mut b = current_pi
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "PI Data without PI target - Bad parser state",
                ))?;
            b.data(Cow::Owned(data));
            if let XmlModel::PI(pi) = b.finish() {
                visitor.visit_processing_instruction(&pi)?;
            }
        }
        BinXMLDeserializedTokens::StartOfStream | BinXMLDeserializedTokens::EndOfStream => {}
        BinXMLDeserializedTokens::TemplateInstance(mut template) => {
            stream_expand_template(
                &mut template,
                chunk,
                visitor,
                element_stack,
                current_element,
                current_pi,
            )?;
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

/// Streaming expansion for borrowed tokens (e.g. template cache tokens).
///
/// This avoids cloning `BinXMLDeserializedTokens` / `BinXmlValue` on the hot path.
fn stream_expand_token_ref<'a, T: BinXmlOutput>(
    token: &'a BinXMLDeserializedTokens<'a>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
    element_stack: &mut Vec<XmlElement<'a>>,
    current_element: &mut Option<XmlElementBuilder<'a>>,
    current_pi: &mut Option<XmlPIBuilder<'a>>,
) -> Result<()> {
    match token {
        BinXMLDeserializedTokens::FragmentHeader(_) | BinXMLDeserializedTokens::AttributeList => {}
        BinXMLDeserializedTokens::OpenStartElement(elem) => {
            let mut builder = XmlElementBuilder::new();
            builder.name(expand_string_ref(&elem.name, chunk)?);
            *current_element = Some(builder);
        }
        BinXMLDeserializedTokens::Attribute(attr) => {
            if let Some(b) = current_element.as_mut() {
                b.attribute_name(expand_string_ref(&attr.name, chunk)?);
            } else {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "attribute - Bad parser state",
                ));
            }
        }
        BinXMLDeserializedTokens::Value(value) => {
            // Handle BinXmlType by expanding nested tokens inline
            if let BinXmlValue::BinXmlType(nested_tokens) = value {
                for nested in nested_tokens.iter() {
                    stream_expand_token_ref(
                        nested,
                        chunk,
                        visitor,
                        element_stack,
                        current_element,
                        current_pi,
                    )?;
                }
            } else if let Some(b) = current_element.as_mut() {
                b.attribute_value(Cow::Borrowed(value))?;
            } else {
                visitor.visit_characters(Cow::Borrowed(value))?;
            }
        }
        BinXMLDeserializedTokens::CloseStartElement => {
            let element = current_element
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close start - Bad parser state",
                ))?
                .finish()?;
            visitor.visit_open_start_element(&element)?;
            element_stack.push(element);
        }
        BinXMLDeserializedTokens::CloseEmptyElement => {
            let element = current_element
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close empty - Bad parser state",
                ))?
                .finish()?;
            visitor.visit_open_start_element(&element)?;
            visitor.visit_close_element(&element)?;
        }
        BinXMLDeserializedTokens::CloseElement => {
            let element = element_stack
                .pop()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "close element - Bad parser state",
                ))?;
            visitor.visit_close_element(&element)?;
        }
        BinXMLDeserializedTokens::EntityRef(entity) => {
            match expand_string_ref(&entity.name, chunk)? {
                Cow::Borrowed(s) => visitor.visit_entity_reference(s)?,
                Cow::Owned(s) => {
                    let tmp = s;
                    visitor.visit_entity_reference(&tmp)?;
                }
            }
        }
        BinXMLDeserializedTokens::PITarget(name) => {
            let mut b = XmlPIBuilder::new();
            b.name(expand_string_ref(&name.name, chunk)?);
            *current_pi = Some(b);
        }
        BinXMLDeserializedTokens::PIData(data) => {
            let mut b = current_pi
                .take()
                .ok_or(EvtxError::FailedToCreateRecordModel(
                    "PI Data without PI target - Bad parser state",
                ))?;
            b.data(Cow::Borrowed(data.as_str()));
            if let XmlModel::PI(pi) = b.finish() {
                visitor.visit_processing_instruction(&pi)?;
            }
        }
        BinXMLDeserializedTokens::StartOfStream | BinXMLDeserializedTokens::EndOfStream => {}
        BinXMLDeserializedTokens::TemplateInstance(template) => {
            // Not expected inside template definitions, but handle defensively.
            let mut owned = template.clone();
            stream_expand_template(
                &mut owned,
                chunk,
                visitor,
                element_stack,
                current_element,
                current_pi,
            )?;
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

/// Expand a template instance inline during streaming.
/// This expands substitution tokens inline.
///
/// We *move* substitution values on their last use and *clone* them for earlier uses when a
/// template references the same substitution index multiple times (the BinXML format permits
/// this).
fn stream_expand_template<'a, T: BinXmlOutput>(
    template: &mut BinXmlTemplateRef<'a>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
    element_stack: &mut Vec<XmlElement<'a>>,
    current_element: &mut Option<XmlElementBuilder<'a>>,
    current_pi: &mut Option<XmlPIBuilder<'a>>,
) -> Result<()> {
    if let Some(template_def) = chunk
        .template_table
        .get_template(template.template_def_offset)
    {
        let mut remaining_uses = vec![0u32; template.substitution_array.len()];
        for t in template_def.tokens.iter() {
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

        for t in template_def.tokens.iter() {
            match t {
                BinXMLDeserializedTokens::Substitution(desc) => {
                    if desc.ignore {
                        continue;
                    }
                    // Move the substitution value only on its last use. If the template
                    // references the same substitution index multiple times, earlier uses must
                    // clone to preserve correctness.
                    let token = take_or_clone_substitution_value(
                        template,
                        desc.substitution_index,
                        &mut remaining_uses,
                    );

                    stream_expand_token(
                        token,
                        chunk,
                        visitor,
                        element_stack,
                        current_element,
                        current_pi,
                    )?;
                }
                // Template definition tokens from cache are handled by reference (no cloning).
                other => stream_expand_token_ref(
                    other,
                    chunk,
                    visitor,
                    element_stack,
                    current_element,
                    current_pi,
                )?,
            }
        }
    } else {
        // Template not in cache - read directly from chunk
        debug!(
            "Template in offset {} was not found in cache (streaming)",
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
        for t in template_def.tokens.iter() {
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

        // For templates read directly, we own the tokens, so iterate them
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

                    stream_expand_token(
                        token,
                        chunk,
                        visitor,
                        element_stack,
                        current_element,
                        current_pi,
                    )?;
                }
                other => stream_expand_token(
                    other,
                    chunk,
                    visitor,
                    element_stack,
                    current_element,
                    current_pi,
                )?,
            }
        }
    }
    Ok(())
}

pub fn parse_tokens_streaming<'a, T: BinXmlOutput>(
    tokens: Vec<BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
) -> Result<()> {
    // OPTIMIZATION: Process tokens directly without pre-expanding templates.
    // Template expansion happens inline in stream_expand_token/stream_expand_template,
    // which allows us to move substitution values (on their last use) instead of always cloning.
    visitor.visit_start_of_stream()?;
    let mut element_stack: Vec<XmlElement<'a>> = Vec::new();
    let mut current_element: Option<XmlElementBuilder<'a>> = None;
    let mut current_pi: Option<XmlPIBuilder<'a>> = None;

    // Ablation: pre-expand templates (older approach) to measure clone/alloc overhead.
    let tokens = if cfg!(feature = "perf_ablate_preexpand_templates") {
        expand_templates(tokens, chunk)?
    } else {
        tokens
    };

    for token in tokens {
        stream_expand_token(
            token,
            chunk,
            visitor,
            &mut element_stack,
            &mut current_element,
            &mut current_pi,
        )?;
    }
    visitor.visit_end_of_stream()?;
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
