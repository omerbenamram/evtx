use crate::binxml::name::BinXmlName;
use crate::binxml::value_variant::BinXmlValue;

use crate::err::EvtxError;
use log::error;
use std::borrow::Cow;

type Name<'a> = BinXmlName<'a>;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum XmlModel<'a> {
    OpenElement(XmlElement<'a>),
    CloseElement,
    PI(BinXmlPI<'a>),
    EntityRef(Cow<'a, Name<'a>>),
    Value(Cow<'a, BinXmlValue<'a>>),
    EndOfStream,
    StartOfStream,
}

#[derive(Debug)]
pub(crate) struct XmlElementBuilder<'a> {
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
    pub fn name(&mut self, name: Cow<'a, Name<'a>>) {
        self.name = Some(name);
    }

    pub fn attribute_name(&mut self, name: Cow<'a, Name<'a>>) {
        match self.current_attribute_name {
            None => self.current_attribute_name = Some(name),
            Some(_) => {
                error!("invalid state, overriding name");
                self.current_attribute_name = Some(name);
            }
        }
    }

    pub fn attribute_value(&mut self, value: Cow<'a, BinXmlValue<'a>>) -> Result<(), EvtxError> {
        // If we are in an attribute value without a name, simply ignore the request.
        // This is consistent with what windows is doing.
        if self.current_attribute_name.is_none() {
            return Ok(());
        }

        match self.current_attribute_value {
            None => self.current_attribute_value = Some(value),
            Some(_) => {
                return Err(EvtxError::FailedToCreateRecordModel(
                    "invalid state, there should not be a value",
                ))
            }
        }

        self.attributes.push(XmlAttribute {
            name: self.current_attribute_name.take().unwrap(),
            value: self.current_attribute_value.take().unwrap(),
        });

        Ok(())
    }

    pub fn finish(self) -> Result<XmlElement<'a>, EvtxError> {
        Ok(XmlElement {
            name: self.name.ok_or(EvtxError::FailedToCreateRecordModel(
                "Element name should be set",
            ))?,
            attributes: self.attributes,
        })
    }
}

pub(crate) struct XmlPIBuilder<'a> {
    name: Option<Cow<'a, Name<'a>>>,
    data: Option<Cow<'a, str>>,
}

impl<'a> XmlPIBuilder<'a> {
    pub fn new() -> Self {
        XmlPIBuilder {
            name: None,
            data: None,
        }
    }
    pub fn name(mut self, name: Cow<'a, Name<'a>>) -> Self {
        self.name = Some(name);
        self
    }

    pub fn data(mut self, data: Cow<'a, str>) -> Self {
        self.data = Some(data);
        self
    }

    pub fn finish(self) -> XmlModel<'a> {
        XmlModel::PI(BinXmlPI {
            name: self.name.expect("Element name should be set"),
            data: self.data.expect("Data should be set"),
        })
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

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlPI<'a> {
    pub name: Cow<'a, Name<'a>>,
    pub data: Cow<'a, str>,
}
