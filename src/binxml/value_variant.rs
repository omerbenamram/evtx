pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::{BinXmlDeserializer, Cache, Context};
use crate::error::Error;
use crate::evtx::ReadSeek;
use crate::guid::Guid;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::ntsid::Sid;
use crate::utils::{datetime_from_filetime, read_len_prefixed_utf16_string};
use chrono::{DateTime, Utc};
use std::borrow::Cow;
use std::io::{Cursor, Seek};
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

impl<'c> BinXmlValue<'c> {
    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'c [u8]>,
        ctx: Context<'c>,
    ) -> Result<BinXmlValue<'c>, Error> {
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXMLValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(
                value_type_token,
                cursor.stream_position().expect("Tell failed"),
            )
        })?;

        let data = Self::deserialize_value_type(&value_type, cursor, Rc::clone(&ctx))?;

        Ok(data)
    }

    pub fn deserialize_value_type(
        value_type: &BinXMLValueType,
        cursor: &mut Cursor<&'c [u8]>,
        ctx: Context<'c>,
    ) -> Result<BinXmlValue<'c>, Error> {
        match value_type {
            BinXMLValueType::NullType => Ok(BinXmlValue::NullType),
            BinXMLValueType::StringType => Ok(BinXmlValue::StringType(Cow::Owned(
                read_len_prefixed_utf16_string(cursor, false)
                    .map_err(|e| {
                        Error::utf16_decode_error(e, cursor.stream_position().expect("Tell failed"))
                    })?
                    .unwrap_or("".to_owned()),
            ))),
            BinXMLValueType::AnsiStringType => unimplemented!(),
            BinXMLValueType::Int8Type => Ok(BinXmlValue::Int8Type(try_read!(cursor, i8))),
            BinXMLValueType::UInt8Type => Ok(BinXmlValue::UInt8Type(try_read!(cursor, u8))),
            BinXMLValueType::Int16Type => Ok(BinXmlValue::Int16Type(try_read!(cursor, i16))),
            BinXMLValueType::UInt16Type => Ok(BinXmlValue::UInt16Type(try_read!(cursor, u16))),
            BinXMLValueType::Int32Type => Ok(BinXmlValue::Int32Type(try_read!(cursor, i32))),
            BinXMLValueType::UInt32Type => Ok(BinXmlValue::UInt32Type(try_read!(cursor, u32))),
            BinXMLValueType::Int64Type => Ok(BinXmlValue::Int64Type(try_read!(cursor, i64))),
            BinXMLValueType::UInt64Type => Ok(BinXmlValue::UInt64Type(try_read!(cursor, u64))),
            BinXMLValueType::Real32Type => unimplemented!("Real32Type"),
            BinXMLValueType::Real64Type => unimplemented!("Real64Type"),
            BinXMLValueType::BoolType => unimplemented!("BoolType"),
            BinXMLValueType::BinaryType => unimplemented!("BinaryType"),
            BinXMLValueType::GuidType => Ok(BinXmlValue::GuidType(
                Guid::from_stream(cursor).map_err(|e| {
                    Error::other(
                        "Failed to read GUID from stream",
                        cursor.stream_position().expect("Tell failed"),
                    )
                })?,
            )),
            BinXMLValueType::SizeTType => unimplemented!("SizeTType"),
            BinXMLValueType::FileTimeType => Ok(BinXmlValue::FileTimeType(datetime_from_filetime(
                try_read!(cursor, u64),
            ))),
            BinXMLValueType::SysTimeType => unimplemented!("SysTimeType"),
            BinXMLValueType::SidType => Ok(BinXmlValue::SidType(
                Sid::from_stream(cursor).map_err(|e| {
                    Error::other(
                        "Failed to read NTSID from stream",
                        cursor.stream_position().expect("Tell failed"),
                    )
                })?,
            )),
            BinXMLValueType::HexInt32Type => Ok(BinXmlValue::HexInt32Type(format!(
                "0x{:2x}",
                try_read!(cursor, i32)
            ))),
            BinXMLValueType::HexInt64Type => Ok(BinXmlValue::HexInt64Type(format!(
                "0x{:2x}",
                try_read!(cursor, i64)
            ))),
            BinXMLValueType::EvtHandle => unimplemented!("EvtHandle"),
            BinXMLValueType::BinXmlType => {
                let data = *cursor.get_ref();
                let deser_temp = BinXmlDeserializer::init_without_cache(
                    data,
                    cursor.stream_position().map_err(Error::io)?,
                );
                let mut tokens = vec![];
                for token in deser_temp.iter_tokens(None) {
                    tokens.push(token?);
                }

                Ok(BinXmlValue::BinXmlType(tokens))
            }
            BinXMLValueType::EvtXml => unimplemented!("EvtXml"),
        }
    }
}

impl<'c> Into<Cow<'c, str>> for BinXmlValue<'c> {
    fn into(self) -> Cow<'c, str> {
        match self {
            BinXmlValue::NullType => Cow::Borrowed(""),
            BinXmlValue::StringType(s) => s,
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
            BinXmlValue::BinaryType(bytes) => Cow::Owned(format!("{:?}", bytes)),
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
        }
    }
}
