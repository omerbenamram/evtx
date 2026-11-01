use jiff::{Timestamp, civil::DateTime, tz::Offset};

use crate::err::{DeserializationError, DeserializationResult};

const WINDOWS_TO_UNIX_SECS: i64 = 11_644_473_600;
/// FILETIME ticks (100 ns) in a second and in a day.
const TICKS_PER_SEC: u64 = 10_000_000;
const TICKS_PER_DAY: u64 = 86_400 * TICKS_PER_SEC;

#[inline]
pub(crate) fn filetime_to_timestamp(filetime: u64) -> DeserializationResult<Timestamp> {
    let secs = (filetime / TICKS_PER_SEC) as i64 - WINDOWS_TO_UNIX_SECS;
    let nanos = ((filetime % TICKS_PER_SEC) * 100) as i32;
    Timestamp::new(secs, nanos).map_err(|_| DeserializationError::InvalidDateTimeError)
}

/// A FILETIME as stored: the 100 ns intervals since 1601-01-01 UTC.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileTime(pub u64);

impl FileTime {
    /// The instant it counts to, if jiff can represent it: until 9999-12-30T22:00 UTC.
    pub fn to_timestamp(self) -> Option<Timestamp> {
        filetime_to_timestamp(self.0).ok()
    }

    /// The date and time Windows renders for it. Years past 9999 keep their last four digits,
    /// and a value past `i64::MAX` renders as zeros.
    pub(crate) fn fields(self) -> DateTimeFields {
        if self.0 > i64::MAX as u64 {
            return DateTimeFields::default();
        }
        let (days, ticks) = (self.0 / TICKS_PER_DAY, self.0 % TICKS_PER_DAY);
        let (year, month, day) = civil_from_days_since_1601(days);
        let secs = ticks / TICKS_PER_SEC;
        DateTimeFields {
            year: (year % 10_000) as u16,
            month,
            day,
            hour: (secs / 3_600) as u8,
            minute: (secs / 60 % 60) as u8,
            second: (secs % 60) as u8,
            ticks: (ticks % TICKS_PER_SEC) as u32,
        }
    }
}

/// The proleptic Gregorian date `days` after 1601-01-01, by Howard Hinnant's `civil_from_days`.
fn civil_from_days_since_1601(days: u64) -> (u64, u8, u8) {
    // The algorithm counts days from 0000-03-01, 584,694 days before 1601-01-01, in eras of
    // 400 years (146,097 days), each starting on a March 1st.
    let z = days + 584_694;
    let (era, doe) = (z / 146_097, z % 146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u8;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u8;
    (era * 400 + yoe + u64::from(month <= 2), month, day)
}

/// A SYSTEMTIME as stored: eight 16-bit fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysTime {
    pub year: u16,
    pub month: u16,
    pub day_of_week: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub milliseconds: u16,
}

impl SysTime {
    pub fn from_bytes(bytes: &[u8; 16]) -> Self {
        let field = |i: usize| u16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]]);
        SysTime {
            year: field(0),
            month: field(1),
            day_of_week: field(2),
            day: field(3),
            hour: field(4),
            minute: field(5),
            second: field(6),
            milliseconds: field(7),
        }
    }

    /// Whether Windows takes it as a time: a year from 1601 to 30827 and every other field in
    /// its range. The day of the week isn't checked, as Windows ignores it.
    pub fn is_valid(&self) -> bool {
        (1601..=30827).contains(&self.year)
            && (1..=12).contains(&self.month)
            && (1..=days_in_month(self.year, self.month)).contains(&self.day)
            && self.hour < 24
            && self.minute < 60
            && self.second < 60
            && self.milliseconds < 1_000
    }

    /// The instant it stands for, if it is valid and jiff can represent it (years up to 9999).
    pub fn to_timestamp(&self) -> Option<Timestamp> {
        if !self.is_valid() {
            return None;
        }
        let narrow = |v: u16| i8::try_from(v).ok();
        let dt = DateTime::new(
            i16::try_from(self.year).ok()?,
            narrow(self.month)?,
            narrow(self.day)?,
            narrow(self.hour)?,
            narrow(self.minute)?,
            narrow(self.second)?,
            i32::from(self.milliseconds) * 1_000_000,
        )
        .ok()?;
        Offset::UTC.to_timestamp(dt).ok()
    }

    /// The date and time Windows renders for it. Years past 9999 keep their last four digits,
    /// and a SYSTEMTIME that isn't valid, an unset one included, renders as 1601-01-01.
    pub(crate) fn fields(&self) -> DateTimeFields {
        if !self.is_valid() {
            return DateTimeFields {
                year: 1601,
                month: 1,
                day: 1,
                ..DateTimeFields::default()
            };
        }
        DateTimeFields {
            year: self.year % 10_000,
            month: self.month as u8,
            day: self.day as u8,
            hour: self.hour as u8,
            minute: self.minute as u8,
            second: self.second as u8,
            ticks: u32::from(self.milliseconds) * 10_000,
        }
    }
}

