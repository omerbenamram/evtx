use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;

use log::error;
use std::borrow::Cow;

type Name<'a> = BinXmlName<'a>;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum XmlModel<'a> {
    OpenElement(XmlElement<'a>),
    CloseElement,
    Value(Cow<'a, BinXmlValue<'a>>),
    EndOfStream,
    StartOfStream,
}

pub struct XmlElementBuilder<'a> {
    name: Option<Cow<'a, Name<'a>>>,
    attributes: Vec<XmlAttribute<'a>>,
    current_attribute_name: Option<Cow<'a, Name<'a>>>,
    current_attribute_value: Option<Cow<'a, BinXmlValue<'a>>>,
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
    pub fn name(mut self, name: Cow<'a, Name<'a>>) -> Self {
        self.name = Some(name);
        self
    }

    pub fn attribute_name(mut self, name: Cow<'a, Name<'a>>) -> Self {
        match self.current_attribute_name {
            None => self.current_attribute_name = Some(name),
            Some(name) => {
                error!("invalid state, overriding name");
                self.current_attribute_name = Some(name);
            }
        }
        self
    }

    pub fn attribute_value(mut self, value: Cow<'a, BinXmlValue<'a>>) -> Self {
        debug_assert!(
            self.current_attribute_name.is_some(),
            "There should be a name"
        );
        match self.current_attribute_value {
            None => self.current_attribute_value = Some(value),
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
    pub name: Cow<'a, Name<'a>>,
    pub value: Cow<'a, BinXmlValue<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct XmlElement<'a> {
    pub name: Cow<'a, Name<'a>>,
    pub attributes: Vec<XmlAttribute<'a>>,
}
