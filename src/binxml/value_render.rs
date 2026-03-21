//! Shared value-to-text rendering for BinXML values.
//!
//! The JSON and XML IR renderers both need to serialize `BinXmlValue` payloads
//! into their textual representation (numbers, GUIDs, timestamps, hex bytes,
//! comma-delimited arrays, etc.). This module centralizes that logic so both
//! renderers stay in sync and avoid allocating intermediate `String`s.

use crate::binxml::value_variant::BinXmlValue;
use crate::binxml::xml_value_format;
use crate::err::{EvtxError, Result};
use crate::utils::Utf16LeSlice;
use jiff::Timestamp;
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
            BinXmlValue::Int8Type(v) => {
                self.formatter.write_i8(writer, *v).map_err(EvtxError::from)
            }
            BinXmlValue::UInt8Type(v) => {
                self.formatter.write_u8(writer, *v).map_err(EvtxError::from)
            }
            BinXmlValue::Int16Type(v) => self
                .formatter
                .write_i16(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt16Type(v) => self
                .formatter
                .write_u16(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Int32Type(v) => self
                .formatter
                .write_i32(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt32Type(v) => self
                .formatter
                .write_u32(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Int64Type(v) => self
                .formatter
                .write_i64(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt64Type(v) => self
                .formatter
                .write_u64(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::Real32Type(v) => self.write_float(writer, *v),
            BinXmlValue::Real64Type(v) => self.write_float(writer, *v),
            BinXmlValue::BoolType(v) => {
                self.write_bytes(writer, if *v { b"true" } else { b"false" })
            }
            BinXmlValue::BinaryType(bytes) => {
                xml_value_format::write_hex_bytes_upper(writer, bytes)
            }
            BinXmlValue::GuidType(guid) => write!(writer, "{}", guid).map_err(EvtxError::from),
            BinXmlValue::SizeTType(v) => self
                .formatter
                .write_u64(writer, *v as u64)
                .map_err(EvtxError::from),
            BinXmlValue::FileTimeType(tm) | BinXmlValue::SysTimeType(tm) => {
                self.write_datetime(writer, tm)
            }
            BinXmlValue::SidType(sid) => write!(writer, "{}", sid).map_err(EvtxError::from),
            BinXmlValue::HexInt32Type(v) => {
                xml_value_format::write_hex_prefixed_u32_lower(writer, *v)
            }
            BinXmlValue::HexInt64Type(v) => {
                xml_value_format::write_hex_prefixed_u64_lower(writer, *v)
            }
            BinXmlValue::StringArrayType(items) => {
                let mut first = true;
                for item in items.iter() {
                    if !first {
                        self.write_byte(writer, b',')?;
                    }
                    first = false;
                    self.write_utf16_escaped(writer, *item, string_mode)?;
                }
                Ok(())
            }
            BinXmlValue::Int8ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::UInt8ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::Int16ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::UInt16ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::Int32ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::UInt32ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::Int64ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::UInt64ArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::Real32ArrayType(items) => self.write_float_list(writer, items),
            BinXmlValue::Real64ArrayType(items) => self.write_float_list(writer, items),
            BinXmlValue::BoolArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::GuidArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::FileTimeArrayType(items) | BinXmlValue::SysTimeArrayType(items) => {
                self.write_datetime_list(writer, items)
            }
            BinXmlValue::SidArrayType(items) => self.write_delimited(writer, items),
            BinXmlValue::HexInt32ArrayType(items) => self.write_hex_list_u32(writer, items),
            BinXmlValue::HexInt64ArrayType(items) => self.write_hex_list_u64(writer, items),
            BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => Err(
                EvtxError::FailedToCreateRecordModel("unsupported BinXML value in renderer"),
            ),
            other => Err(EvtxError::Unimplemented {
                name: format!("value formatting for {:?}", other),
            }),
        }
    }

    fn write_bytes<W: WriteExt>(&mut self, writer: &mut W, bytes: &[u8]) -> Result<()> {
        writer.write_all(bytes).map_err(EvtxError::from)
    }

    fn write_byte<W: WriteExt>(&mut self, writer: &mut W, byte: u8) -> Result<()> {
        writer.write_all(&[byte]).map_err(EvtxError::from)
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

    fn write_hex_list_u32<W: WriteExt>(&mut self, writer: &mut W, items: &[u32]) -> Result<()> {
        let mut first = true;
        for &item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            xml_value_format::write_hex_prefixed_u32_lower(writer, item)?;
        }
        Ok(())
    }

    fn write_hex_list_u64<W: WriteExt>(&mut self, writer: &mut W, items: &[u64]) -> Result<()> {
        let mut first = true;
        for &item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            xml_value_format::write_hex_prefixed_u64_lower(writer, item)?;
        }
        Ok(())
    }

    fn write_float<W: WriteExt, F: zmij::Float>(&mut self, writer: &mut W, value: F) -> Result<()> {
        let s = self.float_buf.format(value);
        writer.write_all(s.as_bytes()).map_err(EvtxError::from)
    }

    fn write_float_list<W: WriteExt, F: zmij::Float>(
        &mut self,
        writer: &mut W,
        items: &[F],
    ) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            let s = self.float_buf.format(*item);
            writer.write_all(s.as_bytes()).map_err(EvtxError::from)?;
        }
        Ok(())
    }

    fn write_datetime<W: WriteExt>(&mut self, writer: &mut W, tm: &Timestamp) -> Result<()> {
        xml_value_format::write_timestamp_utc(writer, tm)
    }

    fn write_datetime_list<W: WriteExt>(
        &mut self,
        writer: &mut W,
        items: &[Timestamp],
    ) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            self.write_datetime(writer, item)?;
        }
        Ok(())
    }

    fn write_delimited<W: WriteExt, T: std::fmt::Display>(
        &mut self,
        writer: &mut W,
        items: &[T],
    ) -> Result<()> {
        let mut first = true;
        for item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            write!(writer, "{}", item).map_err(EvtxError::from)?;
        }
        Ok(())
    }
}

