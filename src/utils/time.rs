use crate::err::{DeserializationError, DeserializationResult};

use crate::evtx_parser::ReadSeek;
use byteorder::ReadBytesExt;
use chrono::prelude::*;

pub fn read_systemtime<R: ReadSeek>(r: &mut R) -> DeserializationResult<DateTime<Utc>> {
    let year = i32::from(try_read!(r, u16)?);
    let month = u32::from(try_read!(r, u16)?);
    let _day_of_week = try_read!(r, u16)?;
    let day = u32::from(try_read!(r, u16)?);
    let hour = u32::from(try_read!(r, u16)?);
    let minute = u32::from(try_read!(r, u16)?);
    let second = u32::from(try_read!(r, u16)?);
    let milliseconds = u32::from(try_read!(r, u16)?);

    // The entire value is unset. By convention, use the "1601-01-01T00:00:00.0000000Z" timestamp.
    if year == 0
        && month == 0
        && day == 0
        && hour == 0
        && minute == 0
        && second == 0
        && milliseconds == 0
    {
        return Ok(DateTime::from_utc(
            NaiveDate::from_ymd(1601, 1, 1).and_hms_nano(0, 0, 0, 0),
            Utc,
        ));
    }

    Ok(DateTime::from_utc(
        NaiveDate::from_ymd_opt(year, month, day)
            .ok_or(DeserializationError::InvalidDateTimeError)?
            .and_hms_nano_opt(hour, minute, second, milliseconds)
            .ok_or(DeserializationError::InvalidDateTimeError)?,
        Utc,
    ))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use chrono::{DateTime, Datelike, NaiveDate, Utc};

    use super::read_systemtime;

    #[test]
    fn test_date_regular() {
        let data = [227u8, 7, 3, 0, 5, 0, 8, 0, 23, 0, 22, 0, 5, 0, 0, 0];

        let date = read_systemtime(&mut Cursor::new(data)).unwrap();
        let expected_date = DateTime::<Utc>::from_utc(
            NaiveDate::from_ymd(2019, 3, 8).and_hms_nano(23, 22, 05, 0),
            Utc,
        );
        assert_eq!(date, expected_date);
    }

    #[test]
    fn test_date_invalid_month() {
        // No such month.
        let data = [227u8, 7, 255, 0, 5, 0, 8, 0, 23, 0, 22, 0, 5, 0, 0, 0];
        let date_res = read_systemtime(&mut Cursor::new(data));
        assert!(date_res.is_err());
    }

    #[test]
    fn test_date_invalid_time() {
        // No such hour 255.
        let data = [227u8, 7, 3, 0, 5, 0, 8, 0, 255, 0, 22, 0, 5, 0, 0, 0];
        let date_res = read_systemtime(&mut Cursor::new(data));
        assert!(date_res.is_err());
    }

    #[test]
    fn test_date_zero() {
        let data = [0u8; 16];
        let date = read_systemtime(&mut Cursor::new(data)).unwrap();
        assert_eq!(date.year_ce(), (true, 1601));
    }
}
