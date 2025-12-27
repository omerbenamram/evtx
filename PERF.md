# Performance theses (living document)

This file is a running log of **hypotheses (“theses”)** and the **measurement protocol** used to validate them one by one.
It is modeled after `~/Workspace/mft/PERF.md` and is intended to be **agent-executable**: another agent should be able to
reproduce the same artifacts and conclusions.

Context / north star:
- We have a Zig implementation (`~/Workspace/zig-evtx`) that is materially faster.
- Our working hypothesis is that a large part of the gap is **allocator churn** in Rust (many small alloc/free + clone/memmove),
  while Zig leans on arena-style allocation and lower-copy dataflow.

Principles:
- **One change per experiment** (or one tightly-coupled set), with before/after measurements.
- Prefer **end-to-end CLI throughput** on a fixed input (`samples/security_big_sample.evtx`) as the primary KPI.
- Keep a **saved profile** for every checkpoint so we can explain wins/regressions.
- When results are noisy, prefer **median** and **min** over mean, and record variance.

---

## Canonical workloads (copy/paste)

Build (always):

```bash
cd /Users/omerba/Workspace/evtx
cargo build --release --features fast-alloc --locked --offline --bin evtx_dump
```

W1 (JSONL, end-to-end, single-thread, write suppressed):

```bash
./target/release/evtx_dump -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null
```

W2 (optional, multi-thread throughput; **not** used for baseline allocator-churn tracking):

```bash
./target/release/evtx_dump -t 8 -o jsonl samples/security_big_sample.evtx > /dev/null
```

Notes:
- Redirecting output is critical; otherwise you benchmark terminal I/O and buffering, not parsing/serialization.
- **All reference baselines in this repo use `-t 1`**. It best highlights allocator churn and per-chunk work on a single core.

---

## Quiet-machine guard (recommended)

Benchmarks are extremely sensitive to background load (Spotlight indexing, builds, browser tabs, etc).
To avoid “busy machine” noise, use `scripts/ensure_quiet.sh`:

```bash
cd /Users/omerba/Workspace/evtx
./scripts/ensure_quiet.sh
```

For hyperfine runs, prefer using it as a prepare hook (prepare time is not included in timings):

```bash
hyperfine --prepare ./scripts/ensure_quiet.sh ...
```

For the Rust-vs-Zig harness, enable it via:

```bash
QUIET_CHECK=1 ./profile_comparison.sh --bench-only
```

Tune thresholds via env vars (see `scripts/ensure_quiet.sh`):
- `QUIET_IDLE_MIN` (default `90`)
- `QUIET_LOAD1_MAX` (default `2.0`)
- `QUIET_MAX_WAIT_SEC` (default `60`)

---

## Baseline harness (Rust vs Zig)

Use `profile_comparison.sh` for quick Rust-vs-Zig baselines and to print top leaf frames (helpful to validate allocator-churn hypotheses):

```bash
cd /Users/omerba/Workspace/evtx
./profile_comparison.sh --bench-only
./profile_comparison.sh --top-leaves
```

Environment variables (see script header for full list):
- `SAMPLE_FILE` (defaults to `samples/security_big_sample.evtx`)
- `RUNS` (hyperfine runs)
- `OUTPUT_DIR` (defaults to `./profile_results`, ignored by git)
- `ZIG_BINARY` (defaults to `~/Workspace/zig-evtx/zig-out/bin/evtx_dump_zig`)

---

## Baseline environment (2025-12-27)

- **OS**: Darwin 25.2.0 (arm64)
- **HW**: Apple M3 Pro, 11 cores, 36 GB RAM
- **Toolchain**: rustc 1.92.0 (LLVM 21.1.3), cargo 1.92.0
- **Tools**: hyperfine 1.20.0, samply 0.13.1, zig 0.15.2

---

## Baseline environment (omer-pc, 2025-12-27)

- **OS**: Arch Linux (kernel 6.17.9, x86_64)
- **HW**: AMD Ryzen 9 3900X (12C/24T), 62 GiB RAM
- **Toolchain**: rustc 1.92.0 (LLVM 21.1.6), cargo 1.92.0
- **Tools**: hyperfine 1.20.0, samply 0.13.1, zig 0.15.2
- **Kernel settings (for sampling)**:
  - `kernel.perf_event_paranoid <= 1` (samply uses perf events)
  - `kernel.perf_event_mlock_kb >= 8192` (otherwise samply can fail with `mmap failed`)

---

## Baseline numbers (omer-pc, 2025-12-27)

Measured on `omer-pc` via SSH. We sync two trees (`origin/master` snapshot and this branch) and compare end-to-end JSONL throughput.
We gate runs with `scripts/ensure_quiet.sh` but loosened load-average tolerance because the box maintains a steady load (~4) while
being effectively idle (CPU idle ~99%).

