pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::{BinXmlDeserializer, Context};
use crate::error::Error;

use crate::guid::Guid;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::ntsid::Sid;
use crate::utils::{
    datetime_from_filetime, read_len_prefixed_utf16_string, read_systemtime, read_utf16_by_size,
};
use chrono::{DateTime, Utc};
use log::trace;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::io::{Cursor, Read, Seek, SeekFrom};
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
    SysTimeType(DateTime<Utc>),
    SidType(Sid),
    HexInt32Type(String),
    HexInt64Type(String),
    EvtHandle,
    // Because of the recursive type, we instantiate this enum via a method of the Deserializer
    BinXmlType(Vec<BinXMLDeserializedTokens<'a>>),
    EvtXml,
    StringArrayType(Vec<Cow<'a, str>>),
    AnsiStringArrayType,
    Int8ArrayType(Vec<i8>),
    UInt8ArrayType(Vec<u8>),
    Int16ArrayType(Vec<i16>),
    UInt16ArrayType(Vec<u16>),
    Int32ArrayType(Vec<i32>),
    UInt32ArrayType(Vec<u32>),
    Int64ArrayType(Vec<i64>),
    UInt64ArrayType(Vec<u64>),
    Real32ArrayType(Vec<f32>),
    Real64ArrayType(Vec<f64>),
    BoolArrayType(Vec<bool>),
    BinaryArrayType,
    GuidArrayType(Vec<Guid>),
    SizeTArrayType,
    FileTimeArrayType(Vec<DateTime<Utc>>),
    SysTimeArrayType(Vec<DateTime<Utc>>),
    SidArrayType(Vec<Sid>),
    HexInt32ArrayType(Vec<String>),
    HexInt64ArrayType(Vec<String>),
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

impl<'a> BinXmlValue<'a> {
    pub fn from_binxml_stream<'c>(
        cursor: &mut Cursor<&'a [u8]>,
        ctx: Context<'a, 'c>,
    ) -> Result<BinXmlValue<'a>, Error> {
        let value_type_token = try_read!(cursor, u8);

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
            Error::not_a_valid_binxml_value_type(value_type_token, cursor.position())
        })?;

        let data = Self::deserialize_value_type(&value_type, cursor, Rc::clone(&ctx))?;

        Ok(data)
    }

    pub fn deserialize_value_type<'c>(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        ctx: Context<'a, 'c>,
    ) -> Result<BinXmlValue<'a>, Error> {
        let value = match value_type {
            BinXmlValueType::NullType => BinXmlValue::NullType,
            BinXmlValueType::StringType => BinXmlValue::StringType(try_read!(cursor, utf_16_str)),
            // TODO: these strings need global code-page configuration to work.
            BinXmlValueType::AnsiStringType => {
                Err(Error::other("Unimplemented: AnsiString", cursor.position()))?
            }
            BinXmlValueType::Int8Type => BinXmlValue::Int8Type(try_read!(cursor, i8)),
            BinXmlValueType::UInt8Type => BinXmlValue::UInt8Type(try_read!(cursor, u8)),
            BinXmlValueType::Int16Type => BinXmlValue::Int16Type(try_read!(cursor, i16)),
            BinXmlValueType::UInt16Type => BinXmlValue::UInt16Type(try_read!(cursor, u16)),
            BinXmlValueType::Int32Type => BinXmlValue::Int32Type(try_read!(cursor, i32)),
            BinXmlValueType::UInt32Type => BinXmlValue::UInt32Type(try_read!(cursor, u32)),
            BinXmlValueType::Int64Type => BinXmlValue::Int64Type(try_read!(cursor, i64)),
            BinXmlValueType::UInt64Type => BinXmlValue::UInt64Type(try_read!(cursor, u64)),
            BinXmlValueType::Real32Type => BinXmlValue::Real32Type(try_read!(cursor, f32)),
            BinXmlValueType::Real64Type => BinXmlValue::Real64Type(try_read!(cursor, f64)),
            BinXmlValueType::BoolType => BinXmlValue::BoolType(try_read!(cursor, bool)),
            BinXmlValueType::GuidType => BinXmlValue::GuidType(try_read!(cursor, guid)),
            BinXmlValueType::SizeTType => {
                Err(Error::other("Unimplemented: SizeTType", cursor.position()))?
            }
            BinXmlValueType::FileTimeType => BinXmlValue::FileTimeType(try_read!(cursor, filetime)),
            BinXmlValueType::SysTimeType => BinXmlValue::SysTimeType(try_read!(cursor, systime)),
            BinXmlValueType::SidType => BinXmlValue::SidType(try_read!(cursor, sid)),
            BinXmlValueType::HexInt32Type => BinXmlValue::HexInt32Type(try_read!(cursor, hex32)),
            BinXmlValueType::HexInt64Type => BinXmlValue::HexInt64Type(try_read!(cursor, hex64)),
            BinXmlValueType::BinXmlType => {
                let tokens =
                    BinXmlDeserializer::read_binxml_fragment(cursor, Rc::clone(&ctx), None)?;

                BinXmlValue::BinXmlType(tokens)
            }
            _ => Err(Error::other(
                format!("Unimplemented: {:?}", value_type),
                cursor.position(),
            ))?,
        };

        Ok(value)
    }

    pub fn deserialized_sized_value_type<'c>(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        ctx: Context<'a, 'c>,
        size: u16,
    ) -> Result<BinXmlValue<'a>, Error> {
        trace!(
            "deserialized_sized_value_type: {:?}, {:?}",
            value_type,
            size
        );
        let value = match value_type {
            // We are not reading len prefixed strings as usual, the string len is passed in the descriptor instead.
            BinXmlValueType::StringType => BinXmlValue::StringType(Cow::Owned(
                read_utf16_by_size(cursor, u64::from(size))
                    .map_err(|e| Error::utf16_decode_error(e, cursor.position()))?
                    .unwrap_or_else(|| "".to_owned()),
            )),
            BinXmlValueType::AnsiStringType => {
                let mut bytes = vec![0; size as usize];
                cursor.read_exact(&mut bytes)?;

                BinXmlValue::AnsiStringType(Cow::Owned(
                    String::from_utf8(bytes)
                        .and_then(|mut s| {
                            if let Some('\0') = s.chars().last() {
                                s.pop();
                            }
                            Ok(s)
                        })
                        .map_err(|e| Error::utf8_decode_error(e, cursor.position()))?,
                ))
            }
            BinXmlValueType::StringArrayType => {
                BinXmlValue::StringArrayType(try_read_sized_array!(cursor, utf_16_str, size))
            }
            BinXmlValueType::BinaryType => {
                // Borrow the underlying data from the cursor, and return a ref to it.
                let data = *cursor.get_ref();
                let bytes = &data
                    [cursor.position() as usize..(cursor.position() + u64::from(size)) as usize];

                cursor.seek(SeekFrom::Current(i64::from(size)))?;
                BinXmlValue::BinaryType(bytes)
            }
            BinXmlValueType::Int8ArrayType => {
                BinXmlValue::Int8ArrayType(try_read_sized_array!(cursor, i8, size))
            }
            BinXmlValueType::UInt8ArrayType => {
                let mut data = vec![0; size as usize];
                cursor.read_exact(&mut data)?;

                BinXmlValue::UInt8ArrayType(data)
            }
            BinXmlValueType::Int16ArrayType => {
                BinXmlValue::Int16ArrayType(try_read_sized_array!(cursor, i16, size))
            }
            BinXmlValueType::UInt16ArrayType => {
                BinXmlValue::UInt16ArrayType(try_read_sized_array!(cursor, u16, size))
            }
            BinXmlValueType::Int32ArrayType => {
                BinXmlValue::Int32ArrayType(try_read_sized_array!(cursor, i32, size))
            }
            BinXmlValueType::UInt32ArrayType => {
                BinXmlValue::UInt32ArrayType(try_read_sized_array!(cursor, u32, size))
            }
            BinXmlValueType::Int64ArrayType => {
                BinXmlValue::Int64ArrayType(try_read_sized_array!(cursor, i64, size))
            }
            BinXmlValueType::UInt64ArrayType => {
                BinXmlValue::UInt64ArrayType(try_read_sized_array!(cursor, u64, size))
            }
            BinXmlValueType::Real32ArrayType => {
                BinXmlValue::Real32ArrayType(try_read_sized_array!(cursor, f32, size))
            }
            BinXmlValueType::Real64ArrayType => {
                BinXmlValue::Real64ArrayType(try_read_sized_array!(cursor, f64, size))
            }
            BinXmlValueType::BoolArrayType => {
                BinXmlValue::BoolArrayType(try_read_sized_array!(cursor, bool, size))
            }
            BinXmlValueType::GuidArrayType => {
                BinXmlValue::GuidArrayType(try_read_sized_array!(cursor, guid, size))
            }
            BinXmlValueType::FileTimeArrayType => {
                BinXmlValue::FileTimeArrayType(try_read_sized_array!(cursor, filetime, size))
            }
            BinXmlValueType::SysTimeArrayType => {
                BinXmlValue::SysTimeArrayType(try_read_sized_array!(cursor, systime, size))
            }
            BinXmlValueType::SidArrayType => {
                BinXmlValue::SidArrayType(try_read_sized_array!(cursor, sid, size))
            }
            BinXmlValueType::HexInt32ArrayType => {
                BinXmlValue::HexInt32ArrayType(try_read_sized_array!(cursor, hex32, size))
            }
            BinXmlValueType::HexInt64ArrayType => {
                BinXmlValue::HexInt64ArrayType(try_read_sized_array!(cursor, hex64, size))
            }
            // Fallback to un-sized variant.
            _ => BinXmlValue::deserialize_value_type(&value_type, cursor, Rc::clone(&ctx))?,
        };

        Ok(value)
    }
}

