//! Shared XML value-format primitives used by both IR and compiled renderers.
//!
//! The helpers in this module avoid intermediate `String` allocations and write
//! directly to the output sink.

use crate::err::{EvtxError, Result};
use jiff::{Timestamp, tz::Offset};
use sonic_rs::writer::WriteExt;

const HEX_UPPER: &[u8; 16] = b"0123456789ABCDEF";

/// UTC datetime broken into numeric fields for direct XML formatting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UtcDateTimeParts {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub micros: u32,
}

impl UtcDateTimeParts {
    fn from_filetime(filetime: u64) -> Self {
        const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;
        const TICKS_PER_SEC: u64 = 10_000_000;

        let total_secs = (filetime / TICKS_PER_SEC) as i64;
        let unix_secs = total_secs - WINDOWS_TO_UNIX_SECS;
        let micros = ((filetime % TICKS_PER_SEC) / 10) as u32;

        let unix_days = unix_secs.div_euclid(86_400);
        let day_secs = unix_secs.rem_euclid(86_400) as u32;
        let (year, month, day) = civil_from_days(unix_days);

        UtcDateTimeParts {
            year,
            month,
            day,
            hour: day_secs / 3_600,
            minute: (day_secs % 3_600) / 60,
            second: day_secs % 60,
            micros,
        }
    }

    fn from_systime(raw: &[u8]) -> Result<Self> {
        if raw.len() < 16 {
            return Err(EvtxError::FailedToCreateRecordModel(
                "SYSTEMTIME value is shorter than 16 bytes",
            ));
        }

        let year = u16::from_le_bytes([raw[0], raw[1]]);
        let month = u16::from_le_bytes([raw[2], raw[3]]);
        let day = u16::from_le_bytes([raw[6], raw[7]]);
        let hour = u16::from_le_bytes([raw[8], raw[9]]);
        let minute = u16::from_le_bytes([raw[10], raw[11]]);
        let second = u16::from_le_bytes([raw[12], raw[13]]);
        let millis = u16::from_le_bytes([raw[14], raw[15]]);

        if year == 0
            && month == 0
            && day == 0
            && hour == 0
            && minute == 0
            && second == 0
            && millis == 0
        {
            return Ok(Self::from_filetime(0));
        }

        Ok(UtcDateTimeParts {
            year: i32::from(year),
            month: u32::from(month),
            day: u32::from(day),
            hour: u32::from(hour),
            minute: u32::from(minute),
            second: u32::from(second),
            micros: u32::from(millis) * 1_000,
        })
    }

    fn from_timestamp(timestamp: &Timestamp) -> Self {
        let dt = Offset::UTC.to_datetime(*timestamp);
        UtcDateTimeParts {
            year: dt.year() as i32,
            month: u32::from(dt.month() as u8),
            day: u32::from(dt.day() as u8),
            hour: u32::from(dt.hour() as u8),
            minute: u32::from(dt.minute() as u8),
            second: u32::from(dt.second() as u8),
            micros: (dt.subsec_nanosecond() / 1_000) as u32,
        }
    }
}

#[inline]
fn write_all<W: WriteExt>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    writer.write_all(bytes).map_err(EvtxError::from)
}

#[inline]
fn push_hex2(out: &mut [u8], pos: &mut usize, value: u8) {
    out[*pos] = HEX_UPPER[(value >> 4) as usize];
    out[*pos + 1] = HEX_UPPER[(value & 0x0F) as usize];
    *pos += 2;
}

#[inline]
fn hex_digit_lower(value: u8) -> u8 {
    if value < 10 {
        b'0' + value
    } else {
        b'a' + (value - 10)
    }
}

pub(crate) fn write_hex_bytes_upper<W: WriteExt>(writer: &mut W, bytes: &[u8]) -> Result<()> {
    let mut out = [0_u8; 512];
    let mut len = 0usize;

    for &b in bytes {
        if len + 2 > out.len() {
            write_all(writer, &out[..len])?;
            len = 0;
        }
        out[len] = HEX_UPPER[(b >> 4) as usize];
        out[len + 1] = HEX_UPPER[(b & 0x0F) as usize];
        len += 2;
    }

    if len > 0 {
        write_all(writer, &out[..len])?;
    }

    Ok(())
}

pub(crate) fn write_hex_prefixed_u32_lower<W: WriteExt>(writer: &mut W, value: u32) -> Result<()> {
    write_all(writer, b"0x")?;
    write_hex_u64_lower(writer, u64::from(value))
}

pub(crate) fn write_hex_prefixed_u64_lower<W: WriteExt>(writer: &mut W, value: u64) -> Result<()> {
    write_all(writer, b"0x")?;
    write_hex_u64_lower(writer, value)
}

fn write_hex_u64_lower<W: WriteExt>(writer: &mut W, mut value: u64) -> Result<()> {
    let mut tmp = [0_u8; 16];
    let mut len = 0usize;

    if value == 0 {
        tmp[0] = b'0';
        len = 1;
    } else {
        while value != 0 {
            tmp[len] = hex_digit_lower((value & 0x0F) as u8);
            len += 1;
            value >>= 4;
        }
        tmp[..len].reverse();
    }

    write_all(writer, &tmp[..len])
}

