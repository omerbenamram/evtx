# `utf16-simd`

SIMD-accelerated UTF-16/UTF-16LE → UTF-8 conversion with **JSON** and **XML** escaping.

This crate is designed for workloads like Windows Event Log (EVTX) parsing where:
- input strings are typically **UTF-16LE bytes** from an unaligned buffer, and
- ~most strings are **pure ASCII**, with occasional escapes.

It provides:
- **JSON escaping** (`\"`, `\\`, `\n`, …, and control chars as `\u00XX`)
- **XML escaping** (`&amp;`, `&lt;`, `&gt;`, plus `&quot;`/`&apos;` for attributes)
- **Raw** UTF-16 → UTF-8 conversion (no escaping)

The hot path is modeled after the SIMD string escaping approach popularized by **`sonic-rs`**, adapted from `u8` lanes to **UTF-16 code-unit (`u16`) lanes**, and cross-validated against the Zig EVTX implementation (`zig-evtx`) that uses a similar ASCII-first strategy.

---

## Quick start

### Escape UTF-16LE bytes (unaligned) to JSON (borrowed output)

```rust
use utf16_simd::Scratch;

let utf16le: &[u8] = b"H\0i\0 \0\"\0!\0"; // "Hi \"!"
let num_units = utf16le.len() / 2;

let mut scratch = Scratch::new();
let out = scratch.escape_json_utf16le(utf16le, num_units, true);

assert_eq!(out, br#""Hi \"!""#);
```

### Escape `&[u16]` (wide strings) to JSON

```rust
use utf16_simd::Scratch;

let wide: &[u16] = &[b'H' as u16, b'i' as u16, b' ' as u16, 0xD83D, 0xDE00]; // "Hi 😀"

let mut scratch = Scratch::new();
let out = scratch.escape_json_utf16(wide, true);

assert_eq!(std::str::from_utf8(out).unwrap(), "\"Hi 😀\"");
```

Notes:
- `&[u16]` works naturally with many “wide string” crates via deref coercions.
- On **Windows**, the `wchar` crate’s `wchar_t` is `u16`, so `wch!()/wchz!()` output can be passed directly.

### Interop: `wchar` / `widestring`

```rust,ignore
// Windows-only: `wchar_t` is UTF-16 (u16).
use wchar::wchz;
use utf16_simd::Scratch;

let wide: &[wchar::wchar_t] = wchz!("Hello"); // NUL-terminated
let wide = &wide[..wide.len() - 1]; // drop trailing NUL

let mut scratch = Scratch::new();
let json = scratch.escape_json_utf16(wide, true);
assert_eq!(std::str::from_utf8(json).unwrap(), "\"Hello\"");
```

```rust,ignore
use widestring::U16CString;
use utf16_simd::Scratch;

let s = U16CString::from_str("Hello").unwrap();

let mut scratch = Scratch::new();
let json = scratch.escape_json_utf16(s.as_slice(), true);
assert_eq!(std::str::from_utf8(json).unwrap(), "\"Hello\"");
```

### Stream into any `std::io::Write`

```rust
use std::io::Write;
use utf16_simd::Scratch;

let utf16le: &[u8] = b"A\0\n\0B\0";
let units = utf16le.len() / 2;

let mut scratch = Scratch::new();
let mut out = Vec::<u8>::new();
scratch.write_json_utf16le_to(&mut out, utf16le, units, true).unwrap();

assert_eq!(out, b"\"A\\nB\"");
```

---

## Support matrix

### Inputs

| Input | API | Notes |
|------:|-----|------|
| `&[u8]` UTF-16LE bytes | `escape_*_utf16le(...)` | Safe for **unaligned** EVTX buffers. Provide `num_units`. Trailing odd byte is ignored. |
| `&[u16]` UTF-16 code units | `escape_*_utf16(...)` | Endianness-independent at the API level. Slice length is the unit count. |

**Wide-string crates**

| Crate/type | Works with | Notes |
|-----------:|------------|------|
| `widestring::U16Str` / `U16CString` | `&[u16]` APIs | Typically deref-coerce to `&[u16]`. |
| `wchar::wch!()/wchz!()` | `&[u16]` APIs on **Windows** | On non-Windows platforms `wchar_t` is usually `u32` → not UTF‑16. |

### Outputs

| Output | API | Allocations |
|------:|-----|------------|
| `&mut [MaybeUninit<u8>]` | `escape_*` | **None** (caller-provided buffer) |
| borrowed `&[u8]` | `Scratch::{escape_*}` | amortized (scratch grows, then reuses) |
| `Vec<u8>` | `escape_*_into` | amortized (reuses vector allocation) |
| `io::Write` | `Scratch::{write_*_to}` | amortized (reuses scratch) |

### SIMD / platforms

| Target | SIMD | Selection |
|-------:|------|-----------|
| `x86_64` | SSE2 | runtime feature detect (`is_x86_feature_detected!("sse2")`) |
| `aarch64` | NEON | always available (baseline) |
| other | none | scalar fallback |

### Features

| Feature | Default | What it does |
|--------:|:-------:|--------------|
| `perf-counters` | off | enable lightweight counters for JSON escaping call distribution |
| `sonic-writeext` | off | (optional) exposes `write_*_utf16le()` functions that write directly into `sonic-rs::writer::WriteExt` spare capacity |

---

## Semantics (important!)

### UTF-16 validity

