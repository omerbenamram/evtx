#![doc = include_str!("../README.md")]

use core::mem::MaybeUninit;
use std::io;

#[cfg(feature = "sonic-writeext")]
use sonic_rs::writer::WriteExt;

mod scalar;
mod simd;

pub use scalar::{
    escape_json_utf16_scalar, escape_json_utf16le_scalar, escape_utf16_raw_scalar,
    escape_utf16le_raw_scalar, escape_xml_utf16_scalar, escape_xml_utf16le_scalar,
};

#[cfg(feature = "perf-counters")]
pub mod perf;

/// Reusable scratch buffer for UTF-16 escaping without zero-filling.
///
/// This owns a `Vec<MaybeUninit<u8>>` so we can resize without clearing and
/// then safely expose the initialized prefix after an escape call.
#[derive(Default)]
pub struct Scratch {
    buf: Vec<MaybeUninit<u8>>,
}

impl Scratch {
    /// Create a new empty scratch buffer.
    #[inline]
    pub fn new() -> Self {
        Scratch { buf: Vec::new() }
    }

    /// JSON-escape UTF-16LE data and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_json_utf16le<'a>(
        &'a mut self,
        utf16le: &[u8],
        num_units: usize,
        need_quote: bool,
    ) -> &'a [u8] {
        let needed = max_escaped_len(num_units, need_quote);
        let dst = self.prepare(needed);
        let len = escape_json_utf16le(utf16le, num_units, dst, need_quote);
        // SAFETY: escape_json_utf16le initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// XML-escape UTF-16LE data and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_xml_utf16le<'a>(
        &'a mut self,
        utf16le: &[u8],
        num_units: usize,
        in_attribute: bool,
    ) -> &'a [u8] {
        let needed = max_escaped_len(num_units, false);
        let dst = self.prepare(needed);
        let len = escape_xml_utf16le(utf16le, num_units, dst, in_attribute);
        // SAFETY: escape_xml_utf16le initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// Convert UTF-16LE data to raw UTF-8 and return a borrowed slice.
    #[inline]
    pub fn escape_utf16le_raw<'a>(&'a mut self, utf16le: &[u8], num_units: usize) -> &'a [u8] {
        let needed = max_escaped_len(num_units, false);
        let dst = self.prepare(needed);
        let len = escape_utf16le_raw(utf16le, num_units, dst);
        // SAFETY: escape_utf16le_raw initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// JSON-escape UTF-16 code units (`u16`) and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_json_utf16<'a>(&'a mut self, utf16: &[u16], need_quote: bool) -> &'a [u8] {
        let needed = max_escaped_len(utf16.len(), need_quote);
        let dst = self.prepare(needed);
        let len = escape_json_utf16(utf16, dst, need_quote);
        // SAFETY: escape_json_utf16 initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// XML-escape UTF-16 code units (`u16`) and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_xml_utf16<'a>(&'a mut self, utf16: &[u16], in_attribute: bool) -> &'a [u8] {
        let needed = max_escaped_len(utf16.len(), false);
        let dst = self.prepare(needed);
        let len = escape_xml_utf16(utf16, dst, in_attribute);
        // SAFETY: escape_xml_utf16 initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// Convert UTF-16 code units (`u16`) to raw UTF-8 bytes and return a borrowed slice.
    #[inline]
    pub fn escape_utf16_raw<'a>(&'a mut self, utf16: &[u16]) -> &'a [u8] {
        let needed = max_escaped_len(utf16.len(), false);
        let dst = self.prepare(needed);
        let len = escape_utf16_raw(utf16, dst);
        // SAFETY: escape_utf16_raw initialized the first `len` bytes.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// Write JSON-escaped UTF-16LE bytes into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_json_utf16le_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16le: &[u8],
        num_units: usize,
        need_quote: bool,
    ) -> io::Result<()> {
        let out = self.escape_json_utf16le(utf16le, num_units, need_quote);
        writer.write_all(out)
    }

    /// Write XML-escaped UTF-16LE bytes into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_xml_utf16le_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16le: &[u8],
        num_units: usize,
        in_attribute: bool,
    ) -> io::Result<()> {
        let out = self.escape_xml_utf16le(utf16le, num_units, in_attribute);
        writer.write_all(out)
    }

    /// Write raw UTF-8 (no escaping) converted from UTF-16LE bytes into an `io::Write`.
    #[inline]
    pub fn write_utf16le_raw_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16le: &[u8],
        num_units: usize,
    ) -> io::Result<()> {
        let out = self.escape_utf16le_raw(utf16le, num_units);
        writer.write_all(out)
    }

    /// Write JSON-escaped UTF-16 code units (`u16`) into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_json_utf16_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
        need_quote: bool,
    ) -> io::Result<()> {
        let out = self.escape_json_utf16(utf16, need_quote);
        writer.write_all(out)
    }

    /// Write XML-escaped UTF-16 code units (`u16`) into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_xml_utf16_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
        in_attribute: bool,
    ) -> io::Result<()> {
        let out = self.escape_xml_utf16(utf16, in_attribute);
        writer.write_all(out)
    }

    /// Write raw UTF-8 (no escaping) converted from UTF-16 code units (`u16`) into an `io::Write`.
    #[inline]
    pub fn write_utf16_raw_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
    ) -> io::Result<()> {
        let out = self.escape_utf16_raw(utf16);
        writer.write_all(out)
    }

    #[inline]
    fn prepare(&mut self, needed: usize) -> &mut [MaybeUninit<u8>] {
        if self.buf.len() < needed {
            self.buf.reserve(needed - self.buf.len());
            // SAFETY: We only write into the first `needed` bytes before reading.
            unsafe {
                self.buf.set_len(needed);
            }
        }
        &mut self.buf[..needed]
    }
}

