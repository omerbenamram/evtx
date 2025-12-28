use crate::err::{DeserializationError, DeserializationResult as Result};
use crate::evtx_chunk::EvtxChunk;
use crate::utils::ByteCursor;
use crate::utils::invalid_data;
use crate::utils::windows::{filetime_to_timestamp, read_sid, read_systime, systime_from_bytes};

use jiff::{Timestamp, tz::Offset};
use encoding::EncodingRef;
use log::{trace, warn};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::fmt::Write;
use std::io::Cursor;
use std::string::ToString;
use winstructs::guid::Guid;
use winstructs::security::Sid;

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
    FileTimeType(Timestamp),
    SysTimeType(Timestamp),
    SidType(Sid),
    HexInt32Type(Cow<'a, str>),
    HexInt64Type(Cow<'a, str>),
    EvtHandle,
    /// Raw BinXML fragment bytes (no length prefix).
    ///
    /// This is stored as a slice into the chunk data and parsed on demand by higher-level code.
    BinXmlType(&'a [u8]),
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
    FileTimeArrayType(Vec<Timestamp>),
    SysTimeArrayType(Vec<Timestamp>),
    SidArrayType(Vec<Sid>),
    HexInt32ArrayType(Vec<Cow<'a, str>>),
    HexInt64ArrayType(Vec<Cow<'a, str>>),
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
    pub(crate) fn from_binxml_cursor(
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let value_type_token = cursor.u8()?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            }
        })?;

        let data =
            Self::deserialize_value_type_cursor(&value_type, cursor, chunk, size, ansi_codec)?;

        Ok(data)
    }

    pub fn from_binxml_stream(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_binxml_cursor(&mut c, chunk, size, ansi_codec)?;
        cursor.set_position(c.position());
        Ok(v)
    }

    pub(crate) fn deserialize_value_type_cursor(
        value_type: &BinXmlValueType,
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let _ = chunk;
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
                    cursor.utf16_by_char_count_trimmed_utf8(sz_bytes / 2, "<string_value>")?
                };
                BinXmlValue::StringType(s.map(Cow::Owned).unwrap_or(Cow::Borrowed("")))
            }
            (BinXmlValueType::StringType, None) => BinXmlValue::StringType(
                cursor
                    .len_prefixed_utf16_string_utf8(false, "<string_value>")?
                    .map(Cow::Owned)
                    .unwrap_or(Cow::Borrowed("")),
            ),

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
                BinXmlValue::AnsiStringType(Cow::Owned(s))
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
                BinXmlValue::HexInt32Type(Cow::Owned(format!("0x{:x}", v)))
            }
            (BinXmlValueType::SizeTType, Some(8)) => {
                let v = i64::from_le_bytes(cursor.array::<8>("sizet64")?);
                BinXmlValue::HexInt64Type(Cow::Owned(format!("0x{:x}", v)))
            }
            (BinXmlValueType::SizeTType, _) => {
                return Err(DeserializationError::UnimplementedValueVariant {
                    name: "SizeT".to_owned(),
                    size,
                    offset: cursor.position(),
                });
            }

            (BinXmlValueType::FileTimeType, _) => {
                BinXmlValue::FileTimeType(filetime_to_timestamp(cursor.u64()?)?)
            }
            (BinXmlValueType::SysTimeType, _) => BinXmlValue::SysTimeType(read_systime(cursor)?),
            (BinXmlValueType::SidType, _) => BinXmlValue::SidType(read_sid(cursor)?),

            (BinXmlValueType::HexInt32Type, _) => {
                let v = i32::from_le_bytes(cursor.array::<4>("hex32")?);
                BinXmlValue::HexInt32Type(Cow::Owned(format!("0x{:x}", v)))
            }
            (BinXmlValueType::HexInt64Type, _) => {
                let v = i64::from_le_bytes(cursor.array::<8>("hex64")?);
                BinXmlValue::HexInt64Type(Cow::Owned(format!("0x{:x}", v)))
            }

            (BinXmlValueType::BinXmlType, size) => {
                let payload = match size {
                    Some(sz) => {
                        if sz == 0 {
                            &[]
                        } else {
                            cursor.take_bytes(usize::from(sz), "binxml_payload")?
                        }
                    }
                    None => {
                        let payload_len = cursor.u16_named("binxml_payload_len")? as usize;
                        if payload_len == 0 {
                            &[]
                        } else {
                            cursor.take_bytes(payload_len, "binxml_payload")?
                        }
                    }
                };
                BinXmlValue::BinXmlType(payload)
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
                let mut out: Vec<Cow<'a, str>> = Vec::new();
                while cursor.pos() < end {
                    let s = cursor.null_terminated_utf16_string_utf8("string_array")?;
                    out.push(Cow::Owned(s));
                }
                BinXmlValue::StringArrayType(out)
            }
            (BinXmlValueType::Int8ArrayType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "i8_array")?;
                BinXmlValue::Int8ArrayType(bytes.iter().map(|&b| b as i8).collect())
            }
            (BinXmlValueType::UInt8ArrayType, Some(sz)) => BinXmlValue::UInt8ArrayType(
                cursor.take_bytes(usize::from(sz), "u8_array")?.to_vec(),
            ),
            (BinXmlValueType::Int16ArrayType, Some(sz)) => BinXmlValue::Int16ArrayType(
                cursor.read_sized_vec_aligned::<2, _>(sz, "i16_array", |_off, b| {
                    Ok(i16::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::UInt16ArrayType, Some(sz)) => BinXmlValue::UInt16ArrayType(
                cursor.read_sized_vec_aligned::<2, _>(sz, "u16_array", |_off, b| {
                    Ok(u16::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::Int32ArrayType, Some(sz)) => BinXmlValue::Int32ArrayType(
                cursor.read_sized_vec_aligned::<4, _>(sz, "i32_array", |_off, b| {
                    Ok(i32::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::UInt32ArrayType, Some(sz)) => BinXmlValue::UInt32ArrayType(
                cursor.read_sized_vec_aligned::<4, _>(sz, "u32_array", |_off, b| {
                    Ok(u32::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::Int64ArrayType, Some(sz)) => BinXmlValue::Int64ArrayType(
                cursor.read_sized_vec_aligned::<8, _>(sz, "i64_array", |_off, b| {
                    Ok(i64::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::UInt64ArrayType, Some(sz)) => BinXmlValue::UInt64ArrayType(
                cursor.read_sized_vec_aligned::<8, _>(sz, "u64_array", |_off, b| {
                    Ok(u64::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::Real32ArrayType, Some(sz)) => BinXmlValue::Real32ArrayType(
                cursor.read_sized_vec_aligned::<4, _>(sz, "f32_array", |_off, b| {
                    Ok(f32::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::Real64ArrayType, Some(sz)) => BinXmlValue::Real64ArrayType(
                cursor.read_sized_vec_aligned::<8, _>(sz, "f64_array", |_off, b| {
                    Ok(f64::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::BoolArrayType, Some(sz)) => BinXmlValue::BoolArrayType(
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
            ),
            (BinXmlValueType::GuidArrayType, Some(sz)) => BinXmlValue::GuidArrayType(
                cursor.read_sized_vec_aligned::<16, _>(sz, "guid_array", |off, b| {
                    Guid::from_buffer(b).map_err(|_| invalid_data("guid", off))
                })?,
            ),
            (BinXmlValueType::FileTimeArrayType, Some(sz)) => BinXmlValue::FileTimeArrayType(
                cursor.read_sized_vec_aligned::<8, _>(sz, "filetime_array", |_off, b| {
                    filetime_to_timestamp(u64::from_le_bytes(*b))
                })?,
            ),
            (BinXmlValueType::SysTimeArrayType, Some(sz)) => BinXmlValue::SysTimeArrayType(
                cursor.read_sized_vec_aligned::<16, _>(sz, "systime_array", |_off, b| {
                    systime_from_bytes(b)
                })?,
            ),
            (BinXmlValueType::SidArrayType, Some(sz)) => {
                // SID size is variable; we can only preallocate with a heuristic.
                BinXmlValue::SidArrayType(cursor.read_sized_vec(sz, 8, |c| read_sid(c))?)
            }
            (BinXmlValueType::HexInt32ArrayType, Some(sz)) => BinXmlValue::HexInt32ArrayType(
                cursor.read_sized_vec_aligned::<4, _>(sz, "hex32_array", |_off, b| {
                    let v = i32::from_le_bytes(*b);
                    Ok(Cow::Owned(format!("0x{:x}", v)))
                })?,
            ),
            (BinXmlValueType::HexInt64ArrayType, Some(sz)) => BinXmlValue::HexInt64ArrayType(
                cursor.read_sized_vec_aligned::<8, _>(sz, "hex64_array", |_off, b| {
                    let v = i64::from_le_bytes(*b);
                    Ok(Cow::Owned(format!("0x{:x}", v)))
                })?,
            ),

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
        size: Option<u16>,
        ansi_codec: EncodingRef,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::deserialize_value_type_cursor(value_type, &mut c, chunk, size, ansi_codec)?;
        cursor.set_position(c.position());
        Ok(v)
    }
}

fn to_delimited_list<N: ToString>(ns: impl AsRef<Vec<N>>) -> String {
    ns.as_ref()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<String>>()
        .join(",")
}

/// Format a timestamp as an RFC 3339-like UTC string with microsecond precision.
///
/// The output uses the `YYYY-MM-DDTHH:MM:SS.microsZ` form, matching EVTX JSON
/// conventions while avoiding allocator-heavy formatting paths.
pub fn format_timestamp(ts: &Timestamp) -> String {
    let dt = Offset::UTC.to_datetime(*ts);
    let mut out = String::with_capacity(27);
    push_4_digits(&mut out, dt.year() as u32);
    out.push('-');
    push_2_digits(&mut out, u32::from(dt.month() as u8));
    out.push('-');
    push_2_digits(&mut out, u32::from(dt.day() as u8));
    out.push('T');
    push_2_digits(&mut out, u32::from(dt.hour() as u8));
    out.push(':');
    push_2_digits(&mut out, u32::from(dt.minute() as u8));
    out.push(':');
    push_2_digits(&mut out, u32::from(dt.second() as u8));
    out.push('.');
    let micros = (dt.subsec_nanosecond() / 1_000) as u32;
    push_6_digits(&mut out, micros);
    out.push('Z');
    out
}

fn push_2_digits(out: &mut String, value: u32) {
    let tens = (value / 10) % 10;
    let ones = value % 10;
    out.push(char::from(b'0' + tens as u8));
    out.push(char::from(b'0' + ones as u8));
}

fn push_4_digits(out: &mut String, value: u32) {
    let thousands = (value / 1000) % 10;
    let hundreds = (value / 100) % 10;
    let tens = (value / 10) % 10;
    let ones = value % 10;
    out.push(char::from(b'0' + thousands as u8));
    out.push(char::from(b'0' + hundreds as u8));
    out.push(char::from(b'0' + tens as u8));
    out.push(char::from(b'0' + ones as u8));
}

fn push_6_digits(out: &mut String, value: u32) {
    let hundred_thousands = (value / 100000) % 10;
    let ten_thousands = (value / 10000) % 10;
    let thousands = (value / 1000) % 10;
    let hundreds = (value / 100) % 10;
    let tens = (value / 10) % 10;
    let ones = value % 10;
    out.push(char::from(b'0' + hundred_thousands as u8));
    out.push(char::from(b'0' + ten_thousands as u8));
    out.push(char::from(b'0' + thousands as u8));
    out.push(char::from(b'0' + hundreds as u8));
    out.push(char::from(b'0' + tens as u8));
    out.push(char::from(b'0' + ones as u8));
}

impl<'c> From<BinXmlValue<'c>> for serde_json::Value {
    fn from(value: BinXmlValue<'c>) -> Self {
        match value {
            BinXmlValue::NullType => Value::Null,
            BinXmlValue::StringType(s) => json!(s.as_ref()),
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
            BinXmlValue::FileTimeType(tm) => json!(format_timestamp(&tm)),
            BinXmlValue::SysTimeType(tm) => json!(format_timestamp(&tm)),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string),
            BinXmlValue::StringArrayType(items) => {
                json!(items.iter().map(|item| item.as_ref()).collect::<Vec<&str>>())
            }
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
            BinXmlValue::FileTimeArrayType(filetimes) => json!(
                filetimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
            ),
            BinXmlValue::SysTimeArrayType(systimes) => json!(
                systimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
            ),
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
            BinXmlValue::StringType(s) => json!(s.as_ref()),
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
            BinXmlValue::FileTimeType(tm) => json!(format_timestamp(tm)),
            BinXmlValue::SysTimeType(tm) => json!(format_timestamp(tm)),
            BinXmlValue::SidType(sid) => json!(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => json!(hex_string),
            BinXmlValue::HexInt64Type(hex_string) => json!(hex_string),
            BinXmlValue::StringArrayType(items) => {
                json!(items.iter().map(|item| item.as_ref()).collect::<Vec<&str>>())
            }
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
            BinXmlValue::FileTimeArrayType(filetimes) => json!(
                filetimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
            ),
            BinXmlValue::SysTimeArrayType(systimes) => json!(
                systimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
            ),
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

impl BinXmlValue<'_> {
    pub fn as_cow_str(&self) -> Cow<'_, str> {
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
            BinXmlValue::BinaryType(bytes) => Cow::Owned(bytes.iter().fold(
                String::with_capacity(bytes.len() * 2),
                |mut acc, &b| {
                    write!(acc, "{:02X}", b).unwrap();
                    acc
                },
            )),
            BinXmlValue::GuidType(guid) => Cow::Owned(guid.to_string()),
            BinXmlValue::SizeTType(sz) => Cow::Owned(sz.to_string()),
            BinXmlValue::FileTimeType(tm) => Cow::Owned(format_timestamp(tm)),
            BinXmlValue::SysTimeType(tm) => Cow::Owned(format_timestamp(tm)),
            BinXmlValue::SidType(sid) => Cow::Owned(sid.to_string()),
            BinXmlValue::HexInt32Type(hex_string) => hex_string.clone(),
            BinXmlValue::HexInt64Type(hex_string) => hex_string.clone(),
            BinXmlValue::StringArrayType(items) => {
                Cow::Owned(items.iter().map(|item| item.as_ref()).collect::<Vec<&str>>().join(","))
            }
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
            BinXmlValue::FileTimeArrayType(filetimes) => Cow::Owned(
                filetimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
                    .join(","),
            ),
            BinXmlValue::SysTimeArrayType(systimes) => Cow::Owned(
                systimes
                    .iter()
                    .map(|ts| format_timestamp(ts))
                    .collect::<Vec<String>>()
                    .join(","),
            ),
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