W1 (JSONL, `-t 1`, output suppressed) — **reference baseline**:
- **master**: **median 883.6 ms**, mean 891.5 ms ± 28.7 ms (range 873.6–993.2 ms)
- **branch**: **median 599.6 ms**, mean 601.1 ms ± 6.1 ms (range 589.7–611.9 ms)
- **speedup**: ~**1.47×** (≈ **32%** lower wall time)

Repro commands (on `omer-pc`):

```bash
BASE=/tmp/evtx-bench
SAMPLE=$BASE/master/samples/security_big_sample.evtx

# Wait for a "quiet enough" machine before each benchmark batch.
QUIET_IDLE_MIN=95 QUIET_LOAD1_MAX=8 $BASE/branch/scripts/ensure_quiet.sh

hyperfine --warmup 3 --runs 20 \
  "$BASE/master/target/release/evtx_dump -t 1 -o jsonl $SAMPLE > /dev/null" \
  "$BASE/branch/target/release/evtx_dump -t 1 -o jsonl $SAMPLE > /dev/null"
```

Raw JSON capture (temporary on that run): `/tmp/evtx-bench.11jAUq/hyperfine_master_vs_branch_t1.json`.

---

## Rust vs Zig snapshot (omer-pc, 2025-12-27, pre-H2)

W1 (JSONL, `-t 1`, output suppressed), built from this working tree and `~/Workspace/zig-evtx`:
- **Rust (fast-alloc)**: median **532.4 ms** (mean 531.3 ms ± 5.5 ms, min 517.0 ms)
- **Zig (ReleaseFast --no-checks)**: median **258.3 ms** (mean 258.0 ms ± 1.1 ms, min 255.2 ms)
- **gap**: Zig is **~2.06× faster**

Artifacts (copied into this repo, ignored by git):
- `target/perf/rust_vs_zig_omerpc_20251227_172444/hyperfine_rust_vs_zig_t1.json`
- `target/perf/rust_vs_zig_omerpc_20251227_172444/samply_rust_t1.profile.json.gz` + `.syms.json`
- `target/perf/rust_vs_zig_omerpc_20251227_172444/samply_zig_t1.profile.json.gz` + `.syms.json`
- Extracted tables:
  - `.../top_leaves_rust_cpu.md`, `.../leaf_callers_rust.md`, `.../top_inclusive_rust_cpu.md`
  - `.../top_leaves_zig_cpu.md`, `.../leaf_callers_zig.md`, `.../top_inclusive_zig_cpu.md`

---

## Rust vs Zig snapshot (omer-pc, 2025-12-27, H2)

W1 (JSONL, `-t 1`, output suppressed), built from this branch and `~/Workspace/zig-evtx`:
- **Rust (fast-alloc, H2)**: mean **428.7 ms** ± 3.2 ms (min 422.6 ms)
- **Zig (ReleaseFast --no-checks)**: mean **256.9 ms** ± 3.1 ms (min 252.0 ms)
- **gap**: Zig is **~1.67× faster** (down from ~2.06× pre-H2)

Artifacts (copied into this repo, ignored by git):
- `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/hyperfine_rust_vs_zig_t1.json`
- `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/samply_rust_t1.profile.json.gz` + `.syms.json`
- `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/samply_zig_t1.profile.json.gz` + `.syms.json`
- Extracted tables:
  - `.../top_leaves_rust_cpu.md`, `.../leaf_callers_rust.md`, `.../top_inclusive_rust_cpu.md`
  - `.../top_leaves_zig_cpu.md`, `.../leaf_callers_zig.md`, `.../top_inclusive_zig_cpu.md`

---

## Rust vs Zig snapshot (macOS, 2025-12-27, H2)

W1 (JSONL, `-t 1`, output suppressed), built from this branch and `~/Workspace/zig-evtx`:
- **Rust (fast-alloc, H2)**: mean **253.3 ms** ± 7.3 ms (min 244.5 ms)
- **Zig (ReleaseFast --no-checks)**: mean **176.8 ms** ± 3.4 ms (min 172.4 ms)
- **gap**: Zig is **~1.43× faster**

Artifacts (ignored by git):
- `target/perf/rust_vs_zig_macos_h2_20251227_194318/hyperfine_rust_vs_zig_t1.json`
- `target/perf/rust_vs_zig_macos_h2_20251227_194318/samply_rust_t1.profile.json.gz` + `.syms.json`
- `target/perf/rust_vs_zig_macos_h2_20251227_194318/samply_zig_t1.profile.json.gz` + `.syms.json`
- Extracted tables:
  - `.../top_leaves_rust_cpu.md`, `.../leaf_callers_rust.md`, `.../top_inclusive_rust_cpu.md`
  - `.../top_leaves_zig_cpu.md`, `.../leaf_callers_zig.md`, `.../top_inclusive_zig_cpu.md`

Notes (what the profiles suggest to do next):
- Rust’s top leaf/self costs now cluster around:
  - template instance value decoding: `read_template_cursor` + `BinXmlValue::deserialize_value_type_cursor` + UTF-16 decode
  - remaining `serde_json` fallback paths in `JsonStreamOutput` (`Value::from`, `serialize_str`)
  - large residual `_platform_memmove` (callers include key writing + serde_json string serialization + UTF-16→String)