/// Raw BinXML-to-XML formatter for the compiled XML fast path.
///
/// Unlike [`ValueRenderer`], this formatter operates directly on raw
/// substitution bytes instead of parsed [`BinXmlValue`] enums.
pub(crate) struct RawXmlRenderer {
    float_buf: ZmijBuffer,
    formatter: CompactFormatter,
}

impl Default for RawXmlRenderer {
    fn default() -> Self {
        RawXmlRenderer {
            float_buf: ZmijBuffer::new(),
            formatter: CompactFormatter,
        }
    }
}

impl RawXmlRenderer {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn value_has_content(raw: &[u8], value_type: u8) -> bool {
        match value_type {
            0x00 => false,
            0x01 => utf16le_content_length(raw) > 0,
            0x02 => raw.iter().any(|&b| b != 0),
            _ => !raw.is_empty(),
        }
    }

    pub(crate) fn split_array_items(raw: &[u8], base_type: u8) -> Vec<&[u8]> {
        if base_type == 0x01 {
            let mut items = Vec::new();
            let mut start = 0usize;
            let mut i = 0usize;

            while i + 1 < raw.len() {
                if raw[i] == 0 && raw[i + 1] == 0 {
                    items.push(&raw[start..i]);
                    start = i + 2;
                    i = start;
                } else {
                    i += 2;
                }
            }

            if start < raw.len() && raw[start..].iter().any(|&b| b != 0) {
                items.push(&raw[start..]);
            }

            items
        } else {
            let item_size = fixed_type_item_size(base_type);
            if item_size == 0 || raw.is_empty() {
                Vec::new()
            } else {
                raw.chunks_exact(item_size).collect()
            }
        }
    }

