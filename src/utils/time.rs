use byteorder::ReadBytesExt;
use chrono::prelude::*;
use std::io::Read;
use time::Duration;

#[derive(Debug, PartialEq, PartialOrd, Clone)]
pub struct FileTime {
    pub year: u32,
    pub month: u32,
    pub day_of_week: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
    pub milis: u32,
}

pub fn read_systemtime<R: Read>(mut r: R) -> Result<DateTime<Utc>, crate::error::Error> {
    let year = try_read!(r, u16);
    let month = try_read!(r, u16);
    let _day_of_week = try_read!(r, u16);
    let day = try_read!(r, u16);
    let hour = try_read!(r, u16);
    let minute = try_read!(r, u16);
    let second = try_read!(r, u16);
    let milliseconds = try_read!(r, u16);

    Ok(DateTime::from_utc(
        NaiveDate::from_ymd(year as i32, month as u32, day as u32).and_hms_nano(
            hour as u32,
            minute as u32,
            second as u32,
            milliseconds as u32,
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
