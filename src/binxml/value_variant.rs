pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::{BinXmlDeserializer, Context};
use crate::error::Error;

use crate::guid::Guid;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::ntsid::Sid;
use crate::utils::{datetime_from_filetime, read_len_prefixed_utf16_string};
use chrono::{DateTime, Utc};
use std::borrow::Cow;
use std::io::Cursor;
use std::rc::Rc;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXmlValue<'a> {
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
    StringArrayType(Vec<Cow<'a, str>>),
    AnsiStringArrayType,
    Int8ArrayType,
    UInt8ArrayType,
    Int16ArrayType,
    UInt16ArrayType,
    Int32ArrayType,
    UInt32ArrayType,
    Int64ArrayType,
    UInt64ArrayType,
    Real32ArrayType,
    Real64ArrayType,
    BoolArrayType,
    BinaryArrayType,
    GuidArrayType,
    SizeTArrayType,
    FileTimeArrayType,
    SysTimeArrayType,
    SidArrayType,
    HexInt32ArrayType,
    HexInt64ArrayType,
    EvtArrayHandle,
    BinXmlArrayType,
    EvtXmlArrayType,
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXmlValueType {
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
    EvtXmlType,
    StringArrayType,
    AnsiStringArrayType,
    Int8ArrayType,
    UInt8ArrayType,
    Int16ArrayType,
    UInt16ArrayType,
    Int32ArrayType,
    UInt32ArrayType,
    Int64ArrayType,
    UInt64ArrayType,
    Real32ArrayType,
    Real64ArrayType,
    BoolArrayType,
    BinaryArrayType,
    GuidArrayType,
    SizeTArrayType,
    FileTimeArrayType,
    SysTimeArrayType,
    SidArrayType,
    HexInt32ArrayType,
    HexInt64ArrayType,
    EvtHaArrayndle,
    BinXmlArrayType,
    EvtXmlArrayType,
}

impl BinXmlValueType {
    pub fn from_u8(byte: u8) -> Option<BinXmlValueType> {
        match byte {
            0x00 => Some(BinXmlValueType::NullType),
            0x01 => Some(BinXmlValueType::StringType),
            0x02 => Some(BinXmlValueType::AnsiStringType),
            0x03 => Some(BinXmlValueType::Int8Type),
            0x04 => Some(BinXmlValueType::UInt8Type),
            0x05 => Some(BinXmlValueType::Int16Type),
            0x06 => Some(BinXmlValueType::UInt16Type),
            0x07 => Some(BinXmlValueType::Int32Type),
            0x08 => Some(BinXmlValueType::UInt32Type),
            0x09 => Some(BinXmlValueType::Int64Type),
            0x0a => Some(BinXmlValueType::UInt64Type),
            0x0b => Some(BinXmlValueType::Real32Type),
            0x0c => Some(BinXmlValueType::Real64Type),
            0x0d => Some(BinXmlValueType::BoolType),
            0x0e => Some(BinXmlValueType::BinaryType),
            0x0f => Some(BinXmlValueType::GuidType),
            0x10 => Some(BinXmlValueType::SizeTType),
            0x11 => Some(BinXmlValueType::FileTimeType),
            0x12 => Some(BinXmlValueType::SysTimeType),
            0x13 => Some(BinXmlValueType::SidType),
            0x14 => Some(BinXmlValueType::HexInt32Type),
            0x15 => Some(BinXmlValueType::HexInt64Type),
            0x20 => Some(BinXmlValueType::EvtHandle),
            0x21 => Some(BinXmlValueType::BinXmlType),
            0x23 => Some(BinXmlValueType::EvtXmlType),
            0x81 => Some(BinXmlValueType::StringArrayType),
            0x82 => Some(BinXmlValueType::AnsiStringArrayType),
            0x83 => Some(BinXmlValueType::Int8ArrayType),
            0x84 => Some(BinXmlValueType::UInt8ArrayType),
            0x85 => Some(BinXmlValueType::Int16ArrayType),
            0x86 => Some(BinXmlValueType::UInt16ArrayType),
            0x87 => Some(BinXmlValueType::Int32ArrayType),
            0x88 => Some(BinXmlValueType::UInt32ArrayType),
            0x89 => Some(BinXmlValueType::Int64ArrayType),
            0x8a => Some(BinXmlValueType::UInt64ArrayType),
            0x8b => Some(BinXmlValueType::Real32ArrayType),
            0x8c => Some(BinXmlValueType::Real64ArrayType),
            0x8d => Some(BinXmlValueType::BoolArrayType),
            0x8e => Some(BinXmlValueType::BinaryArrayType),
            0x8f => Some(BinXmlValueType::GuidArrayType),
            0x90 => Some(BinXmlValueType::SizeTArrayType),
            0x91 => Some(BinXmlValueType::FileTimeArrayType),
            0x92 => Some(BinXmlValueType::SysTimeArrayType),
            0x93 => Some(BinXmlValueType::SidArrayType),
            0x94 => Some(BinXmlValueType::HexInt32ArrayType),
            0x95 => Some(BinXmlValueType::HexInt64ArrayType),
            _ => None,
        }
    }
}

impl<'c> BinXmlValue<'c> {
    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'c [u8]>,
        ctx: Context<'c>,
    ) -> Result<BinXmlValue<'c>, Error> {
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        let data = Self::deserialize_value_type(&value_type, cursor, Rc::clone(&ctx))?;

        Ok(data)
    }

    pub fn deserialize_value_type(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'c [u8]>,
        ctx: Context<'c>,
    ) -> Result<BinXmlValue<'c>, Error> {
        match value_type {
            BinXmlValueType::NullType => Ok(BinXmlValue::NullType),
            BinXmlValueType::StringType => Ok(BinXmlValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(cursor, false)
                    .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
                    .unwrap_or_else(|| "".to_owned()),
            ))),
            BinXmlValueType::StringArrayType => unimplemented!("StringArray"),
            BinXmlValueType::AnsiStringType => unimplemented!("AnsiString"),
            BinXmlValueType::Int8Type => Ok(BinXmlValue::Int8Type(try_read!(cursor, i8))),
            BinXmlValueType::UInt8Type => Ok(BinXmlValue::UInt8Type(try_read!(cursor, u8))),
            BinXmlValueType::Int16Type => Ok(BinXmlValue::Int16Type(try_read!(cursor, i16))),
            BinXmlValueType::UInt16Type => Ok(BinXmlValue::UInt16Type(try_read!(cursor, u16))),
            BinXmlValueType::Int32Type => Ok(BinXmlValue::Int32Type(try_read!(cursor, i32))),
            BinXmlValueType::UInt32Type => Ok(BinXmlValue::UInt32Type(try_read!(cursor, u32))),
            BinXmlValueType::Int64Type => Ok(BinXmlValue::Int64Type(try_read!(cursor, i64))),
            BinXmlValueType::UInt64Type => Ok(BinXmlValue::UInt64Type(try_read!(cursor, u64))),
            BinXmlValueType::Real32Type => unimplemented!("Real32Type"),
            BinXmlValueType::Real64Type => unimplemented!("Real64Type"),
            BinXmlValueType::BoolType => {
                let bool_value = try_read!(cursor, u32);
                match bool_value {
                    0 => Ok(BinXmlValue::BoolType(false)),
                    1 => Ok(BinXmlValue::BoolType(true)),
                    _ => Err(Error::other(
                        format!("{} is invalid value for bool", bool_value),
                        cursor.position(),
                    )),
                }
            }
            BinXmlValueType::BinaryType => unimplemented!("BinaryType"),
            BinXmlValueType::GuidType => {
                Ok(BinXmlValue::GuidType(Guid::from_stream(cursor).map_err(
                    |_e| Error::other("Failed to read GUID from stream", cursor.position()),
                )?))
            }
            BinXmlValueType::SizeTType => unimplemented!("SizeTType"),
            BinXmlValueType::FileTimeType => Ok(BinXmlValue::FileTimeType(datetime_from_filetime(
                try_read!(cursor, u64),
            ))),
            BinXmlValueType::SysTimeType => unimplemented!("SysTimeType"),
            BinXmlValueType::SidType => {
                Ok(BinXmlValue::SidType(Sid::from_stream(cursor).map_err(
                    |_e| Error::other("Failed to read NTSID from stream", cursor.position()),
                )?))
            }
            BinXmlValueType::HexInt32Type => Ok(BinXmlValue::HexInt32Type(format!(
                "0x{:x}",
                try_read!(cursor, i32)
            ))),
            BinXmlValueType::HexInt64Type => Ok(BinXmlValue::HexInt64Type(format!(
                "0x{:x}",
                try_read!(cursor, i64)
            ))),
            BinXmlValueType::EvtHandle => unimplemented!("EvtHandle"),
            BinXmlValueType::BinXmlType => {
                let tokens =
                    BinXmlDeserializer::read_binxml_fragment(cursor, Rc::clone(&ctx), None)?;

                Ok(BinXmlValue::BinXmlType(tokens))
            }
            BinXmlValueType::EvtXmlType => unimplemented!("EvtXml"),
            _ => unimplemented!("{:?}", value_type),
        }
    }
}