- Zig spends most of its time in:
  - `render_json.writeElementBodyJson` + `util_string.writeUtf16LeJsonEscaped` + placeholder resolution (`cloneAndResolve`)

---

## Agent playbook (reproducible workflow)

### Naming & artifacts (do this consistently)

Pick the next hypothesis ID: `H{N}` (monotonic, don’t reuse IDs).

- **Branch**: `perf/h{N}-{short-slug}` (example: `perf/h7-no-clone-template-expansion`)
- **Saved binaries** (so benchmarks are stable and diffable):
  - `target/release/evtx_dump.h{N}_before`
  - `target/release/evtx_dump.h{N}_after`
- **Hyperfine JSON**:
  - `target/perf/h{N}-before-vs-after.hyperfine.json`
- **Samply profiles** (merge by running many iterations):
  - `target/perf/samply/h{N}_before.profile.json.gz`
  - `target/perf/samply/h{N}_after.profile.json.gz`

### Step-by-step: run an experiment end-to-end

#### 0) Start a new thesis

```bash
cd /Users/omerba/Workspace/evtx
git checkout -b perf/h{N}-{short-slug}
```

Add an entry under “Theses / hypotheses backlog” with:
- **Claim**
- **Evidence** (what profile frames point at, especially allocator churn: malloc/free/memmove)
- **Change** (minimal code change to test)
- **Success metric** (e.g. W1 improves ≥ 5% median)
- **Guardrails** (correctness constraints; “don’t regress too much”)

#### 1) Build + snapshot the **before** binary

```bash
cd /Users/omerba/Workspace/evtx
cargo build --release --features fast-alloc --locked --offline --bin evtx_dump
cp -f target/release/evtx_dump target/release/evtx_dump.h{N}_before
```

#### 2) Record a stable **before** profile (Samply)

We merge many iterations so leaf frames are stable.

```bash
cd /Users/omerba/Workspace/evtx
mkdir -p target/perf/samply
samply record --save-only --unstable-presymbolicate --reuse-threads --main-thread-only \
  -o target/perf/samply/h{N}_before.profile.json.gz \
  --iteration-count 200 -- \
  ./target/release/evtx_dump.h{N}_before -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null
```

To view (serve locally and open the printed Firefox Profiler URL):

```bash
cd /Users/omerba/Workspace/evtx
samply load --no-open -P 4033 target/perf/samply/h{N}_before.profile.json.gz
```

What to record from the UI:
- **Invert call stack** for top **leaf/self** frames (watch for malloc/free/memmove, hashing, formatting).
- Normal Call Tree for inclusive buckets (template expansion, JSON emission, UTF-16 decode).

#### 3) Implement the change (keep it tight)

Primary focus areas (given allocator-churn hypothesis):
- Reduce clone/memmove in template expansion / token streaming.
- Avoid building intermediate `serde_json::Value` on hot paths (stream instead).
- Reduce per-record temporary allocations (strings/vectors/buffers), ideally by reusing buffers or using arenas.

If you find yourself changing 5+ unrelated things, split into multiple theses.

#### 4) Build + snapshot the **after** binary

```bash
cd /Users/omerba/Workspace/evtx
cargo build --release --features fast-alloc --locked --offline --bin evtx_dump
cp -f target/release/evtx_dump target/release/evtx_dump.h{N}_after
```

#### 5) Benchmark **before vs after in the same hyperfine command**

Always run both saved binaries in a single invocation and export JSON.

```bash
cd /Users/omerba/Workspace/evtx
mkdir -p target/perf
hyperfine --warmup 5 --runs 40 \
  --export-json target/perf/h{N}-before-vs-after.hyperfine.json \
  './target/release/evtx_dump.h{N}_before -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null' \
  './target/release/evtx_dump.h{N}_after  -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null'
```

If variance is high, amortize noise by looping inside each hyperfine run (keep the before/after pair in one command):

```bash
cd /Users/omerba/Workspace/evtx
hyperfine --warmup 2 --runs 15 \
  --export-json target/perf/h{N}-before-vs-after.hyperfine.json \
  --command-name 'before (20x)' "bash -lc 'for i in {1..20}; do ./target/release/evtx_dump.h{N}_before -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null; done'" \
  --command-name 'after  (20x)' "bash -lc 'for i in {1..20}; do ./target/release/evtx_dump.h{N}_after  -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null; done'"
```

#### 6) Record an **after** profile (Samply)

```bash
cd /Users/omerba/Workspace/evtx
samply record --save-only --unstable-presymbolicate --reuse-threads --main-thread-only \
  -o target/perf/samply/h{N}_after.profile.json.gz \
  --iteration-count 200 -- \
  ./target/release/evtx_dump.h{N}_after -t 1 -o jsonl samples/security_big_sample.evtx > /dev/null
```

#### 7) Correctness checks (pick strictness to match the thesis)

Always:

```bash
cd /Users/omerba/Workspace/evtx
cargo test --features fast-alloc --locked --offline
```

