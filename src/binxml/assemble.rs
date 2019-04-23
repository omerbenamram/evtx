use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::model::xml::{XmlElementBuilder, XmlModel};
use crate::xml_output::BinXmlOutput;
use failure::Error;
use log::trace;
use std::io::Write;
use std::borrow::Cow;
use std::mem;

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
            XmlModel::Value(s) => visitor.visit_characters(&s)?,
            XmlModel::EndOfStream => visitor.visit_end_of_stream()?,
            // Sometimes there are multiple fragment headers,
            // but we only need to write start of stream once.
            XmlModel::StartOfStream => {}
        };
    }

    Ok(())
}

pub fn create_record_model<'a>(tokens: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>) -> Vec<XmlModel<'a>> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        match token {
            Cow::Owned(owned_token) => {
                match owned_token {
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
                                current_element = Some(builder.attribute_name(Cow::Owned(attr.name)));
                            }
                        };
                    }
                    BinXMLDeserializedTokens::OpenStartElement(elem) => {
                        trace!(
                            "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                            elem.name
                        );
                        let builder = XmlElementBuilder::new();
                        current_element = Some(builder.name(Cow::Owned(elem.name)));
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
                                BinXmlValue::EvtXml => {
                                    panic!("Call `expand_templates` before calling this function")
                                }
                                _ => {
                                    model.push(XmlModel::Value(Cow::Owned(value)));
                                }
                            },
                            // A string that is bound to an attribute
                            Some(builder) => {
                                current_element = Some(builder.attribute_value(Cow::Owned(value)));
                            }
                        };
                    }
                    BinXMLDeserializedTokens::RefValue(value) => {
                        trace!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                        match current_element.take() {
                            // A string that is not inside any element, yield it
                            None => match value {
                                BinXmlValue::EvtXml => {
                                    panic!("Call `expand_templates` before calling this function")
                                }
                                _ => {
                                    model.push(XmlModel::Value(Cow::Borrowed(value)));
                                }
                            },
                            // A string that is bound to an attribute
                            Some(builder) => {
                                current_element = Some(builder.attribute_value(Cow::Borrowed(value)));
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
            Cow::Borrowed(ref_token) => {
                match ref_token {
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
                                current_element = Some(builder.attribute_name(Cow::Borrowed(&attr.name)));
                            }
                        };
                    }
                    BinXMLDeserializedTokens::OpenStartElement(elem) => {
                        trace!(
                            "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                            elem.name
                        );
                        let builder = XmlElementBuilder::new();
                        current_element = Some(builder.name(Cow::Borrowed(&elem.name)));
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
                                BinXmlValue::EvtXml => {
                                    panic!("Call `expand_templates` before calling this function")
                                }
                                _ => {
                                    model.push(XmlModel::Value(Cow::Borrowed(value)));
                                }
                            },
                            // A string that is bound to an attribute
                            Some(builder) => {
                                current_element = Some(builder.attribute_value(Cow::Borrowed(value)));
                            }
                        };
                    }
                    BinXMLDeserializedTokens::RefValue(value) => {
                        trace!("BinXMLDeserializedTokens::Value(value) - {:?}", value);
                        match current_element.take() {
                            // A string that is not inside any element, yield it
                            None => match value {
                                BinXmlValue::EvtXml => {
                                    panic!("Call `expand_templates` before calling this function")
                                }
                                _ => {
                                    model.push(XmlModel::Value(Cow::Borrowed(value)));
                                }
                            },
                            // A string that is bound to an attribute
                            Some(builder) => {
                                current_element = Some(builder.attribute_value(Cow::Borrowed(value)));
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
        }
    }
    model
}

pub fn expand_templates(
    token_tree: Vec<BinXMLDeserializedTokens>,
) -> Vec<Cow<BinXMLDeserializedTokens>> {
    // We can assume the new tree will be at least as big as the old one.
    let mut stack = Vec::with_capacity(token_tree.len());

    fn _expand_templates<'a>(
        token: Cow<'a, BinXMLDeserializedTokens<'a>>,
        stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
    ) {
        match token {
            Cow::Owned(owned_token) => {
                match owned_token {
                    BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens)) => {
                        for token in tokens.into_iter() {
                            _expand_templates(Cow::Owned(token), stack);
                        }
                    }
                    BinXMLDeserializedTokens::TemplateInstance(mut template) => {
                        let tokens: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>> = match template.definition {
                            Cow::Owned(owned_def) => {
                                owned_def.tokens.into_iter().map(Cow::Owned).collect()
                            }
                            Cow::Borrowed(ref_def) => {
                                ref_def.tokens.iter().map(Cow::Borrowed).collect()
                            }
                        };

                        for token in tokens {
                            if let BinXMLDeserializedTokens::Substitution(ref substitution_descriptor) = token.as_ref()
                            {
                                if substitution_descriptor.ignore {
                                    continue;
                                } else {
                                    // We swap out the node in the substitution array with a dummy value (to avoid copying it),
                                    // moving control of the original node to the new token tree.
                                    let value = mem::replace(
                                        &mut template.substitution_array[substitution_descriptor.substitution_index as usize],
                                        BinXmlValue::NullType,
                                    );

                                    _expand_templates(Cow::Owned(BinXMLDeserializedTokens::Value(value)), stack);
                                }
                            } else {
                                _expand_templates(token, stack);
                            }
                        }
                    }
                    _ => stack.push(Cow::Owned(owned_token)),
                }
            }
            Cow::Borrowed(ref_token) => {
                match ref_token {
                    BinXMLDeserializedTokens::Value(BinXmlValue::BinXmlType(tokens)) => {
                        for token in tokens.iter() {
                            _expand_templates(Cow::Borrowed(token), stack);
                        }
                    }
                    BinXMLDeserializedTokens::TemplateInstance(template) => {
                        for token in template.definition.as_ref().tokens.iter() {
                            if let BinXMLDeserializedTokens::Substitution(ref substitution_descriptor) = token
                            {
                                if substitution_descriptor.ignore {
                                    continue;
                                } else {
                                    let value = &template.substitution_array[substitution_descriptor.substitution_index as usize];

                                    _expand_templates(Cow::Owned(BinXMLDeserializedTokens::RefValue(value)), stack);
                                }
                            } else {
                                _expand_templates(Cow::Borrowed(token), stack);
                            }
                        }
                    }
                    _ => stack.push(Cow::Borrowed(ref_token)),
                }
            }
        }
    }

    for token in token_tree {
        _expand_templates(Cow::Owned(token), &mut stack)
    }

    stack
}