impl<'c> Into<Cow<'c, str>> for BinXmlValue<'c> {
    fn into(self) -> Cow<'c, str> {
        match self {
            BinXmlValue::NullType => Cow::Borrowed(""),
            BinXmlValue::StringType(s) => s,
            BinXmlValue::StringArrayType(s) => Cow::Owned(s.join(",")),
            BinXmlValue::AnsiStringType(s) => s,
            BinXmlValue::Int8Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::UInt8Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::Int16Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::UInt16Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::Int32Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::UInt32Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::Int64Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::UInt64Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::Real32Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::Real64Type(num) => Cow::Owned(num.to_string()),
            BinXmlValue::BoolType(num) => Cow::Owned(num.to_string()),
            BinXmlValue::BinaryType(bytes) => {
                // Bytes will be formatted as const length of 2 with '0' padding.
                let repr: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                Cow::Owned(repr)
            }
            BinXmlValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXmlValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXmlValue::SysTimeType => unimplemented!("SysTimeType"),
            BinXmlValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => Cow::Owned(hex_string),
            BinXmlValue::HexInt64Type(hex_string) => Cow::Owned(hex_string),
            BinXmlValue::EvtHandle => {
                panic!("Unsupported conversion, call `expand_templates` first")
            }
            BinXmlValue::BinXmlType(_) => {
                panic!("Unsupported conversion, call `expand_templates` first")
            }
            BinXmlValue::EvtXml => panic!("Unsupported conversion, call `expand_templates` first"),
            _ => unimplemented!("{:?}", self),
        }
    }
}
