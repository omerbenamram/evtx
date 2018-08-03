use byteorder::{LittleEndian, ReadBytesExt};
use guid::Guid;
use std::io::{self, Cursor, Read};

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLValueTypes {
    NullType,
    StringType,
    AnsiStringType,
    Int8Type,
    UInt8Type,
    Int16Type,
    UInt16Type,
    Int32Type,
    UInt32Type,
    Int64Type,
    UInt64Type,
    Real32Type,
    Real64Type,
    BoolType,
    BinaryType,
    GuidType,
    SizeTType,
    FileTimeType,
    SysTimeType,
    SidType,
    HexInt32Type,
    HexInt64Type,
    EvtHandle,
    BinXmlType,
    EvtXml,
}

impl BinXMLValueTypes {
    pub fn from_u8(byte: u8) -> Option<BinXMLValueTypes> {
        match byte {
            0x00 => Some(BinXMLValueTypes::NullType),
            0x01 => Some(BinXMLValueTypes::StringType),
            0x02 => Some(BinXMLValueTypes::AnsiStringType),
            0x03 => Some(BinXMLValueTypes::Int8Type),
            0x04 => Some(BinXMLValueTypes::UInt8Type),
            0x05 => Some(BinXMLValueTypes::Int16Type),
            0x06 => Some(BinXMLValueTypes::UInt16Type),
            0x07 => Some(BinXMLValueTypes::Int32Type),
            0x08 => Some(BinXMLValueTypes::UInt32Type),
            0x09 => Some(BinXMLValueTypes::Int64Type),
            0x0a => Some(BinXMLValueTypes::UInt64Type),
            0x0b => Some(BinXMLValueTypes::Real32Type),
            0x0c => Some(BinXMLValueTypes::Real64Type),
            0x0d => Some(BinXMLValueTypes::BoolType),
            0x0e => Some(BinXMLValueTypes::BinaryType),
            0x0f => Some(BinXMLValueTypes::GuidType),
            0x10 => Some(BinXMLValueTypes::SizeTType),
            0x11 => Some(BinXMLValueTypes::FileTimeType),
            0x12 => Some(BinXMLValueTypes::SysTimeType),
            0x13 => Some(BinXMLValueTypes::SidType),
            0x14 => Some(BinXMLValueTypes::HexInt32Type),
            0x15 => Some(BinXMLValueTypes::HexInt64Type),
            0x20 => Some(BinXMLValueTypes::EvtHandle),
            0x21 => Some(BinXMLValueTypes::BinXmlType),
            0x23 => Some(BinXMLValueTypes::EvtXml),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub enum BinXMLToken {
    EndOfStream,
    // True if has attributes, otherwise false.
    OpenStartElement(OpenStartElementTokenMeta),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    TextValue,
    Attribute(AttributeTokenMeta),
    CDataSection,
    EntityReference,
    ProcessingInstructionTarget,
    ProcessingInstructionData,
    TemplateInstance,
    NormalSubstitution,
    ConditionalSubstitution,
    StartOfStream,
}

impl BinXMLToken {
    pub fn from_u8(byte: u8) -> Option<BinXMLToken> {
        match byte {
            0x00 => Some(BinXMLToken::EndOfStream),
            0x01 => Some(BinXMLToken::OpenStartElement(OpenStartElementTokenMeta {
                has_attributes: false,
            })),
            0x41 => Some(BinXMLToken::OpenStartElement(OpenStartElementTokenMeta {
                has_attributes: true,
            })),
            0x02 => Some(BinXMLToken::CloseStartElement),
            0x03 => Some(BinXMLToken::CloseEmptyElement),
            0x04 => Some(BinXMLToken::CloseElement),
            0x05 | 0x45 => Some(BinXMLToken::TextValue),
            0x06 => Some(BinXMLToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: false
            })),
            0x46 => Some(BinXMLToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: true
            })),
            0x07 | 0x47 => Some(BinXMLToken::CDataSection),
            0x08 | 0x48 => Some(BinXMLToken::EntityReference),
            0x0a | 0x49 => Some(BinXMLToken::ProcessingInstructionTarget),
            0x0b => Some(BinXMLToken::ProcessingInstructionData),
            0x0c => Some(BinXMLToken::TemplateInstance),
            0x0d => Some(BinXMLToken::NormalSubstitution),
            0x0e => Some(BinXMLToken::ConditionalSubstitution),
            0x0f => Some(BinXMLToken::StartOfStream),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct OpenStartElementTokenMeta {
    pub has_attributes: bool,
}

#[derive(Debug)]
pub struct AttributeTokenMeta {
    pub more_attributes_expected: bool,
}


#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLParsedNodes {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstance(BinXMLTemplate),
    OpenStartElement(BinXMLOpenStartElement),
    AttributeList,
    Attribute(BinXMLAttribute),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    ValueText(BinXMLValueText),
    CDATASection,
    CharRef,
    EntityRef,
    PITarget,
    PIData,
    NormalSubstitution,
    ConditionalSubstitution,
    EndOfStream,
    StartOfStream,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct EndOfStream {}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLOpenStartElement {
    pub data_size: u32,
    pub name: BinXMLName,
    pub attribute_list: Option<Vec<BinXMLAttribute>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplate {
    pub template_id: u32,
    pub template_offset: u32,
    pub next_template_offset: u32,
    pub template_guid: Guid,
    // This includes the size of the fragment header, element and end of file token;
    // except for the first 33 bytes of the template definition.
    pub data_size: u32,
}

#[derive(Debug)]
pub struct TemplateValueDescriptor {
    pub value_size: u16,
    pub value_type: u8,
}

#[repr(C)]
#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLFragmentHeader {
    pub major_version: u8,
    pub minor_version: u8,
    pub flags: u8,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLValueText {
    pub raw: String,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLAttribute {
    pub name: BinXMLName,
    pub data: BinXMLValueText
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLName {
    pub name: Option<String>,
}
