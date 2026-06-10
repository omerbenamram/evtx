use crate::binxml::value_variant::SidRef;
use crate::err::{DeserializationError, DeserializationResult};
use crate::utils::bytes;
use crate::utils::{Utf16LeSlice, decode_utf16le_bytes_to_bump_str};
use bumpalo::Bump;
use std::io;

macro_rules! le_readers {
    ($($plain:ident / $named:ident: $ty:ty),* $(,)?) => {$(
        #[inline]
        pub(crate) fn $plain(&mut self) -> DeserializationResult<$ty> {
            self.$named(stringify!($plain))
        }

        #[inline]
        pub(crate) fn $named(&mut self, what: &'static str) -> DeserializationResult<$ty> {
            let v = <$ty>::from_le_bytes(bytes::read_array_r(self.buf, self.pos, what)?);
            self.pos += size_of::<$ty>();
            Ok(v)
        }
    )*};
}

/// A lightweight cursor over an immutable byte slice.
///
/// This is the slice/offset equivalent of `Cursor<&[u8]>`, intended for hot-path parsing where:
/// - the data is already in memory, and
/// - we want explicit bounds/offset control without IO-style error plumbing.
///
/// All reads are little-endian and advance the cursor on success.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ByteCursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> ByteCursor<'a> {
    #[inline]
    pub(crate) fn with_pos(buf: &'a [u8], pos: usize) -> DeserializationResult<Self> {
        // Allow pos == len (EOF), reject pos > len.
        let _ = bytes::slice_r(buf, pos, 0, "cursor.position")?;
        Ok(Self { buf, pos })
    }

    #[inline]
    pub(crate) fn buf(&self) -> &'a [u8] {
        self.buf
    }

    #[inline]
    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    #[inline]
    pub(crate) fn position(&self) -> u64 {
        self.pos as u64
    }

    #[inline]
    pub(crate) fn set_pos(&mut self, pos: usize, what: &'static str) -> DeserializationResult<()> {
        let _ = bytes::slice_r(self.buf, pos, 0, what)?;
        self.pos = pos;
        Ok(())
    }

    #[inline]
    pub(crate) fn set_pos_u64(
        &mut self,
        pos: u64,
        what: &'static str,
    ) -> DeserializationResult<()> {
        let pos_usize = usize::try_from(pos).map_err(|_| DeserializationError::Truncated {
            what,
            offset: pos,
            need: 0,
            have: 0,
        })?;
        self.set_pos(pos_usize, what)
    }

    #[inline]
    pub(crate) fn take_bytes(
        &mut self,
        len: usize,
        what: &'static str,
    ) -> DeserializationResult<&'a [u8]> {
        let out = bytes::slice_r(self.buf, self.pos, len, what)?;
        self.pos += len;
        Ok(out)
    }

    #[inline]
    pub(crate) fn array<const N: usize>(
        &mut self,
        what: &'static str,
    ) -> DeserializationResult<[u8; N]> {
        let v = bytes::read_array_r::<N>(self.buf, self.pos, what)?;
        self.pos += N;
        Ok(v)
    }

    le_readers!(
        u8 / u8_named: u8,
        u16 / u16_named: u16,
        u32 / u32_named: u32,
        u64 / u64_named: u64,
    );

    pub(crate) fn read_sid_ref(&mut self) -> DeserializationResult<SidRef<'a>> {
        let start = self.pos();
        let remaining = self
            .buf()
            .get(start..)
            .ok_or_else(|| DeserializationError::Truncated {
                what: "sid",
                offset: start as u64,
                need: 1,
                have: 0,
            })?;

        if remaining.len() < 8 {
            return Err(DeserializationError::Truncated {
                what: "sid",
                offset: start as u64,
                need: 8,
                have: remaining.len(),
            });
        }

        let sub_count = remaining[1] as usize;
        let len = 8usize
            .checked_add(sub_count.saturating_mul(4))
            .ok_or_else(|| DeserializationError::Truncated {
                what: "sid",
                offset: start as u64,
                need: usize::MAX,
                have: remaining.len(),
            })?;

        if remaining.len() < len {
            return Err(DeserializationError::Truncated {
                what: "sid",
                offset: start as u64,
                need: len,
                have: remaining.len(),
            });
        }

        let bytes = self.take_bytes(len, "sid")?;
        Ok(SidRef::new(bytes))
    }

    pub(crate) fn read_sized_slice_aligned_in<const ELEM_BYTES: usize, T>(
        &mut self,
        size_bytes: u16,
        what: &'static str,
        arena: &'a Bump,
        mut parse_one: impl FnMut(u64, &[u8; ELEM_BYTES]) -> DeserializationResult<T>,
    ) -> DeserializationResult<&'a [T]> {
        let size_usize = usize::from(size_bytes);
        if size_usize == 0 {
            return Ok(&[]);
        }
        if ELEM_BYTES == 0 {
            return Err(DeserializationError::Truncated {
                what,
                offset: self.position(),
                need: size_usize,
                have: self.buf().len().saturating_sub(self.pos()),
            });
        }
        if (size_usize % ELEM_BYTES) != 0 {
            return Err(DeserializationError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{what}: misaligned sized array (size_bytes={size_usize}, elem_bytes={ELEM_BYTES}) at offset {}",
                    self.pos()
                ),
            )));
        }

        let start_pos = self.pos();
        let bytes = self.take_bytes(size_usize, what)?;
        let count = size_usize / ELEM_BYTES;

        let out = arena.alloc_slice_try_fill_with(count, |i| {
            let off = start_pos + i * ELEM_BYTES;
            let start = i * ELEM_BYTES;
            let end = start + ELEM_BYTES;
            let chunk: &[u8; ELEM_BYTES] = bytes[start..end]
                .try_into()
                .expect("validated ELEM_BYTES alignment");
            parse_one(off as u64, chunk)
        })?;

        Ok(out)
    }

    #[inline]
    fn invalid_data(what: &'static str, offset: u64) -> DeserializationError {
        DeserializationError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what} at offset {offset}: invalid data"),
        ))
    }

    /// Read `char_count` UTF-16 code units (little-endian), stopping at NUL if present,
    /// without trimming whitespace.
    pub(crate) fn utf16_by_char_count(
        &mut self,
        char_count: usize,
        what: &'static str,
    ) -> DeserializationResult<Option<Utf16LeSlice<'a>>> {
        if char_count == 0 {
            return Ok(None);
        }

        let byte_len =
            char_count
                .checked_mul(2)
                .ok_or_else(|| DeserializationError::Truncated {
                    what,
                    offset: self.pos as u64,
                    need: usize::MAX,
                    have: self.buf.len().saturating_sub(self.pos),
                })?;

        let bytes = self.take_bytes(byte_len, what)?;
        if !bytes.len().is_multiple_of(2) {
            return Err(Self::invalid_data(what, self.pos as u64));
        }

        let mut actual_chars = bytes.len() / 2;
        for (idx, chunk) in bytes.chunks_exact(2).enumerate() {
            if chunk[0] == 0 && chunk[1] == 0 {
                actual_chars = idx;
                break;
            }
        }

        Ok(Some(Utf16LeSlice::new(bytes, actual_chars)))
    }

    /// Read a `u16` length prefix (number of UTF-16 code units), then that many code units,
    /// decoding until NUL (if present). Optionally reads and discards a trailing NUL code unit.
    pub(crate) fn len_prefixed_utf16_string(
        &mut self,
        is_null_terminated: bool,
        what: &'static str,
    ) -> DeserializationResult<Option<Utf16LeSlice<'a>>> {
        let char_count = self.u16_named(what)? as usize;
        let s = self.utf16_by_char_count(char_count, what)?;
        if is_null_terminated {
            let _ = self.u16_named(what)?;
        }
        Ok(s)
    }

    /// Read a length-prefixed UTF-16LE string and decode it into UTF-8.
    ///
    /// This is intended for legacy paths that still require owned `String` values.
    pub(crate) fn len_prefixed_utf16_string_utf8(
        &mut self,
        is_null_terminated: bool,
        what: &'static str,
    ) -> DeserializationResult<Option<String>> {
        let start = self.pos;
        let slice = self.len_prefixed_utf16_string(is_null_terminated, what)?;
        match slice {
            Some(value) => value
                .to_string()
                .map(Some)
                .map_err(|_| Self::invalid_data(what, start as u64)),
            None => Ok(None),
        }
    }

    /// Read UTF-16 code units until a NUL (0x0000) code unit is encountered.
    pub(crate) fn null_terminated_utf16_string(
        &mut self,
        what: &'static str,
    ) -> DeserializationResult<Utf16LeSlice<'a>> {
        let start = self.pos;
        loop {
            let cu = bytes::read_u16_le_r(self.buf, self.pos, what)?;
            self.pos += 2;
            if cu == 0 {
                break;
            }
        }

        let end = self.pos.saturating_sub(2);
        let bytes = self
            .buf
            .get(start..end)
            .ok_or_else(|| DeserializationError::Truncated {
                what,
                offset: start as u64,
                need: 2,
                have: self.buf.len().saturating_sub(start),
            })?;

        Ok(Utf16LeSlice::new(bytes, bytes.len() / 2))
    }

    /// Read a length-prefixed UTF-16LE string and decode into a bump string.
    pub(crate) fn len_prefixed_utf16_string_bump(
        &mut self,
        is_null_terminated: bool,
        what: &'static str,
        arena: &'a Bump,
    ) -> DeserializationResult<Option<&'a str>> {
        let start = self.pos;
        let slice = self.len_prefixed_utf16_string(is_null_terminated, what)?;
        match slice {
            Some(value) => decode_utf16le_bytes_to_bump_str(
                value.as_bytes(),
                value.as_bytes().len() / 2,
                arena,
            )
            .map(Some)
            .map_err(|_| Self::invalid_data(what, start as u64)),
            None => Ok(None),
        }
    }
}
