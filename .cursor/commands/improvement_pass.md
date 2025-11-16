You are working in the `evtx` Rust project and running a **targeted performance improvement pass**.
Treat anything the user types **after this command** as the *focus for this pass* (e.g. "reduce allocations in template cache", "speed up JSON writer", "optimize XML path").

---

## Setup: PRE Baseline (Before Any Code Changes)

- **Choose a TAG**: a short, unique slug for this optimization round (e.g. `json_streaming_expander_01`, `arena_zero_copy_05`).
- **Goal**: clarify what "better" means for this pass:
  - **Primary metric** (e.g. elapsed time of `evtx_dump -t 1 -o json ...`)
  - **Target improvement** (e.g. ≥30% speedup, ≤X ms p95, ≤Y allocations)
  - **Scope** (e.g. JSON-only, XML-only, specific CLI mode)

- **Immediately build and save a PRE binary + benchmark** (no code changes yet):

```bash
set -euo pipefail

# Set this per pass
TAG="REPLACE_WITH_PASS_TAG"

TS=$(date -u +%Y%m%dT%H%M%SZ)
mkdir -p binaries benchmarks

cargo build --release --features fast-alloc
cp target/release/evtx_dump "binaries/evtx_dump_${TAG}_${TS}_pre"
echo "binaries/evtx_dump_${TAG}_${TS}_pre" > .PRE_PATH

PRE=$(cat .PRE_PATH)

# Baseline benchmark: adjust command if needed for this pass
hyperfine -w 10 -r 20 \
  "$PRE -t 1 -o json samples/security_big_sample.evtx > /dev/null" \
  | tee "benchmarks/benchmark_pre_${TAG}.txt"

# Optional: PRE flamegraph for this pass's main scenario
sudo TAG="$TAG" make flamegraph-prod \
  FLAME_FILE="samples/security_big_sample.evtx" \
  DURATION=30 \
  FORMAT=json

mv "profile/flamegraph_${TAG}.svg" "profile/flamegraph_${TAG}_${TS}_pre.svg" || true
cp "profile/top_leaf_${TAG}.txt" "profile/top_leaf_${TAG}_${TS}_pre.txt" || true
cp "profile/top_titles_${TAG}.txt" "profile/top_titles_${TAG}_${TS}_pre.txt" || true
```

- **Use the PRE benchmark + flamegraph** to:
  - Confirm the current performance level for this scenario.
  - Identify 1–3 real hotspots that match the focus of this pass.
  - Avoid speculative work outside what the profiles show.

---

## Implementation Guidelines (Core Optimization Pass)

- **Single-pass focus**:
  - Keep this pass focused on one coherent change family (e.g. allocator churn in a hot path, avoiding intermediate buffers, caching).
  - Defer unrelated cleanups or refactors unless they are trivial and obviously safe.

- **Guardrails**:
  - Do **not** change observable behavior or JSON/XML schema; ordering-only changes are acceptable if tests/snapshots allow it.
  - Prefer borrowing over cloning; avoid `unsafe`.
  - Keep changes local to the targeted area (e.g. a specific module or output path).
  - Maintain debuggability and readability; performance hacks must be explainable.

- **Use existing infrastructure**:
  - Keep `--features fast-alloc` enabled for realistic measurements.
  - Reuse existing arenas, caches, and writers rather than introducing new global state.
  - Tune hot `Vec` capacities and data structures where profiling shows growth or reallocations.

---

## POST: Rebuild, Benchmark Pair, Flamegraph

After implementing the change(s), build a POST binary, then benchmark PRE vs POST on the same scenario and generate a POST flamegraph.

```bash
set -euo pipefail
cd /workspace

export PATH="$HOME/.cargo/bin:/usr/local/cargo/bin:$PATH"

TAG="REPLACE_WITH_PASS_TAG"   # Same tag as PRE
TS=$(date -u +%Y%m%dT%H%M%SZ)

mkdir -p /workspace/binaries /workspace/benchmarks /workspace/profile_post

# Build POST
cargo build --release --features fast-alloc

# Save POST binary
POST="/workspace/binaries/evtx_dump_${TAG}_${TS}_post"
cp /workspace/target/release/evtx_dump "$POST"
echo "$POST" > /workspace/.POST_PATH

# Load PRE
PRE=$(cat /workspace/.PRE_PATH)

# Benchmark pair (adjust command for this pass if needed)
hyperfine -w 10 -r 20 \
  "$PRE -t 1 -o json /workspace/samples/security_big_sample.evtx > /dev/null" \
  "$POST -t 1 -o json /workspace/samples/security_big_sample.evtx > /dev/null" \
  | tee "/workspace/benchmarks/benchmark_pair_${TAG}_${TS}.txt"

# POST flamegraph for the same scenario
OUT_DIR=/workspace/profile_post FORMAT=json DURATION=30 BIN="$POST" \
  /workspace/scripts/flamegraph_prod.sh

mv "/workspace/profile_post/flamegraph_${TAG}.svg" "/workspace/profile_post/flamegraph_${TAG}_${TS}_post.svg" || true
cp "/workspace/profile_post/top_leaf_${TAG}.txt" "/workspace/profile_post/top_leaf_${TAG}_${TS}_post.txt" || true
cp "/workspace/profile_post/top_titles_${TAG}.txt" "/workspace/profile_post/top_titles_${TAG}_${TS}_post.txt" || true

echo "PRE:  $PRE"
echo "POST: $POST"
```

---

## Validation & Acceptance Criteria

Use the **user’s stated goal** plus the baseline to define acceptance criteria for this pass. Examples:

- **Performance target**:
  - POST time ≤ \(k\) × PRE time for the benchmark (e.g. `k = 2/3` for ≥50% speedup).
  - No regressions on other critical scenarios, if they are in scope.

- **Correctness**:
  - `cargo test` must pass.
  - Existing snapshot/CLI tests must continue to pass; if they fail, understand whether it's a genuine behavior change.

- **Profiling deltas**:
  - Hotspots should shift away from previously-identified bottlenecks.
  - Allocator and `reserve/finish_grow` frames should be reduced when the pass targets allocations.
  - New hotspots should be expected and understandable (e.g. shifted to direct writer or necessary loops).

If acceptance criteria are not met, either:
- Iterate with another small, focused change within this pass’s scope, **or**
- Roll back and record why the attempted optimization was rejected.

---

## Reporting & Notes

At the end of the pass, summarize:
- The **focus** of the pass and the key code changes.
- PRE vs POST benchmark numbers and the improvement factor.
- The most important changes in flamegraph hotspots.
- Any trade-offs or follow-up ideas you intentionally deferred to keep this pass scoped.


