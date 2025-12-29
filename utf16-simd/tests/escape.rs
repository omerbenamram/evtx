use std::mem::MaybeUninit;

use sonic_rs::format::{CompactFormatter, Formatter};

use utf16_simd::{
    Scratch, escape_json_utf16, escape_json_utf16_scalar, escape_json_utf16le,
    escape_json_utf16le_scalar, escape_utf16_raw, escape_utf16_raw_scalar, escape_utf16le_raw,
    escape_utf16le_raw_scalar, escape_xml_utf16, escape_xml_utf16_scalar, escape_xml_utf16le,
    escape_xml_utf16le_scalar, max_escaped_len,
};

fn utf16le_from_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn max_units(bytes: &[u8], num_units: usize) -> usize {
    std::cmp::min(num_units, bytes.len() / 2)
}

fn u16s_from_utf16le(bytes: &[u8], num_units: usize) -> Vec<u16> {
    let max = max_units(bytes, num_units);
    bytes
        .chunks_exact(2)
        .take(max)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Convert UTF-16LE into a UTF-8 `String`, skipping invalid surrogate code units.
///
/// This matches the crate's behavior of silently dropping lone surrogates.
fn utf16le_to_string_skip_invalid(bytes: &[u8], num_units: usize) -> String {
    let units = u16s_from_utf16le(bytes, num_units);
    let mut out = String::new();
    for r in std::char::decode_utf16(units.into_iter()) {
        if let Ok(ch) = r {
            out.push(ch);
        }
    }
    out
}

fn uppercase_json_u00xx_escapes_in_place(out: &mut [u8]) {
    // sonic-rs uses lowercase hex digits in `\u00xx` escapes; this crate uses uppercase.
    // JSON treats both as equivalent, but for byte-for-byte comparisons we normalize.
    let mut i = 0usize;
    while i + 5 < out.len() {
        if out[i] == b'\\' && out[i + 1] == b'u' && out[i + 2] == b'0' && out[i + 3] == b'0' {
            // Only treat this '\' as an escape initiator if it's at an odd position within a
            // run of consecutive backslashes (so we don't rewrite literal "\\u00ff" content).
            let mut j = i;
            while j > 0 && out[j - 1] == b'\\' {
                j -= 1;
            }
            let run_len = i - j + 1;
            if run_len % 2 == 1 {
                out[i + 4] = out[i + 4].to_ascii_uppercase();
                out[i + 5] = out[i + 5].to_ascii_uppercase();
            }
        }
        i += 1;
    }
}

fn json_reference(bytes: &[u8], num_units: usize, need_quote: bool) -> Vec<u8> {
    let s = utf16le_to_string_skip_invalid(bytes, num_units);
    let mut fmt = CompactFormatter;
    let mut out = Vec::new();
    fmt.write_string_fast(&mut out, &s, need_quote).unwrap();
    uppercase_json_u00xx_escapes_in_place(&mut out);
    out
}

fn xml_reference(bytes: &[u8], num_units: usize, in_attribute: bool) -> Vec<u8> {
    let s = utf16le_to_string_skip_invalid(bytes, num_units);
    let mut out = String::new();
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if in_attribute => out.push_str("&quot;"),
            '\'' if in_attribute => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out.into_bytes()
}

fn raw_reference(bytes: &[u8], num_units: usize) -> Vec<u8> {
    utf16le_to_string_skip_invalid(bytes, num_units).into_bytes()
}

fn escape_to_vec(buf: &[MaybeUninit<u8>], len: usize) -> Vec<u8> {
    unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, len) }.to_vec()
}