pub(crate) fn write_guid_le_bytes_upper<W: WriteExt>(writer: &mut W, raw: &[u8]) -> Result<()> {
    if raw.len() < 16 {
        return Err(EvtxError::FailedToCreateRecordModel(
            "GUID value is shorter than 16 bytes",
        ));
    }

    let mut out = [0_u8; 36];
    let mut pos = 0usize;

    let d1 = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
    push_hex2(&mut out, &mut pos, (d1 >> 24) as u8);
    push_hex2(&mut out, &mut pos, (d1 >> 16) as u8);
    push_hex2(&mut out, &mut pos, (d1 >> 8) as u8);
    push_hex2(&mut out, &mut pos, d1 as u8);
    out[pos] = b'-';
    pos += 1;

    let d2 = u16::from_le_bytes([raw[4], raw[5]]);
    push_hex2(&mut out, &mut pos, (d2 >> 8) as u8);
    push_hex2(&mut out, &mut pos, d2 as u8);
    out[pos] = b'-';
    pos += 1;

    let d3 = u16::from_le_bytes([raw[6], raw[7]]);
    push_hex2(&mut out, &mut pos, (d3 >> 8) as u8);
    push_hex2(&mut out, &mut pos, d3 as u8);
    out[pos] = b'-';
    pos += 1;

    push_hex2(&mut out, &mut pos, raw[8]);
    push_hex2(&mut out, &mut pos, raw[9]);
    out[pos] = b'-';
    pos += 1;
    push_hex2(&mut out, &mut pos, raw[10]);
    push_hex2(&mut out, &mut pos, raw[11]);
    push_hex2(&mut out, &mut pos, raw[12]);
    push_hex2(&mut out, &mut pos, raw[13]);
    push_hex2(&mut out, &mut pos, raw[14]);
    push_hex2(&mut out, &mut pos, raw[15]);

    write_all(writer, &out[..pos])
}

pub(crate) fn write_filetime_utc<W: WriteExt>(writer: &mut W, filetime: u64) -> Result<()> {
    write_utc_datetime(writer, UtcDateTimeParts::from_filetime(filetime))
}

pub(crate) fn write_systime_utc<W: WriteExt>(writer: &mut W, raw: &[u8]) -> Result<()> {
    write_utc_datetime(writer, UtcDateTimeParts::from_systime(raw)?)
}

pub(crate) fn write_timestamp_utc<W: WriteExt>(
    writer: &mut W,
    timestamp: &Timestamp,
) -> Result<()> {
    write_utc_datetime(writer, UtcDateTimeParts::from_timestamp(timestamp))
}

pub(crate) fn write_utc_datetime<W: WriteExt>(
    writer: &mut W,
    parts: UtcDateTimeParts,
) -> Result<()> {
    let y = parts.year as u32;
    let mut out = [0_u8; 27];
    out[0] = b'0' + ((y / 1_000) % 10) as u8;
    out[1] = b'0' + ((y / 100) % 10) as u8;
    out[2] = b'0' + ((y / 10) % 10) as u8;
    out[3] = b'0' + (y % 10) as u8;
    out[4] = b'-';
    out[5] = b'0' + ((parts.month / 10) % 10) as u8;
    out[6] = b'0' + (parts.month % 10) as u8;
    out[7] = b'-';
    out[8] = b'0' + ((parts.day / 10) % 10) as u8;
    out[9] = b'0' + (parts.day % 10) as u8;
    out[10] = b'T';
    out[11] = b'0' + ((parts.hour / 10) % 10) as u8;
    out[12] = b'0' + (parts.hour % 10) as u8;
    out[13] = b':';
    out[14] = b'0' + ((parts.minute / 10) % 10) as u8;
    out[15] = b'0' + (parts.minute % 10) as u8;
    out[16] = b':';
    out[17] = b'0' + ((parts.second / 10) % 10) as u8;
    out[18] = b'0' + (parts.second % 10) as u8;
    out[19] = b'.';
    out[20] = b'0' + ((parts.micros / 100_000) % 10) as u8;
    out[21] = b'0' + ((parts.micros / 10_000) % 10) as u8;
    out[22] = b'0' + ((parts.micros / 1_000) % 10) as u8;
    out[23] = b'0' + ((parts.micros / 100) % 10) as u8;
    out[24] = b'0' + ((parts.micros / 10) % 10) as u8;
    out[25] = b'0' + (parts.micros % 10) as u8;
    out[26] = b'Z';
    write_all(writer, &out)
}

pub(crate) fn write_sid<W: WriteExt>(writer: &mut W, raw: &[u8]) -> Result<()> {
    if raw.len() < 8 {
        return write_all(writer, b"S-?");
    }

    let revision = raw[0];
    let sub_count = raw[1] as usize;

    let mut authority: u64 = 0;
    for &b in &raw[2..8] {
        authority = (authority << 8) | u64::from(b);
    }

    write_all(writer, b"S-")?;
    write_u64_decimal(writer, u64::from(revision))?;
    write_all(writer, b"-")?;
    write_u64_decimal(writer, authority)?;

    let mut offset = 8usize;
    for _ in 0..sub_count {
        if offset + 4 > raw.len() {
            break;
        }
        let sub = u32::from_le_bytes([
            raw[offset],
            raw[offset + 1],
            raw[offset + 2],
            raw[offset + 3],
        ]);
        write_all(writer, b"-")?;
        write_u64_decimal(writer, u64::from(sub))?;
        offset += 4;
    }

    Ok(())
}

pub(crate) fn write_u64_decimal<W: WriteExt>(writer: &mut W, value: u64) -> Result<()> {
    if value == 0 {
        return write_all(writer, b"0");
    }

    let mut tmp = [0_u8; 20];
    let mut len = 0usize;
    let mut n = value;
    while n > 0 {
        tmp[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
    }
    tmp[..len].reverse();
    write_all(writer, &tmp[..len])
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m, d)
}
