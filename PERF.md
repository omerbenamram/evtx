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

W2 (JSONL, end-to-end, 8 threads, write suppressed):

```bash
./target/release/evtx_dump -t 8 -o jsonl samples/security_big_sample.evtx > /dev/null
```

Notes:
- Redirecting output is critical; otherwise you benchmark terminal I/O and buffering, not parsing/serialization.
- `-t 1` is the primary KPI for single-core throughput and for making profiles readable.

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

## Optional: per-optimization attribution (ablation builds)

For quick “what % did each optimization contribute?”, you can use opt-in feature toggles (no effect unless enabled):
- `perf_ablate_no_utf16_ascii`
- `perf_ablate_serde_json_strings`
- `perf_ablate_serde_json_values`
- `perf_ablate_preexpand_templates`
- `perf_ablate_chrono_datetime_format`

Use isolated target dirs so builds don’t overwrite each other:

```bash
cd /Users/omerba/Workspace/evtx
CARGO_TARGET_DIR=target/perf/ablate/no_utf16_ascii \
  cargo build --release --features 'fast-alloc,perf_ablate_no_utf16_ascii' --locked --offline --bin evtx_dump
```

Then benchmark binaries side-by-side in a single `hyperfine` invocation.

---

## Theses / hypotheses backlog

Template (copy/paste):

### H{N} — {short title}
- **Claim**:
- **Evidence**:
- **Change**:
- **Success metric**:
- **Guardrails**:

---

## Completed optimizations

### (placeholder)
Add completed H{N} sections here following the mft-style format (What changed / Benchmarks / Profile delta / Correctness / Artifacts).

---

## Rejected theses

If the benchmark is within noise or regresses, document it here (numbers + profile evidence + next idea).
