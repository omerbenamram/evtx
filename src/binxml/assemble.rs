use crate::binxml::value_variant::BinXmlValue;
use crate::model::deserialized::{BinXMLDeserializedTokens, BinXmlTemplate};
use crate::model::xml::{XmlElementBuilder, XmlModel};
use crate::xml_output::BinXmlOutput;
use failure::{format_err, Error};
use log::trace;
use std::borrow::Cow;
use std::io::Write;
use std::mem;

pub fn parse_tokens<W: Write, T: BinXmlOutput<W>>(
    tokens: Vec<BinXMLDeserializedTokens>,
    visitor: &mut T,
) -> Result<(), Error> {
    let expanded_tokens = expand_templates(tokens);
    let record_model = create_record_model(expanded_tokens);

    visitor.visit_start_of_stream()?;

    let mut stack = vec![];

    for owned_token in record_model {
        match owned_token {
            XmlModel::OpenElement(open_element) => {
                stack.push(open_element);
                visitor.visit_open_start_element(stack.last().ok_or_else(|| {
                    format_err!("Invalid parser state - expected stack to be non-empty")
                })?)?
            }
            XmlModel::CloseElement => {
                let close_element = stack.pop().ok_or_else(|| {
                    format_err!("Invalid parser state - expected stack to be non-empty")
                })?;
                visitor.visit_close_element(&close_element)?
            }
            XmlModel::Value(s) => visitor.visit_characters(&s)?,
            XmlModel::EndOfStream => visitor.visit_end_of_stream()?,
            // Sometimes there are multiple fragment headers,
            // but we only need to write start of stream once.
            XmlModel::StartOfStream => {}
        };
    }

    Ok(())
}

pub fn create_record_model<'a>(
    tokens: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) -> Vec<XmlModel<'a>> {
    let mut current_element: Option<XmlElementBuilder> = None;
    let mut model: Vec<XmlModel> = Vec::with_capacity(tokens.len());

    for token in tokens {
        // Handle all places where we don't care if it's an Owned or a Borrowed value.
        match token {
            Cow::Owned(BinXMLDeserializedTokens::FragmentHeader(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::FragmentHeader(_)) => {}
            Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(_)) => {
                panic!("Call `expand_templates` before calling this function")
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
                    None => panic!("close start - Bad parser state"),
                    Some(builder) => model.push(XmlModel::OpenElement(builder.finish())),
                };
            }
            Cow::Owned(BinXMLDeserializedTokens::CDATASection)
            | Cow::Borrowed(BinXMLDeserializedTokens::CDATASection) => {}
            Cow::Owned(BinXMLDeserializedTokens::CharRef)
            | Cow::Borrowed(BinXMLDeserializedTokens::CharRef) => {}
            Cow::Owned(BinXMLDeserializedTokens::EntityRef(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::EntityRef(_)) => {
                unimplemented!("EntityRef not implemented")
            }
            Cow::Owned(BinXMLDeserializedTokens::PITarget)
            | Cow::Borrowed(BinXMLDeserializedTokens::PITarget) => {}
            Cow::Owned(BinXMLDeserializedTokens::PIData)
            | Cow::Borrowed(BinXMLDeserializedTokens::PIData) => {}
            Cow::Owned(BinXMLDeserializedTokens::Substitution(_))
            | Cow::Borrowed(BinXMLDeserializedTokens::Substitution(_)) => {
                panic!("Call `expand_templates` before calling this function")
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
                    None => panic!("close empty - Bad parser state"),
                    Some(builder) => {
                        model.push(XmlModel::OpenElement(builder.finish()));
                        model.push(XmlModel::CloseElement);
                    }
                };
            }

            Cow::Owned(BinXMLDeserializedTokens::Attribute(attr)) => {
                trace!("BinXMLDeserializedTokens::Attribute(attr) - {:?}", attr);
                match current_element.take() {
                    None => panic!("attribute - Bad parser state"),
                    Some(builder) => {
                        current_element = Some(builder.attribute_name(Cow::Owned(attr.name)));
                    }
                };
            }

            Cow::Borrowed(BinXMLDeserializedTokens::Attribute(attr)) => {
                trace!("BinXMLDeserializedTokens::Attribute(attr) - {:?}", attr);
                match current_element.take() {
                    None => panic!("attribute - Bad parser state"),
                    Some(builder) => {
                        current_element = Some(builder.attribute_name(Cow::Borrowed(&attr.name)));
                    }
                };
            }

            Cow::Owned(BinXMLDeserializedTokens::OpenStartElement(elem)) => {
                trace!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let builder = XmlElementBuilder::new();
                current_element = Some(builder.name(Cow::Owned(elem.name)));
            }
            Cow::Borrowed(BinXMLDeserializedTokens::OpenStartElement(elem)) => {
                trace!(
                    "BinXMLDeserializedTokens::OpenStartElement(elem) - {:?}",
                    elem.name
                );
                let builder = XmlElementBuilder::new();
                current_element = Some(builder.name(Cow::Borrowed(&elem.name)));
            }

            Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Owned(value))) => {
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
            Cow::Borrowed(BinXMLDeserializedTokens::Value(Cow::Owned(value)))
            | Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Borrowed(value))) => {
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

            // Same as above, but `value` is `&&BinXmlValue` which is not compatible with the match.
            Cow::Borrowed(BinXMLDeserializedTokens::Value(Cow::Borrowed(value))) => {
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
        }
    }
    model
}

