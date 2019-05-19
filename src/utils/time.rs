use crate::err::{self, Result};
use snafu::ResultExt;

use crate::evtx_parser::ReadSeek;
use byteorder::ReadBytesExt;
use chrono::prelude::*;

use time::Duration;

pub fn read_systemtime<R: ReadSeek>(r: &mut R) -> Result<DateTime<Utc>> {
    let year = try_read!(r, u16);
    let month = try_read!(r, u16);
    let _day_of_week = try_read!(r, u16);
    let day = try_read!(r, u16);
    let hour = try_read!(r, u16);
    let minute = try_read!(r, u16);
    let second = try_read!(r, u16);
    let milliseconds = try_read!(r, u16);

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

pub fn datetime_from_filetime(nanos_since_windows_epoch: u64) -> DateTime<Utc> {
    DateTime::from_utc(
        NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0)
            + Duration::microseconds((nanos_since_windows_epoch / 10) as i64),
        Utc,
    )
}
