#![no_main]

use core::mem::MaybeUninit;

use libfuzzer_sys::fuzz_target;

use utf16_simd::{escape_utf16le_raw, escape_utf16le_raw_scalar, max_escaped_len};

const MAX_UNITS: usize = 4096;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }

    let utf16le = &data[..];
    let num_units = ((utf16le.len() + 1) / 2).min(MAX_UNITS);

    let mut out_simd = vec![MaybeUninit::uninit(); max_escaped_len(num_units, false)];
    let mut out_scalar = vec![MaybeUninit::uninit(); max_escaped_len(num_units, false)];

    let len_simd = escape_utf16le_raw(utf16le, num_units, &mut out_simd);
    let len_scalar = escape_utf16le_raw_scalar(utf16le, num_units, &mut out_scalar);

    // SAFETY: the escape functions guarantee the first `len_*` bytes are initialized.
    let simd_bytes = unsafe { core::slice::from_raw_parts(out_simd.as_ptr() as *const u8, len_simd) };
    let scalar_bytes =
        unsafe { core::slice::from_raw_parts(out_scalar.as_ptr() as *const u8, len_scalar) };

    assert_eq!(simd_bytes, scalar_bytes);
    core::str::from_utf8(simd_bytes).unwrap();
});

