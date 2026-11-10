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

`scripts/perf_pass.sh` saves the release binary, benchmarks it with `hyperfine`, and records a `samply` profile with tables of the hottest functions. Run it from the root of the repository:

```bash
scripts/perf_pass.sh <TAG> pre    # before you change any code
scripts/perf_pass.sh <TAG> post   # after the change: benchmarks the pre binary against the new one
```

- The benchmark is `evtx_dump -t 1 -o json` on `samples/security_big_sample.evtx`. For another scenario, pass `--file <path>` and the `evtx_dump` arguments after `--`, the same for both stages: `scripts/perf_pass.sh <TAG> pre --file <path> -- -t 1 -o jsonl --separate-json-attributes`. Use one tag for each scenario that "Speed is a feature" asks for, and add `--skip-samply` to all but the first: the profile does not depend on the `evtx_dump` arguments.
- When the difference is within the noise of `hyperfine`, add `--cachegrind` to the post stage. It counts the instructions of both binaries.
- The profile comes from `bench_evtx_dump_loop`, which always renders JSON without indentation and uses the system allocator. Use it to find hotspots in parsing and rendering. The benchmark, not the profile, is the measure of the pass.
- Where `samply` cannot run, such as a cloud container, the script says so and keeps the benchmark. `--skip-samply` leaves the profile out.
- `scripts/perf_pass.sh --help` lists the files that it writes.

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
