use log::{error, log};
use crate::model::deserialized::*;
use std::borrow::Cow;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum OwnedModel<'a> {
    OpenElement(XmlElement<'a>),
    CloseElement,
    String(Cow<'a, str>),
    EndOfStream,
    StartOfStream,
}

pub struct XmlElementBuilder<'a> {
    name: Option<Name<'a>>,
    attributes: Vec<XmlAttribute<'a>>,
    current_attribute_name: Option<Name<'a>>,
    current_attribute_value: Option<Cow<'a, str>>,
}

impl<'a> XmlElementBuilder<'a> {
    pub fn new() -> Self {
        XmlElementBuilder {
            name: None,
            attributes: Vec::new(),
            current_attribute_name: None,
            current_attribute_value: None,
        }
    }
    pub fn name(mut self, name: Name<'a>) -> Self {
        self.name = Some(name);
        self
    }

    pub fn attribute_name(mut self, name: Name<'a>) -> Self {
        match self.current_attribute_name {
            None => self.current_attribute_name = Some(name),
            Some(name) => {
                error!("invalid state, overriding name");
                self.current_attribute_name = Some(name);
            }
        }
        self
    }

    pub fn attribute_value(mut self, value: BinXMLValue<'a>) -> Self {
        assert!(
            self.current_attribute_name.is_some(),
            "There should be a name"
        );
        match self.current_attribute_value {
            None => {
                self.current_attribute_value = Some(match value {
                    BinXMLValue::StringType(cow) => cow,
                    _ => Cow::Owned(format!("{:?}", value)),
                })
            }
            Some(_) => panic!("invalid state, there should not be a value"),
        }

        self.attributes.push(XmlAttribute {
            name: self.current_attribute_name.take().unwrap(),
            value: self.current_attribute_value.take().unwrap(),
        });

        self
    }

    pub fn finish(self) -> XmlElement<'a> {
        XmlElement {
            name: self.name.expect("Element name should be set"),
            attributes: self.attributes,
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct XmlAttribute<'a> {
    pub name: Name<'a>,
    pub value: Cow<'a, str>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct XmlElement<'a> {
    pub name: Name<'a>,
    pub attributes: Vec<XmlAttribute<'a>>,
}
