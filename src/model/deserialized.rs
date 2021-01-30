use crate::binxml::name::BinXmlNameRef;
use crate::binxml::value_variant::{BinXmlValue, BinXmlValueType};

use crate::ChunkOffset;
use std::fmt::{self, Formatter};
use winstructs::guid::Guid;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLDeserializedTokens<'a> {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstance(BinXmlTemplateRef<'a>),
    OpenStartElement(BinXMLOpenStartElement),
    AttributeList,
    Attribute(BinXMLAttribute),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value(BinXmlValue<'a>),
    CDATASection,
    CharRef,
    EntityRef(BinXmlEntityReference),
    PITarget(BinXMLProcessingInstructionTarget),
    PIData(String),
    Substitution(TemplateSubstitutionDescriptor),
    EndOfStream,
    StartOfStream,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLProcessingInstructionTarget {
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLOpenStartElement {
    pub data_size: u32,
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplateDefinitionHeader {
    /// A pointer to the next template in the bucket.
    pub next_template_offset: ChunkOffset,
    pub guid: Guid,
    pub data_size: u32,
}

impl fmt::Display for BinXmlTemplateDefinitionHeader {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<BinXmlTemplateDefinitionHeader - id: {guid}, data_size: {size}>",
            guid = self.guid.to_string(),
            size = self.data_size
        )
    }
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplateDefinition<'a> {
    pub header: BinXmlTemplateDefinitionHeader,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlEntityReference {
    pub name: BinXmlNameRef,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplateRef<'a> {
    pub template_def_offset: ChunkOffset,
    pub substitution_array: Vec<BinXMLDeserializedTokens<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct TemplateValueDescriptor {
    pub size: u16,
    pub value_type: BinXmlValueType,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct TemplateSubstitutionDescriptor {
    // Zero-based (0 is first replacement)
    pub substitution_index: u16,
    pub value_type: BinXmlValueType,
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
pub struct BinXMLAttribute {
    pub name: BinXmlNameRef,
}