fn run_case_json(bytes: &[u8], num_units: usize, need_quote: bool) {
    let max = max_units(bytes, num_units);
    let expected = json_reference(bytes, num_units, need_quote);

    let mut scalar_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, need_quote)];
    let mut simd_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, need_quote)];

    let scalar_len = escape_json_utf16le_scalar(bytes, num_units, &mut scalar_buf, need_quote);
    let simd_len = escape_json_utf16le(bytes, num_units, &mut simd_buf, need_quote);

    let scalar = escape_to_vec(&scalar_buf, scalar_len);
    let simd = escape_to_vec(&simd_buf, simd_len);

    assert_eq!(scalar, simd);
    assert_eq!(simd, expected);

    let mut scratch = Scratch::new();
    let scratch_out_le = scratch.escape_json_utf16le(bytes, num_units, need_quote);
    assert_eq!(scratch_out_le, expected.as_slice());

    let mut w = Vec::new();
    scratch
        .write_json_utf16le_to(&mut w, bytes, num_units, need_quote)
        .unwrap();
    assert_eq!(w, expected);

    // UTF-16 (u16) API parity.
    let units = u16s_from_utf16le(bytes, num_units);
    let mut scalar_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), need_quote)];
    let mut simd_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), need_quote)];
    let scalar_u16_len = escape_json_utf16_scalar(&units, &mut scalar_u16_buf, need_quote);
    let simd_u16_len = escape_json_utf16(&units, &mut simd_u16_buf, need_quote);
    let scalar_u16 = escape_to_vec(&scalar_u16_buf, scalar_u16_len);
    let simd_u16 = escape_to_vec(&simd_u16_buf, simd_u16_len);
    assert_eq!(scalar_u16, expected);
    assert_eq!(simd_u16, expected);

    let scratch_out_u16 = scratch.escape_json_utf16(&units, need_quote);
    assert_eq!(scratch_out_u16, expected.as_slice());
}

fn run_case_xml(bytes: &[u8], num_units: usize, in_attribute: bool) {
    let max = max_units(bytes, num_units);
    let expected = xml_reference(bytes, num_units, in_attribute);

    let mut scalar_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, false)];
    let mut simd_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, false)];

    let scalar_len = escape_xml_utf16le_scalar(bytes, num_units, &mut scalar_buf, in_attribute);
    let simd_len = escape_xml_utf16le(bytes, num_units, &mut simd_buf, in_attribute);

    let scalar = escape_to_vec(&scalar_buf, scalar_len);
    let simd = escape_to_vec(&simd_buf, simd_len);

    assert_eq!(scalar, simd);
    assert_eq!(simd, expected);

    let mut scratch = Scratch::new();
    let scratch_out_le = scratch.escape_xml_utf16le(bytes, num_units, in_attribute);
    assert_eq!(scratch_out_le, expected.as_slice());

    let mut w = Vec::new();
    scratch
        .write_xml_utf16le_to(&mut w, bytes, num_units, in_attribute)
        .unwrap();
    assert_eq!(w, expected);

    // UTF-16 (u16) API parity.
    let units = u16s_from_utf16le(bytes, num_units);
    let mut scalar_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), false)];
    let mut simd_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), false)];
    let scalar_u16_len = escape_xml_utf16_scalar(&units, &mut scalar_u16_buf, in_attribute);
    let simd_u16_len = escape_xml_utf16(&units, &mut simd_u16_buf, in_attribute);
    let scalar_u16 = escape_to_vec(&scalar_u16_buf, scalar_u16_len);
    let simd_u16 = escape_to_vec(&simd_u16_buf, simd_u16_len);
    assert_eq!(scalar_u16, expected);
    assert_eq!(simd_u16, expected);

    let scratch_out_u16 = scratch.escape_xml_utf16(&units, in_attribute);
    assert_eq!(scratch_out_u16, expected.as_slice());
}

fn run_case_raw(bytes: &[u8], num_units: usize) {
    let max = max_units(bytes, num_units);
    let expected = raw_reference(bytes, num_units);

    let mut scalar_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, false)];
    let mut simd_buf = vec![MaybeUninit::uninit(); max_escaped_len(max, false)];

    let scalar_len = escape_utf16le_raw_scalar(bytes, num_units, &mut scalar_buf);
    let simd_len = escape_utf16le_raw(bytes, num_units, &mut simd_buf);

    let scalar = escape_to_vec(&scalar_buf, scalar_len);
    let simd = escape_to_vec(&simd_buf, simd_len);

    assert_eq!(scalar, simd);
    assert_eq!(simd, expected);

    let mut scratch = Scratch::new();
    let scratch_out_le = scratch.escape_utf16le_raw(bytes, num_units);
    assert_eq!(scratch_out_le, expected.as_slice());

    let mut w = Vec::new();
    scratch.write_utf16le_raw_to(&mut w, bytes, num_units).unwrap();
    assert_eq!(w, expected);

    // UTF-16 (u16) API parity.
    let units = u16s_from_utf16le(bytes, num_units);
    let mut scalar_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), false)];
    let mut simd_u16_buf = vec![MaybeUninit::uninit(); max_escaped_len(units.len(), false)];
    let scalar_u16_len = escape_utf16_raw_scalar(&units, &mut scalar_u16_buf);
    let simd_u16_len = escape_utf16_raw(&units, &mut simd_u16_buf);
    let scalar_u16 = escape_to_vec(&scalar_u16_buf, scalar_u16_len);
    let simd_u16 = escape_to_vec(&simd_u16_buf, simd_u16_len);
    assert_eq!(scalar_u16, expected);
    assert_eq!(simd_u16, expected);

    let scratch_out_u16 = scratch.escape_utf16_raw(&units);
    assert_eq!(scratch_out_u16, expected.as_slice());
}

