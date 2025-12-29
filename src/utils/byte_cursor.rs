use crate::binxml::value_variant::SidRef;
use crate::err::{DeserializationError, DeserializationResult};
use crate::utils::bytes;
use crate::utils::{Utf16LeSlice, decode_utf16le_bytes_to_bump_str, trim_utf16le_whitespace};
use bumpalo::Bump;
use std::io;

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
    pub(crate) fn advance(&mut self, n: usize, what: &'static str) -> DeserializationResult<()> {
        let new_pos = self
            .pos
            .checked_add(n)
            .ok_or_else(|| DeserializationError::Truncated {
                what,
                offset: self.pos as u64,
                need: n,
                have: self.buf.len().saturating_sub(self.pos),
            })?;
        self.set_pos(new_pos, what)
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

    #[inline]
    pub(crate) fn u8(&mut self) -> DeserializationResult<u8> {
        self.u8_named("u8")
    }

    #[inline]
    pub(crate) fn u8_named(&mut self, what: &'static str) -> DeserializationResult<u8> {
        let b =
            bytes::read_u8(self.buf, self.pos).ok_or_else(|| DeserializationError::Truncated {
                what,
                offset: self.pos as u64,
                need: 1,
                have: self.buf.len().saturating_sub(self.pos),
            })?;
        self.pos += 1;
        Ok(b)
    }

    #[inline]
    pub(crate) fn u16(&mut self) -> DeserializationResult<u16> {
        self.u16_named("u16")
    }

    #[inline]
    pub(crate) fn u16_named(&mut self, what: &'static str) -> DeserializationResult<u16> {
        let v = bytes::read_u16_le_r(self.buf, self.pos, what)?;
        self.pos += 2;
        Ok(v)
    }

    #[inline]
    pub(crate) fn u32(&mut self) -> DeserializationResult<u32> {
        self.u32_named("u32")
    }

    #[inline]
    pub(crate) fn u32_named(&mut self, what: &'static str) -> DeserializationResult<u32> {
        let v = bytes::read_u32_le_r(self.buf, self.pos, what)?;
        self.pos += 4;
        Ok(v)
    }

    #[inline]
    pub(crate) fn u64(&mut self) -> DeserializationResult<u64> {
        self.u64_named("u64")
    }

    #[inline]
    pub(crate) fn u64_named(&mut self, what: &'static str) -> DeserializationResult<u64> {
        let v = bytes::read_u64_le_r(self.buf, self.pos, what)?;
        self.pos += 8;
        Ok(v)
    }

    /// Read a sized array encoded as "N bytes of consecutive elements".
    ///
    /// This matches the historical behavior of the old `try_read_sized_array` helpers:
    /// we stop when we've *consumed at least* `size_bytes` bytes since the start of this call.
    ///
    /// `elem_bytes` is only used for capacity preallocation.
    pub(crate) fn read_sized_vec<T>(
        &mut self,
        size_bytes: u16,
        elem_bytes: usize,
        mut read_one: impl FnMut(&mut Self) -> DeserializationResult<T>,
    ) -> DeserializationResult<Vec<T>> {
        let size_usize = usize::from(size_bytes);
        if size_usize == 0 {
            return Ok(Vec::new());
        }

        let start = self.pos;
        let mut out = Vec::with_capacity(size_usize / elem_bytes.max(1));
        loop {
            let cur = self.pos;
            if (cur - start) >= size_usize {
                break;
            }
            out.push(read_one(self)?);
        }
        Ok(out)
    }

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

    /// Read a sized array encoded as `size_bytes` bytes of consecutive **fixed-width** elements,
    /// with strict alignment validation.
    ///
    /// - Validates `size_bytes % ELEM_BYTES == 0`
    /// - Reads *exactly* `size_bytes / ELEM_BYTES` elements
    /// - Uses a single bounds check (`take_bytes`) and then parses by iterating `chunks_exact`
    ///
    /// The parse closure also receives the **absolute byte offset** (within this cursor’s backing
    /// slice) of the current element, which is useful for precise error reporting.
    pub(crate) fn read_sized_vec_aligned<const ELEM_BYTES: usize, T>(
        &mut self,
        size_bytes: u16,
        what: &'static str,
        mut parse_one: impl FnMut(u64, &[u8; ELEM_BYTES]) -> DeserializationResult<T>,
    ) -> DeserializationResult<Vec<T>> {
        let size_usize = usize::from(size_bytes);
        if size_usize == 0 {
            return Ok(Vec::new());
        }
        if ELEM_BYTES == 0 {
            return Err(DeserializationError::Truncated {
                what,
                offset: self.pos as u64,
                need: size_usize,
                have: self.buf.len().saturating_sub(self.pos),
            });
        }
        if (size_usize % ELEM_BYTES) != 0 {
            return Err(DeserializationError::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{what}: misaligned sized array (size_bytes={size_usize}, elem_bytes={ELEM_BYTES}) at offset {}",
                    self.pos
                ),
            )));
        }

        let start_pos = self.pos;
        let bytes = self.take_bytes(size_usize, what)?;
        let count = size_usize / ELEM_BYTES;
        let mut out = Vec::with_capacity(count);
        for (i, chunk) in bytes.chunks_exact(ELEM_BYTES).enumerate() {
            let off = start_pos + i * ELEM_BYTES;
            let arr: &[u8; ELEM_BYTES] = chunk
                .try_into()
                .expect("chunks_exact yields slices of the requested size");
            out.push(parse_one(off as u64, arr)?);
        }
        Ok(out)
    }

    #[inline]
    fn invalid_data(what: &'static str, offset: u64) -> DeserializationError {
        DeserializationError::Io(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{what} at offset {offset}: invalid data"),
        ))
    }

    /// Read `char_count` UTF-16 code units (little-endian), decode (stop at NUL if present),
    /// and trim trailing whitespace.
    pub(crate) fn utf16_by_char_count_trimmed(
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

        let trimmed_chars = trim_utf16le_whitespace(bytes, char_count)
            .map_err(|_| Self::invalid_data(what, (self.pos - byte_len) as u64))?;

        Ok(Some(Utf16LeSlice::new(bytes, trimmed_chars)))
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

    /// Read `char_count` UTF-16 code units, trim trailing whitespace, and decode to UTF-8.
    #[allow(dead_code)]
    pub(crate) fn utf16_by_char_count_trimmed_utf8(
        &mut self,
        char_count: usize,
        what: &'static str,
    ) -> DeserializationResult<Option<String>> {
        let start = self.pos;
        let slice = self.utf16_by_char_count_trimmed(char_count, what)?;
        match slice {
            Some(value) => value
                .to_string()
                .map(Some)
                .map_err(|_| Self::invalid_data(what, start as u64)),
            None => Ok(None),
        }
    }

    /// Read `char_count` UTF-16 code units, trim trailing whitespace, and decode into a bump string.
    #[allow(dead_code)]
    pub(crate) fn utf16_by_char_count_trimmed_bump(
        &mut self,
        char_count: usize,
        what: &'static str,
        arena: &'a Bump,
    ) -> DeserializationResult<Option<&'a str>> {
        let start = self.pos;
        let slice = self.utf16_by_char_count_trimmed(char_count, what)?;
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

        let num_chars = bytes.len() / 2;
        trim_utf16le_whitespace(bytes, num_chars)
            .map_err(|_| Self::invalid_data(what, start as u64))?;
        Ok(Utf16LeSlice::new(bytes, num_chars))
    }

    /// Read UTF-16 code units until NUL and decode into UTF-8.
    #[allow(dead_code)]
    pub(crate) fn null_terminated_utf16_string_utf8(
        &mut self,
        what: &'static str,
    ) -> DeserializationResult<String> {
        let start = self.pos;
        let slice = self.null_terminated_utf16_string(what)?;
        slice
            .to_string()
            .map_err(|_| Self::invalid_data(what, start as u64))
    }

    /// Read UTF-16 code units until NUL and decode into a bump string.
    #[allow(dead_code)]
    pub(crate) fn null_terminated_utf16_string_bump(
        &mut self,
        what: &'static str,
        arena: &'a Bump,
    ) -> DeserializationResult<&'a str> {
        let start = self.pos;
        let slice = self.null_terminated_utf16_string(what)?;
        decode_utf16le_bytes_to_bump_str(slice.as_bytes(), slice.as_bytes().len() / 2, arena)
            .map_err(|_| Self::invalid_data(what, start as u64))
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
