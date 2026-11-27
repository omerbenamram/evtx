//! Scalar UTF-16/UTF-16LE escaping and conversion routines.
//!
//! These routines provide a correctness-first fallback for platforms without
//! SIMD or for non-ASCII segments that fall out of the SIMD fast path. The
//! implementations share UTF-16 decoding helpers and emit UTF-8 bytes directly
//! into a caller-provided buffer to avoid intermediate allocations.

#![allow(unsafe_op_in_unsafe_fn)]

use core::marker::PhantomData;
use core::mem::MaybeUninit;

const HEX: [u8; 16] = *b"0123456789ABCDEF";

/// U+FFFD, which stands in for a lone surrogate outside XML.
const REPLACEMENT: &[u8] = "\u{FFFD}".as_bytes();

#[inline]
pub(crate) fn max_units(num_units: usize, byte_len: usize) -> usize {
    core::cmp::min(num_units, byte_len / 2)
}

/// Abstraction over a bounded sequence of UTF-16 code units.
///
/// # Safety
/// Implementors must guarantee that [`CodeUnits::unit`] is valid to call for
/// any `idx < self.len()`, and that `self.len()` is invariant across calls.
pub(crate) unsafe trait CodeUnits: Copy {
    fn len(self) -> usize;

    /// Read the code unit at `idx`.
    ///
    /// # Safety
    /// Caller must guarantee `idx < self.len()`.
    unsafe fn unit(self, idx: usize) -> u16;
}

/// Little-endian unaligned byte view bounded to `len` UTF-16 code units.
#[derive(Copy, Clone)]
pub(crate) struct Utf16LeUnits<'a> {
    ptr: *const u8,
    len: usize,
    _marker: PhantomData<&'a [u8]>,
}

impl<'a> Utf16LeUnits<'a> {
    #[inline(always)]
    pub(crate) fn new(utf16le: &'a [u8], num_units: usize) -> Self {
        Self {
            ptr: utf16le.as_ptr(),
            len: max_units(num_units, utf16le.len()),
            _marker: PhantomData,
        }
    }
}

// SAFETY: `len` is clamped to `utf16le.len() / 2`, so every `idx < len` stays
// within the 2-byte aligned offset range of `utf16le`.
unsafe impl CodeUnits for Utf16LeUnits<'_> {
    #[inline(always)]
    fn len(self) -> usize {
        self.len
    }

    #[inline(always)]
    unsafe fn unit(self, idx: usize) -> u16 {
        read_u16_le(self.ptr, idx)
    }
}

// SAFETY: `idx < self.len()` guarantees `self.get_unchecked(idx)` is in-bounds.
unsafe impl CodeUnits for &[u16] {
    #[inline(always)]
    fn len(self) -> usize {
        <[u16]>::len(self)
    }

    #[inline(always)]
    unsafe fn unit(self, idx: usize) -> u16 {
        *self.get_unchecked(idx)
    }
}

/// Policy trait capturing differences between JSON, XML, and raw UTF-8 output.
pub(crate) trait EscapeMode: Copy {
    fn ascii_needs_escape(self, b: u8) -> bool;
    unsafe fn write_ascii_slow(self, b: u8, dst: &mut *mut u8);
    unsafe fn write_lone_surrogate(self, dst: &mut *mut u8);
    fn replace_xml_nonchar(self, code_unit: u16) -> bool;
}

#[derive(Copy, Clone)]
pub(crate) struct JsonMode;

#[derive(Copy, Clone)]
pub(crate) struct XmlMode {
    pub in_attribute: bool,
}

#[derive(Copy, Clone)]
pub(crate) struct RawMode;

impl EscapeMode for JsonMode {
    #[inline(always)]
    fn ascii_needs_escape(self, b: u8) -> bool {
        special_escape(b).is_some() || b <= 0x1F
    }

    #[inline(always)]
    unsafe fn write_ascii_slow(self, b: u8, dst: &mut *mut u8) {
        if let Some(esc) = special_escape(b) {
            write_slice(dst, esc);
        } else if b <= 0x1F {
            write_u16_le_escape(dst, b);
        } else {
            write_byte(dst, b);
        }
    }

    #[inline(always)]
    unsafe fn write_lone_surrogate(self, dst: &mut *mut u8) {
        write_slice(dst, REPLACEMENT);
    }

    #[inline(always)]
    fn replace_xml_nonchar(self, _code_unit: u16) -> bool {
        false
    }
}

impl EscapeMode for XmlMode {
    #[inline(always)]
    fn ascii_needs_escape(self, b: u8) -> bool {
        xml_escape(b, self.in_attribute).is_some()
    }

    #[inline(always)]
    unsafe fn write_ascii_slow(self, b: u8, dst: &mut *mut u8) {
        if let Some(esc) = xml_escape(b, self.in_attribute) {
            write_slice(dst, esc);
        } else {
            write_byte(dst, b);
        }
    }

