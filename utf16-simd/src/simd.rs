//! SIMD front-end for UTF-16/UTF-16LE escaping and conversion.
//!
//! Goal: keep SIMD code small and non-duplicative.
//!
//! Strategy:
//! - SIMD is used only for the very common case: 8 UTF-16 code units that are
//!   ASCII and require **no escaping**. In that case we can pack+store 8 bytes
//!   at once.
//! - Everything else (escapes, non-ASCII, surrogates) is handled by the scalar
//!   per-code-unit routines. This avoids maintaining two copies of the "slow
//!   path" per architecture.
//!
//! Implementations:
//! - x86_64: SSE2 (runtime detected)
//! - aarch64: NEON (baseline)

// This module intentionally uses raw pointers and SIMD intrinsics.
//
// We keep the `unsafe` surface area small (mostly in the architecture glue) and
// accompany raw pointer writes / assumptions with explicit `SAFETY:` comments.
#![allow(unsafe_op_in_unsafe_fn)]

use core::mem::MaybeUninit;

// ---------------------------------------------------------------------------
// Macro structure
//
// The high-level control flow is identical across:
// - input representation: UTF-16LE bytes (`&[u8]`) vs native code units (`&[u16]`)
// - escaping mode: JSON vs XML vs raw
// - architecture: SSE2 vs NEON
//
// Each architecture module (`x86` / `neon`) provides a tiny shim API:
// - `load_utf16le(*const u8, idx_units) -> Vec`
// - `load_utf16(*const u16, idx_units) -> Vec`
// - `store_ascii8(*mut u8, Vec)`
// - `json_consts() / xml_consts() / raw_consts()`
// - `json_scan() / xml_scan() / raw_scan()`
//
// `escape_simd_loop!` implements the shared 8-unit chunk loop and scalar
// fallback/tail on top of that shim, and `impl_arch_escapers!` instantiates all
// 6 mode × input combinations inside each architecture module.
//
// NOTE: These macros expand inside `x86`/`neon` modules. We use fully-qualified
// `core::mem::MaybeUninit` so we don't depend on module imports.
// ---------------------------------------------------------------------------

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
macro_rules! escape_simd_loop {
    (
        units: $units:expr,
        src_ptr: $src_ptr:expr,
        dst: $dst:expr,
        need_quote: $need_quote:expr,
        mode: $mode:expr,
        load: $load:ident,
        consts: $consts:expr,
        scan: |$v:ident, $c:ident| $scan:expr $(,)?
    ) => {{
        let units = $units;
        let max_units = crate::scalar::CodeUnits::len(units);
        let max_len = crate::max_escaped_len(max_units, $need_quote);
        assert!($dst.len() >= max_len);

        let src_ptr = $src_ptr;
        // SAFETY: `$dst` is a `MaybeUninit<u8>` output buffer. We only ever
        // write bytes into it (never read from it). `max_len` is an upper bound
        // for *all* writes done by this function (including quotes, SIMD stores,
        // and scalar fallback helpers), and we asserted `$dst.len() >= max_len`.
        let mut dst_ptr = $dst.as_mut_ptr() as *mut u8;
        let dst_start = dst_ptr;

        if $need_quote {
            // SAFETY: `max_len` accounts for the opening and closing quotes and
            // we asserted `$dst.len() >= max_len`.
            *dst_ptr = b'"';
            dst_ptr = dst_ptr.add(1);
        }

        let $c = $consts;

        let mut idx = 0usize;
        // Process full 8-unit chunks. `$load` performs a 16-byte load.
        while idx + 8 <= max_units {
            // SAFETY: `idx + 8 <= max_units` and `max_units` is bounded by the
            // input slice length, so loading 8 code units (16 bytes) is in-bounds.
            let $v = $load(src_ptr, idx);
            let len = $scan;

            // SAFETY: Always store 8 packed bytes. Because `idx + 8 <= max_units`
            // and `max_len` reserves 6 bytes per code unit (plus closing quote if
            // needed), at least `6 * 8 = 48 >= 8` bytes of writable capacity remain
            // from `dst_ptr`. On the fast path (`len == 8`), all 8 bytes are valid;
            // when `len < 8`, only the first `len` bytes are retained and the
            // remaining garbage bytes are overwritten by subsequent scalar writes
            // (or remain in the uninitialized tail beyond the returned length).
            store_ascii8(dst_ptr, $v);
            dst_ptr = dst_ptr.add(len);
            idx += len;

            if len < 8 && idx < max_units {
                // Scalar fallback for the lane that requires escaping or UTF-8 encoding.
                idx = crate::scalar::escape_one(units, idx, $mode, &mut dst_ptr);
            }
        }

        while idx < max_units {
            idx = crate::scalar::escape_one(units, idx, $mode, &mut dst_ptr);
        }

        if $need_quote {
            // SAFETY: same reasoning as the opening quote: `max_len` bounds all
            // writes and includes space for the closing quote.
            *dst_ptr = b'"';
            dst_ptr = dst_ptr.add(1);
        }

        dst_ptr as usize - dst_start as usize
    }};
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
macro_rules! impl_arch_mode {
    (
        $(#[$m:meta])*
        utf16le: $le_fn:ident,
        utf16: $u16_fn:ident,
        $(flag: $flag:ident : $flag_ty:ty,)?
        need_quote: $need_quote:expr,
        mode: $mode:expr,
        consts: $consts:expr,
        scan: |$v:ident, $c:ident| $scan:expr $(,)?
    ) => {
        $(#[$m])*
        pub unsafe fn $le_fn(
            utf16le: &[u8],
            num_units: usize,
            dst: &mut [core::mem::MaybeUninit<u8>],
            $($flag: $flag_ty)?
        ) -> usize {
            escape_simd_loop!(
                units: crate::scalar::Utf16LeUnits::new(utf16le, num_units),
                src_ptr: utf16le.as_ptr(),
                dst: dst,
                need_quote: $need_quote,
                mode: $mode,
                load: load_utf16le,
                consts: $consts,
                scan: |$v, $c| $scan,
            )
        }

        $(#[$m])*
        pub unsafe fn $u16_fn(
            utf16: &[u16],
            dst: &mut [core::mem::MaybeUninit<u8>],
            $($flag: $flag_ty)?
        ) -> usize {
            escape_simd_loop!(
                units: utf16,
                src_ptr: utf16.as_ptr(),
                dst: dst,
                need_quote: $need_quote,
                mode: $mode,
                load: load_utf16,
                consts: $consts,
                scan: |$v, $c| $scan,
            )
        }
    };
}

#[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
macro_rules! impl_arch_escapers {
    ($(#[$m:meta])*) => {
        impl_arch_mode!(
            $(#[$m])*
            utf16le: escape_json_utf16le_simd,
            utf16: escape_json_utf16_simd,
            flag: need_quote: bool,
            need_quote: need_quote,
            mode: crate::scalar::JsonMode,
            consts: json_consts(),
            scan: |v, c| json_scan(v, c),
        );
        impl_arch_mode!(
            $(#[$m])*
            utf16le: escape_xml_utf16le_simd,
            utf16: escape_xml_utf16_simd,
            flag: in_attribute: bool,
            need_quote: false,
            mode: crate::scalar::XmlMode { in_attribute },
            consts: xml_consts(),
            scan: |v, c| xml_scan(v, c, in_attribute),
        );
        impl_arch_mode!(
            $(#[$m])*
            utf16le: escape_utf16le_raw_simd,
            utf16: escape_utf16_raw_simd,
            need_quote: false,
            mode: crate::scalar::RawMode,
            consts: raw_consts(),
            scan: |v, c| raw_scan(v, c),
        );
    };
}

macro_rules! impl_simd_dispatch {
    (
        $(
            $(#[$meta:meta])*
            fn $name:ident($($arg:ident : $ty:ty),* $(,)?) -> $scalar:ident;
        )*
    ) => {
        $(
            $(#[$meta])*
            #[inline]
            pub fn $name($($arg: $ty),*) -> usize {
                #[cfg(target_arch = "x86_64")]
                {
                    if std::arch::is_x86_feature_detected!("sse2") {
                        // SAFETY:
                        // - We only call the SSE2 implementation when the CPU reports
                        //   SSE2 support (`is_x86_feature_detected!("sse2")`).
                        // - The SSE2 implementation is `#[target_feature(enable = "sse2")]`
                        //   so it may freely use SSE2 intrinsics.
                        unsafe {
                            return x86::$name($($arg),*);
                        }
                    }
                }

                #[cfg(target_arch = "aarch64")]
                {
                    // SAFETY:
                    // - NEON is mandatory on aarch64.
                    // - The implementation is still `#[target_feature(enable = "neon")]`,
                    //   which makes the call `unsafe` in Rust.
                    unsafe {
                        return neon::$name($($arg),*);
                    }
                }

                #[cfg(not(target_arch = "aarch64"))]
                {
                    crate::scalar::$scalar($($arg),*)
                }
            }
        )*
    };
}

impl_simd_dispatch! {
    /// SIMD-accelerated JSON escaping for UTF-16LE input.
    fn escape_json_utf16le_simd(
        utf16le: &[u8],
        num_units: usize,
        dst: &mut [MaybeUninit<u8>],
        need_quote: bool,
    ) -> escape_json_utf16le_scalar;

    /// SIMD-accelerated XML escaping for UTF-16LE input.
    fn escape_xml_utf16le_simd(
        utf16le: &[u8],
        num_units: usize,
        dst: &mut [MaybeUninit<u8>],
        in_attribute: bool,
    ) -> escape_xml_utf16le_scalar;

    /// SIMD-accelerated UTF-16LE to UTF-8 conversion with no escaping.
    fn escape_utf16le_raw_simd(
        utf16le: &[u8],
        num_units: usize,
        dst: &mut [MaybeUninit<u8>],
    ) -> escape_utf16le_raw_scalar;

    /// SIMD-accelerated JSON escaping for UTF-16 code units (`u16` slice).
    fn escape_json_utf16_simd(
        utf16: &[u16],
        dst: &mut [MaybeUninit<u8>],
        need_quote: bool,
    ) -> escape_json_utf16_scalar;

    /// SIMD-accelerated XML escaping for UTF-16 code units (`u16` slice).
    fn escape_xml_utf16_simd(
        utf16: &[u16],
        dst: &mut [MaybeUninit<u8>],
        in_attribute: bool,
    ) -> escape_xml_utf16_scalar;

    /// SIMD-accelerated UTF-16 (`u16` slice) to UTF-8 conversion with no escaping.
    fn escape_utf16_raw_simd(
        utf16: &[u16],
        dst: &mut [MaybeUninit<u8>],
    ) -> escape_utf16_raw_scalar;
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use core::arch::x86_64::*;

    // Architecture glue layer for SSE2.
    //
    // All functions are `unsafe` because:
    // - they are `#[target_feature(enable = "sse2")]`
    // - they operate on raw pointers
    //
    // The macro-generated escape loops call these helpers by name.
    type Vec = __m128i;

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn load_utf16le(src_ptr: *const u8, idx: usize) -> Vec {
        // SAFETY: Caller guarantees `src_ptr` is valid for 16 bytes at `idx * 2`.
        // The macro only calls this when `idx + 8 <= max_units` and `max_units`
        // is clamped to `utf16le.len() / 2`, so the 16-byte load stays in-bounds.
        _mm_loadu_si128(src_ptr.add(idx * 2) as *const __m128i)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn load_utf16(src_ptr: *const u16, idx: usize) -> Vec {
        // SAFETY: Caller guarantees `src_ptr` is valid for 8 `u16`s at `idx`.
        // The macro only calls this when `idx + 8 <= utf16.len()`.
        _mm_loadu_si128(src_ptr.add(idx) as *const __m128i)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn store_ascii8(dst_ptr: *mut u8, v: Vec) {
        // Packs 8×u16 to 8×u8. This is only used when the fast predicate has
        // proven all lanes are ASCII (<= 0x7F), so the narrowing is exact.
        //
        // SAFETY: Caller ensures `dst_ptr` is valid for 8 bytes.
        let packed = _mm_packus_epi16(v, v);
        _mm_storel_epi64(dst_ptr as *mut __m128i, packed);
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn first_bad_lane(bad_mask: i32) -> usize {
        if (bad_mask & 0xFFFF) == 0 {
            return 8;
        }
        (bad_mask.trailing_zeros() as usize) / 2
    }

    #[derive(Copy, Clone)]
    struct JsonConsts {
        zero: Vec,
        hi_mask: Vec,
        ctrl_mask: Vec,
        quote: Vec,
        backslash: Vec,
    }

    #[derive(Copy, Clone)]
    struct XmlConsts {
        printable_offset: Vec,
        printable_limit: Vec,
        amp: Vec,
        lt: Vec,
        gt: Vec,
        quote: Vec,
        apos: Vec,
    }

    #[derive(Copy, Clone)]
    struct RawConsts {
        zero: Vec,
        hi_mask: Vec,
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn json_consts() -> JsonConsts {
        JsonConsts {
            zero: _mm_setzero_si128(),
            hi_mask: _mm_set1_epi16(0xFF80u16 as i16),
            ctrl_mask: _mm_set1_epi16(0xFFE0u16 as i16),
            quote: _mm_set1_epi16(0x22),
            backslash: _mm_set1_epi16(0x5C),
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn xml_consts() -> XmlConsts {
        // `v + 0x7FE0 < 0x8060`, as signed 16-bit lanes, holds exactly for 0x20..=0x7F.
        XmlConsts {
            printable_offset: _mm_set1_epi16(0x7FE0),
            printable_limit: _mm_set1_epi16(0x8060u16 as i16),
            amp: _mm_set1_epi16(0x26),
            lt: _mm_set1_epi16(0x3C),
            gt: _mm_set1_epi16(0x3E),
            quote: _mm_set1_epi16(0x22),
            apos: _mm_set1_epi16(0x27),
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn raw_consts() -> RawConsts {
        RawConsts {
            zero: _mm_setzero_si128(),
            hi_mask: _mm_set1_epi16(0xFF80u16 as i16),
        }
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn raw_scan(v: Vec, c: RawConsts) -> usize {
        // True if every lane is ASCII: (v & 0xFF80) == 0 for all lanes.
        let high = _mm_and_si128(v, c.hi_mask);
        let ascii = _mm_cmpeq_epi16(high, c.zero);
        // ascii has 0xFFFF where (high & 0xFF80) == 0 (ASCII).
        let ascii_mask = _mm_movemask_epi8(ascii); // 0xFFFF if all good.
        first_bad_lane(!ascii_mask)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn json_scan(v: Vec, c: JsonConsts) -> usize {
        let high = _mm_and_si128(v, c.hi_mask);
        let ascii = _mm_cmpeq_epi16(high, c.zero);
        let ascii_mask = _mm_movemask_epi8(ascii); // 0xFFFF if all good

        let is_quote = _mm_cmpeq_epi16(v, c.quote);
        let is_backslash = _mm_cmpeq_epi16(v, c.backslash);
        let is_ctrl = _mm_cmpeq_epi16(_mm_and_si128(v, c.ctrl_mask), c.zero);
        let needs = _mm_or_si128(_mm_or_si128(is_quote, is_backslash), is_ctrl);
        let needs_mask = _mm_movemask_epi8(needs); // 0xFFFF if needs escape (bad)

        first_bad_lane((!ascii_mask) | needs_mask)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn xml_scan(v: Vec, c: XmlConsts, in_attribute: bool) -> usize {
        // Printable ASCII only: controls and non-ASCII take the scalar path, which writes `_`
        // for the characters XML doesn't allow. This costs no more than an ASCII test.
        let ascii = _mm_cmplt_epi16(_mm_add_epi16(v, c.printable_offset), c.printable_limit);
        let ascii_mask = _mm_movemask_epi8(ascii);

        let is_amp = _mm_cmpeq_epi16(v, c.amp);
        let is_lt = _mm_cmpeq_epi16(v, c.lt);
        let is_gt = _mm_cmpeq_epi16(v, c.gt);
        let mut needs = _mm_or_si128(_mm_or_si128(is_amp, is_lt), is_gt);
        if in_attribute {
            let is_quote = _mm_cmpeq_epi16(v, c.quote);
            let is_apos = _mm_cmpeq_epi16(v, c.apos);
            needs = _mm_or_si128(needs, _mm_or_si128(is_quote, is_apos));
        }
        let needs_mask = _mm_movemask_epi8(needs);

        first_bad_lane((!ascii_mask) | needs_mask)
    }

    impl_arch_escapers!(
        #[inline]
        #[target_feature(enable = "sse2")]
    );
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use core::arch::aarch64::*;

    // Architecture glue layer for NEON.
    //
    // Similar to `x86`, these are `unsafe` due to `#[target_feature]` and raw
    // pointers. NEON is baseline on aarch64, but Rust still marks calls to
    // `#[target_feature(enable = "neon")]` fns as `unsafe`.
    type Vec = uint16x8_t;

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn load_utf16le(src_ptr: *const u8, idx: usize) -> Vec {
        // SAFETY: Caller guarantees `src_ptr` is valid for 16 bytes at `idx * 2`
        // (same logic as the SSE2 version).
        let ptr = src_ptr.add(idx * 2);
        let bytes = vld1q_u8(ptr);
        vreinterpretq_u16_u8(bytes)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn load_utf16(src_ptr: *const u16, idx: usize) -> Vec {
        // SAFETY: Caller guarantees `src_ptr` is valid for 8 `u16`s at `idx`.
        vld1q_u16(src_ptr.add(idx))
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn store_ascii8(dst_ptr: *mut u8, v: Vec) {
        // Packs 8×u16 to 8×u8. Only used when all lanes are ASCII, so the
        // narrowing is exact.
        //
        // SAFETY: Caller ensures `dst_ptr` is valid for 8 bytes.
        let packed = vqmovn_u16(v);
        vst1_u8(dst_ptr, packed);
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn first_bad_lane(bad: Vec) -> usize {
        let bad_u8 = vqmovn_u16(bad);
        let bad_u64 = vget_lane_u64(vreinterpret_u64_u8(bad_u8), 0);
        if bad_u64 == 0 {
            return 8;
        }
        (bad_u64.trailing_zeros() / 8) as usize
    }

    #[derive(Copy, Clone)]
    struct JsonConsts {
        zero: Vec,
        hi_mask: Vec,
        ctrl_mask: Vec,
        quote: Vec,
        backslash: Vec,
    }

    #[derive(Copy, Clone)]
    struct XmlConsts {
        printable_min: Vec,
        printable_span: Vec,
        amp: Vec,
        lt: Vec,
        gt: Vec,
        quote: Vec,
        apos: Vec,
    }

    #[derive(Copy, Clone)]
    struct RawConsts {
        zero: Vec,
        hi_mask: Vec,
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn json_consts() -> JsonConsts {
        JsonConsts {
            zero: vdupq_n_u16(0),
            hi_mask: vdupq_n_u16(0xFF80),
            ctrl_mask: vdupq_n_u16(0xFFE0),
            quote: vdupq_n_u16(0x22),
            backslash: vdupq_n_u16(0x5C),
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn xml_consts() -> XmlConsts {
        // `v - 0x20 < 0x60`, as unsigned 16-bit lanes, holds exactly for 0x20..=0x7F.
        XmlConsts {
            printable_min: vdupq_n_u16(0x20),
            printable_span: vdupq_n_u16(0x60),
            amp: vdupq_n_u16(0x26),
            lt: vdupq_n_u16(0x3C),
            gt: vdupq_n_u16(0x3E),
            quote: vdupq_n_u16(0x22),
            apos: vdupq_n_u16(0x27),
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn raw_consts() -> RawConsts {
        RawConsts {
            zero: vdupq_n_u16(0),
            hi_mask: vdupq_n_u16(0xFF80),
        }
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn raw_scan(v: Vec, c: RawConsts) -> usize {
        let high = vandq_u16(v, c.hi_mask);
        let not_ascii = vmvnq_u16(vceqq_u16(high, c.zero)); // 0 if good, 0xFFFF if bad
        first_bad_lane(not_ascii)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn json_scan(v: Vec, c: JsonConsts) -> usize {
        let high = vandq_u16(v, c.hi_mask);
        let not_ascii = vmvnq_u16(vceqq_u16(high, c.zero)); // 0 if good

        let is_quote = vceqq_u16(v, c.quote);
        let is_backslash = vceqq_u16(v, c.backslash);
        let is_ctrl = vceqq_u16(vandq_u16(v, c.ctrl_mask), c.zero);
        let needs = vorrq_u16(vorrq_u16(is_quote, is_backslash), is_ctrl);

        first_bad_lane(vorrq_u16(not_ascii, needs))
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn xml_scan(v: Vec, c: XmlConsts, in_attribute: bool) -> usize {
        // Printable ASCII only: controls and non-ASCII take the scalar path, which writes `_`
        // for the characters XML doesn't allow.
        let not_ascii = vcgeq_u16(vsubq_u16(v, c.printable_min), c.printable_span); // 0 if good

        let is_amp = vceqq_u16(v, c.amp);
        let is_lt = vceqq_u16(v, c.lt);
        let is_gt = vceqq_u16(v, c.gt);
        let mut needs = vorrq_u16(vorrq_u16(is_amp, is_lt), is_gt);
        if in_attribute {
            let is_quote = vceqq_u16(v, c.quote);
            let is_apos = vceqq_u16(v, c.apos);
            needs = vorrq_u16(needs, vorrq_u16(is_quote, is_apos));
        }

        first_bad_lane(vorrq_u16(not_ascii, needs))
    }

    impl_arch_escapers!(
        #[inline]
        #[target_feature(enable = "neon")]
    );
}
