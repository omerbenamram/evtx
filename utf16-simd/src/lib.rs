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

    #[inline]
    fn escape_with(
        &mut self,
        max_units: usize,
        need_quote: bool,
        escape: impl FnOnce(&mut [MaybeUninit<u8>]) -> usize,
    ) -> &[u8] {
        let needed = max_escaped_len(max_units, need_quote);
        let dst = self.prepare(needed);
        let len = escape(dst);
        // SAFETY: `escape` initialized the first `len` bytes of `dst`.
        unsafe { core::slice::from_raw_parts(dst.as_ptr() as *const u8, len) }
    }

    /// JSON-escape UTF-16LE data and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_json_utf16le<'a>(
        &'a mut self,
        utf16le: &[u8],
        num_units: usize,
        need_quote: bool,
    ) -> &'a [u8] {
        let max_units = scalar::max_units(num_units, utf16le.len());
        self.escape_with(max_units, need_quote, |dst| {
            escape_json_utf16le(utf16le, max_units, dst, need_quote)
        })
    }

    /// XML-escape UTF-16LE data and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_xml_utf16le<'a>(
        &'a mut self,
        utf16le: &[u8],
        num_units: usize,
        in_attribute: bool,
    ) -> &'a [u8] {
        let max_units = scalar::max_units(num_units, utf16le.len());
        self.escape_with(max_units, false, |dst| {
            escape_xml_utf16le(utf16le, max_units, dst, in_attribute)
        })
    }

    /// Convert UTF-16LE data to raw UTF-8 and return a borrowed slice.
    #[inline]
    pub fn escape_utf16le_raw<'a>(&'a mut self, utf16le: &[u8], num_units: usize) -> &'a [u8] {
        let max_units = scalar::max_units(num_units, utf16le.len());
        self.escape_with(max_units, false, |dst| {
            escape_utf16le_raw(utf16le, max_units, dst)
        })
    }

    /// JSON-escape UTF-16 code units (`u16`) and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_json_utf16<'a>(&'a mut self, utf16: &[u16], need_quote: bool) -> &'a [u8] {
        self.escape_with(utf16.len(), need_quote, |dst| {
            escape_json_utf16(utf16, dst, need_quote)
        })
    }

    /// XML-escape UTF-16 code units (`u16`) and return a borrowed UTF-8 slice.
    #[inline]
    pub fn escape_xml_utf16<'a>(&'a mut self, utf16: &[u16], in_attribute: bool) -> &'a [u8] {
        self.escape_with(utf16.len(), false, |dst| {
            escape_xml_utf16(utf16, dst, in_attribute)
        })
    }

    /// Convert UTF-16 code units (`u16`) to raw UTF-8 bytes and return a borrowed slice.
    #[inline]
    pub fn escape_utf16_raw<'a>(&'a mut self, utf16: &[u16]) -> &'a [u8] {
        self.escape_with(utf16.len(), false, |dst| escape_utf16_raw(utf16, dst))
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
        writer.write_all(self.escape_json_utf16le(utf16le, num_units, need_quote))
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
        writer.write_all(self.escape_xml_utf16le(utf16le, num_units, in_attribute))
    }

    /// Write raw UTF-8 (no escaping) converted from UTF-16LE bytes into an `io::Write`.
    #[inline]
    pub fn write_utf16le_raw_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16le: &[u8],
        num_units: usize,
    ) -> io::Result<()> {
        writer.write_all(self.escape_utf16le_raw(utf16le, num_units))
    }

    /// Write JSON-escaped UTF-16 code units (`u16`) into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_json_utf16_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
        need_quote: bool,
    ) -> io::Result<()> {
        writer.write_all(self.escape_json_utf16(utf16, need_quote))
    }

    /// Write XML-escaped UTF-16 code units (`u16`) into an `io::Write`, reusing this scratch buffer.
    #[inline]
    pub fn write_xml_utf16_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
        in_attribute: bool,
    ) -> io::Result<()> {
        writer.write_all(self.escape_xml_utf16(utf16, in_attribute))
    }

    /// Write raw UTF-8 (no escaping) converted from UTF-16 code units (`u16`) into an `io::Write`.
    #[inline]
    pub fn write_utf16_raw_to<W: io::Write>(
        &mut self,
        writer: &mut W,
        utf16: &[u16],
    ) -> io::Result<()> {
        writer.write_all(self.escape_utf16_raw(utf16))
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
    let max_units = scalar::max_units(num_units, utf16le.len());
    write_with(writer, max_units, need_quote, |buf| {
        escape_json_utf16le(utf16le, max_units, buf, need_quote)
    })
}

/// JSON-escape UTF-16LE data into `out`, reusing its allocation.
#[inline]
pub fn escape_json_utf16le_into(
    utf16le: &[u8],
    num_units: usize,
    out: &mut Vec<u8>,
    need_quote: bool,
) {
    let max_units = scalar::max_units(num_units, utf16le.len());
    escape_into(max_units, out, need_quote, |buf| {
        escape_json_utf16le(utf16le, max_units, buf, need_quote)
    });
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
    let max_units = scalar::max_units(num_units, utf16le.len());
    write_with(writer, max_units, false, |buf| {
        escape_xml_utf16le(utf16le, max_units, buf, in_attribute)
    })
}

