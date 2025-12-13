use crate::err::{EvtxError, Result};

use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::{
    BinXMLDeserializedTokens, BinXmlTemplateRef, TemplateSubstitutionDescriptor,
};
use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
use crate::xml_output::BinXmlOutput;
use log::{debug, trace, warn};
use std::borrow::{BorrowMut, Cow};

use std::mem;

use crate::EvtxChunk;
use crate::binxml::name::{BinXmlName, BinXmlNameRef};
use crate::binxml::tokens::read_template_definition;
use std::io::{Cursor, Seek, SeekFrom};

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
            let mut cursor = Cursor::new(chunk.data);
            let cursor_ref = cursor.borrow_mut();
            try_seek!(
                cursor_ref,
                string_ref.offset + BINXML_NAME_LINK_SIZE,
                "Cache missed string"
            )?;

            let string = BinXmlName::from_stream(cursor_ref)?;
            Ok(Cow::Owned(string))
        }
    }
}

fn expand_token_substitution<'a>(
    template: &mut BinXmlTemplateRef<'a>,
    substitution_descriptor: &TemplateSubstitutionDescriptor,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<BinXMLDeserializedTokens<'a>>,
) -> Result<()> {
    if substitution_descriptor.ignore {
        return Ok(());
    }

    let value = template
        .substitution_array
        .get_mut(substitution_descriptor.substitution_index as usize);

    if let Some(value) = value {
        let value = mem::replace(
            value,
            BinXMLDeserializedTokens::Value(BinXmlValue::NullType),
        );
        _expand_templates(value, chunk, stack)?;
    } else {
        _expand_templates(
            BinXMLDeserializedTokens::Value(BinXmlValue::NullType),
            chunk,
            stack,
        )?;
    }

    Ok(())
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
        // We expect to find all the templates in the template cache.
        // Clone from cache since the cache owns the tokens.
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(&mut template, substitution_descriptor, chunk, stack)?;
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

        let mut cursor = Cursor::new(chunk.data);

        let _ = cursor.seek(SeekFrom::Start(u64::from(template.template_def_offset)));
        let template_def =
            read_template_definition(&mut cursor, chunk.arena, chunk.settings.get_ansi_codec())?;

        for token in template_def.tokens {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(&mut template, &substitution_descriptor, chunk, stack)?;
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
/// This takes ownership of substitution values (no cloning needed) and only clones
/// template tokens from cache (which are mostly cheap - just u32 offsets).
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
        for t in template_def.tokens.iter() {
            match t {
                BinXMLDeserializedTokens::Substitution(desc) => {
                    if desc.ignore {
                        continue;
                    }
                    // OPTIMIZATION: Move substitution value instead of cloning
                    // Each substitution is only used once, so we can take ownership
                    if let Some(val) = template
                        .substitution_array
                        .get_mut(desc.substitution_index as usize)
                    {
                        let owned_val = mem::replace(
                            val,
                            BinXMLDeserializedTokens::Value(BinXmlValue::NullType),
                        );
                        stream_expand_token(
                            owned_val,
                            chunk,
                            visitor,
                            element_stack,
                            current_element,
                            current_pi,
                        )?;
                    } else {
                        visitor.visit_characters(Cow::Owned(BinXmlValue::NullType))?;
                    }
                }
                // Template tokens from cache must be cloned, but most are trivially cheap:
                // - OpenStartElement, Attribute: contain BinXmlNameRef (just u32)
                // - CloseStartElement, CloseElement, etc: no data
                // - Value: expensive but rare in templates (data is in substitutions)
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
        let mut cursor = Cursor::new(chunk.data);
        let _ = cursor.seek(SeekFrom::Start(u64::from(template.template_def_offset)));
        let template_def =
            read_template_definition(&mut cursor, chunk.arena, chunk.settings.get_ansi_codec())?;
        // For templates read directly, we own the tokens, so iterate them
        for t in template_def.tokens {
            match t {
                BinXMLDeserializedTokens::Substitution(desc) => {
                    if desc.ignore {
                        continue;
                    }
                    if let Some(val) = template
                        .substitution_array
                        .get_mut(desc.substitution_index as usize)
                    {
                        let owned_val = mem::replace(
                            val,
                            BinXMLDeserializedTokens::Value(BinXmlValue::NullType),
                        );
                        stream_expand_token(
                            owned_val,
                            chunk,
                            visitor,
                            element_stack,
                            current_element,
                            current_pi,
                        )?;
                    } else {
                        visitor.visit_characters(Cow::Owned(BinXmlValue::NullType))?;
                    }
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
    // which allows us to move substitution values instead of cloning them.
    visitor.visit_start_of_stream()?;
    let mut element_stack: Vec<XmlElement<'a>> = Vec::new();
    let mut current_element: Option<XmlElementBuilder<'a>> = None;
    let mut current_pi: Option<XmlPIBuilder<'a>> = None;
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
