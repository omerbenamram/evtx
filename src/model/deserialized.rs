use crate::binxml::name::BinXmlName;
use crate::guid::Guid;

use crate::binxml::value_variant::{BinXMLValue, BinXMLValueType};
use std::rc::Rc;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLDeserializedTokens<'a> {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstance(BinXmlTemplate<'a>),
    OpenStartElement(BinXMLOpenStartElement<'a>),
    AttributeList,
    Attribute(BinXMLAttribute<'a>),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value(BinXMLValue<'a>),
    CDATASection,
    CharRef,
    EntityRef(BinXmlEntityReference<'a>),
    PITarget,
    PIData,
    Substitution(TemplateSubstitutionDescriptor),
    EndOfStream,
    StartOfStream,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLOpenStartElement<'a> {
    pub data_size: u32,
    pub name: BinXmlName<'a>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplateDefinition<'a> {
    pub next_template_offset: u32,
    pub template_guid: Guid,
    pub data_size: u32,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlEntityReference<'a> {
    pub name: BinXmlName<'a>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplate<'a> {
    pub definition: Rc<BinXMLTemplateDefinition<'a>>,
    pub substitution_array: Vec<BinXMLValue<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct TemplateValueDescriptor {
    pub size: u16,
    pub value_type: BinXMLValueType,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct TemplateSubstitutionDescriptor {
    // Zero-based (0 is first replacement)
    pub substitution_index: u16,
    pub value_type: BinXMLValueType,
    pub ignore: bool,
}

#[repr(C)]
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLFragmentHeader {
    pub major_version: u8,
    pub minor_version: u8,
    pub flags: u8,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXmlAttributeValue<'a> {
    Text(BinXmlName<'a>),
    Substitution,
    CharacterEntityReference,
    EntityReference,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLAttribute<'a> {
    pub name: BinXmlName<'a>,
}