/// Maximum number of bytes required to JSON-escape `num_units` UTF-16 code units.
#[inline]
pub fn max_escaped_len(num_units: usize, need_quote: bool) -> usize {
    num_units
        .saturating_mul(6)
        .saturating_add(if need_quote { 2 } else { 0 })
}

/// JSON-escape UTF-16LE data into `dst`, using SIMD when available.
///
/// - `utf16le` is a byte slice containing little-endian UTF-16 code units.
/// - `num_units` is the number of UTF-16 code units to read (not bytes).
/// - `need_quote` controls whether surrounding `"` are emitted.
///
/// Returns the number of bytes written to `dst`.
///
/// ## Safety
/// The caller must ensure `dst` has capacity for the worst case, typically
/// `max_escaped_len(num_units, need_quote)`.
#[inline]
pub fn escape_json_utf16le(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    simd::escape_json_utf16le_simd(utf16le, num_units, dst, need_quote)
}

/// JSON-escape UTF-16LE data directly into a `sonic-rs` `WriteExt` buffer.
///
/// Enabled only with the `sonic-writeext` feature.
#[cfg(feature = "sonic-writeext")]
pub fn write_json_utf16le<W: WriteExt>(
    writer: &mut W,
    utf16le: &[u8],
    num_units: usize,
    need_quote: bool,
) -> io::Result<()> {
    let max_units = crate::scalar::max_units(num_units, utf16le.len());
    let max_len = max_escaped_len(max_units, need_quote);
    let buf = writer.reserve_with(max_len)?;
    let len = escape_json_utf16le(utf16le, num_units, buf, need_quote);
    // SAFETY: `escape_json_utf16le` initialized the first `len` bytes in `buf`.
    unsafe {
        writer.flush_len(len)?;
    }
    Ok(())
}

/// JSON-escape UTF-16LE data into `out`, reusing its allocation.
#[inline]
pub fn escape_json_utf16le_into(
    utf16le: &[u8],
    num_units: usize,
    out: &mut Vec<u8>,
    need_quote: bool,
) {
    let needed = max_escaped_len(num_units, need_quote);
    let buf = prepare_output(out, needed);
    let len = escape_json_utf16le(utf16le, num_units, buf, need_quote);
    out.truncate(len);
}

/// XML-escape UTF-16LE data into `dst`, using SIMD when available.
///
/// When `in_attribute` is true, the output will also escape `"` and `'`.
#[inline]
pub fn escape_xml_utf16le(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    simd::escape_xml_utf16le_simd(utf16le, num_units, dst, in_attribute)
}

/// XML-escape UTF-16LE data directly into a `sonic-rs` `WriteExt` buffer.
///
/// Enabled only with the `sonic-writeext` feature.
#[cfg(feature = "sonic-writeext")]
pub fn write_xml_utf16le<W: WriteExt>(
    writer: &mut W,
    utf16le: &[u8],
    num_units: usize,
    in_attribute: bool,
) -> io::Result<()> {
    let max_units = crate::scalar::max_units(num_units, utf16le.len());
    if max_units == 0 {
        return Ok(());
    }
    let max_len = max_escaped_len(max_units, false);
    let buf = writer.reserve_with(max_len)?;
    let len = escape_xml_utf16le(utf16le, num_units, buf, in_attribute);
    unsafe {
        writer.flush_len(len)?;
    }
    Ok(())
}

/// XML-escape UTF-16LE data into `out`, reusing its allocation.
#[inline]
pub fn escape_xml_utf16le_into(
    utf16le: &[u8],
    num_units: usize,
    out: &mut Vec<u8>,
    in_attribute: bool,
) {
    let needed = max_escaped_len(num_units, false);
    let buf = prepare_output(out, needed);
    let len = escape_xml_utf16le(utf16le, num_units, buf, in_attribute);
    out.truncate(len);
}

