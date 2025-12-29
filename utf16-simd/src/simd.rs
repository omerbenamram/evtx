// This module intentionally uses raw pointers and SIMD intrinsics.
//
// We keep the `unsafe` surface area small (mostly in the architecture glue) and
// accompany raw pointer writes / assumptions with explicit `SAFETY:` comments.
#![allow(unsafe_op_in_unsafe_fn)]
//! SIMD front-end for UTF-16/UTF-16LE escaping and conversion.
//!
//! Goal: keep SIMD code small and non-duplicative.
//!
//! Strategy:
//! - SIMD is used only for the very common case: 8 UTF-16 code units that are
//!   ASCII and require **no escaping**. In that case we can pack+store 8 bytes
//!   at once.
//! - Everything else (escapes, non-ASCII, surrogates) is handled by the scalar
//!   per-code-unit routines. This avoids maintaining two copies of the “slow
//!   path” per architecture.
//!
//! Implementations:
//! - x86_64: SSE2 (runtime detected)
//! - aarch64: NEON (baseline)

use core::mem::MaybeUninit;

#[cfg(feature = "perf-counters")]
use crate::perf;

// ---------------------------------------------------------------------------
// Macro structure
//
// The high-level control flow is identical across:
// - input representation: UTF-16LE bytes (`&[u8]`) vs native code units (`&[u16]`)
// - architecture: SSE2 vs NEON
//
// The only per-arch differences are the intrinsics needed to:
// - load 8×u16 code units
// - test the “fast path” predicate (ASCII + no escaping required)
// - narrow-pack and store 8 bytes
//
// Each architecture module (`x86` / `neon`) therefore provides a tiny shim API
// with stable names/signatures:
// - `load_utf16le(*const u8, idx_units) -> Vec`
// - `load_utf16(*const u16, idx_units) -> Vec`
// - `store_ascii8(*mut u8, Vec) -> *mut u8`
// - `json_consts()/xml_consts()/raw_consts()`
// - `json_scan()/xml_scan()/raw_scan()`
//
// The macros below generate the actual escape loops on top of that shim:
// - iterate in 8-unit chunks
// - take the fast path when possible
// - otherwise delegate to the scalar per-unit helpers (`escape_one*`) which
//   implement all semantics (escaping rules, UTF-8 encoding, surrogate policy).
//
// NOTE: These macros expand inside `x86`/`neon` modules. We use fully-qualified
// `core::mem::MaybeUninit` so we don't depend on module imports.
// ---------------------------------------------------------------------------