- **Valid surrogate pairs** are decoded and emitted as 4-byte UTF‑8.
- **Lone surrogates** are **silently dropped** (WTF‑16 style). This matches the behavior in the Zig EVTX implementation and is intentional for robustness on “dirty” logs.

### JSON escaping rules

- Always escapes: `"` and `\`
- Named escapes:

```text
0x08 -> \b
0x0C -> \f
0x0A -> \n
0x0D -> \r
0x09 -> \t
```

- Remaining ASCII control bytes `0x00..=0x1F` become `\u00XX` (uppercase hex).
- Non-ASCII characters are emitted as UTF‑8 bytes (no `\u{...}` escaping).

ASCII table (JSON):

```text
+--------------------+----------------+
| input              | output         |
+--------------------+----------------+
| "                  | \"             |
| \                  | \\             |
| 0x08               | \b             |
| 0x0C               | \f             |
| 0x0A               | \n             |
| 0x0D               | \r             |
| 0x09               | \t             |
| 0x00..=0x1F (rest) | \u00XX         |
| everything else    | UTF-8 bytes    |
+--------------------+----------------+
```

### XML escaping rules

- Always escapes: `&`, `<`, `>`
- If `in_attribute=true`, also escapes: `"` and `'`

ASCII table (XML):

```text
+-------+-----------+
| input | output    |
+-------+-----------+
| &     | &amp;     |
| <     | &lt;      |
| >     | &gt;      |
| "     | &quot; (*) |
| '     | &apos; (*) |
+-------+-----------+
(* only when in_attribute = true)
```

---

## Buffer sizing

The crate exposes:

```rust
use utf16_simd::max_escaped_len;

let units = 10;
let cap = max_escaped_len(units, true);
assert!(cap >= units * 6 + 2);
```

It is a safe upper bound for all modes:
- Worst case per code unit is **6 bytes**:
  - JSON: `\u00XX` is 6 bytes.
  - XML: `&quot;` / `&apos;` are 6 bytes.
- JSON optionally adds `+2` for surrounding quotes.

---

## How it works (SIMD + ASCII-first)

### 1) UTF-16LE bytes vs `u16` code units

EVTX strings are stored as **little-endian bytes**:

```text
bytes:  [lo0 hi0 lo1 hi1 lo2 hi2 ...]
units:   u0     u1     u2
```

For ASCII in UTF-16LE:
- `hi == 0`
- `lo <= 0x7F`
- output byte is just `lo` (already valid UTF‑8)

### 2) 8-wide SIMD block

The SIMD path processes 8 UTF‑16 code units at a time (128 bits):

```text
128-bit register as 8 lanes of u16:
+----+----+----+----+----+----+----+----+
| u0 | u1 | u2 | u3 | u4 | u5 | u6 | u7 |
+----+----+----+----+----+----+----+----+
```

### 3) “Fast copy” check (ASCII + no escapes)

For JSON:
- **ASCII check**: `(u & 0xFF80) == 0`  ⇒ `u <= 0x7F`
- **needs escaping** if any lane is:
  - `u == '"'`
  - `u == '\\'`
  - `u <= 0x1F` (control)

If all 8 lanes are ASCII *and* none need escaping:
- pack the 8×`u16` to 8×`u8`
- store 8 output bytes
- advance input by 8 code units

ASCII “bit test” intuition:

```text
u <= 0x7F  <=>  u & 0xFF80 == 0
u <= 0x1F  <=>  u & 0xFFE0 == 0
```

### 4) Slow path (per-lane)

If any lane is non-ASCII or needs escaping, we fall back to a tight per-lane loop
that categorizes each code unit:

```text
ASCII?     -> maybe escape, else write byte
< 0x800    -> write 2-byte UTF-8
surrogate? -> decode pair, write 4-byte UTF-8 (else skip)
else       -> write 3-byte UTF-8
```

Surrogate pairs at the SIMD block boundary are handled by peeking the next unit:

```text
block N ends with:    [ ... 0xD83D ]   (high surrogate)
block N+1 begins:     [ 0xDE00 ... ]   (low surrogate)
=> consume an "extra" unit from block N+1 and emit 4 UTF-8 bytes.
```

### 5) Why this structure wins for EVTX

In EVTX, most strings are:
- short
- ASCII
- rarely contain `"`/`\`/controls

So the fast path often becomes a tight “load + mask + store” loop with very few branches.

---

## Credits / references

- `sonic-rs` (Rust): SIMD string escaping design (adapted for UTF‑16 code units).
  See `sonic-rs`’ `format_string` implementation in `src/util/string.rs`.
- `zig-evtx` (Zig): UTF‑16LE→UTF‑8 fused conversion + ASCII-first approach and edge-case behavior.

Links:
- `https://crates.io/crates/sonic-rs`
- `https://docs.rs/sonic-rs`
- `https://github.com/cloudwego/sonic-rs`
- `https://github.com/omerbenamram/EVTX`
- `https://github.com/omerbenamram/zig-evtx`

---

## Fuzzing

There are `cargo-fuzz` harnesses under `utf16-simd/fuzz/` that:

- Compare **SIMD vs scalar** output for JSON/XML/raw conversion.
- Validate the produced bytes are **valid UTF-8**.

Example:

```bash
# from the repo root
cargo +nightly install cargo-fuzz

cargo +nightly fuzz run json_utf16le --manifest-path utf16-simd/fuzz/Cargo.toml
```