    #[inline(always)]
    unsafe fn write_lone_surrogate(self, dst: &mut *mut u8) {
        write_byte(dst, b'_');
    }

    #[inline(always)]
    fn replace_xml_nonchar(self, code_unit: u16) -> bool {
        is_xml_noncharacter(code_unit)
    }
}

impl EscapeMode for RawMode {
    #[inline(always)]
    fn ascii_needs_escape(self, _b: u8) -> bool {
        false
    }

    #[inline(always)]
    unsafe fn write_ascii_slow(self, b: u8, dst: &mut *mut u8) {
        write_byte(dst, b);
    }

    #[inline(always)]
    unsafe fn write_lone_surrogate(self, dst: &mut *mut u8) {
        write_slice(dst, REPLACEMENT);
    }

    #[inline(always)]
    fn replace_xml_nonchar(self, _code_unit: u16) -> bool {
        false
    }
}

#[inline(always)]
unsafe fn write_byte(dst: &mut *mut u8, byte: u8) {
    **dst = byte;
    *dst = dst.add(1);
}

#[inline(always)]
unsafe fn write_slice(dst: &mut *mut u8, bytes: &[u8]) {
    core::ptr::copy_nonoverlapping(bytes.as_ptr(), *dst, bytes.len());
    *dst = dst.add(bytes.len());
}

#[inline(always)]
unsafe fn write_u16_le_escape(dst: &mut *mut u8, byte: u8) {
    // Writes \u00XX
    write_byte(dst, b'\\');
    write_byte(dst, b'u');
    write_byte(dst, b'0');
    write_byte(dst, b'0');
    write_byte(dst, HEX[(byte >> 4) as usize]);
    write_byte(dst, HEX[(byte & 0x0F) as usize]);
}

#[inline(always)]
unsafe fn write_utf8_2(dst: &mut *mut u8, code_unit: u16) {
    let b0 = 0xC0 | ((code_unit >> 6) as u8);
    let b1 = 0x80 | (code_unit as u8 & 0x3F);
    write_byte(dst, b0);
    write_byte(dst, b1);
}

#[inline(always)]
unsafe fn write_utf8_3(dst: &mut *mut u8, code_unit: u16) {
    let b0 = 0xE0 | ((code_unit >> 12) as u8);
    let b1 = 0x80 | (((code_unit >> 6) as u8) & 0x3F);
    let b2 = 0x80 | (code_unit as u8 & 0x3F);
    write_byte(dst, b0);
    write_byte(dst, b1);
    write_byte(dst, b2);
}

#[inline(always)]
unsafe fn write_utf8_4(dst: &mut *mut u8, codepoint: u32) {
    let b0 = 0xF0 | ((codepoint >> 18) as u8);
    let b1 = 0x80 | (((codepoint >> 12) as u8) & 0x3F);
    let b2 = 0x80 | (((codepoint >> 6) as u8) & 0x3F);
    let b3 = 0x80 | ((codepoint as u8) & 0x3F);
    write_byte(dst, b0);
    write_byte(dst, b1);
    write_byte(dst, b2);
    write_byte(dst, b3);
}

#[inline(always)]
unsafe fn read_u16_le(bytes: *const u8, idx: usize) -> u16 {
    let ptr = bytes.add(idx * 2) as *const u16;
    u16::from_le(core::ptr::read_unaligned(ptr))
}

#[inline(always)]
fn is_surrogate(code_unit: u16) -> bool {
    (0xD800..=0xDFFF).contains(&code_unit)
}

#[inline(always)]
fn is_high_surrogate(code_unit: u16) -> bool {
    (0xD800..=0xDBFF).contains(&code_unit)
}

#[inline(always)]
fn is_low_surrogate(code_unit: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&code_unit)
}

#[inline(always)]
fn decode_surrogate_pair(hi: u16, lo: u16) -> u32 {
    let hi = (hi as u32) - 0xD800;
    let lo = (lo as u32) - 0xDC00;
    0x10000 + ((hi << 10) | lo)
}