/// Convert UTF-16LE data into raw UTF-8 bytes without escaping.
#[inline]
pub fn escape_utf16le_raw(utf16le: &[u8], num_units: usize, dst: &mut [MaybeUninit<u8>]) -> usize {
    simd::escape_utf16le_raw_simd(utf16le, num_units, dst)
}

/// Convert UTF-16LE data to raw UTF-8 directly into a `sonic-rs` `WriteExt` buffer.
///
/// Enabled only with the `sonic-writeext` feature.
#[cfg(feature = "sonic-writeext")]
pub fn write_utf16le_raw<W: WriteExt>(
    writer: &mut W,
    utf16le: &[u8],
    num_units: usize,
) -> io::Result<()> {
    let max_units = crate::scalar::max_units(num_units, utf16le.len());
    if max_units == 0 {
        return Ok(());
    }
    let max_len = max_escaped_len(max_units, false);
    let buf = writer.reserve_with(max_len)?;
    let len = escape_utf16le_raw(utf16le, num_units, buf);
    unsafe {
        writer.flush_len(len)?;
    }
    Ok(())
}

/// Convert UTF-16LE data into raw UTF-8 bytes without escaping, reusing `out`.
#[inline]
pub fn escape_utf16le_raw_into(utf16le: &[u8], num_units: usize, out: &mut Vec<u8>) {
    let needed = max_escaped_len(num_units, false);
    let buf = prepare_output(out, needed);
    let len = escape_utf16le_raw(utf16le, num_units, buf);
    out.truncate(len);
}

/// JSON-escape UTF-16LE data into a new `Vec<u8>`.
///
/// This is a convenience wrapper around [`escape_json_utf16le`].
#[inline]
pub fn escape_json_utf16le_to_vec(utf16le: &[u8], num_units: usize, need_quote: bool) -> Vec<u8> {
    let mut buf = vec![MaybeUninit::uninit(); max_escaped_len(num_units, need_quote)];
    let len = escape_json_utf16le(utf16le, num_units, &mut buf, need_quote);
    // SAFETY: we have initialized the first `len` bytes.
    unsafe {
        let ptr = buf.as_mut_ptr() as *mut u8;
        let mut out = Vec::with_capacity(buf.len());
        out.set_len(len);
        core::ptr::copy_nonoverlapping(ptr, out.as_mut_ptr(), len);
        out
    }
}

/// JSON-escape UTF-16 code units (`u16`) into `dst`, using SIMD when available.
#[inline]
pub fn escape_json_utf16(utf16: &[u16], dst: &mut [MaybeUninit<u8>], need_quote: bool) -> usize {
    simd::escape_json_utf16_simd(utf16, dst, need_quote)
}

/// JSON-escape UTF-16 code units (`u16`) into `out`, reusing its allocation.
#[inline]
pub fn escape_json_utf16_into(utf16: &[u16], out: &mut Vec<u8>, need_quote: bool) {
    let needed = max_escaped_len(utf16.len(), need_quote);
    let buf = prepare_output(out, needed);
    let len = escape_json_utf16(utf16, buf, need_quote);
    out.truncate(len);
}

/// XML-escape UTF-16 code units (`u16`) into `dst`, using SIMD when available.
#[inline]
pub fn escape_xml_utf16(utf16: &[u16], dst: &mut [MaybeUninit<u8>], in_attribute: bool) -> usize {
    simd::escape_xml_utf16_simd(utf16, dst, in_attribute)
}

/// XML-escape UTF-16 code units (`u16`) into `out`, reusing its allocation.
#[inline]
pub fn escape_xml_utf16_into(utf16: &[u16], out: &mut Vec<u8>, in_attribute: bool) {
    let needed = max_escaped_len(utf16.len(), false);
    let buf = prepare_output(out, needed);
    let len = escape_xml_utf16(utf16, buf, in_attribute);
    out.truncate(len);
}

/// Convert UTF-16 code units (`u16`) into raw UTF-8 bytes without escaping.
#[inline]
pub fn escape_utf16_raw(utf16: &[u16], dst: &mut [MaybeUninit<u8>]) -> usize {
    simd::escape_utf16_raw_simd(utf16, dst)
}

/// Convert UTF-16 code units (`u16`) into raw UTF-8 bytes without escaping, reusing `out`.
#[inline]
pub fn escape_utf16_raw_into(utf16: &[u16], out: &mut Vec<u8>) {
    let needed = max_escaped_len(utf16.len(), false);
    let buf = prepare_output(out, needed);
    let len = escape_utf16_raw(utf16, buf);
    out.truncate(len);
}

#[inline]
fn prepare_output(out: &mut Vec<u8>, needed: usize) -> &mut [MaybeUninit<u8>] {
    out.clear();
    out.resize(needed, 0);
    // SAFETY: MaybeUninit<u8> has the same layout as u8.
    unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut MaybeUninit<u8>, out.len()) }
}
