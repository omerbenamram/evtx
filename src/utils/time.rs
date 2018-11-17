use chrono::prelude::*;
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

pub fn datetime_from_filetime(nanos_since_windows_epoch: u64) -> DateTime<Utc> {
    DateTime::from_utc(
        NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0)
            + Duration::microseconds((nanos_since_windows_epoch / 10) as i64),
        Utc,
    )
}