macro_rules! impl_arch_json_escape {
    ($(#[$m:meta])*) => {
        $(#[$m])*
        pub unsafe fn escape_json_utf16le_simd(
            utf16le: &[u8],
            num_units: usize,
            dst: &mut [core::mem::MaybeUninit<u8>],
            need_quote: bool,
        ) -> usize {
            let max_units = crate::scalar::max_units(num_units, utf16le.len());
            let max_len = crate::max_escaped_len(max_units, need_quote);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16le.as_ptr();
            // SAFETY: `dst` is a `MaybeUninit<u8>` output buffer. We only ever
            // write bytes into it (never read from it). `max_len` is an upper
            // bound for *all* writes done by this function (including the scalar
            // fallback helpers), and we asserted `dst.len() >= max_len`.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            if need_quote {
                // SAFETY: `max_len` accounts for the quotes and we have asserted
                // the buffer is at least that large.
                *dst_ptr = b'"';
                dst_ptr = dst_ptr.add(1);
            }

            let (zero, hi_mask, ctrl_mask, quote, backslash) = json_consts();

            let mut idx = 0usize;
            // Process full 8-unit chunks. `load_utf16le` performs a 16-byte load.
            while idx + 8 <= max_units {
                let v = load_utf16le(src_ptr, idx);
                let len = json_scan(v, zero, hi_mask, ctrl_mask, quote, backslash);

                // Always store 8 packed bytes. The fast path (len == 8) uses all of them.
                // The slow path (len < 8) uses the first `len` bytes, and subsequent
                // scalar writes will overwrite the garbage bytes.
                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    // Scalar fallback for the "bad" character.
                    if idx < max_units {
                        idx = crate::scalar::escape_one(src_ptr, max_units, idx, &mut dst_ptr);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one(src_ptr, max_units, idx, &mut dst_ptr);
            }

            if need_quote {
                // SAFETY: same reasoning as the opening quote: `max_len` bounds
                // all writes and includes space for the closing quote.
                *dst_ptr = b'"';
                dst_ptr = dst_ptr.add(1);
            }

            dst_ptr as usize - dst_start as usize
        }

        $(#[$m])*
        pub unsafe fn escape_json_utf16_simd(
            utf16: &[u16],
            dst: &mut [core::mem::MaybeUninit<u8>],
            need_quote: bool,
        ) -> usize {
            let max_units = utf16.len();
            let max_len = crate::max_escaped_len(max_units, need_quote);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16.as_ptr();
            // SAFETY: same as the UTF-16LE variant above. `dst` is
            // `MaybeUninit<u8>` and we only write; `max_len` is an upper bound.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            if need_quote {
                // SAFETY: `max_len` includes the quotes.
                *dst_ptr = b'"';
                dst_ptr = dst_ptr.add(1);
            }

            let (zero, hi_mask, ctrl_mask, quote, backslash) = json_consts();

            let mut idx = 0usize;
            while idx + 8 <= max_units {
                let v = load_utf16(src_ptr, idx);
                let len = json_scan(v, zero, hi_mask, ctrl_mask, quote, backslash);

                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    if idx < max_units {
                        idx = crate::scalar::escape_one_u16(utf16, idx, &mut dst_ptr);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one_u16(utf16, idx, &mut dst_ptr);
            }

            if need_quote {
                // SAFETY: `max_len` includes the quotes.
                *dst_ptr = b'"';
                dst_ptr = dst_ptr.add(1);
            }

            dst_ptr as usize - dst_start as usize
        }
    };
}

macro_rules! impl_arch_xml_escape {
    ($(#[$m:meta])*) => {
        $(#[$m])*
        pub unsafe fn escape_xml_utf16le_simd(
            utf16le: &[u8],
            num_units: usize,
            dst: &mut [core::mem::MaybeUninit<u8>],
            in_attribute: bool,
        ) -> usize {
            let max_units = crate::scalar::max_units(num_units, utf16le.len());
            let max_len = crate::max_escaped_len(max_units, false);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16le.as_ptr();
            // SAFETY: identical rationale as JSON: `dst` is write-only and
            // `max_len` bounds all writes.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            let (zero, hi_mask, amp, lt, gt, quote, apos) = xml_consts();

            let mut idx = 0usize;
            while idx + 8 <= max_units {
                let v = load_utf16le(src_ptr, idx);
                let len = xml_scan(v, zero, hi_mask, amp, lt, gt, quote, apos, in_attribute);

                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    if idx < max_units {
                        idx = crate::scalar::escape_one_xml(src_ptr, max_units, idx, &mut dst_ptr, in_attribute);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one_xml(src_ptr, max_units, idx, &mut dst_ptr, in_attribute);
            }

            dst_ptr as usize - dst_start as usize
        }

        $(#[$m])*
        pub unsafe fn escape_xml_utf16_simd(
            utf16: &[u16],
            dst: &mut [core::mem::MaybeUninit<u8>],
            in_attribute: bool,
        ) -> usize {
            let max_units = utf16.len();
            let max_len = crate::max_escaped_len(max_units, false);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16.as_ptr();
            // SAFETY: identical rationale as XML UTF-16LE: `dst` is write-only
            // and `max_len` bounds all writes.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            let (zero, hi_mask, amp, lt, gt, quote, apos) = xml_consts();

            let mut idx = 0usize;
            while idx + 8 <= max_units {
                let v = load_utf16(src_ptr, idx);
                let len = xml_scan(v, zero, hi_mask, amp, lt, gt, quote, apos, in_attribute);

                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    if idx < max_units {
                        idx = crate::scalar::escape_one_xml_u16(utf16, idx, &mut dst_ptr, in_attribute);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one_xml_u16(utf16, idx, &mut dst_ptr, in_attribute);
            }

            dst_ptr as usize - dst_start as usize
        }
    };
}

macro_rules! impl_arch_raw_escape {
    ($(#[$m:meta])*) => {
        $(#[$m])*
        pub unsafe fn escape_utf16le_raw_simd(
            utf16le: &[u8],
            num_units: usize,
            dst: &mut [core::mem::MaybeUninit<u8>],
        ) -> usize {
            let max_units = crate::scalar::max_units(num_units, utf16le.len());
            let max_len = crate::max_escaped_len(max_units, false);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16le.as_ptr();
            // SAFETY: write-only output; `max_len` bounds all writes.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            let (zero, hi_mask) = raw_consts();

            let mut idx = 0usize;
            while idx + 8 <= max_units {
                let v = load_utf16le(src_ptr, idx);
                let len = raw_scan(v, zero, hi_mask);

                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    if idx < max_units {
                        idx = crate::scalar::escape_one_raw(src_ptr, max_units, idx, &mut dst_ptr);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one_raw(src_ptr, max_units, idx, &mut dst_ptr);
            }

            dst_ptr as usize - dst_start as usize
        }

        $(#[$m])*
        pub unsafe fn escape_utf16_raw_simd(
            utf16: &[u16],
            dst: &mut [core::mem::MaybeUninit<u8>],
        ) -> usize {
            let max_units = utf16.len();
            let max_len = crate::max_escaped_len(max_units, false);
            assert!(dst.len() >= max_len);

            let src_ptr = utf16.as_ptr();
            // SAFETY: write-only output; `max_len` bounds all writes.
            let mut dst_ptr = dst.as_mut_ptr() as *mut u8;
            let dst_start = dst_ptr;

            let (zero, hi_mask) = raw_consts();

            let mut idx = 0usize;
            while idx + 8 <= max_units {
                let v = load_utf16(src_ptr, idx);
                let len = raw_scan(v, zero, hi_mask);

                store_ascii8(dst_ptr, v);
                dst_ptr = dst_ptr.add(len);
                idx += len;

                if len < 8 {
                    if idx < max_units {
                        idx = crate::scalar::escape_one_raw_u16(utf16, idx, &mut dst_ptr);
                    }
                }
            }

            while idx < max_units {
                idx = crate::scalar::escape_one_raw_u16(utf16, idx, &mut dst_ptr);
            }

            dst_ptr as usize - dst_start as usize
        }
    };
}

/// SIMD-accelerated JSON escaping for UTF-16LE input.
#[inline]
pub fn escape_json_utf16le_simd(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            #[cfg(feature = "perf-counters")]
            perf::record_json_escape(crate::scalar::max_units(num_units, utf16le.len()), true);
            // SAFETY:
            // - We only call the SSE2 implementation when the CPU reports SSE2
            //   support (`is_x86_feature_detected!("sse2")`).
            // - The SSE2 implementation is `#[target_feature(enable = "sse2")]`
            //   so it may freely use SSE2 intrinsics.
            unsafe {
                return x86::escape_json_utf16le_simd(utf16le, num_units, dst, need_quote);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY:
        // - NEON is mandatory on aarch64.
        // - The implementation is still `#[target_feature(enable = "neon")]`,
        //   which makes the call `unsafe` in Rust.
        #[cfg(feature = "perf-counters")]
        perf::record_json_escape(crate::scalar::max_units(num_units, utf16le.len()), true);
        unsafe {
            return neon::escape_json_utf16le_simd(utf16le, num_units, dst, need_quote);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(feature = "perf-counters")]
        perf::record_json_escape(crate::scalar::max_units(num_units, utf16le.len()), false);
        crate::scalar::escape_json_utf16le_scalar(utf16le, num_units, dst, need_quote)
    }
}

/// SIMD-accelerated XML escaping for UTF-16LE input.
#[inline]
pub fn escape_xml_utf16le_simd(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            // SAFETY: same as JSON SSE2: runtime-detected SSE2 + `#[target_feature]`.
            unsafe {
                return x86::escape_xml_utf16le_simd(utf16le, num_units, dst, in_attribute);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON baseline on aarch64 + `#[target_feature(enable = "neon")]`.
        unsafe {
            return neon::escape_xml_utf16le_simd(utf16le, num_units, dst, in_attribute);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::scalar::escape_xml_utf16le_scalar(utf16le, num_units, dst, in_attribute)
    }
}

/// SIMD-accelerated UTF-16LE to UTF-8 conversion with no escaping.
#[inline]
pub fn escape_utf16le_raw_simd(
    utf16le: &[u8],
    num_units: usize,
    dst: &mut [MaybeUninit<u8>],
) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            // SAFETY: runtime-detected SSE2 + `#[target_feature(enable = "sse2")]`.
            unsafe {
                return x86::escape_utf16le_raw_simd(utf16le, num_units, dst);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON baseline on aarch64 + `#[target_feature(enable = "neon")]`.
        unsafe {
            return neon::escape_utf16le_raw_simd(utf16le, num_units, dst);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::scalar::escape_utf16le_raw_scalar(utf16le, num_units, dst)
    }
}

/// SIMD-accelerated JSON escaping for UTF-16 code units (`u16` slice).
#[inline]
pub fn escape_json_utf16_simd(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    need_quote: bool,
) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            #[cfg(feature = "perf-counters")]
            perf::record_json_escape(utf16.len(), true);
            // SAFETY: runtime-detected SSE2 + `#[target_feature(enable = "sse2")]`.
            unsafe {
                return x86::escape_json_utf16_simd(utf16, dst, need_quote);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        #[cfg(feature = "perf-counters")]
        perf::record_json_escape(utf16.len(), true);
        // SAFETY: NEON baseline on aarch64 + `#[target_feature(enable = "neon")]`.
        unsafe {
            return neon::escape_json_utf16_simd(utf16, dst, need_quote);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        #[cfg(feature = "perf-counters")]
        perf::record_json_escape(utf16.len(), false);
        crate::scalar::escape_json_utf16_scalar(utf16, dst, need_quote)
    }
}

/// SIMD-accelerated XML escaping for UTF-16 code units (`u16` slice).
#[inline]
pub fn escape_xml_utf16_simd(
    utf16: &[u16],
    dst: &mut [MaybeUninit<u8>],
    in_attribute: bool,
) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            // SAFETY: runtime-detected SSE2 + `#[target_feature(enable = "sse2")]`.
            unsafe {
                return x86::escape_xml_utf16_simd(utf16, dst, in_attribute);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON baseline on aarch64 + `#[target_feature(enable = "neon")]`.
        unsafe {
            return neon::escape_xml_utf16_simd(utf16, dst, in_attribute);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::scalar::escape_xml_utf16_scalar(utf16, dst, in_attribute)
    }
}

/// SIMD-accelerated UTF-16 (`u16` slice) to UTF-8 conversion with no escaping.
#[inline]
pub fn escape_utf16_raw_simd(utf16: &[u16], dst: &mut [MaybeUninit<u8>]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        if std::arch::is_x86_feature_detected!("sse2") {
            // SAFETY: runtime-detected SSE2 + `#[target_feature(enable = "sse2")]`.
            unsafe {
                return x86::escape_utf16_raw_simd(utf16, dst);
            }
        }
    }

    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: NEON baseline on aarch64 + `#[target_feature(enable = "neon")]`.
        unsafe {
            return neon::escape_utf16_raw_simd(utf16, dst);
        }
    }

    #[cfg(not(target_arch = "aarch64"))]
    {
        crate::scalar::escape_utf16_raw_scalar(utf16, dst)
    }
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
        // SAFETY: Caller guarantees `src_ptr` is valid for 16 bytes at `idx*2`.
        // The macro only calls this when `idx + 8 <= max_units` and `max_units`
        // is clamped to `utf16le.len()/2`, so the 16-byte load stays in-bounds.
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
    unsafe fn store_ascii8(dst_ptr: *mut u8, v: Vec) -> *mut u8 {
        // Packs 8×u16 to 8×u8. This is only used when the fast predicate has
        // proven all lanes are ASCII (<= 0x7F), so the narrowing is exact.
        //
        // SAFETY: Caller ensures `dst_ptr` is valid for 8 bytes.
        let packed = _mm_packus_epi16(v, v);
        _mm_storel_epi64(dst_ptr as *mut __m128i, packed);
        dst_ptr.add(8)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn json_consts() -> (Vec, Vec, Vec, Vec, Vec) {
        let zero = _mm_setzero_si128();
        let hi_mask = _mm_set1_epi16(0xFF80u16 as i16);
        let ctrl_mask = _mm_set1_epi16(0xFFE0u16 as i16);
        let quote = _mm_set1_epi16(0x22);
        let backslash = _mm_set1_epi16(0x5C);
        (zero, hi_mask, ctrl_mask, quote, backslash)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn xml_consts() -> (Vec, Vec, Vec, Vec, Vec, Vec, Vec) {
        let zero = _mm_setzero_si128();
        let hi_mask = _mm_set1_epi16(0xFF80u16 as i16);
        let amp = _mm_set1_epi16(0x26);
        let lt = _mm_set1_epi16(0x3C);
        let gt = _mm_set1_epi16(0x3E);
        let quote = _mm_set1_epi16(0x22);
        let apos = _mm_set1_epi16(0x27);
        (zero, hi_mask, amp, lt, gt, quote, apos)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn raw_consts() -> (Vec, Vec) {
        let zero = _mm_setzero_si128();
        let hi_mask = _mm_set1_epi16(0xFF80u16 as i16);
        (zero, hi_mask)
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn raw_scan(v: Vec, zero: Vec, hi_mask: Vec) -> usize {
        // True if every lane is ASCII: (v & 0xFF80) == 0 for all lanes.
        let high = _mm_and_si128(v, hi_mask);
        let ascii = _mm_cmpeq_epi16(high, zero);
        // ascii has 0xFFFF where (high & 0xFF80) == 0 (ASCII).
        let ascii_mask = _mm_movemask_epi8(ascii); // 0xFFFF if all good.

        let bad_mask = !ascii_mask;
        if (bad_mask & 0xFFFF) == 0 {
            return 8;
        }
        (bad_mask.trailing_zeros() as usize) / 2
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn json_scan(
        v: Vec,
        zero: Vec,
        hi_mask: Vec,
        ctrl_mask: Vec,
        quote: Vec,
        backslash: Vec,
    ) -> usize {
        let high = _mm_and_si128(v, hi_mask);
        let ascii = _mm_cmpeq_epi16(high, zero);
        let ascii_mask = _mm_movemask_epi8(ascii); // 0xFFFF if all good

        let is_quote = _mm_cmpeq_epi16(v, quote);
        let is_backslash = _mm_cmpeq_epi16(v, backslash);
        let is_ctrl = _mm_cmpeq_epi16(_mm_and_si128(v, ctrl_mask), zero);
        let needs = _mm_or_si128(_mm_or_si128(is_quote, is_backslash), is_ctrl);
        let needs_mask = _mm_movemask_epi8(needs); // 0xFFFF if needs escape (bad)

        let bad_mask = (!ascii_mask) | needs_mask;
        if (bad_mask & 0xFFFF) == 0 {
            return 8;
        }
        (bad_mask.trailing_zeros() as usize) / 2
    }

    #[inline]
    #[target_feature(enable = "sse2")]
    unsafe fn xml_scan(
        v: Vec,
        zero: Vec,
        hi_mask: Vec,
        amp: Vec,
        lt: Vec,
        gt: Vec,
        quote: Vec,
        apos: Vec,
        in_attribute: bool,
    ) -> usize {
        let high = _mm_and_si128(v, hi_mask);
        let ascii = _mm_cmpeq_epi16(high, zero);
        let ascii_mask = _mm_movemask_epi8(ascii);

        let is_amp = _mm_cmpeq_epi16(v, amp);
        let is_lt = _mm_cmpeq_epi16(v, lt);
        let is_gt = _mm_cmpeq_epi16(v, gt);
        let mut needs = _mm_or_si128(_mm_or_si128(is_amp, is_lt), is_gt);
        if in_attribute {
            let is_quote = _mm_cmpeq_epi16(v, quote);
            let is_apos = _mm_cmpeq_epi16(v, apos);
            needs = _mm_or_si128(needs, _mm_or_si128(is_quote, is_apos));
        }
        let needs_mask = _mm_movemask_epi8(needs);

        let bad_mask = (!ascii_mask) | needs_mask;
        if (bad_mask & 0xFFFF) == 0 {
            return 8;
        }
        (bad_mask.trailing_zeros() as usize) / 2
    }

    impl_arch_json_escape!(
        #[inline]
        #[target_feature(enable = "sse2")]
    );
    impl_arch_xml_escape!(
        #[inline]
        #[target_feature(enable = "sse2")]
    );
    impl_arch_raw_escape!(
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
        // SAFETY: Caller guarantees `src_ptr` is valid for 16 bytes at `idx*2`
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
    unsafe fn store_ascii8(dst_ptr: *mut u8, v: Vec) -> *mut u8 {
        // Packs 8×u16 to 8×u8. Only used when all lanes are ASCII, so the
        // narrowing is exact.
        //
        // SAFETY: Caller ensures `dst_ptr` is valid for 8 bytes.
        let packed = vqmovn_u16(v);
        vst1_u8(dst_ptr, packed);
        dst_ptr.add(8)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn json_consts() -> (Vec, Vec, Vec, Vec, Vec) {
        let zero = vdupq_n_u16(0);
        let hi_mask = vdupq_n_u16(0xFF80);
        let ctrl_mask = vdupq_n_u16(0xFFE0);
        let quote = vdupq_n_u16(0x22);
        let backslash = vdupq_n_u16(0x5C);
        (zero, hi_mask, ctrl_mask, quote, backslash)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn xml_consts() -> (Vec, Vec, Vec, Vec, Vec, Vec, Vec) {
        let zero = vdupq_n_u16(0);
        let hi_mask = vdupq_n_u16(0xFF80);
        let amp = vdupq_n_u16(0x26);
        let lt = vdupq_n_u16(0x3C);
        let gt = vdupq_n_u16(0x3E);
        let quote = vdupq_n_u16(0x22);
        let apos = vdupq_n_u16(0x27);
        (zero, hi_mask, amp, lt, gt, quote, apos)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn raw_consts() -> (Vec, Vec) {
        let zero = vdupq_n_u16(0);
        let hi_mask = vdupq_n_u16(0xFF80);
        (zero, hi_mask)
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn raw_scan(v: Vec, zero: Vec, hi_mask: Vec) -> usize {
        let high = vandq_u16(v, hi_mask);
        let not_ascii = vmvnq_u16(vceqq_u16(high, zero)); // 0 if good, 0xFFFF if bad

        let bad_u8 = vqmovn_u16(not_ascii);
        let bad_u64 = vget_lane_u64(vreinterpret_u64_u8(bad_u8), 0);

        if bad_u64 == 0 {
            return 8;
        }
        (bad_u64.trailing_zeros() / 8) as usize
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn json_scan(
        v: Vec,
        zero: Vec,
        hi_mask: Vec,
        ctrl_mask: Vec,
        quote: Vec,
        backslash: Vec,
    ) -> usize {
        let high = vandq_u16(v, hi_mask);
        let not_ascii = vmvnq_u16(vceqq_u16(high, zero)); // 0 if good

        let is_quote = vceqq_u16(v, quote);
        let is_backslash = vceqq_u16(v, backslash);
        let is_ctrl = vceqq_u16(vandq_u16(v, ctrl_mask), zero);
        let needs = vorrq_u16(vorrq_u16(is_quote, is_backslash), is_ctrl);

        let bad = vorrq_u16(not_ascii, needs);

        let bad_u8 = vqmovn_u16(bad);
        let bad_u64 = vget_lane_u64(vreinterpret_u64_u8(bad_u8), 0);

        if bad_u64 == 0 {
            return 8;
        }
        (bad_u64.trailing_zeros() / 8) as usize
    }

    #[inline]
    #[target_feature(enable = "neon")]
    unsafe fn xml_scan(
        v: Vec,
        zero: Vec,
        hi_mask: Vec,
        amp: Vec,
        lt: Vec,
        gt: Vec,
        quote: Vec,
        apos: Vec,
        in_attribute: bool,
    ) -> usize {
        let high = vandq_u16(v, hi_mask);
        let not_ascii = vmvnq_u16(vceqq_u16(high, zero)); // 0 if good

        let is_amp = vceqq_u16(v, amp);
        let is_lt = vceqq_u16(v, lt);
        let is_gt = vceqq_u16(v, gt);
        let mut needs = vorrq_u16(vorrq_u16(is_amp, is_lt), is_gt);
        if in_attribute {
            let is_quote = vceqq_u16(v, quote);
            let is_apos = vceqq_u16(v, apos);
            needs = vorrq_u16(needs, vorrq_u16(is_quote, is_apos));
        }

        let bad = vorrq_u16(not_ascii, needs);

        let bad_u8 = vqmovn_u16(bad);
        let bad_u64 = vget_lane_u64(vreinterpret_u64_u8(bad_u8), 0);

        if bad_u64 == 0 {
            return 8;
        }
        (bad_u64.trailing_zeros() / 8) as usize
    }

    impl_arch_json_escape!(
        #[inline]
        #[target_feature(enable = "neon")]
    );
    impl_arch_xml_escape!(
        #[inline]
        #[target_feature(enable = "neon")]
    );
    impl_arch_raw_escape!(
        #[inline]
        #[target_feature(enable = "neon")]
    );
}
