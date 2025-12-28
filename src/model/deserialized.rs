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

/// Processing instruction target name.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLProcessingInstructionTarget {
    pub name: BinXmlNameRef,
}

/// Open-start element token payload (name and data size).
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLOpenStartElement {
    pub data_size: u32,
    pub name: BinXmlNameRef,
}

/// Template definition header stored in the chunk template table.
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
            guid = self.guid,
            size = self.data_size
        )
    }
}

/// Parsed template definition with its token stream.
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplateDefinition<'a> {
    pub header: BinXmlTemplateDefinitionHeader,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
}

/// Entity reference token payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXmlEntityReference {
    pub name: BinXmlNameRef,
}

/// Template instance token payload with substitutions.
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXmlTemplateRef<'a> {
    pub template_id: u32,
    pub template_def_offset: ChunkOffset,
    /// When the template definition header is embedded inline in the record's TemplateInstance,
    /// we can read the template GUID directly. Otherwise, the GUID lives in the template
    /// definition referenced by `template_def_offset` (typically in the chunk template table).
    pub template_guid: Option<Guid>,
    pub substitution_array: Vec<BinXMLDeserializedTokens<'a>>,
}

/// Descriptor for a template substitution value payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct TemplateValueDescriptor {
    pub size: u16,
    pub value_type: BinXmlValueType,
}

/// Placeholder descriptor within a template definition.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct TemplateSubstitutionDescriptor {
    // Zero-based (0 is first replacement)
    pub substitution_index: u16,
    pub value_type: BinXmlValueType,
    pub ignore: bool,
    /// True for conditional substitutions; optional values may be omitted when empty.
    pub optional: bool,
}

/// Fragment header at the start of a BinXML stream.
#[repr(C)]
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLFragmentHeader {
    pub major_version: u8,
    pub minor_version: u8,
    pub flags: u8,
}

/// Attribute token payload.
#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
pub struct BinXMLAttribute {
    pub name: BinXmlNameRef,
}
