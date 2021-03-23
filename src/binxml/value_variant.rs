use crate::err::{DeserializationError, DeserializationResult as Result, WrappedIoError};
use encoding::EncodingRef;

pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::BinXmlDeserializer;

use winstructs::guid::Guid;

use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::utils::{
    read_ansi_encoded_string, read_len_prefixed_utf16_string, read_null_terminated_utf16_string,
    read_systemtime, read_utf16_by_size,
};
use chrono::{DateTime, Utc};
use log::trace;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::string::ToString;
use winstructs::security::Sid;

use crate::evtx_chunk::EvtxChunk;
use std::fmt::Write;

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXmlValue<'a> {
    NullType,
    // String may originate in substitution.
    StringType(String),
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
    HexInt32Type(Cow<'a, str>),
    HexInt64Type(Cow<'a, str>),
    EvtHandle,
    // Because of the recursive type, we instantiate this enum via a method of the Deserializer
    BinXmlType(Vec<BinXMLDeserializedTokens<'a>>),
    EvtXml,
    StringArrayType(Vec<String>),
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
    HexInt32ArrayType(Vec<Cow<'a, str>>),
    HexInt64ArrayType(Vec<Cow<'a, str>>),
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
    EvtHandleArray,
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
    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let value_type_token = try_read!(cursor, u8)?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or(
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            },
        )?;

        let data = Self::deserialize_value_type(&value_type, cursor, chunk, size, ansi_codec)?;

        Ok(data)
    }

    pub fn deserialize_value_type(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        trace!(
            "Offset `0x{offset:08x} ({offset}): {value_type:?}, {size:?}",
            offset = cursor.position(),
            value_type = value_type,
            size = size
        );

        let value = match (value_type, size) {
            (BinXmlValueType::NullType, _) => BinXmlValue::NullType,
            (BinXmlValueType::StringType, Some(sz)) => BinXmlValue::StringType(
                read_utf16_by_size(cursor, u64::from(sz))
                    .map_err(|e| {
                        WrappedIoError::io_error_with_message(
                            e,
                            format!("failed to read sized utf-16 string (size `{}`)", sz),
                            cursor,
                        )
                    })?
                    .unwrap_or_else(|| "".to_owned()),
            ),
            (BinXmlValueType::StringType, None) => BinXmlValue::StringType(
                try_read!(cursor, len_prefixed_utf_16_str, "<string_value>")?
                    .unwrap_or_else(|| "".to_string()),
            ),
            (BinXmlValueType::AnsiStringType, Some(sz)) => BinXmlValue::AnsiStringType(Cow::Owned(
                read_ansi_encoded_string(cursor, u64::from(sz), ansi_codec)?
                    .unwrap_or_else(|| "".to_owned()),
            )),
            // AnsiString are always sized according to docs
            (BinXmlValueType::AnsiStringType, None) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "AnsiString".to_owned(),
                    size: None,
                    offset: cursor.position(),
                })
            }
            (BinXmlValueType::Int8Type, _) => BinXmlValue::Int8Type(try_read!(cursor, i8)?),
            (BinXmlValueType::UInt8Type, _) => BinXmlValue::UInt8Type(try_read!(cursor, u8)?),
            (BinXmlValueType::Int16Type, _) => BinXmlValue::Int16Type(try_read!(cursor, i16)?),
            (BinXmlValueType::UInt16Type, _) => BinXmlValue::UInt16Type(try_read!(cursor, u16)?),
            (BinXmlValueType::Int32Type, _) => BinXmlValue::Int32Type(try_read!(cursor, i32)?),
            (BinXmlValueType::UInt32Type, _) => BinXmlValue::UInt32Type(try_read!(cursor, u32)?),
            (BinXmlValueType::Int64Type, _) => BinXmlValue::Int64Type(try_read!(cursor, i64)?),
            (BinXmlValueType::UInt64Type, _) => BinXmlValue::UInt64Type(try_read!(cursor, u64)?),
            (BinXmlValueType::Real32Type, _) => BinXmlValue::Real32Type(try_read!(cursor, f32)?),
            (BinXmlValueType::Real64Type, _) => BinXmlValue::Real64Type(try_read!(cursor, f64)?),
            (BinXmlValueType::BoolType, _) => BinXmlValue::BoolType(try_read!(cursor, bool)?),
            (BinXmlValueType::GuidType, _) => BinXmlValue::GuidType(try_read!(cursor, guid)?),
            // TODO: find a sample with this token.
            (BinXmlValueType::SizeTType, _) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "SizeT".to_owned(),
                    size,
                    offset: cursor.position(),
                })
            }
            (BinXmlValueType::FileTimeType, _) => {
                BinXmlValue::FileTimeType(try_read!(cursor, filetime)?)
            }
            (BinXmlValueType::SysTimeType, _) => {
                BinXmlValue::SysTimeType(try_read!(cursor, systime)?)
            }
            (BinXmlValueType::SidType, _) => BinXmlValue::SidType(try_read!(cursor, sid)?),
            (BinXmlValueType::HexInt32Type, _) => {
                BinXmlValue::HexInt32Type(try_read!(cursor, hex32)?)
            }
            (BinXmlValueType::HexInt64Type, _) => {
                BinXmlValue::HexInt64Type(try_read!(cursor, hex64)?)
            }
            (BinXmlValueType::BinXmlType, None) => {
                let tokens = BinXmlDeserializer::read_binxml_fragment(
                    cursor, chunk, None, true, ansi_codec,
                )?;

                BinXmlValue::BinXmlType(tokens)
            }
            (BinXmlValueType::BinXmlType, Some(sz)) => {
                let tokens = BinXmlDeserializer::read_binxml_fragment(
                    cursor,
                    chunk,
                    Some(u32::from(sz)),
                    true,
                    ansi_codec,
                )?;

                BinXmlValue::BinXmlType(tokens)
            }
            (BinXmlValueType::BinaryType, Some(sz)) => {
                // Borrow the underlying data from the cursor, and return a ref to it.
                let data = *cursor.get_ref();
                let bytes =
                    &data[cursor.position() as usize..(cursor.position() + u64::from(sz)) as usize];

                cursor.seek(SeekFrom::Current(i64::from(sz))).map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        "failed to read binary value_variant",
                        cursor,
                    )
                })?;

                BinXmlValue::BinaryType(bytes)
            }
            // The array types are always sized.
            (BinXmlValueType::StringArrayType, Some(sz)) => BinXmlValue::StringArrayType(
                try_read_sized_array!(cursor, null_terminated_utf_16_str, sz),
            ),
            (BinXmlValueType::Int8ArrayType, Some(sz)) => {
                BinXmlValue::Int8ArrayType(try_read_sized_array!(cursor, i8, sz))
            }
            (BinXmlValueType::UInt8ArrayType, Some(sz)) => {
                let mut data = vec![0; sz as usize];
                cursor.read_exact(&mut data).map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        "Failed to read `UInt8ArrayType`",
                        cursor,
                    )
                })?;

                BinXmlValue::UInt8ArrayType(data)
            }
            (BinXmlValueType::Int16ArrayType, Some(sz)) => {
                BinXmlValue::Int16ArrayType(try_read_sized_array!(cursor, i16, sz))
            }
            (BinXmlValueType::UInt16ArrayType, Some(sz)) => {
                BinXmlValue::UInt16ArrayType(try_read_sized_array!(cursor, u16, sz))
            }
            (BinXmlValueType::Int32ArrayType, Some(sz)) => {
                BinXmlValue::Int32ArrayType(try_read_sized_array!(cursor, i32, sz))
            }
            (BinXmlValueType::UInt32ArrayType, Some(sz)) => {
                BinXmlValue::UInt32ArrayType(try_read_sized_array!(cursor, u32, sz))
            }
            (BinXmlValueType::Int64ArrayType, Some(sz)) => {
                BinXmlValue::Int64ArrayType(try_read_sized_array!(cursor, i64, sz))
            }
            (BinXmlValueType::UInt64ArrayType, Some(sz)) => {
                BinXmlValue::UInt64ArrayType(try_read_sized_array!(cursor, u64, sz))
            }
            (BinXmlValueType::Real32ArrayType, Some(sz)) => {
                BinXmlValue::Real32ArrayType(try_read_sized_array!(cursor, f32, sz))
            }
            (BinXmlValueType::Real64ArrayType, Some(sz)) => {
                BinXmlValue::Real64ArrayType(try_read_sized_array!(cursor, f64, sz))
            }
            (BinXmlValueType::BoolArrayType, Some(sz)) => {
                BinXmlValue::BoolArrayType(try_read_sized_array!(cursor, bool, sz))
            }
            (BinXmlValueType::GuidArrayType, Some(sz)) => {
                BinXmlValue::GuidArrayType(try_read_sized_array!(cursor, guid, sz))
            }
            (BinXmlValueType::FileTimeArrayType, Some(sz)) => {
                BinXmlValue::FileTimeArrayType(try_read_sized_array!(cursor, filetime, sz))
            }
            (BinXmlValueType::SysTimeArrayType, Some(sz)) => {
                BinXmlValue::SysTimeArrayType(try_read_sized_array!(cursor, systime, sz))
            }
            (BinXmlValueType::SidArrayType, Some(sz)) => {
                BinXmlValue::SidArrayType(try_read_sized_array!(cursor, sid, sz))
            }
            (BinXmlValueType::HexInt32ArrayType, Some(sz)) => {
                BinXmlValue::HexInt32ArrayType(try_read_sized_array!(cursor, hex32, sz))
            }
            (BinXmlValueType::HexInt64ArrayType, Some(sz)) => {
                BinXmlValue::HexInt64ArrayType(try_read_sized_array!(cursor, hex64, sz))
            }

            _ => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: format!("{:?}", value_type),
                    size,
                    offset: cursor.position(),
                })
            }
        };

        Ok(value)
    }
}

