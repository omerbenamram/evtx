use crate::err::{DeserializationError, DeserializationResult as Result};
use crate::evtx_chunk::EvtxChunk;
use crate::utils::invalid_data;
use crate::utils::windows::{filetime_to_timestamp, read_systime, systime_from_bytes};
use crate::utils::{ByteCursor, Utf16LeSlice};

use bumpalo::Bump;
use encoding::EncodingRef;
use jiff::{Timestamp, tz::Offset};
use log::{trace, warn};
use serde_json::{Value, json};
use std::borrow::Cow;
use std::fmt::Write;
use std::fmt::{self, Display};
use std::io::Cursor;
use std::string::ToString;
use winstructs::guid::Guid;

/// Borrowed SID bytes (used to avoid heap allocation in the hot path).
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SidRef<'a> {
    bytes: &'a [u8],
}

impl<'a> SidRef<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }
}

impl fmt::Display for SidRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let bytes = self.bytes;
        if bytes.len() < 8 {
            return write!(f, "S-?");
        }
        let revision = bytes[0];
        let sub_count = bytes[1] as usize;

        // IdentifierAuthority is a 48-bit big-endian integer.
        let mut authority: u64 = 0;
        for &b in &bytes[2..8] {
            authority = (authority << 8) | u64::from(b);
        }

        write!(f, "S-{}-{}", revision, authority)?;

        let mut off = 8usize;
        for _ in 0..sub_count {
            if off + 4 > bytes.len() {
                break;
            }
            let sub =
                u32::from_le_bytes([bytes[off], bytes[off + 1], bytes[off + 2], bytes[off + 3]]);
            write!(f, "-{}", sub)?;
            off += 4;
        }
        Ok(())
    }
}

