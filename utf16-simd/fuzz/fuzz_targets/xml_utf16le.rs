#![no_main]

use core::mem::MaybeUninit;

use libfuzzer_sys::fuzz_target;

use utf16_simd::{escape_xml_utf16le, escape_xml_utf16le_scalar, max_escaped_len};

const MAX_UNITS: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let in_attribute = (data[0] & 1) != 0;
    let utf16le = &data[1..];
    let num_units = ((utf16le.len() + 1) / 2).min(MAX_UNITS);

    let mut out_simd = vec![MaybeUninit::uninit(); max_escaped_len(num_units, false)];
    let mut out_scalar = vec![MaybeUninit::uninit(); max_escaped_len(num_units, false)];

    let len_simd = escape_xml_utf16le(utf16le, num_units, &mut out_simd, in_attribute);
    let len_scalar = escape_xml_utf16le_scalar(utf16le, num_units, &mut out_scalar, in_attribute);

    // SAFETY: the escape functions guarantee the first `len_*` bytes are initialized.
    let simd_bytes = unsafe { core::slice::from_raw_parts(out_simd.as_ptr() as *const u8, len_simd) };
    let scalar_bytes =
        unsafe { core::slice::from_raw_parts(out_scalar.as_ptr() as *const u8, len_scalar) };

    assert_eq!(simd_bytes, scalar_bytes);
    core::str::from_utf8(simd_bytes).unwrap();
});

