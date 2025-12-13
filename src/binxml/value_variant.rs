use crate::err::{DeserializationError, DeserializationResult as Result, WrappedIoError};
use encoding::EncodingRef;

pub use byteorder::{LittleEndian, ReadBytesExt};

use crate::binxml::deserializer::BinXmlDeserializer;
use bumpalo::Bump;
use bumpalo::collections::String as BumpString;
use bumpalo::collections::Vec as BumpVec;

use winstructs::guid::Guid;

use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::utils::{
    read_ansi_encoded_string, read_len_prefixed_utf16_string, read_null_terminated_utf16_string,
    read_systemtime, read_utf16_by_size,
};
use chrono::{DateTime, Utc};
use log::trace;
use serde_json::{Value, json};
use std::borrow::Cow;
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::string::ToString;
use winstructs::security::Sid;

use std::fmt::Write;

static DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";

#[derive(Debug, PartialEq, Clone)]
pub enum BinXmlValue<'a> {
    NullType,
    // Arena-allocated strings for O(1) mass deallocation
    StringType(BumpString<'a>),
    AnsiStringType(BumpString<'a>),
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
    HexInt32Type(BumpString<'a>),
    HexInt64Type(BumpString<'a>),
    EvtHandle,
    // Tokens allocated in arena
    BinXmlType(BumpVec<'a, BinXMLDeserializedTokens<'a>>),
    EvtXml,
    // All arrays allocated in arena for O(1) mass deallocation
    StringArrayType(BumpVec<'a, BumpString<'a>>),
    AnsiStringArrayType,
    Int8ArrayType(BumpVec<'a, i8>),
    UInt8ArrayType(BumpVec<'a, u8>),
    Int16ArrayType(BumpVec<'a, i16>),
    UInt16ArrayType(BumpVec<'a, u16>),
    Int32ArrayType(BumpVec<'a, i32>),
    UInt32ArrayType(BumpVec<'a, u32>),
    Int64ArrayType(BumpVec<'a, i64>),
    UInt64ArrayType(BumpVec<'a, u64>),
    Real32ArrayType(BumpVec<'a, f32>),
    Real64ArrayType(BumpVec<'a, f64>),
    BoolArrayType(BumpVec<'a, bool>),
    BinaryArrayType,
    GuidArrayType(BumpVec<'a, Guid>),
    SizeTArrayType,
    FileTimeArrayType(BumpVec<'a, DateTime<Utc>>),
    SysTimeArrayType(BumpVec<'a, DateTime<Utc>>),
    SidArrayType(BumpVec<'a, Sid>),
    HexInt32ArrayType(BumpVec<'a, BumpString<'a>>),
    HexInt64ArrayType(BumpVec<'a, BumpString<'a>>),
    EvtArrayHandle,
    BinXmlArrayType,
    EvtXmlArrayType,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone)]
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
    /// Helper to allocate a string in the arena
    #[inline]
    fn alloc_str(s: &str, arena: &'a Bump) -> BumpString<'a> {
        BumpString::from_str_in(s, arena)
    }

    /// Helper to convert Vec to BumpVec
    #[inline]
    fn to_bump_vec<T: Clone>(v: Vec<T>, arena: &'a Bump) -> BumpVec<'a, T> {
        let mut bv = BumpVec::with_capacity_in(v.len(), arena);
        bv.extend(v);
        bv
    }

    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        arena: &'a Bump,
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

        let data = Self::deserialize_value_type(&value_type, cursor, arena, size, ansi_codec)?;

        Ok(data)
    }

    pub fn deserialize_value_type(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        arena: &'a Bump,
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
            (BinXmlValueType::StringType, Some(sz)) => {
                let s = read_utf16_by_size(cursor, u64::from(sz))
                    .map_err(|e| {
                        WrappedIoError::io_error_with_message(
                            e,
                            format!("failed to read sized utf-16 string (size `{}`)", sz),
                            cursor,
                        )
                    })?
                    .unwrap_or_default();
                BinXmlValue::StringType(Self::alloc_str(&s, arena))
            }
            (BinXmlValueType::StringType, None) => {
                let s = try_read!(cursor, len_prefixed_utf_16_str, "<string_value>")?
                    .unwrap_or_default();
                BinXmlValue::StringType(Self::alloc_str(&s, arena))
            }
            (BinXmlValueType::AnsiStringType, Some(sz)) => {
                let s = read_ansi_encoded_string(cursor, u64::from(sz), ansi_codec)?
                    .unwrap_or_default();
                BinXmlValue::AnsiStringType(Self::alloc_str(&s, arena))
            }
            // AnsiString are always sized according to docs
            (BinXmlValueType::AnsiStringType, None) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "AnsiString".to_owned(),
                    size: None,
                    offset: cursor.position(),
                });
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
            (BinXmlValueType::SizeTType, Some(4)) => {
                let cow: Cow<'_, str> = try_read!(cursor, hex32)?;
                BinXmlValue::HexInt32Type(Self::alloc_str(cow.as_ref(), arena))
            }
            (BinXmlValueType::SizeTType, Some(8)) => {
                let cow: Cow<'_, str> = try_read!(cursor, hex64)?;
                BinXmlValue::HexInt64Type(Self::alloc_str(cow.as_ref(), arena))
            }
            (BinXmlValueType::SizeTType, _) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "SizeT".to_owned(),
                    size,
                    offset: cursor.position(),
                });
            }
            (BinXmlValueType::FileTimeType, _) => {
                BinXmlValue::FileTimeType(try_read!(cursor, filetime)?)
            }
            (BinXmlValueType::SysTimeType, _) => {
                BinXmlValue::SysTimeType(try_read!(cursor, systime)?)
            }
            (BinXmlValueType::SidType, _) => BinXmlValue::SidType(try_read!(cursor, sid)?),
            (BinXmlValueType::HexInt32Type, _) => {
                let cow: Cow<'_, str> = try_read!(cursor, hex32)?;
                BinXmlValue::HexInt32Type(Self::alloc_str(cow.as_ref(), arena))
            }
            (BinXmlValueType::HexInt64Type, _) => {
                let cow: Cow<'_, str> = try_read!(cursor, hex64)?;
                BinXmlValue::HexInt64Type(Self::alloc_str(cow.as_ref(), arena))
            }
            (BinXmlValueType::BinXmlType, None) => {
                let tokens = BinXmlDeserializer::read_binxml_fragment(
                    cursor, arena, None, true, ansi_codec,
                )?;
                BinXmlValue::BinXmlType(Self::to_bump_vec(tokens, arena))
            }
            (BinXmlValueType::BinXmlType, Some(sz)) => {
                let tokens = BinXmlDeserializer::read_binxml_fragment(
                    cursor,
                    arena,
                    Some(u32::from(sz)),
                    true,
                    ansi_codec,
                )?;
                BinXmlValue::BinXmlType(Self::to_bump_vec(tokens, arena))
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
            // The array types are always sized. All use arena allocation.
            (BinXmlValueType::StringArrayType, Some(sz)) => {
                let strings: Vec<String> =
                    try_read_sized_array!(cursor, null_terminated_utf_16_str, sz);
                let mut bv = BumpVec::with_capacity_in(strings.len(), arena);
                for s in strings {
                    bv.push(BumpString::from_str_in(&s, arena));
                }
                BinXmlValue::StringArrayType(bv)
            }
            (BinXmlValueType::Int8ArrayType, Some(sz)) => BinXmlValue::Int8ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, i8, sz), arena),
            ),
            (BinXmlValueType::UInt8ArrayType, Some(sz)) => {
                let mut data = vec![0; sz as usize];
                cursor.read_exact(&mut data).map_err(|e| {
                    WrappedIoError::io_error_with_message(
                        e,
                        "Failed to read `UInt8ArrayType`",
                        cursor,
                    )
                })?;
                BinXmlValue::UInt8ArrayType(Self::to_bump_vec(data, arena))
            }
            (BinXmlValueType::Int16ArrayType, Some(sz)) => BinXmlValue::Int16ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, i16, sz), arena),
            ),
            (BinXmlValueType::UInt16ArrayType, Some(sz)) => BinXmlValue::UInt16ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, u16, sz), arena),
            ),
            (BinXmlValueType::Int32ArrayType, Some(sz)) => BinXmlValue::Int32ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, i32, sz), arena),
            ),
            (BinXmlValueType::UInt32ArrayType, Some(sz)) => BinXmlValue::UInt32ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, u32, sz), arena),
            ),
            (BinXmlValueType::Int64ArrayType, Some(sz)) => BinXmlValue::Int64ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, i64, sz), arena),
            ),
            (BinXmlValueType::UInt64ArrayType, Some(sz)) => BinXmlValue::UInt64ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, u64, sz), arena),
            ),
            (BinXmlValueType::Real32ArrayType, Some(sz)) => BinXmlValue::Real32ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, f32, sz), arena),
            ),
            (BinXmlValueType::Real64ArrayType, Some(sz)) => BinXmlValue::Real64ArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, f64, sz), arena),
            ),
            (BinXmlValueType::BoolArrayType, Some(sz)) => BinXmlValue::BoolArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, bool, sz), arena),
            ),
            (BinXmlValueType::GuidArrayType, Some(sz)) => BinXmlValue::GuidArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, guid, sz), arena),
            ),
            (BinXmlValueType::FileTimeArrayType, Some(sz)) => BinXmlValue::FileTimeArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, filetime, sz), arena),
            ),
            (BinXmlValueType::SysTimeArrayType, Some(sz)) => BinXmlValue::SysTimeArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, systime, sz), arena),
            ),
            (BinXmlValueType::SidArrayType, Some(sz)) => BinXmlValue::SidArrayType(
                Self::to_bump_vec(try_read_sized_array!(cursor, sid, sz), arena),
            ),
            (BinXmlValueType::HexInt32ArrayType, Some(sz)) => {
                let cows: Vec<Cow<'_, str>> = try_read_sized_array!(cursor, hex32, sz);
                let mut bv = BumpVec::with_capacity_in(cows.len(), arena);
                for cow in cows {
                    bv.push(BumpString::from_str_in(cow.as_ref(), arena));
                }
                BinXmlValue::HexInt32ArrayType(bv)
            }
            (BinXmlValueType::HexInt64ArrayType, Some(sz)) => {
                let cows: Vec<Cow<'_, str>> = try_read_sized_array!(cursor, hex64, sz);
                let mut bv = BumpVec::with_capacity_in(cows.len(), arena);
                for cow in cows {
                    bv.push(BumpString::from_str_in(cow.as_ref(), arena));
                }
                BinXmlValue::HexInt64ArrayType(bv)
            }

            _ => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: format!("{:?}", value_type),
                    size,
                    offset: cursor.position(),
                });
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
            BinXmlValue::StringType(s) => json!(s.as_str()),
            BinXmlValue::AnsiStringType(s) => json!(s.as_str()),
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
                json!(
                    bytes
                        .iter()
                        .fold(String::with_capacity(bytes.len() * 2), |mut acc, &b| {
                            write!(acc, "{:02X}", b).unwrap();
                            acc
                        })
                )
            }
            BinXmlValue::GuidType(guid) => json!(guid.to_string()),
            BinXmlValue::FileTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SysTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::StringArrayType(s) => {
                json!(s.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::Int8ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt8ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int16ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt16ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Real32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Real64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::BoolArrayType(bools) => json!(bools.as_slice()),
            BinXmlValue::GuidArrayType(guids) => {
                json!(guids.iter().map(Guid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::FileTimeArrayType(filetimes) => json!(filetimes.as_slice()),
            BinXmlValue::SysTimeArrayType(systimes) => json!(systimes.as_slice()),
            BinXmlValue::SidArrayType(sids) => {
                json!(sids.iter().map(Sid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::HexInt32ArrayType(hex_strings) => {
                json!(hex_strings.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::HexInt64ArrayType(hex_strings) => {
                json!(hex_strings.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
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
            BinXmlValue::StringType(s) => json!(s.as_str()),
            BinXmlValue::AnsiStringType(s) => json!(s.as_str()),
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
                json!(
                    bytes
                        .iter()
                        .fold(String::with_capacity(bytes.len() * 2), |mut acc, &b| {
                            write!(acc, "{:02X}", b).unwrap();
                            acc
                        })
                )
            }
            BinXmlValue::GuidType(guid) => json!(guid.to_string()),
            BinXmlValue::FileTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SysTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::StringArrayType(s) => {
                json!(s.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::Int8ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt8ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int16ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt16ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Int64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::UInt64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Real32ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::Real64ArrayType(numbers) => json!(numbers.as_slice()),
            BinXmlValue::BoolArrayType(bools) => json!(bools.as_slice()),
            BinXmlValue::GuidArrayType(guids) => {
                json!(guids.iter().map(Guid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::FileTimeArrayType(filetimes) => json!(filetimes.as_slice()),
            BinXmlValue::SysTimeArrayType(systimes) => json!(systimes.as_slice()),
            BinXmlValue::SidArrayType(sids) => {
                json!(sids.iter().map(Sid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::HexInt32ArrayType(hex_strings) => {
                json!(hex_strings.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::HexInt64ArrayType(hex_strings) => {
                json!(hex_strings.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
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

fn bump_strings_to_delimited(strings: &BumpVec<'_, BumpString<'_>>) -> String {
    strings
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn bump_vec_to_delimited<T: ToString>(v: &BumpVec<'_, T>) -> String {
    v.iter()
        .map(|x| x.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

impl BinXmlValue<'_> {
    /// Check if the value is null (NullType or equivalent).
    #[inline]
    pub fn is_null(&self) -> bool {
        matches!(
            self,
            BinXmlValue::NullType
                | BinXmlValue::EvtHandle
                | BinXmlValue::EvtXml
                | BinXmlValue::EvtArrayHandle
                | BinXmlValue::AnsiStringArrayType
                | BinXmlValue::BinaryArrayType
                | BinXmlValue::SizeTArrayType
        )
    }

    pub fn as_cow_str(&self) -> Cow<'_, str> {
        match self {
            BinXmlValue::NullType => Cow::Borrowed(""),
            BinXmlValue::StringType(s) => Cow::Borrowed(s.as_str()),
            BinXmlValue::AnsiStringType(s) => Cow::Borrowed(s.as_str()),
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
            BinXmlValue::BinaryType(bytes) => Cow::Owned(bytes.iter().fold(
                String::with_capacity(bytes.len() * 2),
                |mut acc, &b| {
                    write!(acc, "{:02X}", b).unwrap();
                    acc
                },
            )),
            BinXmlValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXmlValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => Cow::Owned(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SysTimeType(tm) => Cow::Owned(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => Cow::Borrowed(hex_string.as_str()),
            BinXmlValue::HexInt64Type(hex_string) => Cow::Borrowed(hex_string.as_str()),
            BinXmlValue::StringArrayType(s) => Cow::Owned(bump_strings_to_delimited(s)),
            BinXmlValue::Int8ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::UInt8ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::Int16ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::UInt16ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::Int32ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::UInt32ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::Int64ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::UInt64ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::Real32ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::Real64ArrayType(numbers) => Cow::Owned(bump_vec_to_delimited(numbers)),
            BinXmlValue::BoolArrayType(bools) => Cow::Owned(bump_vec_to_delimited(bools)),
            BinXmlValue::GuidArrayType(guids) => Cow::Owned(bump_vec_to_delimited(guids)),
            BinXmlValue::FileTimeArrayType(filetimes) => {
                Cow::Owned(bump_vec_to_delimited(filetimes))
            }
            BinXmlValue::SysTimeArrayType(systimes) => Cow::Owned(bump_vec_to_delimited(systimes)),
            BinXmlValue::SidArrayType(sids) => Cow::Owned(bump_vec_to_delimited(sids)),
            BinXmlValue::HexInt32ArrayType(hex_strings) => {
                Cow::Owned(bump_strings_to_delimited(hex_strings))
            }
            BinXmlValue::HexInt64ArrayType(hex_strings) => {
                Cow::Owned(bump_strings_to_delimited(hex_strings))
            }
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