#[derive(Debug, PartialOrd, PartialEq, Clone)]
pub enum BinXmlValue<'a> {
    NullType,
    /// UTF-16LE string slice (BinXML `StringType`).
    StringType(Utf16LeSlice<'a>),
    /// ANSI string decoded to UTF-8, stored as a borrowed slice (typically bump-allocated).
    AnsiStringType(&'a str),
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
    SidType(SidRef<'a>),
    HexInt32Type(u32),
    HexInt64Type(u64),
    EvtHandle,
    /// Raw BinXML fragment bytes (no length prefix).
    ///
    /// This is stored as a slice into the chunk data and parsed on demand by higher-level code.
    BinXmlType(&'a [u8]),
    EvtXml,
    /// Array of UTF-16LE strings (null-terminated items).
    StringArrayType(&'a [Utf16LeSlice<'a>]),
    AnsiStringArrayType,
    Int8ArrayType(&'a [i8]),
    UInt8ArrayType(&'a [u8]),
    Int16ArrayType(&'a [i16]),
    UInt16ArrayType(&'a [u16]),
    Int32ArrayType(&'a [i32]),
    UInt32ArrayType(&'a [u32]),
    Int64ArrayType(&'a [i64]),
    UInt64ArrayType(&'a [u64]),
    Real32ArrayType(&'a [f32]),
    Real64ArrayType(&'a [f64]),
    BoolArrayType(&'a [bool]),
    BinaryArrayType,
    GuidArrayType(&'a [Guid]),
    SizeTArrayType,
    FileTimeArrayType(&'a [Timestamp]),
    SysTimeArrayType(&'a [Timestamp]),
    SidArrayType(&'a [SidRef<'a>]),
    HexInt32ArrayType(&'a [u32]),
    HexInt64ArrayType(&'a [u64]),
    EvtArrayHandle,
    BinXmlArrayType,
    EvtXmlArrayType,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Clone, Copy)]
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
    /// Lookup table for O(1) byte-to-type conversion without indirect branches.
    const LOOKUP: [Option<BinXmlValueType>; 256] = {
        use BinXmlValueType::*;
        let mut t: [Option<BinXmlValueType>; 256] = [None; 256];
        t[0x00] = Some(NullType);
        t[0x01] = Some(StringType);
        t[0x02] = Some(AnsiStringType);
        t[0x03] = Some(Int8Type);
        t[0x04] = Some(UInt8Type);
        t[0x05] = Some(Int16Type);
        t[0x06] = Some(UInt16Type);
        t[0x07] = Some(Int32Type);
        t[0x08] = Some(UInt32Type);
        t[0x09] = Some(Int64Type);
        t[0x0a] = Some(UInt64Type);
        t[0x0b] = Some(Real32Type);
        t[0x0c] = Some(Real64Type);
        t[0x0d] = Some(BoolType);
        t[0x0e] = Some(BinaryType);
        t[0x0f] = Some(GuidType);
        t[0x10] = Some(SizeTType);
        t[0x11] = Some(FileTimeType);
        t[0x12] = Some(SysTimeType);
        t[0x13] = Some(SidType);
        t[0x14] = Some(HexInt32Type);
        t[0x15] = Some(HexInt64Type);
        t[0x20] = Some(EvtHandle);
        t[0x21] = Some(BinXmlType);
        t[0x23] = Some(EvtXmlType);
        t[0x81] = Some(StringArrayType);
        t[0x82] = Some(AnsiStringArrayType);
        t[0x83] = Some(Int8ArrayType);
        t[0x84] = Some(UInt8ArrayType);
        t[0x85] = Some(Int16ArrayType);
        t[0x86] = Some(UInt16ArrayType);
        t[0x87] = Some(Int32ArrayType);
        t[0x88] = Some(UInt32ArrayType);
        t[0x89] = Some(Int64ArrayType);
        t[0x8a] = Some(UInt64ArrayType);
        t[0x8b] = Some(Real32ArrayType);
        t[0x8c] = Some(Real64ArrayType);
        t[0x8d] = Some(BoolArrayType);
        t[0x8e] = Some(BinaryArrayType);
        t[0x8f] = Some(GuidArrayType);
        t[0x90] = Some(SizeTArrayType);
        t[0x91] = Some(FileTimeArrayType);
        t[0x92] = Some(SysTimeArrayType);
        t[0x93] = Some(SidArrayType);
        t[0x94] = Some(HexInt32ArrayType);
        t[0x95] = Some(HexInt64ArrayType);
        t
    };

    #[inline]
    pub fn from_u8(byte: u8) -> Option<BinXmlValueType> {
        Self::LOOKUP[byte as usize]
    }
}

impl<'a> BinXmlValue<'a> {
    pub(crate) fn from_binxml_cursor_in(
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
        arena: &'a Bump,
    ) -> Result<BinXmlValue<'a>> {
        let value_type_token = cursor.u8()?;

        let value_type = BinXmlValueType::from_u8(value_type_token).ok_or_else(|| {
            DeserializationError::InvalidValueVariant {
                value: value_type_token,
                offset: cursor.position(),
            }
        })?;

        let data = Self::deserialize_value_type_cursor_in(
            &value_type,
            cursor,
            chunk,
            size,
            ansi_codec,
            arena,
        )?;

        Ok(data)
    }

    pub fn from_binxml_stream_in(
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
        arena: &'a Bump,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::from_binxml_cursor_in(&mut c, chunk, size, ansi_codec, arena)?;
        cursor.set_position(c.position());
        Ok(v)
    }

    pub(crate) fn deserialize_value_type_cursor(
        value_type: &BinXmlValueType,
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
        arena: &'a Bump,
    ) -> Result<BinXmlValue<'a>> {
        Self::deserialize_value_type_cursor_in(value_type, cursor, chunk, size, ansi_codec, arena)
    }

    pub(crate) fn deserialize_value_type_cursor_in(
        value_type: &BinXmlValueType,
        cursor: &mut ByteCursor<'a>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
        arena: &'a Bump,
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
                    cursor.utf16_by_char_count(sz_bytes / 2, "<string_value>")?
                };
                BinXmlValue::StringType(s.unwrap_or_else(Utf16LeSlice::empty))
            }
            (BinXmlValueType::StringType, None) => {
                let s = cursor.len_prefixed_utf16_string(false, "<string_value>")?;
                BinXmlValue::StringType(s.unwrap_or_else(Utf16LeSlice::empty))
            }

            (BinXmlValueType::AnsiStringType, Some(sz)) => {
                let sz_bytes = usize::from(sz);
                let raw = cursor.take_bytes(sz_bytes, "<ansi_string_value>")?;
                // Filter embedded NUL bytes (historical behavior).
                let mut filtered = bumpalo::collections::Vec::with_capacity_in(sz_bytes, arena);
                for &b in raw {
                    if b != 0 {
                        filtered.push(b);
                    }
                }
                let filtered = filtered.into_bump_slice();
                let decoded = ansi_codec
                    .decode(filtered, encoding::DecoderTrap::Strict)
                    .map_err(|m| DeserializationError::AnsiDecodeError {
                        encoding_used: ansi_codec.name(),
                        inner_message: m.to_string(),
                    })?;
                BinXmlValue::AnsiStringType(arena.alloc_str(&decoded))
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
                let v = u32::from_le_bytes(cursor.array::<4>("sizet32")?);
                BinXmlValue::HexInt32Type(v)
            }
            (BinXmlValueType::SizeTType, Some(8)) => {
                let v = u64::from_le_bytes(cursor.array::<8>("sizet64")?);
                BinXmlValue::HexInt64Type(v)
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
            (BinXmlValueType::SidType, _) => BinXmlValue::SidType(cursor.read_sid_ref()?),

            (BinXmlValueType::HexInt32Type, _) => {
                let v = u32::from_le_bytes(cursor.array::<4>("hex32")?);
                BinXmlValue::HexInt32Type(v)
            }
            (BinXmlValueType::HexInt64Type, _) => {
                let v = u64::from_le_bytes(cursor.array::<8>("hex64")?);
                BinXmlValue::HexInt64Type(v)
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
                if size_usize == 0 {
                    return Ok(BinXmlValue::StringArrayType(&[]));
                }
                let start = cursor.pos();
                let end = start.saturating_add(size_usize);
                let mut out = bumpalo::collections::Vec::new_in(arena);
                while cursor.pos() < end {
                    let s = cursor.null_terminated_utf16_string("string_array")?;
                    out.push(s);
                }
                BinXmlValue::StringArrayType(out.into_bump_slice())
            }
            (BinXmlValueType::Int8ArrayType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "i8_array")?;
                let out = arena.alloc_slice_fill_iter(bytes.iter().map(|&b| b as i8));
                BinXmlValue::Int8ArrayType(out)
            }
            (BinXmlValueType::UInt8ArrayType, Some(sz)) => {
                let bytes = cursor.take_bytes(usize::from(sz), "u8_array")?;
                BinXmlValue::UInt8ArrayType(bytes)
            }
            (BinXmlValueType::Int16ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<2, _>(
                    sz,
                    "i16_array",
                    arena,
                    |_off, b| Ok(i16::from_le_bytes(*b)),
                )?;
                BinXmlValue::Int16ArrayType(out)
            }
            (BinXmlValueType::UInt16ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<2, _>(
                    sz,
                    "u16_array",
                    arena,
                    |_off, b| Ok(u16::from_le_bytes(*b)),
                )?;
                BinXmlValue::UInt16ArrayType(out)
            }
            (BinXmlValueType::Int32ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<4, _>(
                    sz,
                    "i32_array",
                    arena,
                    |_off, b| Ok(i32::from_le_bytes(*b)),
                )?;
                BinXmlValue::Int32ArrayType(out)
            }
            (BinXmlValueType::UInt32ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<4, _>(
                    sz,
                    "u32_array",
                    arena,
                    |_off, b| Ok(u32::from_le_bytes(*b)),
                )?;
                BinXmlValue::UInt32ArrayType(out)
            }
            (BinXmlValueType::Int64ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<8, _>(
                    sz,
                    "i64_array",
                    arena,
                    |_off, b| Ok(i64::from_le_bytes(*b)),
                )?;
                BinXmlValue::Int64ArrayType(out)
            }
            (BinXmlValueType::UInt64ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<8, _>(
                    sz,
                    "u64_array",
                    arena,
                    |_off, b| Ok(u64::from_le_bytes(*b)),
                )?;
                BinXmlValue::UInt64ArrayType(out)
            }
            (BinXmlValueType::Real32ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<4, _>(
                    sz,
                    "f32_array",
                    arena,
                    |_off, b| Ok(f32::from_le_bytes(*b)),
                )?;
                BinXmlValue::Real32ArrayType(out)
            }
            (BinXmlValueType::Real64ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<8, _>(
                    sz,
                    "f64_array",
                    arena,
                    |_off, b| Ok(f64::from_le_bytes(*b)),
                )?;
                BinXmlValue::Real64ArrayType(out)
            }
            (BinXmlValueType::BoolArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<4, _>(
                    sz,
                    "bool_array",
                    arena,
                    |off, b| {
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
                    },
                )?;
                BinXmlValue::BoolArrayType(out)
            }
            (BinXmlValueType::GuidArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<16, _>(
                    sz,
                    "guid_array",
                    arena,
                    |off, b| Guid::from_buffer(b).map_err(|_| invalid_data("guid", off)),
                )?;
                BinXmlValue::GuidArrayType(out)
            }
            (BinXmlValueType::FileTimeArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<8, _>(
                    sz,
                    "filetime_array",
                    arena,
                    |_off, b| filetime_to_timestamp(u64::from_le_bytes(*b)),
                )?;
                BinXmlValue::FileTimeArrayType(out)
            }
            (BinXmlValueType::SysTimeArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<16, _>(
                    sz,
                    "systime_array",
                    arena,
                    |_off, b| systime_from_bytes(b),
                )?;
                BinXmlValue::SysTimeArrayType(out)
            }
            (BinXmlValueType::SidArrayType, Some(sz)) => {
                let size_usize = usize::from(sz);
                if size_usize == 0 {
                    return Ok(BinXmlValue::SidArrayType(&[]));
                }
                let start_pos = cursor.pos();
                let mut out = bumpalo::collections::Vec::with_capacity_in(size_usize / 8, arena);
                while (cursor.pos() - start_pos) < size_usize {
                    out.push(cursor.read_sid_ref()?);
                }
                BinXmlValue::SidArrayType(out.into_bump_slice())
            }
            (BinXmlValueType::HexInt32ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<4, _>(
                    sz,
                    "hex32_array",
                    arena,
                    |_off, b| Ok(u32::from_le_bytes(*b)),
                )?;
                BinXmlValue::HexInt32ArrayType(out)
            }
            (BinXmlValueType::HexInt64ArrayType, Some(sz)) => {
                let out = cursor.read_sized_slice_aligned_in::<8, _>(
                    sz,
                    "hex64_array",
                    arena,
                    |_off, b| Ok(u64::from_le_bytes(*b)),
                )?;
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

    pub fn deserialize_value_type_in(
        value_type: &BinXmlValueType,
        cursor: &mut Cursor<&'a [u8]>,
        chunk: Option<&'a EvtxChunk<'a>>,
        size: Option<u16>,
        ansi_codec: EncodingRef,
        arena: &'a Bump,
    ) -> Result<BinXmlValue<'a>> {
        let start = cursor.position() as usize;
        let buf = *cursor.get_ref();
        let mut c = ByteCursor::with_pos(buf, start)?;
        let v = Self::deserialize_value_type_cursor(
            value_type, &mut c, chunk, size, ansi_codec, arena,
        )?;
        cursor.set_position(c.position());
        Ok(v)
    }
}

impl<'a> Display for BinXmlValue<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut vr = crate::binxml::value_render::ValueRenderer::default();
        let mut writer = Vec::new();
        vr.write_json_value_text(&mut writer, self)
            .map_err(|_| fmt::Error)?;

        match self {
            BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => Ok(()),
            _ => write!(f, "{}", String::from_utf8(writer).unwrap()),
        }
    }
}
