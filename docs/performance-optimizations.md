# Performance Optimizations: Closing the Gap with Zig

This document details the performance optimizations applied to the Rust EVTX parser, inspired by analysis of a Zig implementation that was ~5x faster.

## Summary

| Optimization | Individual Speedup |
|--------------|-------------------|
| ASCII Fast Path | ~5% faster |
| Direct JSON Writing | ~4% faster |

**Current benchmark** (single-threaded, security_big_sample.evtx):
- Rust: **574 ms**
- Zig: **166 ms**
- Gap: **Zig is 3.46x faster**

The remaining gap is due to **architectural differences** identified via profiling.

---

## Profiling Analysis (Current State)

Flamegraph profiling using macOS `sample` + FlameGraph perl scripts:

```
Top leaf functions (by samples):
  56  _xzm_free                    ─┐
  48  _xzm_xzone_malloc_tiny        │ Memory allocation: ~170 samples (29%)
  17  _xzm_xzone_malloc             │
  15  _free                         │
  14  _malloc_zone_malloc          ─┘
  38  _platform_memmove            ── Copying (from clones): 38 samples (6%)
  36  stream_expand_token          ─┐
  18  _expand_templates             │ Template expansion: 54 samples (9%)
  34  SipHash::write               ── HashMap hashing: 34 samples (6%)
  16  read_utf16_string            ── String conversion: 16 samples (3%)
  16  BinXmlValue::from            ── serde_json conversion: 16 samples (3%)
```

**Key insight**: The #1 bottleneck is **memory allocation/deallocation** (~29% of CPU time).

This is fundamentally architectural:
- Rust clones `BinXMLDeserializedTokens` during template expansion
- Each clone allocates memory for `Vec<>` and `String` fields
- Zig uses arena allocation (no individual malloc/free calls)

---

## Optimization 1: ASCII Fast Path for UTF-16 to UTF-8 Conversion

**File**: `src/utils/binxml_utils.rs`
**Speedup**: ~5% faster

### The Problem

The original `read_utf16_string` function used Rust's `decode_utf16` iterator for every string:

```rust
// Before: Every character goes through the iterator
decode_utf16(buffer.into_iter().take_while(|&byte| byte != 0x00))
    .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
    .collect()
```

This approach has overhead:
- Iterator state management per character
- Surrogate pair handling for every codepoint
- Allocations for collecting results

### The Insight

~95% of EVTX strings are **pure ASCII**:
- Element names: `"Event"`, `"System"`, `"Provider"`
- Attribute names: `"Name"`, `"Guid"`, `"EventID"`
- Short values: `"SYSTEM"`, `"Security"`, `"4624"`

For ASCII, UTF-16LE is trivial: the low byte IS the UTF-8 character (high byte is 0).

### The Solution

```rust
// Find actual string length (stop at NUL)
let actual_len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());

// ASCII fast path: if all code units are <= 0x7F, directly convert
let all_ascii = buffer[..actual_len].iter().all(|&c| c <= 0x7F);

if all_ascii {
    // Direct conversion: each u16 <= 0x7F maps to exactly one u8
    let mut result = String::with_capacity(actual_len);
    for &c in &buffer[..actual_len] {
        result.push(c as u8 as char);
    }
    return Ok(result);
}

// Fallback: use decode_utf16 for non-ASCII strings
decode_utf16(buffer.into_iter().take(actual_len))
    .map(|r| r.map_err(|_e| Error::from(ErrorKind::InvalidData)))
    .collect()
```

### Why It's Faster

1. **No iterator overhead** for ASCII strings
2. **Simple loop** instead of complex surrogate handling
3. **Pre-allocated capacity** based on known length
4. **Single scan** to check ASCII + convert

### Benchmark

```
Before: 146.1 ms ± 8.0 ms
After:  139.2 ms ± 7.6 ms
Speedup: 1.05x (5% faster)
```

---

## Optimization 2: Direct JSON String Writing

**File**: `src/json_stream_output.rs`
**Speedup**: ~4% faster

### The Problem

The streaming JSON output used `serde_json::to_writer` for all string serialization:

```rust
fn write_key(&mut self, key: &str) -> SerializationResult<()> {
    self.write_comma_if_needed()?;
    let unique_key = self.reserve_unique_key(key);

    // Overhead: serde_json parsing, escaping, buffering
    serde_json::to_writer(self.writer_mut(), &unique_key)?;
    self.write_bytes(b":")
}
```

