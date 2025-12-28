use std::io;
use std::io::Cursor;

use jiff::{Timestamp, civil::DateTime, tz::Offset};
use winstructs::security::Sid;

use crate::err::{DeserializationError, DeserializationResult};
use crate::utils::ByteCursor;

const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;

#[inline]
pub(crate) fn filetime_to_timestamp(filetime: u64) -> DeserializationResult<Timestamp> {
    let secs = (filetime / 10_000_000) as i64 - WINDOWS_TO_UNIX_SECS;
    let nanos = ((filetime % 10_000_000) * 100) as i32;
    Timestamp::new(secs, nanos).map_err(|_| DeserializationError::InvalidDateTimeError)
}

pub(crate) fn read_systime(cursor: &mut ByteCursor<'_>) -> DeserializationResult<Timestamp> {
    let bytes = cursor.array::<16>("systime")?;
    systime_from_bytes(&bytes)
}

pub(crate) fn systime_from_bytes(bytes: &[u8; 16]) -> DeserializationResult<Timestamp> {
    let year = i32::from(u16::from_le_bytes([bytes[0], bytes[1]]));
    let month = u32::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    let _day_of_week = u16::from_le_bytes([bytes[4], bytes[5]]);
    let day = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
    let hour = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
    let minute = u32::from(u16::from_le_bytes([bytes[10], bytes[11]]));
    let second = u32::from(u16::from_le_bytes([bytes[12], bytes[13]]));
    let milliseconds = u32::from(u16::from_le_bytes([bytes[14], bytes[15]]));

    // The entire value is unset. By convention, use the "1601-01-01T00:00:00.0000000Z" timestamp.
    if year == 0
        && month == 0
        && day == 0
        && hour == 0
        && minute == 0
        && second == 0
        && milliseconds == 0
    {
        return filetime_to_timestamp(0);
    }

    let year = i16::try_from(year).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let month = i8::try_from(month).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let day = i8::try_from(day).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let hour = i8::try_from(hour).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let minute = i8::try_from(minute).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let second = i8::try_from(second).map_err(|_| DeserializationError::InvalidDateTimeError)?;
    let nanos = i32::try_from(milliseconds * 1_000_000)
        .map_err(|_| DeserializationError::InvalidDateTimeError)?;

    let dt = DateTime::new(year, month, day, hour, minute, second, nanos)
        .map_err(|_| DeserializationError::InvalidDateTimeError)?;
    Offset::UTC
        .to_timestamp(dt)
        .map_err(|_| DeserializationError::InvalidDateTimeError)
}

pub(crate) fn read_sid(cursor: &mut ByteCursor<'_>) -> DeserializationResult<Sid> {
    let start = cursor.pos();
    let remaining = cursor
        .buf()
        .get(start..)
        .ok_or_else(|| DeserializationError::Truncated {
            what: "sid",
            offset: start as u64,
            need: 1,
            have: 0,
        })?;

    let mut c = Cursor::new(remaining);
    let sid = Sid::from_reader(&mut c).map_err(|e| {
        DeserializationError::Io(io::Error::new(io::ErrorKind::InvalidData, e))
    })?;
    cursor.advance(c.position() as usize, "sid")?;
    Ok(sid)
}
