use crate::binxml::deserializer::BinXmlDeserializer;
use crate::err::{DeserializationError, DeserializationResult as Result};
use crate::evtx_chunk::EvtxChunk;
use crate::model::deserialized::BinXMLDeserializedTokens;
use crate::utils::ByteCursor;
use crate::utils::invalid_data;
use crate::utils::windows::{filetime_to_datetime, read_sid, read_systime, systime_from_bytes};

use bumpalo::Bump;
use bumpalo::collections::String as BumpString;
use bumpalo::collections::Vec as BumpVec;
use chrono::{DateTime, Utc};
use encoding::EncodingRef;
use log::{trace, warn};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::fmt::Write;
use std::io::Cursor;
use std::string::ToString;
use winstructs::guid::Guid;
use winstructs::security::Sid;

static DATETIME_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";

#[derive(Debug, PartialEq, Clone)]
pub enum BinXmlValue<'a> {
    NullType,
    // Arena-allocated strings for O(1) mass deallocation.
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
    // Because of the recursive type, we instantiate this enum via a method of the Deserializer
    BinXmlType(BumpVec<'a, BinXMLDeserializedTokens<'a>>),
    EvtXml,
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
    /// Allocate a string into the provided arena.
    #[inline]
    fn alloc_str(s: &str, arena: &'a Bump) -> BumpString<'a> {
        BumpString::from_str_in(s, arena)
    }

    /// Move a heap `Vec` into a bump-allocated `Vec` (same element type).
    #[inline]
    fn vec_to_bump_vec<T>(v: Vec<T>, arena: &'a Bump) -> BumpVec<'a, T> {
        let mut out = BumpVec::with_capacity_in(v.len(), arena);
        out.extend(v);
        out
    }

    pub(crate) fn from_binxml_cursor(
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        arena: &'a Bump,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let value_type_token = cursor.u8()?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or(
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            },
        )?;

        let data =
            Self::deserialize_value_type_cursor(&value_type, cursor, chunk, arena, size, ansi_codec)?;

        Ok(data)
    }

    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        arena: &'a Bump,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_binxml_cursor(&mut c, chunk, arena, size, ansi_codec)?;
        cursor.set_position(c.position());
        Ok(v)
    }

    pub(crate) fn deserialize_value_type_cursor(
        value_type: &BinXmlValueType,
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
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
                let sz_bytes = usize::from(sz);
                let s = if sz_bytes == 0 {
                    None
                } else if !sz_bytes.is_multiple_of(2) {
                    return Err(invalid_data("sized utf-16 string", cursor.position()));
                } else {
                    cursor.utf16_by_char_count_trimmed(sz_bytes / 2, "<string_value>")?
                };
                let s = s.unwrap_or_default();
                BinXmlValue::StringType(Self::alloc_str(&s, arena))
            }
            (BinXmlValueType::StringType, None) => {
                let s = cursor
                    .len_prefixed_utf16_string(false, "<string_value>")?
                    .unwrap_or_default();
                BinXmlValue::StringType(Self::alloc_str(&s, arena))
            }

            (BinXmlValueType::AnsiStringType, Some(sz)) => {
                let sz_bytes = usize::from(sz);
                let raw = cursor.take_bytes(sz_bytes, "<ansi_string_value>")?;
                let mut data = raw.to_vec();
                data.retain(|&b| b != 0);
                let s = ansi_codec
                    .decode(&data[..], encoding::DecoderTrap::Strict)
                    .map_err(|m| DeserializationError::AnsiDecodeError {
                        encoding_used: ansi_codec.name(),
                        inner_message: m.to_string(),
                    })?;
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

            (BinXmlValueType::Int8Type, _) => BinXmlValue::Int8Type(cursor.u8()? as i8),
            (BinXmlValueType::UInt8Type, _) => BinXmlValue::UInt8Type(cursor.u8()?),

            (BinXmlValueType::Int16Type, _) => {
                BinXmlValue::Int16Type(i16::from_le_bytes(cursor.array::<2>("i16")?))
            }
            (BinXmlValueType::UInt16Type, _) => BinXmlValue::UInt16Type(cursor.u16()?),

            (BinXmlValueType::Int32Type, _) => {
                BinXmlValue::Int32Type(i32::from_le_bytes(cursor.array::<4>("i32")?))
            }
            (BinXmlValueType::UInt32Type, _) => BinXmlValue::UInt32Type(cursor.u32()?),

            (BinXmlValueType::Int64Type, _) => {
                BinXmlValue::Int64Type(i64::from_le_bytes(cursor.array::<8>("i64")?))
            }
            (BinXmlValueType::UInt64Type, _) => BinXmlValue::UInt64Type(cursor.u64()?),

            (BinXmlValueType::Real32Type, _) => {
                BinXmlValue::Real32Type(f32::from_le_bytes(cursor.array::<4>("f32")?))
            }
            (BinXmlValueType::Real64Type, _) => {
                BinXmlValue::Real64Type(f64::from_le_bytes(cursor.array::<8>("f64")?))
            }

            (BinXmlValueType::BoolType, _) => {
                let raw = i32::from_le_bytes(cursor.array::<4>("bool")?);
                let v = match raw {
                    0 => false,
                    1 => true,
                    other => {
                        warn!(
                            "invalid boolean value {} at offset {}; treating as {}",
                            other,
                            cursor.position(),
                            other != 0
                        );
                        other != 0
                    }
                };
                BinXmlValue::BoolType(v)
            }

            (BinXmlValueType::GuidType, _) => {
                let bytes = cursor.take_bytes(16, "guid")?;
                let guid = Guid::from_buffer(bytes)
                    .map_err(|_| invalid_data("guid", cursor.position()))?;
                BinXmlValue::GuidType(guid)
            }

            (BinXmlValueType::SizeTType, Some(4)) => {
                let v = i32::from_le_bytes(cursor.array::<4>("sizet32")?);
                let mut s = BumpString::new_in(arena);
                write!(&mut s, "0x{:x}", v).expect("write to bump string");
                BinXmlValue::HexInt32Type(s)
            }
            (BinXmlValueType::SizeTType, Some(8)) => {
                let v = i64::from_le_bytes(cursor.array::<8>("sizet64")?);
                let mut s = BumpString::new_in(arena);
                write!(&mut s, "0x{:x}", v).expect("write to bump string");
                BinXmlValue::HexInt64Type(s)
            }
            (BinXmlValueType::SizeTType, _) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "SizeT".to_owned(),
                    size,
                    offset: cursor.position(),
                });
            }

            (BinXmlValueType::FileTimeType, _) => {
                BinXmlValue::FileTimeType(filetime_to_datetime(cursor.u64()?))
            }
            (BinXmlValueType::SysTimeType, _) => BinXmlValue::SysTimeType(read_systime(cursor)?),
            (BinXmlValueType::SidType, _) => BinXmlValue::SidType(read_sid(cursor)?),

            (BinXmlValueType::HexInt32Type, _) => {
                let v = i32::from_le_bytes(cursor.array::<4>("hex32")?);
                let mut s = BumpString::new_in(arena);
                write!(&mut s, "0x{:x}", v).expect("write to bump string");
                BinXmlValue::HexInt32Type(s)
            }
            (BinXmlValueType::HexInt64Type, _) => {
                let v = i64::from_le_bytes(cursor.array::<8>("hex64")?);
                let mut s = BumpString::new_in(arena);
                write!(&mut s, "0x{:x}", v).expect("write to bump string");
                BinXmlValue::HexInt64Type(s)
            }

            (BinXmlValueType::BinXmlType, size) => {
                let data_size = size.map(u32::from);
                let start_pos = cursor.position();
                let mut c = Cursor::new(cursor.buf());
                c.set_position(start_pos);
                let tokens = BinXmlDeserializer::read_binxml_fragment(
                    &mut c,
                    chunk,
                    arena,
                    data_size,
                    false,
                    ansi_codec,
                )?;
                cursor.set_pos_u64(c.position(), "advance after BinXmlType")?;
                let mut out = BumpVec::with_capacity_in(tokens.len(), arena);
                for t in tokens {
                    out.push(t);
                }
                BinXmlValue::BinXmlType(out)
            }

            (BinXmlValueType::BinaryType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "binary")?;
                BinXmlValue::BinaryType(bytes)
            }

            // The array types are always sized.
            (BinXmlValueType::StringArrayType, Some(sz)) => {
                let size_usize = usize::from(sz);
                let start = cursor.pos();
                let end = start.saturating_add(size_usize);
                let mut out: BumpVec<'a, BumpString<'a>> = BumpVec::new_in(arena);
                while cursor.pos() < end {
                    let s = cursor.null_terminated_utf16_string("string_array")?;
                    out.push(Self::alloc_str(&s, arena));
                }
                BinXmlValue::StringArrayType(out)
            }
            (BinXmlValueType::Int8ArrayType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "i8_array")?;
                let mut out = BumpVec::with_capacity_in(bytes.len(), arena);
                for &b in bytes {
                    out.push(b as i8);
                }
                BinXmlValue::Int8ArrayType(out)
            }
            (BinXmlValueType::UInt8ArrayType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "u8_array")?;
                let mut out = BumpVec::with_capacity_in(bytes.len(), arena);
                out.extend(bytes.iter().copied());
                BinXmlValue::UInt8ArrayType(out)
            }
            (BinXmlValueType::Int16ArrayType, Some(sz)) => BinXmlValue::Int16ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<2, _>(sz, "i16_array", |_off, b| {
                        Ok(i16::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::UInt16ArrayType, Some(sz)) => BinXmlValue::UInt16ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<2, _>(sz, "u16_array", |_off, b| {
                        Ok(u16::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::Int32ArrayType, Some(sz)) => BinXmlValue::Int32ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<4, _>(sz, "i32_array", |_off, b| {
                        Ok(i32::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::UInt32ArrayType, Some(sz)) => BinXmlValue::UInt32ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<4, _>(sz, "u32_array", |_off, b| {
                        Ok(u32::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::Int64ArrayType, Some(sz)) => BinXmlValue::Int64ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<8, _>(sz, "i64_array", |_off, b| {
                        Ok(i64::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::UInt64ArrayType, Some(sz)) => BinXmlValue::UInt64ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<8, _>(sz, "u64_array", |_off, b| {
                        Ok(u64::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::Real32ArrayType, Some(sz)) => BinXmlValue::Real32ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<4, _>(sz, "f32_array", |_off, b| {
                        Ok(f32::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::Real64ArrayType, Some(sz)) => BinXmlValue::Real64ArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<8, _>(sz, "f64_array", |_off, b| {
                        Ok(f64::from_le_bytes(*b))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::BoolArrayType, Some(sz)) => BinXmlValue::BoolArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<4, _>(sz, "bool_array", |off, b| {
                        let raw = i32::from_le_bytes(*b);
                        Ok(match raw {
                            0 => false,
                            1 => true,
                            other => {
                                warn!(
                                    "invalid boolean value {} at offset {}; treating as {}",
                                    other,
                                    off,
                                    other != 0
                                );
                                other != 0
                            }
                        })
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::GuidArrayType, Some(sz)) => BinXmlValue::GuidArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<16, _>(sz, "guid_array", |off, b| {
                        Guid::from_buffer(b).map_err(|_| invalid_data("guid", off))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::FileTimeArrayType, Some(sz)) => BinXmlValue::FileTimeArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<8, _>(sz, "filetime_array", |_off, b| {
                        Ok(filetime_to_datetime(u64::from_le_bytes(*b)))
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::SysTimeArrayType, Some(sz)) => BinXmlValue::SysTimeArrayType(
                Self::vec_to_bump_vec(
                    cursor.read_sized_vec_aligned::<16, _>(sz, "systime_array", |_off, b| {
                        systime_from_bytes(b)
                    })?,
                    arena,
                ),
            ),
            (BinXmlValueType::SidArrayType, Some(sz)) => BinXmlValue::SidArrayType(
                Self::vec_to_bump_vec(cursor.read_sized_vec(sz, 8, |c| read_sid(c))?, arena),
            ),
            (BinXmlValueType::HexInt32ArrayType, Some(sz)) => {
                let hex_strings = cursor.read_sized_vec_aligned::<4, _>(
                    sz,
                    "hex32_array",
                    |_off, b| {
                        let v = i32::from_le_bytes(*b);
                        Ok(Cow::Owned(format!("0x{:x}", v)))
                    },
                )?;
                let mut out = BumpVec::with_capacity_in(hex_strings.len(), arena);
                for s in hex_strings {
                    out.push(Self::alloc_str(s.as_ref(), arena));
                }
                BinXmlValue::HexInt32ArrayType(out)
            }
            (BinXmlValueType::HexInt64ArrayType, Some(sz)) => {
                let hex_strings = cursor.read_sized_vec_aligned::<8, _>(
                    sz,
                    "hex64_array",
                    |_off, b| {
                        let v = i64::from_le_bytes(*b);
                        Ok(Cow::Owned(format!("0x{:x}", v)))
                    },
                )?;
                let mut out = BumpVec::with_capacity_in(hex_strings.len(), arena);
                for s in hex_strings {
                    out.push(Self::alloc_str(s.as_ref(), arena));
                }
                BinXmlValue::HexInt64ArrayType(out)
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

    pub fn deserialize_value_type(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        arena: &'a Bump,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::deserialize_value_type_cursor(value_type, &mut c, chunk, arena, size, ansi_codec)?;
        cursor.set_position(c.position());
        Ok(v)
    }
}

fn to_delimited_list<N: ToString>(ns: &[N]) -> String {
    ns.iter()
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
            //            BinXmlValue::SizeTType(sz) => json!(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SysTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::StringArrayType(s) => {
                json!(s.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::Int8ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt8ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int16ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt16ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Real32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Real64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::BoolArrayType(bools) => json!(bools.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::GuidArrayType(guids) => {
                json!(guids.iter().map(Guid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::FileTimeArrayType(filetimes) => {
                json!(filetimes.iter().map(|tm| tm.format(DATETIME_FORMAT).to_string()).collect::<Vec<_>>())
            }
            BinXmlValue::SysTimeArrayType(systimes) => {
                json!(systimes.iter().map(|tm| tm.format(DATETIME_FORMAT).to_string()).collect::<Vec<_>>())
            }
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
            //            BinXmlValue::SizeTType(sz) => json!(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SysTimeType(tm) => json!(tm.format(DATETIME_FORMAT).to_string()),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string.as_str()),
            BinXmlValue::StringArrayType(s) => {
                json!(s.iter().map(|bs| bs.as_str()).collect::<Vec<_>>())
            }
            BinXmlValue::Int8ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt8ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int16ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt16ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Int64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::UInt64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Real32ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::Real64ArrayType(numbers) => json!(numbers.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::BoolArrayType(bools) => json!(bools.iter().copied().collect::<Vec<_>>()),
            BinXmlValue::GuidArrayType(guids) => {
                json!(guids.iter().map(Guid::to_string).collect::<Vec<String>>())
            }
            BinXmlValue::FileTimeArrayType(filetimes) => {
                json!(filetimes.iter().map(|tm| tm.format(DATETIME_FORMAT).to_string()).collect::<Vec<_>>())
            }
            BinXmlValue::SysTimeArrayType(systimes) => {
                json!(systimes.iter().map(|tm| tm.format(DATETIME_FORMAT).to_string()).collect::<Vec<_>>())
            }
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

impl BinXmlValue<'_> {
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
            BinXmlValue::StringArrayType(s) => {
                Cow::Owned(s.iter().map(|bs| bs.as_str()).collect::<Vec<_>>().join(","))
            }
            BinXmlValue::Int8ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::UInt8ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::Int16ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::UInt16ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::Int32ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::UInt32ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::Int64ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::UInt64ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::Real32ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::Real64ArrayType(numbers) => Cow::Owned(to_delimited_list(&numbers[..])),
            BinXmlValue::BoolArrayType(bools) => Cow::Owned(to_delimited_list(&bools[..])),
            BinXmlValue::GuidArrayType(guids) => Cow::Owned(to_delimited_list(&guids[..])),
            BinXmlValue::FileTimeArrayType(filetimes) => Cow::Owned(to_delimited_list(&filetimes[..])),
            BinXmlValue::SysTimeArrayType(systimes) => Cow::Owned(to_delimited_list(&systimes[..])),
            BinXmlValue::SidArrayType(sids) => Cow::Owned(to_delimited_list(&sids[..])),
            BinXmlValue::HexInt32ArrayType(hex_strings) => Cow::Owned(
                hex_strings
                    .iter()
                    .map(|bs| bs.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            BinXmlValue::HexInt64ArrayType(hex_strings) => Cow::Owned(
                hex_strings
                    .iter()
                    .map(|bs| bs.as_str())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
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
