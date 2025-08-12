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

fn stream_visit_from_expanded<'a, T: BinXmlOutput>(
    expanded: &[Cow<'a, BinXMLDeserializedTokens<'a>>],
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
) -> Result<()> {
    visitor.visit_start_of_stream()?;

    // Minimal stack of open elements to match close
    let mut element_stack: Vec<XmlElement> = Vec::new();

    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;

    for token in expanded.iter() {
        match token {
            Cow::Borrowed(BinXMLDeserializedTokens::Substitution(_))
            | Cow::Owned(BinXMLDeserializedTokens::Substitution(_)) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            Cow::Borrowed(BinXMLDeserializedTokens::FragmentHeader(_))
            | Cow::Owned(BinXMLDeserializedTokens::FragmentHeader(_)) => {}
            Cow::Borrowed(BinXMLDeserializedTokens::AttributeList)
            | Cow::Owned(BinXMLDeserializedTokens::AttributeList) => {}

            Cow::Borrowed(BinXMLDeserializedTokens::OpenStartElement(elem))
            | Cow::Owned(BinXMLDeserializedTokens::OpenStartElement(elem)) => {
                let mut builder = XmlElementBuilder::new_in(&chunk.arena);
                builder.name(expand_string_ref(&elem.name, chunk)?);
                current_element = Some(builder);
            }

            Cow::Borrowed(BinXMLDeserializedTokens::Attribute(attr))
            | Cow::Owned(BinXMLDeserializedTokens::Attribute(attr)) => {
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(expand_string_ref(&attr.name, chunk)?);
                } else {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
            }

            Cow::Borrowed(BinXMLDeserializedTokens::Value(value)) => {
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_value(Cow::Borrowed(value))?;
                } else {
                    visitor.visit_characters(Cow::Borrowed(value))?;
                }
            }
            Cow::Owned(BinXMLDeserializedTokens::Value(value)) => {
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_value(Cow::Owned(value.clone()))?;
                } else {
                    visitor.visit_characters(Cow::Owned(value.clone()))?;
                }
            }

            Cow::Borrowed(BinXMLDeserializedTokens::CloseStartElement)
            | Cow::Owned(BinXMLDeserializedTokens::CloseStartElement) => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close start - Bad parser state",
                    ))?
                    .finish()?;
                visitor.visit_open_start_element(&element)?;
                element_stack.push(element);
            }

            Cow::Borrowed(BinXMLDeserializedTokens::CloseEmptyElement)
            | Cow::Owned(BinXMLDeserializedTokens::CloseEmptyElement) => {
                let element = current_element
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close empty - Bad parser state",
                    ))?
                    .finish()?;
                visitor.visit_open_start_element(&element)?;
                visitor.visit_close_element(&element)?;
            }

            Cow::Borrowed(BinXMLDeserializedTokens::CloseElement)
            | Cow::Owned(BinXMLDeserializedTokens::CloseElement) => {
                let element = element_stack
                    .pop()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "close element - Bad parser state",
                    ))?;
                visitor.visit_close_element(&element)?;
            }

            Cow::Borrowed(BinXMLDeserializedTokens::EntityRef(entity))
            | Cow::Owned(BinXMLDeserializedTokens::EntityRef(entity)) => {
                match expand_string_ref(&entity.name, chunk)? {
                    Cow::Borrowed(s) => visitor.visit_entity_reference(s)?,
                    Cow::Owned(s) => {
                        let tmp = s;
                        visitor.visit_entity_reference(&tmp)?;
                    }
                }
            }

            Cow::Borrowed(BinXMLDeserializedTokens::PITarget(name))
            | Cow::Owned(BinXMLDeserializedTokens::PITarget(name)) => {
                let mut b = XmlPIBuilder::new();
                b.name(expand_string_ref(&name.name, chunk)?);
                current_pi = Some(b);
            }
            Cow::Borrowed(BinXMLDeserializedTokens::PIData(data)) => {
                let mut b = current_pi
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "PI Data without PI target - Bad parser state",
                    ))?;
                b.data(Cow::Borrowed(data));
                if let XmlModel::PI(pi) = b.finish() {
                    visitor.visit_processing_instruction(&pi)?;
                }
            }
            Cow::Owned(BinXMLDeserializedTokens::PIData(data)) => {
                let mut b = current_pi
                    .take()
                    .ok_or(EvtxError::FailedToCreateRecordModel(
                        "PI Data without PI target - Bad parser state",
                    ))?;
                b.data(Cow::Owned(data.clone()));
                if let XmlModel::PI(pi) = b.finish() {
                    visitor.visit_processing_instruction(&pi)?;
                }
            }

            Cow::Borrowed(BinXMLDeserializedTokens::StartOfStream)
            | Cow::Owned(BinXMLDeserializedTokens::StartOfStream) => {}
            Cow::Borrowed(BinXMLDeserializedTokens::EndOfStream)
            | Cow::Owned(BinXMLDeserializedTokens::EndOfStream) => {}

            Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(_))
            | Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(_)) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            Cow::Borrowed(BinXMLDeserializedTokens::CDATASection)
            | Cow::Owned(BinXMLDeserializedTokens::CDATASection) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            Cow::Borrowed(BinXMLDeserializedTokens::CharRef)
            | Cow::Owned(BinXMLDeserializedTokens::CharRef) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
        }
    }

    visitor.visit_end_of_stream()?;
    Ok(())
}