Semantic JSONL equality on a bounded range (preferred; formatting differences allowed):

```bash
cd /Users/omerba/Workspace/evtx
rm -f /tmp/evtx_before.jsonl /tmp/evtx_after.jsonl
./target/release/evtx_dump.h{N}_before -t 1 -o jsonl samples/security_big_sample.evtx > /tmp/evtx_before.jsonl
./target/release/evtx_dump.h{N}_after  -t 1 -o jsonl samples/security_big_sample.evtx > /tmp/evtx_after.jsonl
python3 - <<'PY'
import json
b = [json.loads(l) for l in open("/tmp/evtx_before.jsonl")]
a = [json.loads(l) for l in open("/tmp/evtx_after.jsonl")]
assert b == a, "semantic JSONL mismatch"
print("OK: semantic JSONL identical")
PY
```

#### 8) Update this file (`PERF.md`) with a write-up

Add a section under “Completed optimizations” (or “Rejected”) with:
- **What changed**
- **Benchmarks** (paste exact hyperfine command)
- **Extracted medians** (from exported JSON)
- **Speedup** (ratio and %)
- **Profile delta** (top leaf before/after; call out allocator churn shifts explicitly)
- **Correctness check**
- **Artifacts**: profile paths + hyperfine JSON path

#### 9) PR-quality finish

```bash
cd /Users/omerba/Workspace/evtx
cargo fmt
cargo clippy --all-targets --features fast-alloc --locked --offline
```

Commit message should match the thesis and observable change:

```bash
git commit -am "perf: H{N} {short description}"
```

---

## Attribution study: per-optimization deltas (omer-pc, 2025-12-27)

We measured how much each optimization contributes by doing a “one change reverted at a time” run:
- **Baseline**: this branch (`--features fast-alloc`), `-t 1`, JSONL, output suppressed.
- **Variant**: same, but revert exactly one optimization.

Artifact (exported hyperfine JSON, includes exact commands + full run distributions):
- `benchmarks/omer-pc_ablation_matrix_t1_20251227.json`

Results (median wall time deltas vs baseline; lower is better):

| Variant | Median (ms) | Δ vs baseline |
|---|---:|---:|
| baseline | 605.5 | (base) |
| revert: pre-expand templates | 750.1 | +23.88% |
| revert: chrono datetime formatting | 625.6 | +3.31% |
| revert: serde_json values | 615.6 | +1.66% |
| revert: serde_json strings | 611.8 | +1.03% |
| revert: UTF-16 ASCII fast-path | 600.6 | -0.81% |

Notes:
- This run was quiet-gated (`scripts/ensure_quiet.sh`, `QUIET_IDLE_MIN=95 QUIET_LOAD1_MAX=8`).
- The feature toggles used to build these variants were temporary and have since been removed; the JSON is the stable record.

---

## Theses / hypotheses backlog

Template (copy/paste):

### H{N} — {short title}
- **Claim**:
- **Evidence**:
- **Change**:
- **Success metric**:
- **Guardrails**:

### H1 — Kill remaining allocator churn in streaming JSON output (keys + buffered values)
- **Claim**: We can get a meaningful additional W1 speedup by eliminating the remaining hot-path heap churn in `JsonStreamOutput`
  (key allocation + `serde_json::Value` buffering), which currently shows up as `_rjem_malloc` / `_rjem_sdallocx` + `_platform_memmove`.
- **Evidence**:
  - **Samply (macOS, W1 `-t 1`, 120 iterations, output→`/dev/null`)** shows allocator + memmove as major leaf cost:
    - `_platform_memmove` ~7.1% leaf (top caller: `JsonStreamOutput::visit_open_start_element` ~29.5%, then `write_key` / `write_json_string_ncname`)
    - `_rjem_malloc` ~3.0% leaf (top caller: `RawVec::finish_grow` ~28.8%, then `JsonStreamOutput::visit_open_start_element` / `write_key`)
    - `RawVec::grow_one` callers: `XmlElementBuilder::attribute_value` ~35.7% and `JsonStreamOutput::visit_characters` ~35.6%
    - Remaining `serde_json` overhead is still measurable (`BinXmlValue -> serde_json::Value` + `Serializer::serialize_str` show up in top leaves),
      due to `buffered_values` / `data_values` paths.
  - **Zig renderer avoids this class of overhead entirely**:
    - It writes JSON directly from IR nodes without allocating per-key `String`s, and without buffering into `serde_json::Value`.
    - It uses a fixed-size, stack-allocated name-count table (`MAX_UNIQUE_NAMES = 64`) + pointer-equality fast path for name keys
      instead of hashing/allocating keys (`zig-evtx/src/parser/render_json.zig`, and rationale in `zig-evtx/docs/architecture.md`).