#[inline(always)]
fn special_escape(byte: u8) -> Option<&'static [u8]> {
    match byte {
        b'"' => Some(br#"\""#),
        b'\\' => Some(br#"\\"#),
        b'\n' => Some(br#"\n"#),
        b'\r' => Some(br#"\r"#),
        b'\t' => Some(br#"\t"#),
        0x08 => Some(br#"\b"#),
        0x0C => Some(br#"\f"#),
        _ => None,
    }
}

/// Bit `b` is set for each byte `b` below 64 that XML text can't hold as it is: `&`, `<`, `>`,
/// and the C0 controls but tab, LF and CR, which XML 1.0 doesn't allow even as character
/// references (Windows writes `_` for them).
const XML_TEXT_SPECIAL: u64 = {
    let mut mask = (1u64 << 0x20) - 1;
    mask &= !(1 << b'\t' | 1 << b'\n' | 1 << b'\r');
    mask | 1 << b'&' | 1 << b'<' | 1 << b'>'
};
/// Attributes also escape both quotes.
const XML_ATTRIBUTE_SPECIAL: u64 = XML_TEXT_SPECIAL | 1 << b'"' | 1 << b'\'';

#[inline(always)]
fn xml_escape(byte: u8, in_attribute: bool) -> Option<&'static [u8]> {
    let special = if in_attribute {
        XML_ATTRIBUTE_SPECIAL
    } else {
        XML_TEXT_SPECIAL
    };
    if byte >= 64 || special >> byte & 1 == 0 {
        return None;
    }
    Some(match byte {
        b'&' => b"&amp;",
        b'<' => b"&lt;",
        b'>' => b"&gt;",
        b'"' => b"&quot;",
        b'\'' => b"&apos;",
        _ => b"_",
    })
}

/// U+FFFE and U+FFFF, which XML 1.0 doesn't allow either. Windows writes `_` for them too.
#[inline(always)]
fn is_xml_noncharacter(code_unit: u16) -> bool {
    code_unit >= 0xFFFE
}

/// Process a single UTF-16 code unit (and possibly a surrogate pair).
///
/// Returns the next code unit index.
///
/// # Safety
/// - `idx < units.len()`
/// - `dst` must have writable capacity for at least 6 bytes (worst case for one unit).
#[inline(always)]
pub(crate) unsafe fn escape_one<U: CodeUnits, M: EscapeMode>(
    units: U,
    idx: usize,
    mode: M,
    dst: &mut *mut u8,
) -> usize {
    let code_unit = units.unit(idx);

    if code_unit <= 0x7F {
        mode.write_ascii_slow(code_unit as u8, dst);
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_surrogate(code_unit) {
        if is_high_surrogate(code_unit) && idx + 1 < units.len() {
            let lo = units.unit(idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        // A lone surrogate.
        mode.write_lone_surrogate(dst);
        return idx + 1;
    }

    if mode.replace_xml_nonchar(code_unit) {
        write_byte(dst, b'_');
    } else {
        write_utf8_3(dst, code_unit);
    }
    idx + 1
}

#[inline(always)]
fn escape_scalar<U: CodeUnits, M: EscapeMode>(
    units: U,
    mode: M,
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    let max_units = units.len();
    let max_len = crate::max_escaped_len(max_units, need_quote);
    assert!(dst.len() >= max_len);

    // SAFETY: `dst` is write-only `MaybeUninit<u8>` and `dst.len() >= max_len`
    // bounds all bytes written by quotes, the ASCII fast path, and `escape_one`.
    unsafe {
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        if need_quote {
            write_byte(&mut dst_ptr, b'"');
        }

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units that need no escaping.
            while idx < max_units {
                let cu = units.unit(idx);
                if cu > 0x7F {
                    break;
                }
                let b = cu as u8;
                if mode.ascii_needs_escape(b) {
                    break;
                }
                write_byte(&mut dst_ptr, b);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one(units, idx, mode, &mut dst_ptr);
        }

        if need_quote {
            write_byte(&mut dst_ptr, b'"');
        }

        dst_ptr as usize - dst_start as usize
    }
}

/// Scalar JSON escaping for UTF-16LE input.
pub fn escape_json_utf16le_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    escape_scalar(
        Utf16LeUnits::new(utf16le, num_units),
        JsonMode,
        dst,
        need_quote,
    )
}

/// Scalar XML escaping for UTF-16LE input.
pub fn escape_xml_utf16le_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    escape_scalar(
        Utf16LeUnits::new(utf16le, num_units),
        XmlMode { in_attribute },
        dst,
        false,
    )
}

/// Scalar UTF-16LE -> UTF-8 conversion with no escaping.
pub fn escape_utf16le_raw_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
) -> usize {
    escape_scalar(Utf16LeUnits::new(utf16le, num_units), RawMode, dst, false)
}

/// Scalar JSON escaping for UTF-16 code units (`u16` slice).
pub fn escape_json_utf16_scalar(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    escape_scalar(utf16, JsonMode, dst, need_quote)
}

/// Scalar XML escaping for UTF-16 code units (`u16` slice).
pub fn escape_xml_utf16_scalar(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    escape_scalar(utf16, XmlMode { in_attribute }, dst, false)
}

/// Scalar UTF-16 (`u16` slice) -> UTF-8 conversion with no escaping.
pub fn escape_utf16_raw_scalar(utf16: &[u16], dst: &mut [MaybeUninit<u8>]) -> usize {
    escape_scalar(utf16, RawMode, dst, false)
}
