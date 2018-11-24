use crate::guid::Guid;
use crate::utils::read_len_prefixed_utf16_string;
use crate::utils::{datetime_from_filetime, FileTime};
use byteorder::{LittleEndian, ReadBytesExt};
use chrono::{DateTime, Utc};
use std::{
    borrow::Cow,
    fmt::Debug,
    io::{self, Cursor, Read},
    rc::Rc,
};

use failure::Error;
use log::{error, log};
use std::collections::HashMap;

pub type Name<'a> = Cow<'a, str>;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLValueType {
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

impl BinXMLValueType {
    pub fn from_u8(byte: u8) -> BinXMLValueType {
        match byte {
            0x00 => BinXMLValueType::NullType,
            0x01 => BinXMLValueType::StringType,
            0x02 => BinXMLValueType::AnsiStringType,
            0x03 => BinXMLValueType::Int8Type,
            0x04 => BinXMLValueType::UInt8Type,
            0x05 => BinXMLValueType::Int16Type,
            0x06 => BinXMLValueType::UInt16Type,
            0x07 => BinXMLValueType::Int32Type,
            0x08 => BinXMLValueType::UInt32Type,
            0x09 => BinXMLValueType::Int64Type,
            0x0a => BinXMLValueType::UInt64Type,
            0x0b => BinXMLValueType::Real32Type,
            0x0c => BinXMLValueType::Real64Type,
            0x0d => BinXMLValueType::BoolType,
            0x0e => BinXMLValueType::BinaryType,
            0x0f => BinXMLValueType::GuidType,
            0x10 => BinXMLValueType::SizeTType,
            0x11 => BinXMLValueType::FileTimeType,
            0x12 => BinXMLValueType::SysTimeType,
            0x13 => BinXMLValueType::SidType,
            0x14 => BinXMLValueType::HexInt32Type,
            0x15 => BinXMLValueType::HexInt64Type,
            0x20 => BinXMLValueType::EvtHandle,
            0x21 => BinXMLValueType::BinXmlType,
            0x23 => BinXMLValueType::EvtXml,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXMLValue<'a> {
    NullType,
    // String may originate in substitution.
    StringType(Cow<'a, str>),
    AnsiStringType(Cow<'a, str>),
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
    BinaryType(&'a [u8]),
    GuidType(Guid),
    SizeTType(usize),
    FileTimeType(DateTime<Utc>),
    SysTimeType,
    SidType,
    HexInt32Type,
    HexInt64Type(String),
    EvtHandle,
    // Because of the recursive type, we instantiate this enum via a method of the Deserializer
    BinXmlType(Vec<BinXMLDeserializedTokens<'a>>),
    EvtXml,
}

impl<'a> Into<Cow<'a, str>> for BinXMLValue<'a> {
    fn into(self) -> Cow<'a, str> {
        match self {
            BinXMLValue::NullType => Cow::Borrowed(""),
            BinXMLValue::StringType(s) => s,
            BinXMLValue::AnsiStringType(s) => s,
            BinXMLValue::Int8Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::UInt8Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::Int16Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::UInt16Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::Int32Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::UInt32Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::Int64Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::UInt64Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::Real32Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::Real64Type(num) => Cow::Owned(num.to_string()),
            BinXMLValue::BoolType(num) => Cow::Owned(num.to_string()),
            BinXMLValue::BinaryType(bytes) => Cow::Owned(format!("{:?}", bytes)),
            BinXMLValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXMLValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXMLValue::FileTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXMLValue::SysTimeType => unimplemented!(),
            BinXMLValue::SidType => unimplemented!(),
            BinXMLValue::HexInt32Type => unimplemented!(),
            BinXMLValue::HexInt64Type(hex_string) => Cow::Owned(hex_string),
            BinXMLValue::EvtHandle => {
                panic!("Unsupported conversion, call `expand_templates` first")
            }
            BinXMLValue::BinXmlType(_) => {
                panic!("Unsupported conversion, call `expand_templates` first")
            }
            BinXMLValue::EvtXml => panic!("Unsupported conversion, call `expand_templates` first"),
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq)]
pub enum BinXMLRawToken {
    EndOfStream,
    // True if has attributes, otherwise false.
    OpenStartElement(OpenStartElementTokenMeta),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value,
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

impl BinXMLRawToken {
    pub fn from_u8(byte: u8) -> Option<BinXMLRawToken> {
        match byte {
            0x00 => Some(BinXMLRawToken::EndOfStream),
            // <Event>
            0x01 => Some(BinXMLRawToken::OpenStartElement(
                OpenStartElementTokenMeta {
                    has_attributes: false,
                },
            )),
            0x41 => Some(BinXMLRawToken::OpenStartElement(
                OpenStartElementTokenMeta {
                    has_attributes: true,
                },
            )),
            // Indicates end of start element
            0x02 => Some(BinXMLRawToken::CloseStartElement),
            0x03 => Some(BinXMLRawToken::CloseEmptyElement),
            // </Event>
            0x04 => Some(BinXMLRawToken::CloseElement),
            0x05 | 0x45 => Some(BinXMLRawToken::Value),
            0x06 => Some(BinXMLRawToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: false,
            })),
            0x46 => Some(BinXMLRawToken::Attribute(AttributeTokenMeta {
                more_attributes_expected: true,
            })),
            0x07 | 0x47 => Some(BinXMLRawToken::CDataSection),
            0x08 | 0x48 => Some(BinXMLRawToken::EntityReference),
            0x0a | 0x49 => Some(BinXMLRawToken::ProcessingInstructionTarget),
            0x0b => Some(BinXMLRawToken::ProcessingInstructionData),
            0x0c => Some(BinXMLRawToken::TemplateInstance),
            0x0d => Some(BinXMLRawToken::NormalSubstitution),
            0x0e => Some(BinXMLRawToken::ConditionalSubstitution),
            0x0f => Some(BinXMLRawToken::StartOfStream),
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
pub enum BinXMLDeserializedTokens<'a> {
    FragmentHeader(BinXMLFragmentHeader),
    TemplateInstance(BinXMLTemplate<'a>),
    OpenStartElement(BinXMLOpenStartElement<'a>),
    AttributeList,
    Attribute(BinXMLAttribute<'a>),
    CloseStartElement,
    CloseEmptyElement,
    CloseElement,
    Value(BinXMLValue<'a>),
    CDATASection,
    CharRef,
    EntityRef,
    PITarget,
    PIData,
    Substitution(TemplateSubstitutionDescriptor),
    EndOfStream,
    StartOfStream,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLOpenStartElement<'a> {
    pub data_size: u32,
    pub name: Cow<'a, str>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplateDefinition<'a> {
    pub next_template_offset: u32,
    pub template_guid: Guid,
    pub data_size: u32,
    pub tokens: Vec<BinXMLDeserializedTokens<'a>>,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLTemplate<'a> {
    pub definition: Rc<BinXMLTemplateDefinition<'a>>,
    pub substitution_array: Vec<BinXMLValue<'a>>,
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

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum OwnedModel<'a> {
    OpenElement(XmlElement<'a>),
    CloseElement,
    String(Cow<'a, str>),
    EndOfStream,
    StartOfStream,
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
    Text(Cow<'a, str>),
    Substitution,
    CharacterEntityReference,
    EntityReference,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub struct BinXMLAttribute<'a> {
    pub name: Cow<'a, str>,
}