- **Change**:
  - **Reuse memory across records** (Zig-style) instead of allocating fresh per record:
    - Today `EvtxRecord::into_json_stream()` constructs a new `Vec<u8>` + a new `JsonStreamOutput` every record. Introduce a
      reusable per-thread/per-chunk “scratch” JSON emitter that:
        - keeps the output `Vec<u8>` and calls `clear()` per record (capacity retained),
        - keeps `frames` / `elements` vectors and clears them per record (capacity retained),
        - reuses duplicate-key tracking storage (see next bullets) instead of re-allocating HashSets.
    - The existing `EvtxChunkData.arena` is **per-chunk** and cannot be reset per record because it backs template cache + values,
      but we can add a **separate scratch bump** (per record) and `reset()` it after each record to recycle memory aggressively.
  - Make `JsonStreamOutput` lifetime-aware (`JsonStreamOutput<'a, W>`) so it can **store borrowed keys**:
    - Change `ElementState.name: String` → `Cow<'a, str>` (or `&'a str` where possible) to avoid `to_owned()`/`clone()` per element.
    - Replace `ObjectFrame.used_keys: HashSet<String>` with a borrowed-key structure and only allocate suffix keys on collision.
      If we keep hashing, store `&'a str` (borrowed) and allocate only suffixed strings into the per-record scratch bump.
      (Alternative: Zig-style fixed table + linear scan for ≤64 keys, avoiding hashing altogether.)
  - Replace `buffered_values: Vec<serde_json::Value>` and `data_values: Vec<serde_json::Value>` with a **borrow-friendly scalar buffer**
    (plain `Vec` with preallocation + reuse; avoid `smallvec`), and serialize via `write_binxml_value` / `write_json_string_*`
    to eliminate `serde_json::to_writer` from the hot path.
- **Success metric**:
  - **W1 median improves ≥ 8%** on `omer-pc` (quiet-gated), vs current branch baseline.
  - Samply shows reduced share of `_platform_memmove`, `_rjem_malloc`, and fewer `RawVec::grow_one` samples under JSON output.
- **Guardrails**:
  - Preserve legacy JSON semantics (duplicate key suffixing, EventData/Data special handling, `separate_json_attributes` behavior).
  - `cargo test --features fast-alloc --locked --offline` stays green, especially streaming parity suites.

### H2 — Compile templates to resolved-name JSON ops (avoid `XmlElementBuilder` + hashing in the hot path)
- **Claim**: The remaining Rust-vs-Zig gap is dominated by `stream_expand_token*` work (template expansion + name resolution +
  intermediate `XmlElementBuilder` objects). If we compile template definitions into a “ready-to-render” representation with
  **resolved names + precomputed key IDs**, and drive JSON output directly from those ops (no `XmlElementBuilder` / no per-token
  `StringCache` hashing / no `lasso` interning), we can plausibly win **≥20%** on W1.
- **Evidence** (omer-pc, samply, W1 `-t 1`, 200 iterations, output→`/dev/null`):
  - **Inclusive**: `stream_expand_token` ~73.8%, `stream_expand_template` ~73.0%, `stream_expand_token_ref` ~60.5%,
    `expand_string_ref` ~9.8%, `Rodeo::try_get_or_intern` ~5.9%, `BuildHasher::hash_one` ~5.5%.
  - **Leaf**: `stream_expand_token_ref` ~13.8%, `read_template_cursor` ~8.1%, `expand_string_ref` ~4.5%,
    `BuildHasher::hash_one` ~4.5% (mostly under `expand_string_ref`), `Rodeo::try_get_or_intern` ~4.5%,
    `XmlElementBuilder::{attribute_value,finish}` ~5.3% combined.
  - Zig’s hot path avoids these specific costs by:
    - Using IR with pre-converted names (`NameKey` pointer-equality fast path) and an arena allocator
      (`zig-evtx/src/parser/render_json.zig`), and
    - Fusing UTF-16LE→UTF-8 conversion + JSON escaping in one pass (`zig-evtx/src/parser/util_string.zig`).
  - Saved profiles + extracted tables: `target/perf/rust_vs_zig_omerpc_20251227_172444/` (see snapshot section above).
- **Change**:
  - **Compile template definitions** into a `CompiledTemplate` (per chunk) where open/attr/entity tokens store resolved names
    (`&'chunk str` or an offset-based `NameId`) and precomputed “JSON key bytes” where applicable (NCName fast path).
  - Add a **JSON-only fast visitor** that consumes these compiled ops directly (no `XmlElementBuilder`, no `Vec<XmlAttribute>`),
    and uses offset- or pointer-based IDs for duplicate key tracking (avoid `lasso` hashing).
  - (Stretch) Store template substitution values as **raw spans** (type + `&[u8]`) and serialize directly, enabling a fused
    UTF-16LE→JSON escape writer on the Rust side too.
- **Success metric**:
  - **W1 median improves ≥ 20%** on `omer-pc` (quiet-gated if possible).
  - Samply shows `expand_string_ref` / `hash_one` / `Rodeo::try_get_or_intern` and `XmlElementBuilder::*` largely disappear
    from the top hot path for JSON streaming.