    pub(crate) fn write_xml_value<W: WriteExt>(
        &mut self,
        writer: &mut W,
        raw: &[u8],
        value_type: u8,
        size: u16,
        in_attribute: bool,
    ) -> Result<()> {
        macro_rules! read_le {
            ($ty:ty, $len:expr, $label:expr) => {{
                if raw.len() < $len {
                    return Err(EvtxError::FailedToCreateRecordModel($label));
                }
                <$ty>::from_le_bytes(raw[..$len].try_into().expect("checked slice length"))
            }};
        }

        match value_type {
            0x00 => Ok(()),
            0x01 => {
                let content_len = utf16le_content_length(raw);
                if content_len == 0 {
                    return Ok(());
                }
                utf16_simd::write_xml_utf16le(
                    writer,
                    &raw[..content_len],
                    content_len / 2,
                    in_attribute,
                )
                .map_err(EvtxError::from)
            }
            0x02 => self.write_ansi_xml_bytes(writer, raw, in_attribute),
            0x03 => self
                .formatter
                .write_i8(writer, read_le!(i8, 1, "Int8 value is shorter than 1 byte"))
                .map_err(EvtxError::from),
            0x04 => self
                .formatter
                .write_u8(
                    writer,
                    read_le!(u8, 1, "UInt8 value is shorter than 1 byte"),
                )
                .map_err(EvtxError::from),
            0x05 => self
                .formatter
                .write_i16(
                    writer,
                    read_le!(i16, 2, "Int16 value is shorter than 2 bytes"),
                )
                .map_err(EvtxError::from),
            0x06 => self
                .formatter
                .write_u16(
                    writer,
                    read_le!(u16, 2, "UInt16 value is shorter than 2 bytes"),
                )
                .map_err(EvtxError::from),
            0x07 => self
                .formatter
                .write_i32(
                    writer,
                    read_le!(i32, 4, "Int32 value is shorter than 4 bytes"),
                )
                .map_err(EvtxError::from),
            0x08 => self
                .formatter
                .write_u32(
                    writer,
                    read_le!(u32, 4, "UInt32 value is shorter than 4 bytes"),
                )
                .map_err(EvtxError::from),
            0x09 => self
                .formatter
                .write_i64(
                    writer,
                    read_le!(i64, 8, "Int64 value is shorter than 8 bytes"),
                )
                .map_err(EvtxError::from),
            0x0A => self
                .formatter
                .write_u64(
                    writer,
                    read_le!(u64, 8, "UInt64 value is shorter than 8 bytes"),
                )
                .map_err(EvtxError::from),
            0x0B => self.write_float(
                writer,
                read_le!(f32, 4, "Real32 value is shorter than 4 bytes"),
            ),
            0x0C => self.write_float(
                writer,
                read_le!(f64, 8, "Real64 value is shorter than 8 bytes"),
            ),
            0x0D => writer
                .write_all(
                    if read_le!(u32, 4, "Bool value is shorter than 4 bytes") != 0 {
                        b"true"
                    } else {
                        b"false"
                    },
                )
                .map_err(EvtxError::from),
            0x0E => xml_value_format::write_hex_bytes_upper(writer, raw),
            0x0F => xml_value_format::write_guid_le_bytes_upper(writer, raw),
            0x10 => match size {
                4 => xml_value_format::write_hex_prefixed_u32_lower(
                    writer,
                    read_le!(u32, 4, "SizeT value is shorter than 4 bytes"),
                ),
                8 => xml_value_format::write_hex_prefixed_u64_lower(
                    writer,
                    read_le!(u64, 8, "SizeT value is shorter than 8 bytes"),
                ),
                _ => Err(EvtxError::FailedToCreateRecordModel(
                    "unsupported SizeT width in raw XML renderer",
                )),
            },
            0x11 => xml_value_format::write_filetime_utc(
                writer,
                read_le!(u64, 8, "FILETIME value is shorter than 8 bytes"),
            ),
            0x12 => xml_value_format::write_systime_utc(writer, raw),
            0x13 => xml_value_format::write_sid(writer, raw),
            0x14 => xml_value_format::write_hex_prefixed_u32_lower(
                writer,
                read_le!(u32, 4, "HexInt32 value is shorter than 4 bytes"),
            ),
            0x15 => xml_value_format::write_hex_prefixed_u64_lower(
                writer,
                read_le!(u64, 8, "HexInt64 value is shorter than 8 bytes"),
            ),
            _ => Err(EvtxError::FailedToCreateRecordModel(
                "unsupported raw BinXML value in XML renderer",
            )),
        }
    }

    fn write_ansi_xml_bytes<W: WriteExt>(
        &mut self,
        writer: &mut W,
        raw: &[u8],
        in_attribute: bool,
    ) -> Result<()> {
        for &byte in raw {
            if byte == 0 {
                continue;
            }
            match byte {
                b'&' => writer.write_all(b"&amp;").map_err(EvtxError::from)?,
                b'<' => writer.write_all(b"&lt;").map_err(EvtxError::from)?,
                b'>' => writer.write_all(b"&gt;").map_err(EvtxError::from)?,
                b'"' if in_attribute => writer.write_all(b"&quot;").map_err(EvtxError::from)?,
                b'\'' if in_attribute => writer.write_all(b"&apos;").map_err(EvtxError::from)?,
                _ => writer.write_all(&[byte]).map_err(EvtxError::from)?,
            }
        }
        Ok(())
    }

    fn write_float<W: WriteExt, F: zmij::Float>(&mut self, writer: &mut W, value: F) -> Result<()> {
        let s = self.float_buf.format(value);
        writer.write_all(s.as_bytes()).map_err(EvtxError::from)
    }
}

fn fixed_type_item_size(base_type: u8) -> usize {
    match base_type {
        0x03 | 0x04 => 1,
        0x05 | 0x06 => 2,
        0x07 | 0x08 | 0x0B | 0x0D | 0x14 => 4,
        0x09 | 0x0A | 0x0C | 0x11 | 0x15 => 8,
        0x0F | 0x12 => 16,
        _ => 0,
    }
}

fn utf16le_content_length(raw: &[u8]) -> usize {
    for (idx, chunk) in raw.chunks_exact(2).enumerate() {
        if chunk[0] == 0 && chunk[1] == 0 {
            return idx * 2;
        }
    }
    raw.len() & !1
}
