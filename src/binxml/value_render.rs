//! Shared value-to-text rendering for BinXML values.
//!
//! The JSON and XML IR renderers both need to serialize `BinXmlValue` payloads
//! into their textual representation (numbers, GUIDs, timestamps, hex bytes,
//! comma-delimited arrays, etc.). This module centralizes that logic so both
//! renderers stay in sync and avoid allocating intermediate `String`s.

use crate::binxml::value_variant::BinXmlValue;
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
            BinXmlValue::AnsiStringType(s) => self.write_str_escaped(writer, *s, string_mode),
            BinXmlValue::Int8Type(v) => self
                .formatter
                .write_i8(writer, *v)
                .map_err(EvtxError::from),
            BinXmlValue::UInt8Type(v) => self
                .formatter
                .write_u8(writer, *v)
                .map_err(EvtxError::from),
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
            BinXmlValue::BoolType(v) => self.write_bytes(writer, if *v { b"true" } else { b"false" }),
            BinXmlValue::BinaryType(bytes) => self.write_hex_bytes_upper(writer, bytes),
            BinXmlValue::GuidType(guid) => {
                write!(writer, "{}", guid).map_err(EvtxError::from)
            }
            BinXmlValue::SizeTType(v) => self
                .formatter
                .write_u64(writer, *v as u64)
                .map_err(EvtxError::from),
            BinXmlValue::FileTimeType(tm) | BinXmlValue::SysTimeType(tm) => {
                self.write_datetime(writer, tm)
            }
            BinXmlValue::SidType(sid) => write!(writer, "{}", sid).map_err(EvtxError::from),
            BinXmlValue::HexInt32Type(v) => self.write_hex_prefixed_u32_lower(writer, *v),
            BinXmlValue::HexInt64Type(v) => self.write_hex_prefixed_u64_lower(writer, *v),
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
            StringEscapeMode::Json => utf16_simd::write_json_utf16le(writer, bytes, units, false)
                .map_err(EvtxError::from),
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
            StringEscapeMode::Xml { in_attribute } => self.write_xml_escaped_str(writer, value, in_attribute),
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
        for &b in bytes {
            let hi = (b >> 4) & 0x0f;
            let lo = b & 0x0f;
            self.write_byte(writer, to_hex_digit(hi))?;
            self.write_byte(writer, to_hex_digit(lo))?;
        }
        Ok(())
    }

    fn write_hex_prefixed_u32_lower<W: WriteExt>(&mut self, writer: &mut W, value: u32) -> Result<()> {
        self.write_bytes(writer, b"0x")?;
        self.write_hex_u32_lower(writer, value)
    }

    fn write_hex_prefixed_u64_lower<W: WriteExt>(&mut self, writer: &mut W, value: u64) -> Result<()> {
        self.write_bytes(writer, b"0x")?;
        self.write_hex_u64_lower(writer, value)
    }

    fn write_hex_u32_lower<W: WriteExt>(&mut self, writer: &mut W, mut value: u32) -> Result<()> {
        let mut buf = [0_u8; 8];
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
            // Reverse in-place.
            for i in 0..(len / 2) {
                buf.swap(i, len - 1 - i);
            }
        }
        writer.write_all(&buf[..len]).map_err(EvtxError::from)
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
            for i in 0..(len / 2) {
                buf.swap(i, len - 1 - i);
            }
        }
        writer.write_all(&buf[..len]).map_err(EvtxError::from)
    }

    fn write_hex_list_u32<W: WriteExt>(&mut self, writer: &mut W, items: &[u32]) -> Result<()> {
        let mut first = true;
        for &item in items {
            if !first {
                self.write_byte(writer, b',')?;
            }
            first = false;
            self.write_hex_prefixed_u32_lower(writer, item)?;
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
            self.write_hex_prefixed_u64_lower(writer, item)?;
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
        let dt = Offset::UTC.to_datetime(*tm);
        let year = dt.year() as i32;

        self.write_4_digits(writer, year as u32)?;
        self.write_byte(writer, b'-')?;
        self.write_2_digits(writer, u32::from(dt.month() as u8))?;
        self.write_byte(writer, b'-')?;
        self.write_2_digits(writer, u32::from(dt.day() as u8))?;
        self.write_byte(writer, b'T')?;
        self.write_2_digits(writer, u32::from(dt.hour() as u8))?;
        self.write_byte(writer, b':')?;
        self.write_2_digits(writer, u32::from(dt.minute() as u8))?;
        self.write_byte(writer, b':')?;
        self.write_2_digits(writer, u32::from(dt.second() as u8))?;
        self.write_byte(writer, b'.')?;
        let micros = (dt.subsec_nanosecond() / 1_000) as u32;
        self.write_6_digits(writer, micros)?;
        self.write_byte(writer, b'Z')?;
        Ok(())
    }

    fn write_datetime_list<W: WriteExt>(&mut self, writer: &mut W, items: &[Timestamp]) -> Result<()> {
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

    fn write_2_digits<W: WriteExt>(&mut self, writer: &mut W, value: u32) -> Result<()> {
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(writer, b'0' + tens as u8)?;
        self.write_byte(writer, b'0' + ones as u8)?;
        Ok(())
    }

    fn write_4_digits<W: WriteExt>(&mut self, writer: &mut W, value: u32) -> Result<()> {
        let thousands = (value / 1000) % 10;
        let hundreds = (value / 100) % 10;
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(writer, b'0' + thousands as u8)?;
        self.write_byte(writer, b'0' + hundreds as u8)?;
        self.write_byte(writer, b'0' + tens as u8)?;
        self.write_byte(writer, b'0' + ones as u8)?;
        Ok(())
    }

    fn write_6_digits<W: WriteExt>(&mut self, writer: &mut W, value: u32) -> Result<()> {
        let hundred_thousands = (value / 100000) % 10;
        let ten_thousands = (value / 10000) % 10;
        let thousands = (value / 1000) % 10;
        let hundreds = (value / 100) % 10;
        let tens = (value / 10) % 10;
        let ones = value % 10;
        self.write_byte(writer, b'0' + hundred_thousands as u8)?;
        self.write_byte(writer, b'0' + ten_thousands as u8)?;
        self.write_byte(writer, b'0' + thousands as u8)?;
        self.write_byte(writer, b'0' + hundreds as u8)?;
        self.write_byte(writer, b'0' + tens as u8)?;
        self.write_byte(writer, b'0' + ones as u8)?;
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