- **Guardrails**:
  - Preserve JSON semantics (duplicate key suffixing, EventData/Data flattening, `separate_json_attributes` behavior).
  - Keep `cargo test --features fast-alloc --locked --offline` green.

### H3 — TemplateInstance “raw substitution spans” + `serde_json`-free streaming (fuse UTF-16→JSON)
- **Claim**: We can plausibly win **≥20%** more on W1 by eliminating the remaining “decode + allocate + convert” work that still
  dominates Rust profiles post-H2:
  - eager TemplateInstance substitution decoding (`read_template_cursor` + `BinXmlValue::deserialize_value_type_cursor`),
  - UTF-16→`String` allocations (`String::from_utf16`), and
  - `serde_json::Value` buffering + serialization (`Value::from`, `serialize_str`).
  Zig’s renderer avoids these by keeping values as raw spans + fusing UTF-16→JSON escaping.
- **Evidence**:
  - **macOS, H2, W1 `-t 1`, 200 iterations (Samply)** (`target/perf/rust_vs_zig_macos_h2_20251227_194318/`):
    - **Inclusive**: `read_template_cursor` **~24.0%**, `BinXmlValue::deserialize_value_type_cursor` **~19.0%**,
      `ByteCursor::utf16_by_char_count_trimmed` **~8.2%**
    - **Leaf**: `_platform_memmove` **~7.0%** (callers include `write_key_id`, `serde_json::serialize_str`, UTF-16→String),
      `read_template_cursor` **~5.0%**, `drop_in_place<DeserializationError>` **~4.6%**
  - **Zig, same run**:
    - `render_json.writeElementBodyJson` **~15.5%** leaf
    - `util_string.writeUtf16LeJsonEscaped` **~14.5%** leaf (fused UTF-16LE→JSON escape)
- **Change** (big, but contained to the JSON streaming path):
  - Parse `TemplateInstance` into a lightweight per-record structure of **raw substitution spans**
    (`(value_type, size, value_offset)`) instead of eagerly producing `Vec<BinXmlValue>`.
  - Teach the compiled-template JSON interpreter (`Asm::expand_template`) to:
    - decode a substitution **only when it is used** by `CompiledTemplateOp::Substitution`,
    - cache decoded values by index (borrowed where possible) to avoid cloning on repeated references,
    - write values directly via a `write_binxml_value_*` path (no `serde_json::Value` in `BufferedValues` / `data_values`).
  - Implement a fused writer for UTF-16LE strings to JSON (Zig-style): write JSON-escaped UTF-8 directly from `&[u8]` UTF-16LE
    bytes, avoiding `Vec<u16>` + `String::from_utf16` + second-pass escaping.
  - Fix the pathological “construct-and-drop errors on success path” hot spot by making value-type decoding lazy
    (`ok_or_else` / `match` instead of `ok_or(DeserializationError { ... })`) in the TemplateInstance loops.
- **Success metric**:
  - **W1 median improves ≥ 20%** on at least one stable machine (omer-pc or macOS), and preferably both.
  - Samply top leafs show `read_template_cursor`, `String::from_utf16`, and the `serde_json` frames dropping materially.
- **Guardrails**:
  - Preserve JSON semantics (duplicate key suffixing, EventData/Data special handling, `separate_json_attributes` behavior).
  - Keep `cargo test --features fast-alloc --locked --offline` green (especially streaming parity suites).

---

## Completed optimizations

### Stream template expansion (avoid pre-expanding templates)
- **What changed**: Template expansion happens inline during streaming output, so substitution values can be *moved on last use* instead of cloned. This avoids building an expanded token Vec up-front.
- **Where**: `src/binxml/assemble.rs` (streaming path).
- **Impact (omer-pc, `-t 1`)**: reverting to the older “pre-expand templates” approach regresses **+23.88%** median (605.5 ms → 750.1 ms). This is the dominant contributor in the ablation study.

### JSON string serialization (avoid `serde_json` for string escaping)
- **What changed**: Serialize strings directly with a fast “no-escape needed” check + manual escaping for `"` `\\` control chars.
- **Where**: `src/json_stream_output.rs` (`write_json_string_*`).
- **Impact (omer-pc, `-t 1`)**: reverting to `serde_json::to_writer` for strings regresses **+1.03%** median (605.5 ms → 611.8 ms).

### JSON value serialization (avoid `serde_json::Value` allocations)
- **What changed**: Serialize `BinXmlValue` primitives directly (itoa/ryu for numbers; direct writes for bool/null/binary), avoiding intermediate JSON value construction.
- **Where**: `src/json_stream_output.rs` (`write_binxml_value`).
- **Impact (omer-pc, `-t 1`)**: reverting to `serde_json::Value` regresses **+1.66%** median (605.5 ms → 615.6 ms).

### Datetime formatting (avoid chrono format string parsing)
- **What changed**: Write ISO-8601 timestamps directly (`YYYY-MM-DDTHH:MM:SS.ffffffZ`) instead of `dt.format(...).to_string()`.
- **Where**: `src/json_stream_output.rs` (FileTime/SysTime serialization).
- **Impact (omer-pc, `-t 1`)**: reverting to chrono formatting regresses **+3.31%** median (605.5 ms → 625.6 ms).