fn to_delimited_list<N: ToString>(ns: impl AsRef<Vec<N>>) -> String {
    ns.as_ref()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(",")
}

impl<'c> From<BinXmlValue<'c>> for serde_json::Value {
    fn from(value: BinXmlValue<'c>) -> Self {
        match value {
            BinXmlValue::NullType => Value::Null,
            BinXmlValue::StringType(s) => json!(s),
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
            _ => unimplemented!("{:?}", value),
        }
    }
}

impl<'c> From<&'c BinXmlValue<'c>> for serde_json::Value {
    fn from(value: &'c BinXmlValue) -> Self {
        match value {
            BinXmlValue::NullType => Value::Null,
            BinXmlValue::StringType(s) => json!(s),
            BinXmlValue::AnsiStringType(s) => json!(s.as_ref()),
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
            _ => unimplemented!("{:?}", value),
        }
    }
}

impl<'a> BinXmlValue<'a> {
    pub fn as_cow_str(&self) -> Cow<str> {
        match self {
            BinXmlValue::NullType => Cow::Borrowed(""),
            BinXmlValue::StringType(s) => Cow::Borrowed(s.as_ref()),
            BinXmlValue::AnsiStringType(s) => Cow::Borrowed(s.as_ref()),
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
                let mut repr = String::with_capacity(bytes.len() * 2);

                for b in bytes.iter() {
                    write!(repr, "{:02X}", b).expect("Writing to a String cannot fail");
                }

                Cow::Owned(repr)
            }
            BinXmlValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXmlValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXmlValue::SysTimeType(tm) => Cow::Owned(tm.to_string()),
            BinXmlValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => hex_string.clone(),
            BinXmlValue::HexInt64Type(hex_string) => hex_string.clone(),
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
