pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::ParsingContext;
use crate::error::Error;
use crate::guid::Guid;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::ntsid::Sid;
use crate::utils::{datetime_from_filetime, read_len_prefixed_utf16_string};
use chrono::{DateTime, Utc};
use std::borrow::Cow;
use std::io::Cursor;

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
    SidType(Sid),
    HexInt32Type(String),
    HexInt64Type(String),
    EvtHandle,
    // Because of the recursive type, we instantiate this enum via a method of the Deserializer
    BinXmlType(Vec<BinXMLDeserializedTokens<'a>>),
    EvtXml,
}

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
    pub fn from_u8(byte: u8) -> Option<BinXMLValueType> {
        match byte {
            0x00 => Some(BinXMLValueType::NullType),
            0x01 => Some(BinXMLValueType::StringType),
            0x02 => Some(BinXMLValueType::AnsiStringType),
            0x03 => Some(BinXMLValueType::Int8Type),
            0x04 => Some(BinXMLValueType::UInt8Type),
            0x05 => Some(BinXMLValueType::Int16Type),
            0x06 => Some(BinXMLValueType::UInt16Type),
            0x07 => Some(BinXMLValueType::Int32Type),
            0x08 => Some(BinXMLValueType::UInt32Type),
            0x09 => Some(BinXMLValueType::Int64Type),
            0x0a => Some(BinXMLValueType::UInt64Type),
            0x0b => Some(BinXMLValueType::Real32Type),
            0x0c => Some(BinXMLValueType::Real64Type),
            0x0d => Some(BinXMLValueType::BoolType),
            0x0e => Some(BinXMLValueType::BinaryType),
            0x0f => Some(BinXMLValueType::GuidType),
            0x10 => Some(BinXMLValueType::SizeTType),
            0x11 => Some(BinXMLValueType::FileTimeType),
            0x12 => Some(BinXMLValueType::SysTimeType),
            0x13 => Some(BinXMLValueType::SidType),
            0x14 => Some(BinXMLValueType::HexInt32Type),
            0x15 => Some(BinXMLValueType::HexInt64Type),
            0x20 => Some(BinXMLValueType::EvtHandle),
            0x21 => Some(BinXMLValueType::BinXmlType),
            0x23 => Some(BinXMLValueType::EvtXml),
            _ => None,
        }
    }
}

impl<'a> BinXMLValue<'a> {
    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        ctx: &ParsingContext,
    ) -> Result<Self, Error> {
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXMLValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        let data = Self::deserialize_value_type(&value_type, cursor)?;

        Ok(data)
    }

    pub fn deserialize_value_type(
        value_type: &BinXMLValueType,
        cursor: &mut Cursor<&'a [u8]>,
    ) -> Result<BinXMLValue<'a>, Error> {
        match value_type {
            BinXMLValueType::NullType => Ok(BinXMLValue::NullType),
            BinXMLValueType::StringType => Ok(BinXMLValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(cursor, false)
                    .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
                    .unwrap_or("".to_owned()),
            ))),
            BinXMLValueType::AnsiStringType => unimplemented!(),
            BinXMLValueType::Int8Type => Ok(BinXMLValue::Int8Type(try_read!(cursor, i8))),
            BinXMLValueType::UInt8Type => Ok(BinXMLValue::UInt8Type(try_read!(cursor, u8))),
            BinXMLValueType::Int16Type => Ok(BinXMLValue::Int16Type(try_read!(cursor, i16))),
            BinXMLValueType::UInt16Type => Ok(BinXMLValue::UInt16Type(try_read!(cursor, u16))),
            BinXMLValueType::Int32Type => Ok(BinXMLValue::Int32Type(try_read!(cursor, i32))),
            BinXMLValueType::UInt32Type => Ok(BinXMLValue::UInt32Type(try_read!(cursor, u32))),
            BinXMLValueType::Int64Type => Ok(BinXMLValue::Int64Type(try_read!(cursor, i64))),
            BinXMLValueType::UInt64Type => Ok(BinXMLValue::UInt64Type(try_read!(cursor, u64))),
            BinXMLValueType::Real32Type => unimplemented!("Real32Type"),
            BinXMLValueType::Real64Type => unimplemented!("Real64Type"),
            BinXMLValueType::BoolType => unimplemented!("BoolType"),
            BinXMLValueType::BinaryType => unimplemented!("BinaryType"),
            BinXMLValueType::GuidType => {
                Ok(BinXMLValue::GuidType(Guid::from_stream(cursor).map_err(
                    |e| Error::other("Failed to read GUID from stream", cursor.position()),
                )?))
            }
            BinXMLValueType::SizeTType => unimplemented!("SizeTType"),
            BinXMLValueType::FileTimeType => Ok(BinXMLValue::FileTimeType(datetime_from_filetime(
                try_read!(cursor, u64),
            ))),
            BinXMLValueType::SysTimeType => unimplemented!("SysTimeType"),
            BinXMLValueType::SidType => {
                Ok(BinXMLValue::SidType(Sid::from_stream(cursor).map_err(
                    |_| Error::other("Failed to read NTSID from stream", cursor.position()),
                )?))
            }
            BinXMLValueType::HexInt32Type => Ok(BinXMLValue::HexInt32Type(format!(
                "0x{:2x}",
                try_read!(cursor, i32)
            ))),
            BinXMLValueType::HexInt64Type => Ok(BinXMLValue::HexInt64Type(format!(
                "0x{:2x}",
                try_read!(cursor, i64)
            ))),
            BinXMLValueType::EvtHandle => unimplemented!("EvtHandle"),
            BinXMLValueType::BinXmlType => {
                Ok(BinXMLValue::BinXmlType(read_until_end_of_stream(cursor)?))
            }
            BinXMLValueType::EvtXml => unimplemented!("EvtXml"),
        }
    }
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
            BinXMLValue::SysTimeType => unimplemented!("SysTimeType"),
            BinXMLValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXMLValue::HexInt32Type(hex_string) => Cow::Owned(hex_string),
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
