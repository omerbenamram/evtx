//! Shared value-to-text rendering for BinXML values.
//!
//! The JSON and XML IR renderers both need to serialize `BinXmlValue` payloads
//! into their textual representation (numbers, GUIDs, timestamps, hex bytes,
//! comma-delimited arrays, etc.). This module centralizes that logic so both
//! renderers stay in sync and avoid allocating intermediate `String`s.

use crate::binxml::compiled::value_ty;
use crate::binxml::value_variant::{BinXmlValue, SidRef};
use crate::err::{EvtxError, Result};
use crate::utils::Utf16LeSlice;
use jiff::{Timestamp, tz::Offset};
use sonic_rs::format::{CompactFormatter, Formatter};
use sonic_rs::writer::WriteExt;
use zmij::Buffer as ZmijBuffer;

#[derive(Debug, Clone, Copy)]
pub(crate) enum StringEscapeMode {
    /// Escape string content for embedding inside a JSON string literal
    /// (no surrounding quotes).
    Json,
    /// Escape string content for embedding in XML text/attribute contexts.
    Xml { in_attribute: bool },
}

/// Stateful BinXmlValue formatter (owns reusable scratch buffers).
pub(crate) struct ValueRenderer {
    float_buf: ZmijBuffer,
    formatter: CompactFormatter,
}

impl Default for ValueRenderer {
    fn default() -> Self {
        ValueRenderer {
            float_buf: ZmijBuffer::new(),
            formatter: CompactFormatter,
        }
    }
}

macro_rules! write_int {
    ($self:ident, $writer:ident, $m:ident, $v:expr) => {
        $self.formatter.$m($writer, $v).map_err(EvtxError::from)
    };
}

