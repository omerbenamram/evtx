use encoding::EncodingRef;

use super::binxml::{parse_temp_binxml_fragment, parse_wevt_binxml_fragment, TEMP_BINXML_OFFSET};

/// Render a `TEMP` entry to an XML string (with `{sub:N}` placeholders for substitutions).
pub fn render_temp_to_xml(
    temp_bytes: &[u8],
    ansi_codec: EncodingRef,
) -> crate::err::Result<String> {
    use crate::ParserSettings;
    use crate::binxml::name::read_wevt_inline_name_at;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::err::{EvtxError, Result};
    use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
    use crate::xml_output::{BinXmlOutput, XmlOutput};
    use std::borrow::Cow;

    if temp_bytes.len() < TEMP_BINXML_OFFSET {
        return Err(EvtxError::calculation_error(format!(
            "TEMP too small to contain BinXML fragment header (len={}, need >= {})",
            temp_bytes.len(),
            TEMP_BINXML_OFFSET
        )));
    }

    let binxml = &temp_bytes[TEMP_BINXML_OFFSET..];
    let (tokens, _bytes_consumed) = parse_temp_binxml_fragment(temp_bytes, ansi_codec)?;

    fn resolve_name<'a>(
        binxml: &'a [u8],
        name_ref: &crate::binxml::name::BinXmlNameRef,
    ) -> Result<Cow<'a, crate::binxml::name::BinXmlName>> {
        Ok(Cow::Owned(read_wevt_inline_name_at(
            binxml,
            name_ref.offset,
        )?))
    }

    // Build a record model similar to `binxml::assemble::create_record_model`,
    // but resolving names via WEVT inline-name decoding and allowing substitution placeholders.
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {}
            crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT TEMP BinXML".to_string(),
                });
            }
            crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {}
            crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(resolve_name(binxml, &entity.name)?))
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                builder.name(resolve_name(binxml, &name.name)?);
                current_pi = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PIData(data) => {
                match current_pi.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "PI Data without PI target - Bad parser state",
                        ));
                    }
                    Some(mut builder) => {
                        builder.data(Cow::Owned(data));
                        model.push(builder.finish());
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Substitution(sub) => {
                let placeholder = format!("{{sub:{}}}", sub.substitution_index);
                let value = BinXmlValue::StringType(placeholder);
                match current_element {
                    None => model.push(XmlModel::Value(Cow::Owned(value))),
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                model.push(XmlModel::EndOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                model.push(XmlModel::StartOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
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
            crate::model::deserialized::BinXMLDeserializedTokens::Attribute(ref attr) => {
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(resolve_name(binxml, &attr.name)?)
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                let mut builder = XmlElementBuilder::new();
                builder.name(resolve_name(binxml, &elem.name)?);
                current_element = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Value(value) => {
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Unexpected EvtXml in WEVT TEMP BinXML",
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

    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut output = XmlOutput::with_writer(Vec::new(), &settings);

    output.visit_start_of_stream()?;
    let mut stack: Vec<XmlElement> = Vec::new();

    for owned_token in model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                output.visit_open_start_element(stack.last().ok_or({
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
                output.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => output.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => output.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => output.visit_entity_reference(&entity)?,
        };
    }

    output.visit_end_of_stream()?;

    String::from_utf8(output.into_writer()).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

/// Render a parsed template definition to XML.
///
/// Compared to `render_temp_to_xml`, this variant can annotate substitutions using the parsed
/// template item descriptors/names (from the CRIM blob).
pub fn render_template_definition_to_xml(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    ansi_codec: EncodingRef,
) -> crate::err::Result<String> {
    use crate::ParserSettings;
    use crate::binxml::name::read_wevt_inline_name_at;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::err::{EvtxError, Result};
    use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
    use crate::xml_output::{BinXmlOutput, XmlOutput};
    use std::borrow::Cow;

    let binxml = template.binxml;
    let (tokens, _bytes_consumed) = parse_wevt_binxml_fragment(binxml, ansi_codec)?;

    fn resolve_name<'a>(
        binxml: &'a [u8],
        name_ref: &crate::binxml::name::BinXmlNameRef,
    ) -> Result<Cow<'a, crate::binxml::name::BinXmlName>> {
        Ok(Cow::Owned(read_wevt_inline_name_at(
            binxml,
            name_ref.offset,
        )?))
    }

    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {}
            crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT TEMP BinXML".to_string(),
                });
            }
            crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {}
            crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(resolve_name(binxml, &entity.name)?))
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                builder.name(resolve_name(binxml, &name.name)?);
                current_pi = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PIData(data) => {
                match current_pi.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "PI Data without PI target - Bad parser state",
                        ));
                    }
                    Some(mut builder) => {
                        builder.data(Cow::Owned(data));
                        model.push(builder.finish());
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Substitution(sub) => {
                let idx = sub.substitution_index as usize;
                let mut placeholder = format!("{{sub:{idx}}}");

                if let Some(name) = template
                    .items
                    .get(idx)
                    .and_then(|item| item.name.as_deref())
                {
                    placeholder = format!("{{sub:{idx}:{name}}}");
                }

                let value = BinXmlValue::StringType(placeholder);
                match current_element {
                    None => model.push(XmlModel::Value(Cow::Owned(value))),
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                model.push(XmlModel::EndOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                model.push(XmlModel::StartOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
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
            crate::model::deserialized::BinXMLDeserializedTokens::Attribute(ref attr) => {
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(resolve_name(binxml, &attr.name)?)
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                let mut builder = XmlElementBuilder::new();
                builder.name(resolve_name(binxml, &elem.name)?);
                current_element = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Value(value) => {
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Unexpected EvtXml in WEVT TEMP BinXML",
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

    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut output = XmlOutput::with_writer(Vec::new(), &settings);

    output.visit_start_of_stream()?;
    let mut stack: Vec<XmlElement> = Vec::new();

    for owned_token in model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                output.visit_open_start_element(stack.last().ok_or({
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
                output.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => output.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => output.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => output.visit_entity_reference(&entity)?,
        };
    }

    output.visit_end_of_stream()?;

    String::from_utf8(output.into_writer()).map_err(|e| EvtxError::calculation_error(e.to_string()))
}

/// Render a parsed template definition to XML, applying substitution values.
///
/// This is the "last mile" for offline rendering: given a template definition (from `WEVT_TEMPLATE`)
/// and the corresponding substitution values array (from an EVTX record's `TemplateInstance`),
/// emit a fully-rendered XML event fragment.
///
/// The `substitution_values` are provided as strings and are inserted as text/attribute values.
/// XML escaping is handled by `XmlOutput`.
pub fn render_template_definition_to_xml_with_substitution_values(
    template: &crate::wevt_templates::manifest::TemplateDefinition<'_>,
    substitution_values: &[String],
    ansi_codec: EncodingRef,
) -> crate::err::Result<String> {
    use crate::ParserSettings;
    use crate::binxml::name::read_wevt_inline_name_at;
    use crate::binxml::value_variant::BinXmlValue;
    use crate::err::{EvtxError, Result};
    use crate::model::xml::{XmlElement, XmlElementBuilder, XmlModel, XmlPIBuilder};
    use crate::xml_output::{BinXmlOutput, XmlOutput};
    use std::borrow::Cow;

    let binxml = template.binxml;
    let (tokens, _bytes_consumed) = parse_wevt_binxml_fragment(binxml, ansi_codec)?;

    fn resolve_name<'a>(
        binxml: &'a [u8],
        name_ref: &crate::binxml::name::BinXmlNameRef,
    ) -> Result<Cow<'a, crate::binxml::name::BinXmlName>> {
        Ok(Cow::Owned(read_wevt_inline_name_at(
            binxml,
            name_ref.offset,
        )?))
    }

    let mut current_element: Option<XmlElementBuilder> = None;
    let mut current_pi: Option<XmlPIBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            crate::model::deserialized::BinXMLDeserializedTokens::FragmentHeader(_) => {}
            crate::model::deserialized::BinXMLDeserializedTokens::TemplateInstance(_) => {
                return Err(EvtxError::Unimplemented {
                    name: "TemplateInstance inside WEVT TEMP BinXML".to_string(),
                });
            }
            crate::model::deserialized::BinXMLDeserializedTokens::AttributeList => {}
            crate::model::deserialized::BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseStartElement => {
                match current_element.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "close start - Bad parser state",
                        ));
                    }
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish()?)),
                };
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CDATASection => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CDATA",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CharRef => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "Unimplemented - CharacterReference",
                ));
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EntityRef(ref entity) => {
                model.push(XmlModel::EntityRef(resolve_name(binxml, &entity.name)?))
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PITarget(ref name) => {
                let mut builder = XmlPIBuilder::new();
                builder.name(resolve_name(binxml, &name.name)?);
                current_pi = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::PIData(data) => {
                match current_pi.take() {
                    None => {
                        return Err(EvtxError::FailedToCreateRecordModel(
                            "PI Data without PI target - Bad parser state",
                        ));
                    }
                    Some(mut builder) => {
                        builder.data(Cow::Owned(data));
                        model.push(builder.finish());
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Substitution(sub) => {
                if sub.ignore {
                    continue;
                }
                let idx = sub.substitution_index as usize;
                let s = substitution_values.get(idx).cloned().unwrap_or_default();
                let value = BinXmlValue::StringType(s);

                match current_element {
                    None => model.push(XmlModel::Value(Cow::Owned(value))),
                    Some(ref mut builder) => {
                        builder.attribute_value(Cow::Owned(value))?;
                    }
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::EndOfStream => {
                model.push(XmlModel::EndOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::StartOfStream => {
                model.push(XmlModel::StartOfStream)
            }
            crate::model::deserialized::BinXMLDeserializedTokens::CloseEmptyElement => {
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
            crate::model::deserialized::BinXMLDeserializedTokens::Attribute(ref attr) => {
                if current_element.is_none() {
                    return Err(EvtxError::FailedToCreateRecordModel(
                        "attribute - Bad parser state",
                    ));
                }
                if let Some(builder) = current_element.as_mut() {
                    builder.attribute_name(resolve_name(binxml, &attr.name)?)
                }
            }
            crate::model::deserialized::BinXMLDeserializedTokens::OpenStartElement(ref elem) => {
                let mut builder = XmlElementBuilder::new();
                builder.name(resolve_name(binxml, &elem.name)?);
                current_element = Some(builder);
            }
            crate::model::deserialized::BinXMLDeserializedTokens::Value(value) => {
                match current_element {
                    None => match value {
                        BinXmlValue::EvtXml => {
                            return Err(EvtxError::FailedToCreateRecordModel(
                                "Unexpected EvtXml in WEVT TEMP BinXML",
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

    let settings = ParserSettings::default().ansi_codec(ansi_codec);
    let mut output = XmlOutput::with_writer(Vec::new(), &settings);

    output.visit_start_of_stream()?;
    let mut stack: Vec<XmlElement> = Vec::new();

    for owned_token in model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                output.visit_open_start_element(stack.last().ok_or({
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
                output.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => output.visit_characters(s)?,
            XmlModel::EndOfStream => {}
            XmlModel::StartOfStream => {}
            XmlModel::PI(pi) => output.visit_processing_instruction(&pi)?,
            XmlModel::EntityRef(entity) => output.visit_entity_reference(&entity)?,
        };
    }

    output.visit_end_of_stream()?;

    String::from_utf8(output.into_writer()).map_err(|e| EvtxError::calculation_error(e.to_string()))
}