### H1 (partial) — Reuse scratch buffer + reduce key/value churn in streaming JSONL output
- **What changed**:
  - `evtx_dump` (`-o jsonl`, `--json-parser streaming`, `-t 1`) now reuses a single `JsonStreamOutput<Vec<u8>>` across records and
    writes it directly to the output stream (avoids per-record `Vec<u8>` + `String` allocation in `EvtxRecord::into_json_stream()`).
  - `JsonStreamOutput` reduces per-record heap churn by:
    - interning element keys (`Arc<str>`) instead of allocating `String` per element,
    - using an inline “one value” buffer for `buffered_values` / aggregated `Data` values (avoids many small `Vec` allocations),
    - recycling per-object duplicate-key tracking frames (reuses `HashSet` allocations across records).
- **Benchmarks (omer-pc, quiet-gated, W1)**:
  - **before**: median **607.0 ms**
  - **after**: median **572.4 ms**
  - **speedup**: **1.061×** (≈ **5.7%** lower median)
  - **Command (omer-pc)**:

```bash
BASE=/tmp/evtx-h1-bench
SAMPLE=$BASE/before/samples/security_big_sample.evtx

QUIET_IDLE_MIN=95 QUIET_LOAD1_MAX=8 $BASE/after/scripts/ensure_quiet.sh
hyperfine --warmup 3 --runs 25 \
  --export-json $BASE/h1-before-vs-after.hyperfine.json \
  "$BASE/before/target/release/evtx_dump -t 1 -o jsonl $SAMPLE > /dev/null" \
  "$BASE/after/target/release/evtx_dump  -t 1 -o jsonl $SAMPLE > /dev/null"
```

  - **Artifact**: `target/perf/h1-before-vs-after.hyperfine.json` (copied from `omer-pc:/tmp/evtx-h1-bench/h1-before-vs-after.hyperfine.json`)

- **Profile delta (macOS, samply, W1, 200 iterations)**:
  - `_platform_memmove`: **7.38% → 4.33%** leaf
  - `alloc::raw_vec::RawVecInner<A>::finish_grow`: **1.62% → 0.88%** leaf
  - `alloc::raw_vec::RawVec<T,A>::grow_one`: **0.71% → 0.44%** leaf
  - `_rjem_malloc`: **3.15% → 1.09%** leaf
  - `_rjem_sdallocx.cold.1`: **3.77% → 1.75%** leaf
  - **Artifacts**:
    - `target/perf/samply/h1_before.profile.json.gz` + `target/perf/samply/h1_before.profile.json.syms.json`
    - `target/perf/samply/h1_after.profile.json.gz` + `target/perf/samply/h1_after.profile.json.syms.json`
- **Correctness check**: `cargo test --features fast-alloc --locked`
- **Notes**: This was a partial step; the follow-up “Zig-style duplicate-key tracking” below removes hash/memcmp hotspots and
  crosses the original H1 ≥8% target on `omer-pc`.

### H1 (finish) — Zig-style duplicate-key tracking (fixed table + interned-key IDs)
- **What changed**:
  - Replaced per-object `HashSet` duplicate-key tracking with a Zig-style fixed table (`MAX_UNIQUE_NAMES = 64`) + per-base suffix counters
    in `JsonStreamOutput` (`UniqueKeyTable`).
  - Duplicate-key membership checks are against interned key IDs (no per-key hashing on the hot path); suffixed keys (`_1`, `_2`, …)
    are only allocated on collision.
  - Switched the streaming key interner to `lasso::Rodeo` (enabled `ahasher` + `inline-more`) to reduce interning hashing overhead.
- **Benchmarks (omer-pc, quiet-gated, W1)**:
  - **before**: median **609.1 ms**
  - **after**: median **526.3 ms**
  - **speedup**: **1.157×** (≈ **13.6%** lower median)
  - **Command (omer-pc)**:

```bash
BASE=/tmp/evtx-h1-bench
SAMPLE=$BASE/before/samples/security_big_sample.evtx

QUIET_IDLE_MIN=95 QUIET_LOAD1_MAX=8 $BASE/after/scripts/ensure_quiet.sh
hyperfine --warmup 3 --runs 25 \
  --export-json $BASE/h1-lasso-ahash-before-vs-after.hyperfine.json \
  "$BASE/before/target/release/evtx_dump -t 1 -o jsonl $SAMPLE > /dev/null" \
  "$BASE/after/target/release/evtx_dump  -t 1 -o jsonl $SAMPLE > /dev/null"
```

  - **Artifact**: `target/perf/h1-lasso-ahash-before-vs-after.hyperfine.json` (copied from `omer-pc:/tmp/evtx-h1-bench/h1-lasso-ahash-before-vs-after.hyperfine.json`)