pub fn parse_tokens<'a, T: BinXmlOutput>(
    tokens: &'a [BinXMLDeserializedTokens<'a>],
    chunk: &'a EvtxChunk<'a>,
    visitor: &mut T,
) -> Result<()> {
    let expanded_tokens = expand_templates(tokens, chunk)?;
    stream_visit_from_expanded(expanded_tokens, chunk, visitor)
}

pub fn create_record_model<'a>(
    tokens: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
    chunk: &'a EvtxChunk<'a>,
) -> Result<Vec<XmlModel<'a>>> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        // Handle all places where we don't care if it's an Owned or a Borrowed value.
        match token {
            Cow::Owned(BinXMLDeserializedTokens::FragmentHeader(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::FragmentHeader(_)) => {}
            Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(_)) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            Cow::Owned(BinXMLDeserializedTokens::AttributeList)
            | Cow::Borrowed(BinXMLDeserializedTokens::AttributeList) => {}

            Cow::Owned(BinXMLDeserializedTokens::CloseElement)
            | Cow::Borrowed(BinXMLDeserializedTokens::CloseElement) => {
                model.push(XmlModel::CloseElement);
            }

            Cow::Owned(BinXMLDeserializedTokens::CloseStartElement)
            | Cow::Borrowed(BinXMLDeserializedTokens::CloseStartElement) => {
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
            Cow::Owned(BinXMLDeserializedTokens::CDATASection)
            | Cow::Borrowed(BinXMLDeserializedTokens::CDATASection) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            Cow::Owned(BinXMLDeserializedTokens::CharRef)
            | Cow::Borrowed(BinXMLDeserializedTokens::CharRef) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            Cow::Owned(BinXMLDeserializedTokens::EntityRef(ref entity))
            | Cow::Borrowed(&BinXMLDeserializedTokens::EntityRef(ref entity)) => {
                model.push(XmlModel::EntityRef(expand_string_ref(&entity.name, chunk)?))
            }
            Cow::Owned(BinXMLDeserializedTokens::PITarget(ref name))
            | Cow::Borrowed(&BinXMLDeserializedTokens::PITarget(ref name)) => {
                let mut builder = XmlPIBuilder::new();
                if current_pi.is_some() {
                    warn!("PITarget without following PIData, previous target will be ignored.")
                }
                builder.name(expand_string_ref(&name.name, chunk)?);
                current_pi = Some(builder);
            }
            Cow::Owned(BinXMLDeserializedTokens::PIData(data)) => match current_pi.take() {
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
            Cow::Borrowed(BinXMLDeserializedTokens::PIData(data)) => match current_pi.take() {
                None => {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "PI Data without PI target - Bad parser state",
                    ));
                }
                Some(mut builder) => {
                    builder.data(Cow::Borrowed(data));
                    model.push(builder.finish());
                }
            },
            Cow::Owned(BinXMLDeserializedTokens::Substitution(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::Substitution(_)) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Call `expand_templates` before calling this function",
                ));
            }
            Cow::Owned(BinXMLDeserializedTokens::EndOfStream)
            | Cow::Borrowed(BinXMLDeserializedTokens::EndOfStream) => {
                model.push(XmlModel::EndOfStream)
            }
            Cow::Owned(BinXMLDeserializedTokens::StartOfStream)
            | Cow::Borrowed(BinXMLDeserializedTokens::StartOfStream) => {
                model.push(XmlModel::StartOfStream)
            }

            Cow::Owned(BinXMLDeserializedTokens::CloseEmptyElement)
            | Cow::Borrowed(BinXMLDeserializedTokens::CloseEmptyElement) => {
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

            Cow::Owned(BinXMLDeserializedTokens::Attribute(ref attr))
            | Cow::Borrowed(&BinXMLDeserializedTokens::Attribute(ref attr)) => {
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
            Cow::Owned(BinXMLDeserializedTokens::OpenStartElement(ref elem))
            | Cow::Borrowed(&BinXMLDeserializedTokens::OpenStartElement(ref elem)) => {
                trace!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let mut builder = XmlElementBuilder::new_in(&chunk.arena);
                builder.name(expand_string_ref(&elem.name, chunk)?);
                current_element = Some(builder);
            }
            Cow::Owned(BinXMLDeserializedTokens::Value(value)) => {
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
            Cow::Borrowed(BinXMLDeserializedTokens::Value(value)) => {
                trace!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Call `expand_templates` before calling this function",
                            ));
                        }
                        _ => {
                            model.push(XmlModel::Value(Cow::Borrowed(value)));
                        }
                    },
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Borrowed(value))?;
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
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
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
        _expand_templates(Cow::Owned(value), chunk, stack)?;
    } else {
        _expand_templates(
            Cow::Owned(BinXMLDeserializedTokens::Value(BinXmlValue::NullType)),
            chunk,
            stack,
        )?;
    }

    Ok(())
}

