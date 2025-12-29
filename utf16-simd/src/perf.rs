//! Performance counters for UTF-16 escaping paths.
//!
//! These counters are only enabled when the `perf-counters` feature is set.
//! They are intended for profiling / diagnostics and are not optimized for
//! minimal overhead.

use std::sync::atomic::{AtomicU64, Ordering};

const BUCKETS: usize = 8;

#[derive(Debug, Clone)]
pub struct EscapeStats {
    pub calls: u64,
    pub units: u64,
    pub scalar_calls: u64,
    pub simd_calls: u64,
    pub buckets: [u64; BUCKETS],
}

const ZERO: AtomicU64 = AtomicU64::new(0);

static JSON_CALLS: AtomicU64 = AtomicU64::new(0);
static JSON_UNITS: AtomicU64 = AtomicU64::new(0);
static JSON_SCALAR: AtomicU64 = AtomicU64::new(0);
static JSON_SIMD: AtomicU64 = AtomicU64::new(0);
static JSON_BUCKETS: [AtomicU64; BUCKETS] = [ZERO; BUCKETS];

#[inline]
fn bucket_index(units: usize) -> usize {
    match units {
        0..=8 => 0,
        9..=16 => 1,
        17..=32 => 2,
        33..=64 => 3,
        65..=128 => 4,
        129..=256 => 5,
        257..=512 => 6,
        _ => 7,
    }
}

#[inline]
pub fn record_json_escape(units: usize, used_simd: bool) {
    JSON_CALLS.fetch_add(1, Ordering::Relaxed);
    JSON_UNITS.fetch_add(units as u64, Ordering::Relaxed);
    if used_simd {
        JSON_SIMD.fetch_add(1, Ordering::Relaxed);
    } else {
        JSON_SCALAR.fetch_add(1, Ordering::Relaxed);
    }
    let idx = bucket_index(units);
    JSON_BUCKETS[idx].fetch_add(1, Ordering::Relaxed);
}

pub fn reset_json() {
    JSON_CALLS.store(0, Ordering::Relaxed);
    JSON_UNITS.store(0, Ordering::Relaxed);
    JSON_SCALAR.store(0, Ordering::Relaxed);
    JSON_SIMD.store(0, Ordering::Relaxed);
    for bucket in &JSON_BUCKETS {
        bucket.store(0, Ordering::Relaxed);
    }
}

pub fn snapshot_json() -> EscapeStats {
    let mut buckets = [0u64; BUCKETS];
    for (idx, bucket) in JSON_BUCKETS.iter().enumerate() {
        buckets[idx] = bucket.load(Ordering::Relaxed);
    }
    EscapeStats {
        calls: JSON_CALLS.load(Ordering::Relaxed),
        units: JSON_UNITS.load(Ordering::Relaxed),
        scalar_calls: JSON_SCALAR.load(Ordering::Relaxed),
        simd_calls: JSON_SIMD.load(Ordering::Relaxed),
        buckets,
    }
}