#[test]
fn ascii() {
    let bytes = utf16le_from_str("Hello &<>\"' World");
    run_case_json(&bytes, bytes.len() / 2, true);
    run_case_xml(&bytes, bytes.len() / 2, false);
    run_case_xml(&bytes, bytes.len() / 2, true);
    run_case_raw(&bytes, bytes.len() / 2);
}

#[test]
fn long_ascii() {
    let bytes = utf16le_from_str("aaaaaaa&bbbbbbb&ccccccc<dddddd>eeeeee\"fffffff'gggggg");
    run_case_json(&bytes, bytes.len() / 2, true);
    run_case_xml(&bytes, bytes.len() / 2, false);
    run_case_xml(&bytes, bytes.len() / 2, true);
    run_case_raw(&bytes, bytes.len() / 2);
}

#[test]
fn euro_sign() {
    let bytes = vec![0xAC, 0x20];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn e_acute() {
    let bytes = vec![0xE9, 0x00];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn two_byte_max() {
    let bytes = vec![0xFF, 0x07];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn grinning_face() {
    let bytes = vec![0x3D, 0xD8, 0x00, 0xDE];
    run_case_json(&bytes, 2, true);
    run_case_xml(&bytes, 2, false);
    run_case_raw(&bytes, 2);
}

#[test]
fn hi_only_surrogate() {
    let bytes = vec![0x00, 0xD8];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn lo_only_surrogate() {
    let bytes = vec![0x00, 0xDC];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn control_1f() {
    let bytes = vec![0x1F, 0x00];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn newline() {
    let bytes = vec![b'\n', 0x00];
    run_case_json(&bytes, 1, true);
    run_case_xml(&bytes, 1, false);
    run_case_raw(&bytes, 1);
}

#[test]
fn json_need_quote_false() {
    let bytes = utf16le_from_str("a\"b\\c");
    run_case_json(&bytes, bytes.len() / 2, false);
}

#[test]
fn empty_string_writes_quotes_when_requested() {
    run_case_json(&[], 0, true);
    run_case_json(&[], 0, false);
}

#[test]
fn odd_byte_len_ignores_trailing_byte() {
    // "A" in UTF-16LE plus one dangling byte.
    let bytes = [b'A', 0x00, 0xFF];
    run_case_json(&bytes, 2, true);
    run_case_xml(&bytes, 2, false);
    run_case_raw(&bytes, 2);
}

#[test]
fn surrogate_pair_across_simd_block_boundary() {
    // 7 ASCII 'a', then 😀 split across the 8-wide SIMD boundary, then 'b'.
    let mut units: Vec<u16> = vec![b'a' as u16; 7];
    units.push(0xD83D);
    units.push(0xDE00);
    units.push(b'b' as u16);

    let mut bytes = Vec::with_capacity(units.len() * 2);
    for u in units {
        bytes.extend_from_slice(&u.to_le_bytes());
    }

    run_case_json(&bytes, bytes.len() / 2, false);
    run_case_xml(&bytes, bytes.len() / 2, false);
    run_case_raw(&bytes, bytes.len() / 2);
}

#[test]
fn high_surrogate_at_simd_block_end_not_followed_by_low_is_dropped() {
    // 7 ASCII 'a', then a lone high surrogate, then 'b'.
    let mut units: Vec<u16> = vec![b'a' as u16; 7];
    units.push(0xD83D);
    units.push(b'b' as u16);

    let mut bytes = Vec::with_capacity(units.len() * 2);
    for u in units {
        bytes.extend_from_slice(&u.to_le_bytes());
    }

    run_case_json(&bytes, bytes.len() / 2, false);
    run_case_xml(&bytes, bytes.len() / 2, false);
    run_case_raw(&bytes, bytes.len() / 2);
}