fn expand_template<'a>(
    mut template: BinXmlTemplateRef<'a>,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) -> Result<()> {
    if let Some(template_def) = chunk
        .template_table
        .get_template(template.template_def_offset)
    {
        // We expect to find all the templates in the template cache.
        for token in template_def.tokens.iter() {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(&mut template, substitution_descriptor, chunk, stack)?;
            } else {
                _expand_templates(Cow::Borrowed(token), chunk, stack)?;
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
            read_template_definition(&mut cursor, Some(chunk), chunk.settings.get_ansi_codec())?;

        for token in template_def.tokens {
            if let BinXMLDeserializedTokens::Substitution(substitution_descriptor) = token {
                expand_token_substitution(&mut template, &substitution_descriptor, chunk, stack)?;
            } else {
                _expand_templates(Cow::Owned(token), chunk, stack)?;
            }
        }
    };

    Ok(())
}

fn _expand_templates<'a>(
    token: Cow<'a, BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) -> Result<()> {
    // unchanged
    match token {
        Cow::Owned(BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens))) => {
            for token in tokens.into_iter() {
                _expand_templates(Cow::Owned(token), chunk, stack)?;
            }
        }
        Cow::Borrowed(BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens))) => {
            for token in tokens.iter() {
                _expand_templates(Cow::Borrowed(token), chunk, stack)?;
            }
        }
        Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            expand_template(template, chunk, stack)?;
        }
        Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            expand_template(template.clone(), chunk, stack)?;
        }
        _ => stack.push(token),
    }
    Ok(())
}

fn _expand_templates_bv<'a>(
    token: Cow<'a, BinXMLDeserializedTokens<'a>>,
    chunk: &'a EvtxChunk<'a>,
    stack: &mut bumpalo::collections::Vec<'a, Cow<'a, BinXMLDeserializedTokens<'a>>>,
) -> Result<()> {
    match token {
        Cow::Owned(BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens))) => {
            for token in tokens.into_iter() {
                _expand_templates_bv(Cow::Owned(token), chunk, stack)?;
            }
        }
        Cow::Borrowed(BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens))) => {
            for token in tokens.iter() {
                _expand_templates_bv(Cow::Borrowed(token), chunk, stack)?;
            }
        }
        Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            let mut tmp: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>> = Vec::new();
            expand_template(template, chunk, &mut tmp)?;
            for t in tmp.into_iter() {
                stack.push(t);
            }
        }
        Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            let mut tmp: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>> = Vec::new();
            expand_template(template.clone(), chunk, &mut tmp)?;
            for t in tmp.into_iter() {
                stack.push(t);
            }
        }
        _ => stack.push(token),
    }
    Ok(())
}

pub fn expand_templates<'a>(
    token_tree: &'a [BinXMLDeserializedTokens<'a>],
    chunk: &'a EvtxChunk<'a>,
) -> Result<&'a [Cow<'a, BinXMLDeserializedTokens<'a>>]> {
    let mut stack_bv = bumpalo::collections::Vec::with_capacity_in(token_tree.len(), &chunk.arena);
    for token in token_tree.iter() {
        _expand_templates_bv(Cow::Borrowed(token), chunk, &mut stack_bv)?
    }
    Ok(stack_bv.into_bump_slice())
}
