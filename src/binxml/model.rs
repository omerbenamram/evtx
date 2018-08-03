use binxml::utils::read_len_prefixed_utf16_string;
use byteorder::{LittleEndian, ReadBytesExt};
use guid::Guid;
use std::io::{self, Cursor, Read};

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLValue {
    NullType,
    StringType(String),
    AnsiStringType(String),
    Int8Type(i8),
    UInt8Type(u8),
    Int16Type(i16),
    UInt16Type(u16),
    Int32Type(i32),
    UInt32Type(u32),
    Int64Type(i64),
    UInt64Type(u64),
    Real32Type(f32),
    Real64Type(f64),
    BoolType(bool),
    // TODO: make generic over lifetime in the future
//    BinaryType(&'a [u8]),
    GuidType(Guid),
    SizeTType(usize),
    // TODO: check that this is actually i64
    FileTimeType(i64),
    SysTimeType,
    SidType,
    HexInt32Type,
    HexInt64Type,
    EvtHandle,
    BinXmlType,
    EvtXml,
}

impl BinXMLValue {
    pub fn read(stream: &mut Cursor<&[u8]>) -> io::Result<BinXMLValue> {
        let token_value = BinXMLToken::from_u8(stream.read_u8()?).expect("Unexpected byte for token");
        assert_eq!(token_value, BinXMLToken::TextValue, "Token must be 0x5 | 0x45");
        let value_type = stream.read_u8()?;

        match value_type {
            0x00 => Ok(BinXMLValue::NullType),
            0x01 => Ok(BinXMLValue::StringType(
                read_len_prefixed_utf16_string(stream, false)?.expect("String cannot be empty"),
            )),
//            0x02 => Ok(BinXMLValue::AnsiStringType),
            0x03 => Ok(BinXMLValue::Int8Type(stream.read_u8()? as i8)),
            0x04 => Ok(BinXMLValue::UInt8Type(stream.read_u8()?)),
            0x05 => Ok(BinXMLValue::Int16Type(stream.read_u16::<LittleEndian>()? as i16)),
            0x06 => Ok(BinXMLValue::UInt16Type(stream.read_u16::<LittleEndian>()?)),
            0x07 => Ok(BinXMLValue::Int32Type(stream.read_u32::<LittleEndian>()? as i32)),
            0x08 => Ok(BinXMLValue::UInt32Type(stream.read_u32::<LittleEndian>()?)),
            0x09 => Ok(BinXMLValue::Int64Type(stream.read_u64::<LittleEndian>()? as i64)),
            0x0a => Ok(BinXMLValue::UInt64Type(stream.read_u64::<LittleEndian>()?)),
//            0x0b => Ok(BinXMLValue::Real32Type),
//            0x0c => Ok(BinXMLValue::Real64Type),
//            0x0d => Ok(BinXMLValue::BoolType),
//            0x0e => Ok(BinXMLValue::BinaryType),
//            0x0f => Ok(BinXMLValue::GuidType),
//            0x10 => Ok(BinXMLValue::SizeTType),
//            0x11 => Ok(BinXMLValue::FileTimeType),
//            0x12 => Ok(BinXMLValue::SysTimeType),
//            0x13 => Ok(BinXMLValue::SidType),
//            0x14 => Ok(BinXMLValue::HexInt32Type),
//            0x15 => Ok(BinXMLValue::HexInt64Type),
//            0x20 => Ok(BinXMLValue::EvtHandle),
//            0x21 => Ok(BinXMLValue::BinXmlType),
//            0x23 => Ok(BinXMLValue::EvtXml),
            _ => unimplemented!("{}", value_type),
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq)]
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
                more_attributes_expected: false,
            })),
            0x46 => Some(BinXMLToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: true,
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

#[derive(Debug, PartialOrd, PartialEq)]
pub struct OpenStartElementTokenMeta {
    pub has_attributes: bool,
}

#[derive(Debug, PartialOrd, PartialEq)]
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
    pub data: BinXMLValue
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLName {
    pub name: Option<String>,
}