fn expand_owned_template<'a>(
    mut template: BinXmlTemplate<'a>,
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) {
    // If the template owns the definition, we can consume the tokens.
    let tokens: Vec<Cow<'a, BinXMLDeserializedTokens<'a>>> = match template.definition {
        Cow::Owned(owned_def) => owned_def.tokens.into_iter().map(Cow::Owned).collect(),
        Cow::Borrowed(ref_def) => ref_def.tokens.iter().map(Cow::Borrowed).collect(),
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
                    &mut template.substitution_array
                        [substitution_descriptor.substitution_index as usize],
                    BinXmlValue::NullType,
                );

                _expand_templates(
                    Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Owned(value))),
                    stack,
                );
            }
        } else {
            _expand_templates(token, stack);
        }
    }
}

fn expand_borrowed_template<'a>(
    template: &'a BinXmlTemplate<'a>,
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) {
    // Here we can always use refs, since even if the definition is owned by the template,
    // we do not own it.
    for token in template.definition.as_ref().tokens.iter() {
        if let BinXMLDeserializedTokens::Substitution(ref substitution_descriptor) = token {
            if substitution_descriptor.ignore {
                continue;
            } else {
                let value = &template.substitution_array
                    [substitution_descriptor.substitution_index as usize];

                _expand_templates(
                    Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Borrowed(value))),
                    stack,
                );
            }
        } else {
            _expand_templates(Cow::Borrowed(token), stack);
        }
    }
}

fn _expand_templates<'a>(
    token: Cow<'a, BinXMLDeserializedTokens<'a>>,
    stack: &mut Vec<Cow<'a, BinXMLDeserializedTokens<'a>>>,
) {
    match token {
        // Owned values can be consumed when flatting, and passed on as owned.
        Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Owned(BinXmlValue::BinXmlType(
            tokens,
        )))) => {
            for token in tokens.into_iter() {
                _expand_templates(Cow::Owned(token), stack);
            }
        }

        // All borrowed values are flattened and kept borrowed.
        Cow::Owned(BinXMLDeserializedTokens::Value(Cow::Borrowed(BinXmlValue::BinXmlType(
            tokens,
        ))))
        | Cow::Borrowed(BinXMLDeserializedTokens::Value(Cow::Owned(BinXmlValue::BinXmlType(
            tokens,
        ))))
        | Cow::Borrowed(BinXMLDeserializedTokens::Value(Cow::Borrowed(BinXmlValue::BinXmlType(
            tokens,
        )))) => {
            for token in tokens.iter() {
                _expand_templates(Cow::Borrowed(token), stack);
            }
        }

        // Actual template handling.
        Cow::Owned(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            expand_owned_template(template, stack);
        }
        Cow::Borrowed(BinXMLDeserializedTokens::TemplateInstance(template)) => {
            expand_borrowed_template(template, stack);
        }

        _ => stack.push(token),
    }
}

pub fn expand_templates(
    token_tree: Vec<BinXMLDeserializedTokens>,
) -> Vec<Cow<BinXMLDeserializedTokens>> {
    // We can assume the new tree will be at least as big as the old one.
    let mut stack = Vec::with_capacity(token_tree.len());

    for token in token_tree {
        _expand_templates(Cow::Owned(token), &mut stack)
    }

    stack
}