- **Profile delta (macOS, samply, W1, 200 iterations)**:
  - **Key-tracking hot path (after1 → after2)**:
    - `hashbrown::map::HashMap<K,V,S,A>::get_inner`: **3.20% → 0.00%** leaf
    - `hashbrown::map::HashMap<K,V,S,A>::insert`: **1.83% → 0.00%** leaf
    - `_platform_memcmp`: **2.99% → 2.43%** leaf
    - `evtx::json_stream_output::UniqueKeyTable::reserve_unique_index`: **0.00% → 2.17%** leaf (replacement cost)
  - **Key interning (after3 → after4)**:
    - `<core::hash::sip::Hasher<S> as core::hash::Hasher>::write`: **7.32% → 2.01%** leaf (enabling `lasso` `ahasher`)
  - **Final vs baseline (before → after4)**:
    - `_platform_memmove`: **7.38% → 4.80%** leaf
    - `_rjem_malloc`: **3.15% → 1.23%** leaf
    - `alloc::raw_vec::RawVecInner<A>::finish_grow`: **1.62% → 0.96%** leaf
  - **Artifacts**:
    - `target/perf/samply/h1_after2.profile.json.gz` + `target/perf/samply/h1_after2.profile.json.syms.json`
    - `target/perf/samply/h1_after3.profile.json.gz` + `target/perf/samply/h1_after3.profile.json.syms.json`
    - `target/perf/samply/h1_after4.profile.json.gz` + `target/perf/samply/h1_after4.profile.json.syms.json`

### H2 — Compiled template ops + offset-indexed names + JSON streaming without `XmlElementBuilder`
- **What changed**:
  - Replaced `StringCache(HashMap<ChunkOffset, BinXmlName>)` with an **offset-indexed table** to eliminate per-name hashing
    in `expand_string_ref`-style lookups.
    - **Where**: `src/string_cache.rs`
  - `TemplateCache` now stores a **compiled template program** (`CompiledTemplateOp`) and **precomputed substitution use-counts**,
    so expanding a `TemplateInstance` no longer scans template tokens twice to count substitution references.
    - **Where**: `src/template_cache.rs`, used by `src/binxml/assemble.rs` `parse_tokens_streaming_json`
  - Introduced a JSON-only streaming assembler that:
    - expands templates using the compiled ops,
    - collects attributes as `(name_offset, BinXmlValue)` (no `XmlElementBuilder` / `Vec<XmlAttribute>`),
    - calls new offset-based JSON visitor hooks (`visit_open_start_element_offsets` / `visit_close_element_offset`).
    - **Where**: `src/binxml/assemble.rs` (`parse_tokens_streaming_json`)
  - `EvtxRecord::{into_json_stream, write_json_stream}` now use this new JSON streaming path.
    - **Where**: `src/evtx_record.rs`
- **Benchmarks (omer-pc, W1, `-t 1`, output suppressed)**:
  - **pre-H2** (Rust only): median **532.4 ms** (from `target/perf/rust_vs_zig_omerpc_20251227_172444/hyperfine_rust_vs_zig_t1.json`)
  - **H2**: median **428.6 ms** (from `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/hyperfine_rust_vs_zig_t1.json`)
  - **speedup**: **1.24×** (≈ **19.5%** lower median wall time)
  - **Zig baseline** (same run): median **256.2 ms** → Zig still **~1.67× faster** than Rust after H2
- **Profile delta (omer-pc, samply, 200 iterations, W1)**:
  - **Gone from the top**: `evtx::binxml::assemble::stream_expand_token_ref` and `core::hash::BuildHasher::hash_one`
    (the old “template streaming + HashMap string cache hashing” hotspot).
  - **New top leafs (Rust, H2)**:
    - `evtx::binxml::tokens::read_template_cursor` (~9.9% leaf) — substitution value parsing
    - `JsonStreamOutput::visit_open_start_element_offsets` (~5.0% leaf) — offset-based open hook
    - `Asm::expand_template` (~5.0% leaf) — compiled-template expansion driver
    - `BinXmlValue::clone` (~4.5% leaf) — mostly cloning borrowed template values when buffering attrs
  - **Artifacts**:
    - `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/samply_rust_t1.profile.json.gz` + `.syms.json`
    - `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/top_leaves_rust_cpu.md`
    - `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/top_inclusive_rust_cpu.md`
    - `target/perf/rust_vs_zig_omerpc_h2_20251227_182359/leaf_callers_rust.md`
- **Correctness check**: `cargo test --features fast-alloc --locked --offline` (incl. full streaming parity suites)

---

## Rejected theses

### UTF-16 ASCII fast-path (rejected; removed)
- **What changed**: Tried scanning UTF-16 units for “all <= 0x7F” and building an ASCII string directly.
- **Where**: `src/utils/utf16.rs` (`decode_utf16_units_z`).
- **Result (omer-pc, `-t 1`)**: reverting this “fast path” was **-0.81%** (slightly faster), i.e. the scan overhead outweighed the benefit for our canonical workload (within noise but wrong direction).
- **Decision**: Removed the ASCII fast-path; use `String::from_utf16` unconditionally.
