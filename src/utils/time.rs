use crate::err::DeserializationResult;

use crate::evtx_parser::ReadSeek;
use byteorder::ReadBytesExt;
use chrono::prelude::*;

pub fn read_systemtime<R: ReadSeek>(r: &mut R) -> DeserializationResult<DateTime<Utc>> {
    let year = try_read!(r, u16)?;
    let month = try_read!(r, u16)?;
    let _day_of_week = try_read!(r, u16)?;
    let day = try_read!(r, u16)?;
    let hour = try_read!(r, u16)?;
    let minute = try_read!(r, u16)?;
    let second = try_read!(r, u16)?;
    let milliseconds = try_read!(r, u16)?;

    Ok(DateTime::from_utc(
        NaiveDate::from_ymd(i32::from(year), u32::from(month), u32::from(day)).and_hms_nano(
            u32::from(hour),
            u32::from(minute),
            u32::from(second),
            u32::from(milliseconds),
        ),
        Utc,
    ))
}