/// XML-escape UTF-16LE data into `out`, reusing its allocation.
#[inline]
pub fn escape_xml_utf16le_into(
    utf16le: &[u8],
    num_units: usize,
    out: &mut Vec<u8>,
    in_attribute: bool,
) {
    let max_units = scalar::max_units(num_units, utf16le.len());
    escape_into(max_units, out, false, |buf| {
        escape_xml_utf16le(utf16le, max_units, buf, in_attribute)
    });
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
    let max_units = scalar::max_units(num_units, utf16le.len());
    write_with(writer, max_units, false, |buf| {
        escape_utf16le_raw(utf16le, max_units, buf)
    })
}

/// Convert UTF-16LE data into raw UTF-8 bytes without escaping, reusing `out`.
#[inline]
pub fn escape_utf16le_raw_into(utf16le: &[u8], num_units: usize, out: &mut Vec<u8>) {
    let max_units = scalar::max_units(num_units, utf16le.len());
    escape_into(max_units, out, false, |buf| {
        escape_utf16le_raw(utf16le, max_units, buf)
    });
}

/// JSON-escape UTF-16LE data into a new `Vec<u8>`.
///
/// This is a convenience wrapper around [`escape_json_utf16le`].
#[inline]
pub fn escape_json_utf16le_to_vec(utf16le: &[u8], num_units: usize, need_quote: bool) -> Vec<u8> {
    let max_units = scalar::max_units(num_units, utf16le.len());
    let needed = max_escaped_len(max_units, need_quote);
    let mut out = Vec::with_capacity(needed);
    let len = escape_json_utf16le(
        utf16le,
        max_units,
        &mut out.spare_capacity_mut()[..needed],
        need_quote,
    );
    debug_assert!(len <= needed);
    // SAFETY: `escape_json_utf16le` initialized the first `len` bytes of
    // `out`'s spare capacity, and `len <= needed <= out.capacity()`.
    unsafe {
        out.set_len(len);
    }
    out
}

/// JSON-escape UTF-16 code units (`u16`) into `dst`, using SIMD when available.
#[inline]
pub fn escape_json_utf16(utf16: &[u16], dst: &mut [MaybeUninit<u8>], need_quote: bool) -> usize {
    simd::escape_json_utf16_simd(utf16, dst, need_quote)
}

/// JSON-escape UTF-16 code units (`u16`) into `out`, reusing its allocation.
#[inline]
pub fn escape_json_utf16_into(utf16: &[u16], out: &mut Vec<u8>, need_quote: bool) {
    escape_into(utf16.len(), out, need_quote, |buf| {
        escape_json_utf16(utf16, buf, need_quote)
    });
}

/// XML-escape UTF-16 code units (`u16`) into `dst`, using SIMD when available.
#[inline]
pub fn escape_xml_utf16(utf16: &[u16], dst: &mut [MaybeUninit<u8>], in_attribute: bool) -> usize {
    simd::escape_xml_utf16_simd(utf16, dst, in_attribute)
}

/// XML-escape UTF-16 code units (`u16`) into `out`, reusing its allocation.
#[inline]
pub fn escape_xml_utf16_into(utf16: &[u16], out: &mut Vec<u8>, in_attribute: bool) {
    escape_into(utf16.len(), out, false, |buf| {
        escape_xml_utf16(utf16, buf, in_attribute)
    });
}

/// Convert UTF-16 code units (`u16`) into raw UTF-8 bytes without escaping.
#[inline]
pub fn escape_utf16_raw(utf16: &[u16], dst: &mut [MaybeUninit<u8>]) -> usize {
    simd::escape_utf16_raw_simd(utf16, dst)
}

/// Convert UTF-16 code units (`u16`) into raw UTF-8 bytes without escaping, reusing `out`.
#[inline]
pub fn escape_utf16_raw_into(utf16: &[u16], out: &mut Vec<u8>) {
    escape_into(utf16.len(), out, false, |buf| escape_utf16_raw(utf16, buf));
}

#[cfg(feature = "sonic-writeext")]
#[inline]
fn write_with<W: WriteExt>(
    writer: &mut W,
    max_units: usize,
    need_quote: bool,
    escape: impl FnOnce(&mut [MaybeUninit<u8>]) -> usize,
) -> io::Result<()> {
    let max_len = max_escaped_len(max_units, need_quote);
    if max_len == 0 {
        return Ok(());
    }
    let buf = writer.reserve_with(max_len)?;
    let len = escape(buf);
    // SAFETY: `escape` initialized the first `len` bytes in `buf`.
    unsafe {
        writer.flush_len(len)?;
    }
    Ok(())
}

#[inline]
fn escape_into(
    max_units: usize,
    out: &mut Vec<u8>,
    need_quote: bool,
    escape: impl FnOnce(&mut [MaybeUninit<u8>]) -> usize,
) {
    let needed = max_escaped_len(max_units, need_quote);
    let buf = prepare_output(out, needed);
    let len = escape(buf);
    out.truncate(len);
}

#[inline]
fn prepare_output(out: &mut Vec<u8>, needed: usize) -> &mut [MaybeUninit<u8>] {
    out.clear();
    out.resize(needed, 0);
    // SAFETY: MaybeUninit<u8> has the same layout as u8.
    unsafe { core::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut MaybeUninit<u8>, out.len()) }
}
