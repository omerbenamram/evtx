//! Scalar UTF-16LE escaping/conversion routines.
//!
//! These routines provide a correctness-first fallback for platforms without
//! SIMD or for non-ASCII segments that fall out of the SIMD fast path. The
//! implementations share UTF-16 decoding helpers and emit UTF-8 bytes directly
//! into a caller-provided buffer to avoid intermediate allocations.

#![allow(unsafe_op_in_unsafe_fn)]

use core::mem::MaybeUninit;

const HEX: [u8; 16] = *b"0123456789ABCDEF";

#[inline]
pub(crate) fn max_units(num_units: usize, byte_len: usize) -> usize {
    core::cmp::min(num_units, byte_len / 2)
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
    // Writes \\u00XX
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

#[inline(always)]
fn xml_escape(byte: u8, in_attribute: bool) -> Option<&'static [u8]> {
    match byte {
        b'&' => Some(b"&amp;"),
        b'<' => Some(b"&lt;"),
        b'>' => Some(b"&gt;"),
        b'"' if in_attribute => Some(b"&quot;"),
        b'\'' if in_attribute => Some(b"&apos;"),
        _ => None,
    }
}

/// Process a single UTF-16 code unit (and possibly a surrogate pair).
///
/// Returns the next code unit index.
pub(crate) unsafe fn escape_one(
    bytes: *const u8,
    max_units: usize,
    idx: usize,
    dst: &mut *mut u8,
) -> usize {
    let code_unit = read_u16_le(bytes, idx);

    if code_unit <= 0x7F {
        let b = code_unit as u8;
        if let Some(esc) = special_escape(b) {
            write_slice(dst, esc);
        } else if b <= 0x1F {
            write_u16_le_escape(dst, b);
        } else {
            write_byte(dst, b);
        }
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < max_units {
            let lo = read_u16_le(bytes, idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        // Lone high surrogate: skip.
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        // Lone low surrogate: skip.
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

#[inline(always)]
pub(crate) unsafe fn escape_one_xml(
    bytes: *const u8,
    max_units: usize,
    idx: usize,
    dst: &mut *mut u8,
    in_attribute: bool,
) -> usize {
    let code_unit = read_u16_le(bytes, idx);

    if code_unit <= 0x7F {
        let b = code_unit as u8;
        if let Some(esc) = xml_escape(b, in_attribute) {
            write_slice(dst, esc);
        } else {
            write_byte(dst, b);
        }
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < max_units {
            let lo = read_u16_le(bytes, idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

#[inline(always)]
pub(crate) unsafe fn escape_one_raw(
    bytes: *const u8,
    max_units: usize,
    idx: usize,
    dst: &mut *mut u8,
) -> usize {
    let code_unit = read_u16_le(bytes, idx);

    if code_unit <= 0x7F {
        write_byte(dst, code_unit as u8);
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < max_units {
            let lo = read_u16_le(bytes, idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

/// Scalar JSON escaping for UTF-16LE input.
pub fn escape_json_utf16le_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    let max_units = max_units(num_units, utf16le.len());
    let max_len = crate::max_escaped_len(max_units, need_quote);
    assert!(dst.len() >= max_len);

    unsafe {
        let src_ptr = utf16le.as_ptr();
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        if need_quote {
            write_byte(&mut dst_ptr, b'"');
        }

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units that need no escaping.
            while idx < max_units {
                let lo = *src_ptr.add(idx * 2);
                let hi = *src_ptr.add(idx * 2 + 1);
                if hi != 0 || lo > 0x7F {
                    break;
                }
                if special_escape(lo).is_some() || lo <= 0x1F {
                    break;
                }
                write_byte(&mut dst_ptr, lo);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one(src_ptr, max_units, idx, &mut dst_ptr);
        }

        if need_quote {
            write_byte(&mut dst_ptr, b'"');
        }

        dst_ptr as usize - dst_start as usize
    }
}

/// Scalar XML escaping for UTF-16LE input.
pub fn escape_xml_utf16le_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    let max_units = max_units(num_units, utf16le.len());
    let max_len = crate::max_escaped_len(max_units, false);
    assert!(dst.len() >= max_len);

    unsafe {
        let src_ptr = utf16le.as_ptr();
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units that need no escaping.
            while idx < max_units {
                let lo = *src_ptr.add(idx * 2);
                let hi = *src_ptr.add(idx * 2 + 1);
                if hi != 0 || lo > 0x7F {
                    break;
                }
                if xml_escape(lo, in_attribute).is_some() {
                    break;
                }
                write_byte(&mut dst_ptr, lo);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one_xml(src_ptr, max_units, idx, &mut dst_ptr, in_attribute);
        }

        dst_ptr as usize - dst_start as usize
    }
}

/// Scalar UTF-16LE -> UTF-8 conversion with no escaping.
pub fn escape_utf16le_raw_scalar(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
) -> usize {
    let max_units = max_units(num_units, utf16le.len());
    let max_len = crate::max_escaped_len(max_units, false);
    assert!(dst.len() >= max_len);

    unsafe {
        let src_ptr = utf16le.as_ptr();
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units directly.
            while idx < max_units {
                let lo = *src_ptr.add(idx * 2);
                let hi = *src_ptr.add(idx * 2 + 1);
                if hi != 0 || lo > 0x7F {
                    break;
                }
                write_byte(&mut dst_ptr, lo);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one_raw(src_ptr, max_units, idx, &mut dst_ptr);
        }

        dst_ptr as usize - dst_start as usize
    }
}

// ============================================================================
// UTF-16 (u16 slice) scalar routines
// ============================================================================

/// Process a single UTF-16 code unit from a `&[u16]` slice (and possibly a surrogate pair).
///
/// Returns the next code unit index.
pub(crate) unsafe fn escape_one_u16(utf16: &[u16], idx: usize, dst: &mut *mut u8) -> usize {
    let code_unit = *utf16.get_unchecked(idx);

    if code_unit <= 0x7F {
        let b = code_unit as u8;
        if let Some(esc) = special_escape(b) {
            write_slice(dst, esc);
        } else if b <= 0x1F {
            write_u16_le_escape(dst, b);
        } else {
            write_byte(dst, b);
        }
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < utf16.len() {
            let lo = *utf16.get_unchecked(idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        // Lone high surrogate: skip.
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        // Lone low surrogate: skip.
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

pub(crate) unsafe fn escape_one_xml_u16(
    utf16: &[u16],
    idx: usize,
    dst: &mut *mut u8,
    in_attribute: bool,
) -> usize {
    let code_unit = *utf16.get_unchecked(idx);

    if code_unit <= 0x7F {
        let b = code_unit as u8;
        if let Some(esc) = xml_escape(b, in_attribute) {
            write_slice(dst, esc);
        } else {
            write_byte(dst, b);
        }
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < utf16.len() {
            let lo = *utf16.get_unchecked(idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

pub(crate) unsafe fn escape_one_raw_u16(utf16: &[u16], idx: usize, dst: &mut *mut u8) -> usize {
    let code_unit = *utf16.get_unchecked(idx);

    if code_unit <= 0x7F {
        write_byte(dst, code_unit as u8);
        return idx + 1;
    }

    if code_unit < 0x800 {
        write_utf8_2(dst, code_unit);
        return idx + 1;
    }

    if is_high_surrogate(code_unit) {
        if idx + 1 < utf16.len() {
            let lo = *utf16.get_unchecked(idx + 1);
            if is_low_surrogate(lo) {
                let codepoint = decode_surrogate_pair(code_unit, lo);
                write_utf8_4(dst, codepoint);
                return idx + 2;
            }
        }
        return idx + 1;
    }

    if is_low_surrogate(code_unit) {
        return idx + 1;
    }

    write_utf8_3(dst, code_unit);
    idx + 1
}

/// Scalar JSON escaping for UTF-16 code units (`u16` slice).
pub fn escape_json_utf16_scalar(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    let max_units = utf16.len();
    let max_len = crate::max_escaped_len(max_units, need_quote);
    assert!(dst.len() >= max_len);

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
                let cu = *utf16.get_unchecked(idx);
                if cu > 0x7F {
                    break;
                }
                let b = cu as u8;
                if special_escape(b).is_some() || b <= 0x1F {
                    break;
                }
                write_byte(&mut dst_ptr, b);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one_u16(utf16, idx, &mut dst_ptr);
        }

        if need_quote {
            write_byte(&mut dst_ptr, b'"');
        }

        dst_ptr as usize - dst_start as usize
    }
}

/// Scalar XML escaping for UTF-16 code units (`u16` slice).
pub fn escape_xml_utf16_scalar(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    let max_units = utf16.len();
    let max_len = crate::max_escaped_len(max_units, false);
    assert!(dst.len() >= max_len);

    unsafe {
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units that need no escaping.
            while idx < max_units {
                let cu = *utf16.get_unchecked(idx);
                if cu > 0x7F {
                    break;
                }
                let b = cu as u8;
                if xml_escape(b, in_attribute).is_some() {
                    break;
                }
                write_byte(&mut dst_ptr, b);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one_xml_u16(utf16, idx, &mut dst_ptr, in_attribute);
        }

        dst_ptr as usize - dst_start as usize
    }
}

/// Scalar UTF-16 (`u16` slice) -> UTF-8 conversion with no escaping.
pub fn escape_utf16_raw_scalar(utf16: &[u16], dst: &mut [MaybeUninit<u8>]) -> usize {
    let max_units = utf16.len();
    let max_len = crate::max_escaped_len(max_units, false);
    assert!(dst.len() >= max_len);

    unsafe {
        let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        let mut idx = 0;
        while idx < max_units {
            // ASCII fast path: copy runs of ASCII code units directly.
            while idx < max_units {
                let cu = *utf16.get_unchecked(idx);
                if cu > 0x7F {
                    break;
                }
                write_byte(&mut dst_ptr, cu as u8);
                idx += 1;
            }

            if idx >= max_units {
                break;
            }
            idx = escape_one_raw_u16(utf16, idx, &mut dst_ptr);
        }

        dst_ptr as usize - dst_start as usize
    }
}