This adds overhead:
- Function call into serde_json
- Escape character scanning
- Potential buffering

### The Insight

XML element and attribute names follow **NCName rules** (Namespaced Colon-less Name):
- Start with letter or underscore
- Contain only letters, digits, hyphens, underscores, periods
- **No characters that need JSON escaping** (no quotes, backslashes, control chars)

### The Solution

```rust
/// Write a JSON string directly without escaping.
/// Only safe for NCName strings (XML element/attribute names).
#[inline]
fn write_json_string_ncname(&mut self, s: &str) -> SerializationResult<()> {
    self.write_bytes(b"\"")?;
    self.write_bytes(s.as_bytes())?;
    self.write_bytes(b"\"")
}

fn write_key(&mut self, key: &str) -> SerializationResult<()> {
    self.write_comma_if_needed()?;
    let unique_key = self.reserve_unique_key(key);

    // Direct write: no escaping needed for NCName
    self.write_json_string_ncname(&unique_key)?;
    self.write_bytes(b":")
}
```

Also replaced fixed string keys with direct byte writes:

```rust
// Before
serde_json::to_writer(self.writer_mut(), "#attributes")?;

// After
self.write_bytes(b"\"#attributes\":")?;
```

### Why It's Faster

1. **No escape scanning** for NCName strings
2. **No function call overhead** to serde_json
3. **Direct byte writes** avoid intermediate processing
4. **Inlined** for hot path optimization

### Benchmark

```
Before: 140.1 ms ± 10.8 ms
After:  135.3 ms ± 12.1 ms
Speedup: 1.04x (4% faster)
```

---

## Current Benchmark

Single-threaded JSON output on `security_big_sample.evtx` (30 MB):

```
Rust:  574 ms ± 5 ms
Zig:   166 ms ± 12 ms

Gap: Zig is 3.46x faster
```

Multi-threaded:
```
Rust:  273 ms (8 threads)
Zig:   ~50 ms (estimated)
```

---

## Remaining Opportunities (Architectural Changes Required)

The Zig parser is ~3.5x faster due to fundamental architectural differences:

### 1. Arena Allocator (~29% of CPU time)

**Problem**: Profiling shows 170+ samples in malloc/free - the #1 bottleneck.

**Zig approach**:
- Uses arena allocation (`std.heap.ArenaAllocator`) for all chunk processing
- Allocations are bump-pointer (O(1), no metadata)
- Atomic deallocation: just reset the bump pointer when done with chunk
- No individual `free()` calls

**Rust solution**: Use `bumpalo` crate for per-chunk allocations. Requires:
- Modifying `EvtxChunk` to hold an arena
- Changing token types to allocate from arena
- Resetting arena between chunks

### 2. Reference-Based Template Expansion (~15% of CPU time)

**Problem**: Rust clones `BinXMLDeserializedTokens` for every token during template expansion.

```rust
// Current: clones for every token
stream_expand_token(val.clone(), chunk, ...)?;
stream_expand_token(other.clone(), chunk, ...)?;
```

**Zig approach**:
- Templates stored as IR with `Placeholder` nodes
- Instantiation clones just the tree structure (cheap memcpy)
- Actual data (strings, etc.) is shared via arena references

**Rust solution**: Change `stream_expand_token` to take `&BinXMLDeserializedTokens<'a>` instead of owned value. Requires careful lifetime management.

### 3. Reduce HashMap Usage (~6% of CPU time)

**Problem**: `reserve_unique_key` does HashSet lookups for every JSON key to detect duplicates.

**Observation**: Most JSON objects have < 20 keys. A linear scan of a `SmallVec` would be faster than hashing for small N.

### 4. SIMD String Processing (~3% of CPU time)

**Problem**: UTF-16 to UTF-8 conversion is still 16 samples despite ASCII fast path.

**Zig approach**: SIMD for strings >= 16 code units, processing 8 characters at once.

**Rust solution**: Use `simdutf` or `encoding_rs` crate for bulk conversion.

---

## Conclusion

The low-hanging fruit (ASCII fast path, direct JSON writing) gave ~10% improvement total.

The remaining 3.5x gap requires **architectural changes**:
1. Arena allocator (biggest impact, most invasive)
2. Reference-based template expansion
3. Smaller data structures for key deduplication
4. SIMD string conversion

These changes would require significant refactoring of the core data structures.