impl ValueRenderer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn write_json_value_text<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: &BinXmlValue<'_>,
    ) -> Result<()> {
        self.write_value_text(writer, value, StringEscapeMode::Json)
    }

    pub(crate) fn write_xml_value_text<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: &BinXmlValue<'_>,
        in_attribute: bool,
    ) -> Result<()> {
        self.write_value_text(writer, value, StringEscapeMode::Xml { in_attribute })
    }

    fn write_value_text<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: &BinXmlValue<'_>,
        string_mode: StringEscapeMode,
    ) -> Result<()> {
        match value {
            BinXmlValue::NullType => Ok(()),
            BinXmlValue::StringType(s) => self.write_utf16_escaped(writer, *s, string_mode),
            BinXmlValue::AnsiStringType(s) => self.write_str_escaped(writer, s, string_mode),
            BinXmlValue::Int8Type(v) => write_int!(self, writer, write_i8, *v),
            BinXmlValue::UInt8Type(v) => write_int!(self, writer, write_u8, *v),
            BinXmlValue::Int16Type(v) => write_int!(self, writer, write_i16, *v),
            BinXmlValue::UInt16Type(v) => write_int!(self, writer, write_u16, *v),
            BinXmlValue::Int32Type(v) => write_int!(self, writer, write_i32, *v),
            BinXmlValue::UInt32Type(v) => write_int!(self, writer, write_u32, *v),
            BinXmlValue::Int64Type(v) => write_int!(self, writer, write_i64, *v),
            BinXmlValue::UInt64Type(v) => write_int!(self, writer, write_u64, *v),
            BinXmlValue::Real32Type(v) => self.write_float(writer, *v),
            BinXmlValue::Real64Type(v) => self.write_float(writer, *v),
            BinXmlValue::BoolType(v) => {
                self.write_bytes(writer, if *v { b"true" } else { b"false" })
            }
            BinXmlValue::BinaryType(bytes) => self.write_hex_bytes_upper(writer, bytes),
            BinXmlValue::GuidType(guid) => self.write_guid(writer, guid),
            BinXmlValue::SizeTType(v) => write_int!(self, writer, write_u64, *v as u64),
            BinXmlValue::FileTimeType(tm) | BinXmlValue::SysTimeType(tm) => {
                self.write_datetime(writer, tm)
            }
            BinXmlValue::SidType(sid) => self.write_sid(writer, sid),
            BinXmlValue::HexInt32Type(v) => self.write_hex_prefixed_u32_lower(writer, *v),
            BinXmlValue::HexInt64Type(v) => self.write_hex_prefixed_u64_lower(writer, *v),
            BinXmlValue::StringArrayType(items) => self.write_list(writer, items, |s, w, item| {
                s.write_utf16_escaped(w, *item, string_mode)
            }),
            BinXmlValue::Int8ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_i8, *v))
            }
            BinXmlValue::UInt8ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_u8, *v))
            }
            BinXmlValue::Int16ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_i16, *v))
            }
            BinXmlValue::UInt16ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_u16, *v))
            }
            BinXmlValue::Int32ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_i32, *v))
            }
            BinXmlValue::UInt32ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_u32, *v))
            }
            BinXmlValue::Int64ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_i64, *v))
            }
            BinXmlValue::UInt64ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| write_int!(s, w, write_u64, *v))
            }
            BinXmlValue::Real32ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| s.write_float(w, *v))
            }
            BinXmlValue::Real64ArrayType(items) => {
                self.write_list(writer, items, |s, w, v| s.write_float(w, *v))
            }
            BinXmlValue::BoolArrayType(items) => self.write_list(writer, items, |s, w, v| {
                s.write_bytes(w, if *v { b"true" } else { b"false" })
            }),
            BinXmlValue::GuidArrayType(items) => {
                self.write_list(writer, items, |s, w, v| s.write_guid(w, v))
            }
            BinXmlValue::FileTimeArrayType(items) | BinXmlValue::SysTimeArrayType(items) => {
                self.write_list(writer, items, |s, w, v| s.write_datetime(w, v))
            }
            BinXmlValue::SidArrayType(items) => {
                self.write_list(writer, items, |s, w, v| s.write_sid(w, v))
            }
            BinXmlValue::HexInt32ArrayType(items) => self.write_list(writer, items, |s, w, v| {
                s.write_hex_prefixed_u32_lower(w, *v)
            }),
            BinXmlValue::HexInt64ArrayType(items) => self.write_list(writer, items, |s, w, v| {
                s.write_hex_prefixed_u64_lower(w, *v)
            }),
            BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => Err(
                EvtxError::FailedToCreateRecordModel("unsupported BinXML value in renderer"),
            ),
            other => Err(EvtxError::Unimplemented {
                name: format!("value formatting for {:?}", other),
            }),
        }
    }

    /// Write a substitution value's text directly from its raw chunk bytes
    /// (compiled-template path). Mirrors `deserialize_value_type_cursor_in` +
    /// `write_value_text` for the scalar types the compiled pre-flight admits;
    /// the pre-flight guarantees sizes match (so the `expect`s are unreachable).
    ///
    /// `bytes` is exactly the descriptor-sized value window. `decoded_ansi` is
    /// the pre-decoded ANSI payload for `value_ty::ANSI_STRING` (pre-flight).
    pub(crate) fn write_raw_value_text<W: WriteExt>(
        &mut self,
        writer: &mut W,
        ty: u8,
        bytes: &[u8],
        decoded_ansi: Option<&str>,
        mode: StringEscapeMode,
    ) -> Result<()> {
        match ty {
            value_ty::NULL => Ok(()),
            // StringType: truncate at the first NUL unit (mirrors `utf16_by_char_count`).
            value_ty::UTF16_STRING => {
                let mut units = bytes.len() / 2;
                for (idx, chunk) in bytes.chunks_exact(2).enumerate() {
                    if chunk[0] == 0 && chunk[1] == 0 {
                        units = idx;
                        break;
                    }
                }
                self.write_utf16_escaped(writer, Utf16LeSlice::new(bytes, units), mode)
            }
            value_ty::ANSI_STRING => {
                self.write_str_escaped(writer, decoded_ansi.unwrap_or(""), mode)
            }
            value_ty::INT8 => write_int!(self, writer, write_i8, bytes[0] as i8),
            value_ty::UINT8 => write_int!(self, writer, write_u8, bytes[0]),
            value_ty::INT16 => write_int!(self, writer, write_i16, i16::from_le_bytes(le2(bytes))),
            value_ty::UINT16 => write_int!(self, writer, write_u16, u16::from_le_bytes(le2(bytes))),
            value_ty::INT32 => write_int!(self, writer, write_i32, i32::from_le_bytes(le4(bytes))),
            value_ty::UINT32 => write_int!(self, writer, write_u32, u32::from_le_bytes(le4(bytes))),
            value_ty::INT64 => write_int!(self, writer, write_i64, i64::from_le_bytes(le8(bytes))),
            value_ty::UINT64 => write_int!(self, writer, write_u64, u64::from_le_bytes(le8(bytes))),
            value_ty::REAL32 => self.write_float(writer, f32::from_le_bytes(le4(bytes))),
            value_ty::REAL64 => self.write_float(writer, f64::from_le_bytes(le8(bytes))),
            value_ty::BOOL => {
                let raw = i32::from_le_bytes(le4(bytes));
                self.write_bytes(writer, if raw != 0 { b"true" } else { b"false" })
            }
            value_ty::BINARY => self.write_hex_bytes_upper(writer, bytes),
            value_ty::GUID => self.write_guid(writer, bytes.try_into().expect("guid size")),
            // SizeT renders as HexInt of its width.
            value_ty::SIZE_T if bytes.len() == 4 => {
                self.write_hex_prefixed_u32_lower(writer, u32::from_le_bytes(le4(bytes)))
            }
            value_ty::SIZE_T => {
                self.write_hex_prefixed_u64_lower(writer, u64::from_le_bytes(le8(bytes)))
            }
            value_ty::FILETIME => {
                let tm =
                    crate::utils::windows::filetime_to_timestamp(u64::from_le_bytes(le8(bytes)))?;
                self.write_datetime(writer, &tm)
            }
            value_ty::SYSTIME => {
                let mut cursor = crate::utils::ByteCursor::with_pos(bytes, 0)?;
                let tm = crate::utils::windows::read_systime(&mut cursor)?;
                self.write_datetime(writer, &tm)
            }
            value_ty::SID => self.write_sid(writer, &SidRef::new(bytes)),
            value_ty::HEX_INT32 => {
                self.write_hex_prefixed_u32_lower(writer, u32::from_le_bytes(le4(bytes)))
            }
            value_ty::HEX_INT64 => {
                self.write_hex_prefixed_u64_lower(writer, u64::from_le_bytes(le8(bytes)))
            }
            // Only reachable for empty payloads (non-empty BinXml classifies
            // as an element); mirrors the NullType conversion.
            value_ty::BIN_XML => Ok(()),
            _ => Err(EvtxError::FailedToCreateRecordModel(
                "unsupported raw value type in compiled renderer",
            )),
        }
    }

    fn write_bytes<W: WriteExt>(&mut self, writer: &mut W, bytes: &[u8]) -> Result<()> {
        writer.write_all(bytes).map_err(EvtxError::from)
    }

    fn write_byte<W: WriteExt>(&mut self, writer: &mut W, byte: u8) -> Result<()> {
        writer.write_all(&[byte]).map_err(EvtxError::from)
    }

    /// Write `items` comma-delimited, formatting each via `f`.
    fn write_list<W: WriteExt, T>(
        &mut self,
        writer: &mut W,
        items: &[T],
        mut f: impl FnMut(&mut Self, &mut W, &T) -> Result<()>,
    ) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            f(self, writer, item)?;
        }
        Ok(())
    }

    fn write_utf16_escaped<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: Utf16LeSlice<'_>,
        mode: StringEscapeMode,
    ) -> Result<()> {
        let bytes = value.as_bytes();
        let units = bytes.len() / 2;
        if units == 0 {
            return Ok(());
        }
        match mode {
            StringEscapeMode::Json => {
                utf16_simd::write_json_utf16le(writer, bytes, units, false).map_err(EvtxError::from)
            }
            StringEscapeMode::Xml { in_attribute } => {
                utf16_simd::write_xml_utf16le(writer, bytes, units, in_attribute)
                    .map_err(EvtxError::from)
            }
        }
    }

    fn write_str_escaped<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: &str,
        mode: StringEscapeMode,
    ) -> Result<()> {
        match mode {
            StringEscapeMode::Json => self
                .formatter
                .write_string_fast(writer, value, false)
                .map_err(EvtxError::from),
            StringEscapeMode::Xml { in_attribute } => {
                self.write_xml_escaped_str(writer, value, in_attribute)
            }
        }
    }

    fn write_xml_escaped_str<W: WriteExt>(
        &mut self,
        writer: &mut W,
        text: &str,
        in_attribute: bool,
    ) -> Result<()> {
        for ch in text.chars() {
            match ch {
                '&' => self.write_bytes(writer, b"&amp;")?,
                '<' => self.write_bytes(writer, b"&lt;")?,
                '>' => self.write_bytes(writer, b"&gt;")?,
                '"' if in_attribute => self.write_bytes(writer, b"&quot;")?,
                '\'' if in_attribute => self.write_bytes(writer, b"&apos;")?,
                _ => {
                    let mut buf = [0_u8; 4];
                    let slice = ch.encode_utf8(&mut buf).as_bytes();
                    self.write_bytes(writer, slice)?;
                }
            }
        }
        Ok(())
    }

    fn write_hex_bytes_upper<W: WriteExt>(&mut self, writer: &mut W, bytes: &[u8]) -> Result<()> {
        let mut buf = [0_u8; 128];
        for chunk in bytes.chunks(buf.len() / 2) {
            let mut n = 0;
            for &b in chunk {
                buf[n] = to_hex_digit(b >> 4);
                buf[n + 1] = to_hex_digit(b & 0x0f);
                n += 2;
            }
            writer.write_all(&buf[..n]).map_err(EvtxError::from)?;
        }
        Ok(())
    }

    /// Write a GUID in the canonical uppercase-hex form (`D1-D2-D3-D4[0..2]-D4[2..8]`,
    /// data1-3 little-endian), matching `winstructs::guid::Guid`'s `Display`.
    fn write_guid<W: WriteExt>(&mut self, writer: &mut W, bytes: &[u8; 16]) -> Result<()> {
        const ORDER: [usize; 16] = [3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15];
        let mut out = [b'-'; 36];
        let mut n = 0;
        for (i, &idx) in ORDER.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                n += 1;
            }
            out[n] = to_hex_digit(bytes[idx] >> 4);
            out[n + 1] = to_hex_digit(bytes[idx] & 0x0f);
            n += 2;
        }
        writer.write_all(&out).map_err(EvtxError::from)
    }

    /// Write a SID in `S-R-A-S1-..-Sn` form, matching `SidRef`'s `Display`.
    fn write_sid<W: WriteExt>(&mut self, writer: &mut W, sid: &SidRef<'_>) -> Result<()> {
        let bytes = sid.as_bytes();
        if bytes.len() < 8 {
            return self.write_bytes(writer, b"S-?");
        }
        // IdentifierAuthority is a 48-bit big-endian integer.
        let mut authority: u64 = 0;
        for &b in &bytes[2..8] {
            authority = (authority << 8) | u64::from(b);
        }
        self.write_bytes(writer, b"S-")?;
        write_int!(self, writer, write_u8, bytes[0])?;
        self.write_byte(writer, b'-')?;
        write_int!(self, writer, write_u64, authority)?;
        for chunk in bytes[8..].chunks_exact(4).take(bytes[1] as usize) {
            self.write_byte(writer, b'-')?;
            let sub = u32::from_le_bytes(chunk.try_into().expect("4-byte chunk"));
            write_int!(self, writer, write_u32, sub)?;
        }
        Ok(())
    }

    fn write_hex_prefixed_u32_lower<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: u32,
    ) -> Result<()> {
        self.write_bytes(writer, b"0x")?;
        self.write_hex_u64_lower(writer, u64::from(value))
    }

    fn write_hex_prefixed_u64_lower<W: WriteExt>(
        &mut self,
        writer: &mut W,
        value: u64,
    ) -> Result<()> {
        self.write_bytes(writer, b"0x")?;
        self.write_hex_u64_lower(writer, value)
    }

    fn write_hex_u64_lower<W: WriteExt>(&mut self, writer: &mut W, mut value: u64) -> Result<()> {
        let mut buf = [0_u8; 16];
        let mut len = 0usize;
        if value == 0 {
            buf[0] = b'0';
            len = 1;
        } else {
            while value != 0 {
                let nib = (value & 0x0f) as u8;
                buf[len] = to_hex_digit_lower(nib);
                len += 1;
                value >>= 4;
            }
            buf[..len].reverse();
        }
        writer.write_all(&buf[..len]).map_err(EvtxError::from)
    }

    fn write_float<W: WriteExt, F: zmij::Float>(&mut self, writer: &mut W, value: F) -> Result<()> {
        let s = self.float_buf.format(value);
        writer.write_all(s.as_bytes()).map_err(EvtxError::from)
    }

    fn write_datetime<W: WriteExt>(&mut self, writer: &mut W, tm: &Timestamp) -> Result<()> {
        let dt = Offset::UTC.to_datetime(*tm);
        let mut buf = *b"0000-00-00T00:00:00.000000Z";
        pack_digits(&mut buf[0..4], dt.year() as u32);
        pack_digits(&mut buf[5..7], u32::from(dt.month() as u8));
        pack_digits(&mut buf[8..10], u32::from(dt.day() as u8));
        pack_digits(&mut buf[11..13], u32::from(dt.hour() as u8));
        pack_digits(&mut buf[14..16], u32::from(dt.minute() as u8));
        pack_digits(&mut buf[17..19], u32::from(dt.second() as u8));
        pack_digits(&mut buf[20..26], (dt.subsec_nanosecond() / 1_000) as u32);
        writer.write_all(&buf).map_err(EvtxError::from)
    }
}

/// Fill `out` with the zero-padded decimal digits of `value` (lowest digit last).
fn pack_digits(out: &mut [u8], mut value: u32) {
    for b in out.iter_mut().rev() {
        *b = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

fn to_hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'A' + (value - 10),
    }
}

fn to_hex_digit_lower(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + (value - 10),
    }
}

fn le2(b: &[u8]) -> [u8; 2] {
    b[..2].try_into().expect("preflight-validated size")
}
fn le4(b: &[u8]) -> [u8; 4] {
    b[..4].try_into().expect("preflight-validated size")
}
fn le8(b: &[u8]) -> [u8; 8] {
    b[..8].try_into().expect("preflight-validated size")
}
