use std::borrow::Cow;
use std::io::SeekFrom;

use byteorder::{LittleEndian, ReadBytesExt};
use winstructs::guid::Guid;
use winstructs::security::Sid;

use crate::err::{DeserializationError, DeserializationResult, WrappedIoError};
use crate::evtx_parser::ReadSeek;

pub(crate) trait ReadExt: ReadSeek + Sized {
    #[inline]
    fn try_seek_abs_named(
        &mut self,
        offset: u64,
        _name: &'static str,
    ) -> DeserializationResult<u64> {
        // Note: legacy `try_seek!` dropped the provided name. Keep behavior stable for now.
        match self.seek(SeekFrom::Start(offset)) {
            Ok(v) => Ok(v),
            Err(e) => {
                let inner = WrappedIoError::capture_hexdump(Box::new(e), self);
                Err(DeserializationError::from(inner))
            }
        }
    }

    #[inline]
    fn try_u8(&mut self) -> DeserializationResult<u8> {
        self.try_u8_named("<Unknown>")
    }
    #[inline]
    fn try_u8_named(&mut self, name: &'static str) -> DeserializationResult<u8> {
        self.read_u8()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "u8".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_i8(&mut self) -> DeserializationResult<i8> {
        self.try_i8_named("<Unknown>")
    }
    #[inline]
    fn try_i8_named(&mut self, name: &'static str) -> DeserializationResult<i8> {
        self.read_i8()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "i8".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_u16(&mut self) -> DeserializationResult<u16> {
        self.try_u16_named("<Unknown>")
    }
    #[inline]
    fn try_u16_named(&mut self, name: &'static str) -> DeserializationResult<u16> {
        self.read_u16::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "u16".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_i16(&mut self) -> DeserializationResult<i16> {
        self.try_i16_named("<Unknown>")
    }
    #[inline]
    fn try_i16_named(&mut self, name: &'static str) -> DeserializationResult<i16> {
        self.read_i16::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "i16".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_i32(&mut self) -> DeserializationResult<i32> {
        self.try_i32_named("<Unknown>")
    }
    #[inline]
    fn try_i32_named(&mut self, name: &'static str) -> DeserializationResult<i32> {
        self.read_i32::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "i32".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_u32(&mut self) -> DeserializationResult<u32> {
        self.try_u32_named("<Unknown>")
    }
    #[inline]
    fn try_u32_named(&mut self, name: &'static str) -> DeserializationResult<u32> {
        self.read_u32::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "u32".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_f32(&mut self) -> DeserializationResult<f32> {
        self.try_f32_named("<Unknown>")
    }
    #[inline]
    fn try_f32_named(&mut self, name: &'static str) -> DeserializationResult<f32> {
        self.read_f32::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "f32".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_i64(&mut self) -> DeserializationResult<i64> {
        self.try_i64_named("<Unknown>")
    }
    #[inline]
    fn try_i64_named(&mut self, name: &'static str) -> DeserializationResult<i64> {
        self.read_i64::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "i64".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_u64(&mut self) -> DeserializationResult<u64> {
        self.try_u64_named("<Unknown>")
    }
    #[inline]
    fn try_u64_named(&mut self, name: &'static str) -> DeserializationResult<u64> {
        self.read_u64::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "u64".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_f64(&mut self) -> DeserializationResult<f64> {
        self.try_f64_named("<Unknown>")
    }
    #[inline]
    fn try_f64_named(&mut self, name: &'static str) -> DeserializationResult<f64> {
        self.read_f64::<LittleEndian>()
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "f64".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
    }

    #[inline]
    fn try_bool(&mut self) -> DeserializationResult<bool> {
        // Match the legacy macro behavior: read an i32 and coerce {0,1} to false/true, any other
        // value logs a warning and becomes true. Read errors bubble up unchanged (as i32 errors).
        let bool_value = self.try_i32();
        match bool_value {
            Ok(0) => Ok(false),
            Ok(1) => Ok(true),
            Ok(number) => {
                log::warn!(
                    "{:} is an unknown value for bool, coercing to `true`",
                    number
                );
                Ok(true)
            }
            Err(e) => Err(e),
        }
    }

    #[inline]
    fn try_guid(&mut self) -> DeserializationResult<Guid> {
        self.try_guid_named("<Unknown>")
    }
    #[inline]
    fn try_guid_named(&mut self, name: &'static str) -> DeserializationResult<Guid> {
        Guid::from_reader(self).map_err(|e| DeserializationError::FailedToReadToken {
            t: "guid".to_owned(),
            token_name: name,
            source: WrappedIoError::capture_hexdump(Box::new(e), self),
        })
    }

    #[inline]
    fn try_len_prefixed_utf16_string_named(
        &mut self,
        name: &'static str,
    ) -> DeserializationResult<Option<String>> {
        crate::utils::read_len_prefixed_utf16_string(self, false).map_err(|e| {
            DeserializationError::FailedToReadToken {
                t: "len_prefixed_utf_16_str".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            }
        })
    }

    #[inline]
    fn try_len_prefixed_utf16_string_nul_terminated_named(
        &mut self,
        name: &'static str,
    ) -> DeserializationResult<Option<String>> {
        crate::utils::read_len_prefixed_utf16_string(self, true).map_err(|e| {
            DeserializationError::FailedToReadToken {
                t: "len_prefixed_utf_16_str_nul_terminated".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            }
        })
    }

    #[inline]
    fn try_null_terminated_utf16_string(&mut self) -> DeserializationResult<String> {
        self.try_null_terminated_utf16_string_named("<Unknown>")
    }
    #[inline]
    fn try_null_terminated_utf16_string_named(
        &mut self,
        name: &'static str,
    ) -> DeserializationResult<String> {
        crate::utils::read_null_terminated_utf16_string(self).map_err(|e| {
            DeserializationError::FailedToReadToken {
                t: "null_terminated_utf_16_str".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            }
        })
    }

    #[inline]
    fn try_sid(&mut self) -> DeserializationResult<Sid> {
        self.try_sid_named("<Unknown>")
    }
    #[inline]
    fn try_sid_named(&mut self, name: &'static str) -> DeserializationResult<Sid> {
        Sid::from_reader(self).map_err(|e| DeserializationError::FailedToReadToken {
            t: "ntsid".to_owned(),
            token_name: name,
            source: WrappedIoError::capture_hexdump(Box::new(e), self),
        })
    }

    #[inline]
    fn try_hex32<'a>(&mut self) -> DeserializationResult<Cow<'a, str>> {
        self.try_hex32_named("<Unknown>")
    }
    #[inline]
    fn try_hex32_named<'a>(&mut self, _name: &'static str) -> DeserializationResult<Cow<'a, str>> {
        // Macro version used i32 reads and then formatted; preserve that.
        self.try_i32()
            .map(|value| Cow::Owned(format!("0x{:x}", value)))
    }

    #[inline]
    fn try_hex64<'a>(&mut self) -> DeserializationResult<Cow<'a, str>> {
        self.try_hex64_named("<Unknown>")
    }
    #[inline]
    fn try_hex64_named<'a>(&mut self, _name: &'static str) -> DeserializationResult<Cow<'a, str>> {
        self.try_i64()
            .map(|value| Cow::Owned(format!("0x{:x}", value)))
    }

    #[inline]
    fn try_filetime(&mut self) -> DeserializationResult<chrono::DateTime<chrono::Utc>> {
        self.try_filetime_named("<Unknown>")
    }
    #[inline]
    fn try_filetime_named(
        &mut self,
        name: &'static str,
    ) -> DeserializationResult<chrono::DateTime<chrono::Utc>> {
        winstructs::timestamp::WinTimestamp::from_reader(self)
            .map_err(|e| DeserializationError::FailedToReadToken {
                t: "filetime".to_owned(),
                token_name: name,
                source: WrappedIoError::capture_hexdump(Box::new(e), self),
            })
            .map(|t| t.to_datetime())
    }

    #[inline]
    fn try_systime(&mut self) -> DeserializationResult<chrono::DateTime<chrono::Utc>> {
        crate::utils::read_systemtime(self)
    }

    fn try_read_sized_array<T, F>(
        &mut self,
        size: u16,
        mut read_one: F,
    ) -> DeserializationResult<Vec<T>>
    where
        Self: Sized,
        F: FnMut(&mut Self) -> DeserializationResult<T>,
    {
        let start_pos = match self.tell() {
            Ok(p) => p,
            Err(e) => {
                let inner = WrappedIoError::capture_hexdump(Box::new(e), self);
                return Err(DeserializationError::from(inner));
            }
        };

        let mut out = vec![];
        loop {
            let cur = match self.tell() {
                Ok(p) => p,
                Err(e) => {
                    let inner = WrappedIoError::capture_hexdump(Box::new(e), self);
                    return Err(DeserializationError::from(inner));
                }
            };
            if (cur - start_pos) >= u64::from(size) {
                break;
            }
            out.push(read_one(self)?);
        }

        Ok(out)
    }

    #[inline]
    fn try_read_sized_u16_array(&mut self, size: u16) -> DeserializationResult<Vec<u16>> {
        self.try_read_sized_array(size, |c| c.try_u16())
    }
    #[inline]
    fn try_read_sized_i8_array(&mut self, size: u16) -> DeserializationResult<Vec<i8>> {
        self.try_read_sized_array(size, |c| c.try_i8())
    }
    #[inline]
    fn try_read_sized_i16_array(&mut self, size: u16) -> DeserializationResult<Vec<i16>> {
        self.try_read_sized_array(size, |c| c.try_i16())
    }
    #[inline]
    fn try_read_sized_u32_array(&mut self, size: u16) -> DeserializationResult<Vec<u32>> {
        self.try_read_sized_array(size, |c| c.try_u32())
    }
    #[inline]
    fn try_read_sized_i32_array(&mut self, size: u16) -> DeserializationResult<Vec<i32>> {
        self.try_read_sized_array(size, |c| c.try_i32())
    }
    #[inline]
    fn try_read_sized_i64_array(&mut self, size: u16) -> DeserializationResult<Vec<i64>> {
        self.try_read_sized_array(size, |c| c.try_i64())
    }
    #[inline]
    fn try_read_sized_u64_array(&mut self, size: u16) -> DeserializationResult<Vec<u64>> {
        self.try_read_sized_array(size, |c| c.try_u64())
    }
    #[inline]
    fn try_read_sized_f32_array(&mut self, size: u16) -> DeserializationResult<Vec<f32>> {
        self.try_read_sized_array(size, |c| c.try_f32())
    }
    #[inline]
    fn try_read_sized_f64_array(&mut self, size: u16) -> DeserializationResult<Vec<f64>> {
        self.try_read_sized_array(size, |c| c.try_f64())
    }
    #[inline]
    fn try_read_sized_bool_array(&mut self, size: u16) -> DeserializationResult<Vec<bool>> {
        self.try_read_sized_array(size, |c| c.try_bool())
    }
    #[inline]
    fn try_read_sized_guid_array(&mut self, size: u16) -> DeserializationResult<Vec<Guid>> {
        self.try_read_sized_array(size, |c| c.try_guid())
    }
    #[inline]
    fn try_read_sized_filetime_array(
        &mut self,
        size: u16,
    ) -> DeserializationResult<Vec<chrono::DateTime<chrono::Utc>>> {
        self.try_read_sized_array(size, |c| c.try_filetime())
    }
    #[inline]
    fn try_read_sized_systime_array(
        &mut self,
        size: u16,
    ) -> DeserializationResult<Vec<chrono::DateTime<chrono::Utc>>> {
        self.try_read_sized_array(size, |c| c.try_systime())
    }
    #[inline]
    fn try_read_sized_sid_array(&mut self, size: u16) -> DeserializationResult<Vec<Sid>> {
        self.try_read_sized_array(size, |c| c.try_sid())
    }
    #[inline]
    fn try_read_sized_null_terminated_utf16_string_array(
        &mut self,
        size: u16,
    ) -> DeserializationResult<Vec<String>> {
        self.try_read_sized_array(size, |c| c.try_null_terminated_utf16_string())
    }
    #[inline]
    fn try_read_sized_hex32_array<'a>(
        &mut self,
        size: u16,
    ) -> DeserializationResult<Vec<Cow<'a, str>>> {
        self.try_read_sized_array(size, |c| c.try_hex32())
    }
    #[inline]
    fn try_read_sized_hex64_array<'a>(
        &mut self,
        size: u16,
    ) -> DeserializationResult<Vec<Cow<'a, str>>> {
        self.try_read_sized_array(size, |c| c.try_hex64())
    }
}

impl<T: ReadSeek> ReadExt for T {}
