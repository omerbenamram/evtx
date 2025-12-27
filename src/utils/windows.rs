use std::io;
use std::io::Cursor;

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use winstructs::security::Sid;

use crate::err::{DeserializationError, DeserializationResult};
use crate::utils::ByteCursor;

#[inline]
pub(crate) fn filetime_to_datetime(filetime: u64) -> DateTime<Utc> {
    // Match historical behavior (`winstructs::timestamp::WinTimestamp::to_datetime`).
    let naive = NaiveDate::from_ymd_opt(1601, 1, 1)
        .and_then(|x| x.and_hms_nano_opt(0, 0, 0, 0))
        .expect("filetime epoch should be valid")
        + Duration::microseconds((filetime / 10) as i64);
    Utc.from_utc_datetime(&naive)
}

pub(crate) fn read_systime(cursor: &mut ByteCursor<'_>) -> DeserializationResult<DateTime<Utc>> {
    let bytes = cursor.array::<16>("systime")?;
    systime_from_bytes(&bytes)
}

pub(crate) fn systime_from_bytes(bytes: &[u8; 16]) -> DeserializationResult<DateTime<Utc>> {
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
        return Ok(Utc.from_utc_datetime(
            &NaiveDate::from_ymd_opt(1601, 1, 1)
                .expect("Always valid")
                .and_hms_nano_opt(0, 0, 0, 0)
                .expect("Always valid"),
        ));
    }

    Ok(Utc.from_utc_datetime(
        &NaiveDate::from_ymd_opt(year, month, day)
            .ok_or_else(|| DeserializationError::InvalidDateTimeError)?
            .and_hms_nano_opt(hour, minute, second, milliseconds * 1_000_000) // Convert milliseconds to nanoseconds
            .ok_or_else(|| DeserializationError::InvalidDateTimeError)?,
    ))
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
    let sid = Sid::from_reader(&mut c)
        .map_err(|e| DeserializationError::Io(io::Error::new(io::ErrorKind::InvalidData, e)))?;
    cursor.advance(c.position() as usize, "sid")?;
    Ok(sid)
}
