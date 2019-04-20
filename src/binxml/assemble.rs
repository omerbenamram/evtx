use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::model::xml::{XmlElementBuilder, XmlModel};
use crate::xml_output::BinXmlOutput;
use failure::Error;
use log::trace;
use std::io::Write;

pub fn parse_tokens<W: Write, T: BinXmlOutput<W>>(
    tokens: Vec<BinXMLDeserializedTokens>,
    visitor: &mut T,
) -> Result<(), Error> {
    let expanded_tokens = expand_templates(tokens);
    let record_model = create_record_model(expanded_tokens);

    visitor.visit_start_of_stream()?;

    for owned_token in record_model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                visitor.visit_open_start_element(&open_element)?
            }
            XmlModel::CloseElement => visitor.visit_close_element()?,
            XmlModel::String(s) => visitor.visit_characters(&s)?,
            XmlModel::EndOfStream => visitor.visit_end_of_stream()?,
            // Sometimes there are multiple fragment headers,
            // but we only need to write start of stream once.
            XmlModel::StartOfStream => {}
        };
    }

    Ok(())
}

pub fn create_record_model(tokens: Vec<BinXMLDeserializedTokens>) -> Vec<XmlModel> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut model: Vec<XmlModel> = vec![];

    for token in tokens {
        match token {
            BinXMLDeserializedTokens::FragmentHeader(_) => {}
            BinXMLDeserializedTokens::TemplateInstance(_) => {
                panic!("Call `expand_templates` before calling this function")
            }
            BinXMLDeserializedTokens::AttributeList => {}
            BinXMLDeserializedTokens::Attribute(attr) => {
                trace!("BinXMLDeserializedTokens::Attribute(attr) - {:?}", attr);
                match current_element.take() {
                    None => panic!("attribute - Bad parser state"),
                    Some(builder) => {
                        current_element = Some(builder.attribute_name(attr.name));
                    }
                };
            }
            BinXMLDeserializedTokens::OpenStartElement(elem) => {
                trace!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let builder = XmlElementBuilder::new();
                current_element = Some(builder.name(elem.name));
            }
            BinXMLDeserializedTokens::CloseStartElement => {
                trace!("BinXMLDeserializedTokens::CloseStartElement");
                match current_element.take() {
                    None => panic!("close start - Bad parser state"),
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish())),
                };
            }
            BinXMLDeserializedTokens::CloseEmptyElement => {
                trace!("BinXMLDeserializedTokens::CloseEmptyElement");
                match current_element.take() {
                    None => panic!("close empty - Bad parser state"),
                    Some(builder) => {
                        model.push(XmlModel::OpenElement(builder.finish()));
                        model.push(XmlModel::CloseElement);
                    }
                };
            }
            BinXMLDeserializedTokens::CloseElement => {
                model.push(XmlModel::CloseElement);
            }
            BinXMLDeserializedTokens::Value(value) => {
                trace!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                match current_element.take() {
                    // A string that is not inside any element, yield it
                    None => match value {
                        BinXmlValue::StringType(cow) => {
                            model.push(XmlModel::String(cow.clone()));
                        }
                        BinXmlValue::EvtXml => {
                            panic!("Call `expand_templates` before calling this function")
                        }
                        _ => {
                            model.push(XmlModel::String(value.into()));
                        }
                    },
                    // A string that is bound to an attribute
                    Some(builder) => {
                        current_element =
                            Some(builder.attribute_value(BinXmlValue::StringType(value.into())));
                    }
                };
            }
            BinXMLDeserializedTokens::CDATASection => {}
            BinXMLDeserializedTokens::CharRef => {}
            BinXMLDeserializedTokens::EntityRef(e) => unimplemented!("{}", &format!("{:?}", e)),
            BinXMLDeserializedTokens::PITarget => {}
            BinXMLDeserializedTokens::PIData => {}
            BinXMLDeserializedTokens::Substitution(_) => {
                panic!("Call `expand_templates` before calling this function")
            }
            BinXMLDeserializedTokens::EndOfStream => model.push(XmlModel::EndOfStream),
            BinXMLDeserializedTokens::StartOfStream => model.push(XmlModel::StartOfStream),
        }
    }
    model
}

pub fn expand_templates(
    token_tree: Vec<BinXMLDeserializedTokens>,
) -> Vec<BinXMLDeserializedTokens> {
    let mut stack = Vec::new();

    fn _expand_templates<'c>(
        token: BinXMLDeserializedTokens<'c>,
        stack: &mut Vec<BinXMLDeserializedTokens<'c>>,
    ) {
        match token {
            BinXMLDeserializedTokens::Value(ref value) => {
                if let BinXmlValue::BinXmlType(tokens) = value {
                    for token in tokens.iter() {
                        _expand_templates(token.clone(), stack);
                    }
                } else {
                    stack.push(token)
                }
            }
            BinXMLDeserializedTokens::TemplateInstance(template) => {
                // We have to clone here since the templates **definitions** are shared.
                for token in template.definition.tokens.iter().cloned() {
                    if let BinXMLDeserializedTokens::Substitution(ref substitution_descriptor) =
                        token
                    {
                        if substitution_descriptor.ignore {
                            continue;
                        } else {
                            // TODO: see if we can avoid this copy
                            let value = &template.substitution_array
                                [substitution_descriptor.substitution_index as usize];

                            _expand_templates(
                                BinXMLDeserializedTokens::Value(value.clone()),
                                stack,
                            );
                        }
                    } else {
                        _expand_templates(token, stack);
                    }
                }
            }
            _ => stack.push(token),
        }
    }

    for token in token_tree {
        _expand_templates(token, &mut stack)
    }

    stack
}
