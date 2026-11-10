You are working in the `evtx` Rust project and running a **targeted performance improvement pass**.
Treat anything the user types **after this command** as the *focus for this pass* (e.g. "reduce allocations in template cache", "speed up JSON writer", "optimize XML path").

`AGENTS.md` holds the rules of this repository. Its code rule "Speed is a feature" names the sample and the tools to measure with.

Run every command from the root of the repository. The folders `binaries/`, `benchmarks/` and `profile/`, and the file `.PRE_PATH`, are in `.gitignore`.

---

## Setup: PRE baseline (before any code change)

- **Choose a TAG**: a short, unique slug for this pass (e.g. `json_writer_01`).
- **Goal**: say what "better" means for this pass:
  - **Primary metric** (e.g. elapsed time of `evtx_dump -t 1 -o json ...`)
  - **Target improvement** (e.g. at least 5% faster, fewer allocations)
  - **Scope** (e.g. JSON only, XML only, one CLI mode)

- **Build and save a PRE binary, benchmark and profile** before you change any code:

```bash
set -euo pipefail

# Set this per pass
TAG="REPLACE_WITH_PASS_TAG"

TS=$(date -u +%Y%m%dT%H%M%SZ)
SAMPLE=samples/security_big_sample.evtx
mkdir -p binaries benchmarks profile

# `bench` builds `bench_evtx_dump_loop`, which the profile uses.
cargo build --release --features "fast-alloc bench" --bin evtx_dump --bin bench_evtx_dump_loop
PRE="binaries/evtx_dump_${TAG}_${TS}_pre"
cp target/release/evtx_dump "$PRE"
cp target/release/bench_evtx_dump_loop "${PRE}_loop"
echo "$PRE" > .PRE_PATH

# Baseline benchmark: change the command to fit this pass
hyperfine -w 10 -r 20 \
  "$PRE -t 1 -o json $SAMPLE" \
  | tee "benchmarks/benchmark_pre_${TAG}.txt"

# PRE profile: `samply` records it, and the script writes tables of the hottest functions
# (`top_leaves_<label>_cpu.md`, `top_inclusive_<label>_cpu.md`, `leaf_callers_<label>.md`).
samply record --save-only --unstable-presymbolicate \
  -o "profile/${TAG}_pre.json.gz" -- \
  "${PRE}_loop" --file "$SAMPLE" --loops 20 --threads 1
python3 scripts/samply_extract_tables.py \
  --profile "profile/${TAG}_pre.json.gz" \
  --syms "profile/${TAG}_pre.json.syms.json" \
  --out-dir profile --label "${TAG}_pre"
```

`bench_evtx_dump_loop` renders JSON. For a pass on another output, profile `evtx_dump` itself with the options of that pass.

- **Use the PRE benchmark and profile** to:
  - Confirm the current speed for this scenario.
  - Find one to three real hotspots that match the focus of this pass.
  - Avoid speculative work outside what the profile shows.

---

## Implementation guidelines

- **One focus**:
  - Keep this pass to one family of changes (e.g. allocations in one hot path, one intermediate buffer, one cache).
  - Leave unrelated cleanups for a pull request of their own.

- **Guardrails**:
  - Do **not** change the XML or JSON output: it is an API (see "Code rules" in `AGENTS.md`).
  - The library has `#![forbid(unsafe_code)]`. Keep it.
  - Prefer borrowing over cloning.
  - Keep the change local to the targeted area.
  - A performance trick must be explainable in a comment.

- **Use what exists**:
  - Keep `--features fast-alloc` on for every measurement.
  - Reuse the existing arenas, caches and writers before you add new state.

---

## POST: rebuild, benchmark the pair, profile

After the change, build a POST binary, benchmark PRE against POST on the same scenario, and record a POST profile.

```bash
set -euo pipefail

TAG="REPLACE_WITH_PASS_TAG"   # Same tag as PRE
TS=$(date -u +%Y%m%dT%H%M%SZ)
SAMPLE=samples/security_big_sample.evtx

cargo build --release --features "fast-alloc bench" --bin evtx_dump --bin bench_evtx_dump_loop
POST="binaries/evtx_dump_${TAG}_${TS}_post"
cp target/release/evtx_dump "$POST"
cp target/release/bench_evtx_dump_loop "${POST}_loop"

PRE=$(cat .PRE_PATH)

# Benchmark pair: the same command as PRE
hyperfine -w 10 -r 20 \
  "$PRE -t 1 -o json $SAMPLE" \
  "$POST -t 1 -o json $SAMPLE" \
  | tee "benchmarks/benchmark_pair_${TAG}_${TS}.txt"

samply record --save-only --unstable-presymbolicate \
  -o "profile/${TAG}_post.json.gz" -- \
  "${POST}_loop" --file "$SAMPLE" --loops 20 --threads 1
python3 scripts/samply_extract_tables.py \
  --profile "profile/${TAG}_post.json.gz" \
  --syms "profile/${TAG}_post.json.syms.json" \
  --out-dir profile --label "${TAG}_post"

echo "PRE:  $PRE"
echo "POST: $POST"
```

When the difference is within the noise of `hyperfine`, count instructions instead: `valgrind --tool=cachegrind --cache-sim=no "$PRE" -t 1 -o json "$SAMPLE" > /dev/null`, and the same for `$POST`.

---

## Validation and acceptance

Use the **user's stated goal** and the baseline to set the bar for this pass.

- **Speed**:
  - POST meets the target on the benchmark of this pass.
  - No other scenario in scope gets slower. A change on the path that builds or walks the record tree must also be measured with `--separate-json-attributes` and with `-o xml`, because the default JSON run of this sample does not use all of that path.

- **Correctness**:
  - `cargo test` passes.
  - No snapshot in `tests/snapshots` changes.

- **Profile**:
  - The hotspots that the pass targeted have shrunk.
  - Any new hotspot is expected and can be explained.

If the bar is not met, either:
- make another small change within the scope of this pass, **or**
- revert, and record why the attempt was rejected.

---

## Report

At the end of the pass, summarize:
- The **focus** of the pass and the main code changes.
- The PRE and POST numbers and the improvement factor.
- What changed among the hotspots of the profile.
- Trade-offs, and follow-up ideas that you left out to keep the pass in scope.