fn days_in_month(year: u16, month: u16) -> u16 {
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A date and time of day as rendered, with the 100 ns ticks of the second.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DateTimeFields {
    pub(crate) year: u16,
    pub(crate) month: u8,
    pub(crate) day: u8,
    pub(crate) hour: u8,
    pub(crate) minute: u8,
    pub(crate) second: u8,
    pub(crate) ticks: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields_of(ts: Timestamp) -> DateTimeFields {
        let dt = Offset::UTC.to_datetime(ts);
        DateTimeFields {
            year: dt.year() as u16,
            month: dt.month() as u8,
            day: dt.day() as u8,
            hour: dt.hour() as u8,
            minute: dt.minute() as u8,
            second: dt.second() as u8,
            ticks: (dt.subsec_nanosecond() / 100) as u32,
        }
    }

    #[test]
    fn filetime_fields_agree_with_jiff() {
        let last: u64 = 2_650_466_807_999_999_999; // 9999-12-30T21:59:59.9999999Z, near jiff's limit
        let step: u64 = 1_000_000_000_000_037; // about 3.2 years, prime, so every part of a date moves
        let leap_days = [
            DateTime::constant(1904, 2, 29, 23, 59, 59, 999_999_900),
            DateTime::constant(2000, 2, 29, 0, 0, 0, 0),
            DateTime::constant(2000, 3, 1, 0, 0, 0, 0),
            DateTime::constant(2100, 2, 28, 12, 0, 0, 0),
            DateTime::constant(2100, 3, 1, 0, 0, 0, 100),
        ]
        .map(|dt| {
            let ts = Offset::UTC.to_timestamp(dt).unwrap();
            (ts.as_nanosecond() / 100 + i128::from(WINDOWS_TO_UNIX_SECS) * 10_000_000) as u64
        });
        let filetimes = (0..last)
            .step_by(step as usize)
            .chain([0, 1, last])
            .chain(leap_days);
        for ft in filetimes {
            let ts = FileTime(ft).to_timestamp().unwrap();
            assert_eq!(FileTime(ft).fields(), fields_of(ts), "FILETIME {ft:#x}");
        }
    }

    #[test]
    fn systime_validity_follows_windows() {
        let st = |f: [u16; 8]| {
            SysTime::from_bytes(&f.map(u16::to_le_bytes).concat().try_into().unwrap())
        };
        assert!(st([2019, 3, 5, 1, 12, 34, 56, 789]).is_valid());
        assert!(st([2020, 2, 0, 29, 0, 0, 0, 0]).is_valid());
        assert!(st([30827, 12, 0, 31, 23, 59, 59, 999]).is_valid());
        for invalid in [
            [0, 0, 0, 0, 0, 0, 0, 0],
            [1600, 12, 0, 31, 0, 0, 0, 0],
            [30828, 1, 0, 1, 0, 0, 0, 0],
            [2019, 13, 0, 1, 0, 0, 0, 0],
            [2019, 2, 0, 29, 0, 0, 0, 0],
            [2019, 3, 0, 0, 0, 0, 0, 0],
            [2019, 3, 0, 1, 24, 0, 0, 0],
            [2019, 3, 0, 1, 0, 60, 0, 0],
            [2019, 3, 0, 1, 0, 0, 60, 0],
            [2019, 3, 0, 1, 0, 0, 0, 1000],
        ] {
            assert!(!st(invalid).is_valid(), "{invalid:?}");
            assert_eq!(st(invalid).to_timestamp(), None);
        }
        let ts = st([2019, 3, 5, 1, 12, 34, 56, 789]).to_timestamp().unwrap();
        assert_eq!(ts.to_string(), "2019-03-01T12:34:56.789Z");
    }
}
