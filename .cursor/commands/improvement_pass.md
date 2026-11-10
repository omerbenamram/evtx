You are working in the `evtx` Rust project and running a **targeted performance improvement pass**.
Treat anything the user types **after this command** as the *focus for this pass* (e.g. "reduce allocations in template cache", "speed up JSON writer", "optimize XML path").

`AGENTS.md` holds the rules of this repository. Its code rules apply to this pass, and "Speed is a feature" there names the sample, the tools and the scenarios to measure.

---

## Goal

Before any code change, say what "better" means for this pass:

- **Primary metric** (e.g. elapsed time of `evtx_dump -t 1 -o json ...`)
- **Target improvement** (e.g. at least 5% faster, fewer allocations)
- **Scope** (e.g. JSON only, XML only, one CLI mode)

Choose a **TAG**: a short, unique slug for this pass (e.g. `json_writer_01`).

---

## Measure: once before the change, once after

Run this script from the root of the repository with `STAGE=pre` before you change any code, and with `STAGE=post` after the change. It saves the binary, benchmarks it, and records a profile. The folders `binaries/`, `benchmarks/` and `profile/` are in `.gitignore`.

```bash
set -euo pipefail

TAG="REPLACE_WITH_PASS_TAG"
STAGE="pre"   # "pre" before the change, "post" after it
SAMPLE=samples/security_big_sample.evtx
mkdir -p binaries benchmarks profile

# `bench` builds `bench_evtx_dump_loop`, which the profile uses.
cargo build --release --features "fast-alloc bench" --bin evtx_dump --bin bench_evtx_dump_loop
BIN="binaries/evtx_dump_${TAG}_${STAGE}"
cp target/release/evtx_dump "$BIN"
cp target/release/bench_evtx_dump_loop "${BIN}_loop"

# Benchmark: change the command to fit this pass. The post run compares both binaries.
CMDS=("binaries/evtx_dump_${TAG}_pre -t 1 -o json $SAMPLE")
if [ "$STAGE" = post ]; then
  CMDS+=("$BIN -t 1 -o json $SAMPLE")
fi
hyperfine -w 10 -r 20 "${CMDS[@]}" | tee "benchmarks/benchmark_${TAG}_${STAGE}.txt"

# Profile: `samply` records it, and the script writes tables of the hottest functions
# (`top_leaves_<label>_cpu.md`, `top_inclusive_<label>_cpu.md`, `leaf_callers_<label>.md`).
samply record --save-only --unstable-presymbolicate \
  -o "profile/${TAG}_${STAGE}.json.gz" -- \
  "${BIN}_loop" --file "$SAMPLE" --loops 20 --threads 1
python3 scripts/samply_extract_tables.py \
  --profile "profile/${TAG}_${STAGE}.json.gz" \
  --syms "profile/${TAG}_${STAGE}.json.syms.json" \
  --out-dir profile --label "${TAG}_${STAGE}"
```

`bench_evtx_dump_loop` renders JSON. For a pass on another output, profile `evtx_dump` itself with the options of that pass.

When the difference is within the noise of `hyperfine`, count instructions instead: `valgrind --tool=cachegrind --cache-sim=no binaries/evtx_dump_${TAG}_pre -t 1 -o json "$SAMPLE" > /dev/null`, and the same for the post binary.

Use the pre benchmark and profile to:

- Confirm the current speed for this scenario.
- Find one to three real hotspots that match the focus of this pass.
- Avoid speculative work outside what the profile shows.

---

## Implementation guidelines

- **One focus**:
  - Keep this pass to one family of changes (e.g. allocations in one hot path, one intermediate buffer, one cache).
  - Leave unrelated cleanups for a pull request of their own.

- **Guardrails**:
  - Prefer borrowing over cloning.
  - Keep the change local to the targeted area.
  - A performance trick must be explainable in a comment.

- **Use what exists**:
  - Keep `--features fast-alloc` on for every measurement.
  - Reuse the existing arenas, caches and writers before you add new state.

---

## Validation and acceptance

Use the **user's stated goal** and the baseline to set the bar for this pass.

- **Speed**:
  - The post binary meets the target on the benchmark of this pass.
  - No other scenario in scope gets slower.

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
- The numbers before and after, and the improvement factor.
- What changed among the hotspots of the profile.
- Trade-offs, and follow-up ideas that you left out to keep the pass in scope.