fn to_delimited_list<N: ToString>(ns: Vec<N>) -> String {
    ns.iter()
        .map(|n| n.to_string())
        .collect::<Vec<String>>()
        .join(",")
}

impl<'c> Into<serde_json::Value> for BinXmlValue<'c> {
    fn into(self) -> Value {
        match self {
            BinXmlValue::NullType => Value::Null,
            BinXmlValue::StringType(s) => json!(s.into_owned()),
            BinXmlValue::AnsiStringType(s) => json!(s.into_owned()),
            BinXmlValue::Int8Type(num) => json!(num),
            BinXmlValue::UInt8Type(num) => json!(num),
            BinXmlValue::Int16Type(num) => json!(num),
            BinXmlValue::UInt16Type(num) => json!(num),
            BinXmlValue::Int32Type(num) => json!(num),
            BinXmlValue::UInt32Type(num) => json!(num),
            BinXmlValue::Int64Type(num) => json!(num),
            BinXmlValue::UInt64Type(num) => json!(num),
            BinXmlValue::Real32Type(num) => json!(num),
            BinXmlValue::Real64Type(num) => json!(num),
            BinXmlValue::BoolType(num) => json!(num),
            BinXmlValue::BinaryType(bytes) => {
                // Bytes will be formatted as const length of 2 with '0' padding.
                let repr: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                json!(repr)
            }
            BinXmlValue::GuidType(guid) => json!(guid.to_string()),
            //            BinXmlValue::SizeTType(sz) => json!(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => json!(tm),
            BinXmlValue::SysTimeType(tm) => json!(tm),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string),
            BinXmlValue::StringArrayType(s) => json!(s),
            BinXmlValue::Int8ArrayType(numbers) => json!(numbers),
            BinXmlValue::UInt8ArrayType(numbers) => json!(numbers),
            BinXmlValue::Int16ArrayType(numbers) => json!(numbers),
            BinXmlValue::UInt16ArrayType(numbers) => json!(numbers),
            BinXmlValue::Int32ArrayType(numbers) => json!(numbers),
            BinXmlValue::UInt32ArrayType(numbers) => json!(numbers),
            BinXmlValue::Int64ArrayType(numbers) => json!(numbers),
            BinXmlValue::UInt64ArrayType(numbers) => json!(numbers),
            BinXmlValue::Real32ArrayType(numbers) => json!(numbers),
            BinXmlValue::Real64ArrayType(numbers) => json!(numbers),
            BinXmlValue::BoolArrayType(bools) => json!(bools),
            BinXmlValue::GuidArrayType(guids) => {
                json!(guids.iter().map(Guid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::FileTimeArrayType(filetimes) => json!(filetimes),
            BinXmlValue::SysTimeArrayType(systimes) => json!(systimes),
            BinXmlValue::SidArrayType(sids) => {
                json!(sids.iter().map(Sid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::HexInt32ArrayType(hex_strings) => json!(hex_strings),
            BinXmlValue::HexInt64ArrayType(hex_strings) => json!(hex_strings),
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
            BinXmlValue::BinaryType(bytes) => {
                // Bytes will be formatted as const length of 2 with '0' padding.
                let repr: String = bytes.iter().map(|b| format!("{:02X}", b)).collect();
                Cow::Owned(repr)
            }
            BinXmlValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXmlValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXmlValue::SysTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXmlValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => Cow::Owned(hex_string),
            BinXmlValue::HexInt64Type(hex_string) => Cow::Owned(hex_string),
            BinXmlValue::StringArrayType(s) => Cow::Owned(s.join(",")),
            BinXmlValue::Int8ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::UInt8ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::Int16ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::UInt16ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::Int32ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::UInt32ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::Int64ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::UInt64ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::Real32ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::Real64ArrayType(numbers) => Cow::Owned(to_delimited_list(numbers)),
            BinXmlValue::BoolArrayType(bools) => Cow::Owned(to_delimited_list(bools)),
            BinXmlValue::GuidArrayType(guids) => Cow::Owned(to_delimited_list(guids)),
            BinXmlValue::FileTimeArrayType(filetimes) => Cow::Owned(to_delimited_list(filetimes)),
            BinXmlValue::SysTimeArrayType(systimes) => Cow::Owned(to_delimited_list(systimes)),
            BinXmlValue::SidArrayType(sids) => Cow::Owned(to_delimited_list(sids)),
            BinXmlValue::HexInt32ArrayType(hex_strings) => Cow::Owned(hex_strings.join(",")),
            BinXmlValue::HexInt64ArrayType(hex_strings) => Cow::Owned(hex_strings.join(",")),
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
