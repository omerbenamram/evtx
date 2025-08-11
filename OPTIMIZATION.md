## EVTX XML path optimization plan

This document tracks hotspots, a ranked optimization roadmap, and a strict measure-apply-verify loop for each change. We apply optimizations one-by-one, capturing baseline and post-change timings and profiling deltas.

### Environment

- commit: ab8521c
- rustc: 1.88.0
- cargo: 1.88.0
- hyperfine: 1.19.0
- cpu: Apple M3 Pro (11 cores)
- os: macOS 15.6
- date: 2025-08-11T17:35:16Z (UTC)

### Baseline benchmark (XML, single thread)

Command:

```
hyperfine -w 2 '/Users/omerba/Workspace/evtx/target/release/evtx_dump -o xml -t 1 /Users/omerba/Workspace/evtx/samples/security_big_sample.evtx > /dev/null'
```

Result:

```
Time (mean ± σ): 515.4 ms ± 11.6 ms
Range (min … max): 501.6 ms … 536.0 ms (10 runs)
User: 501.3 ms, System: 10.9 ms
```

Notes:
- Output redirected to /dev/null to avoid terminal IO overhead.
- Build was a standard `cargo build --release` without additional features.

### Baseline hotspots (30s sample; format: xml, file: security_big_sample.evtx)

- Evtx serialization
  - EvtxRecord::into_xml: 61.5%
  - quick_xml Writer: write_event 10.8%, push_attr 10.1%, write_wrapped 7.1%
- Token/model pipeline
  - create_record_model: 12.0%
  - expand_templates/_expand_templates/read_template: ~23% combined
  - IterTokens::inner_next + read_binxml_fragment: ~14.6%
  - BinXmlValue::deserialize_value_type: 6.7%
- Allocations and copies
  - RawVec::reserve/finish_grow + malloc/realloc: ~21%
- String/UTF-16
  - read_utf16_string notable; BinXmlValue::as_cow_str shows numeric formatting allocs

### Optimization loop for each change

1) Benchmark: run hyperfine (same command as baseline) and record mean, σ, min/max.
2) Profile: run `make flamegraph-prod FLAME_FILE="samples/security_big_sample.evtx" DURATION=30 FORMAT=xml` and attach deltas for top items.
3) Decide: if improvement is statistically meaningful and hotspots moved as expected, keep the change; otherwise revert.
4) Document: write a short note with impact and observations.

### Standard commands

Baseline/after each change:

```
# Benchmark
hyperfine -w 2 '/Users/omerba/Workspace/evtx/target/release/evtx_dump -o xml -t 1 /Users/omerba/Workspace/evtx/samples/security_big_sample.evtx > /dev/null'

# Profile (outputs to profile/)
make flamegraph-prod FLAME_FILE="samples/security_big_sample.evtx" DURATION=30 FORMAT=xml
```

### Ranked optimization plan (highest impact first)

1) Preallocate XML writer buffer
- Hypothesis: reduce Vec growth/realloc in `EvtxRecord::into_xml`/quick_xml.
- Change: in `EvtxRecord::into_xml`, create `XmlOutput` with `Vec::with_capacity(estimate)`.
- Measurement: hyperfine; profile should show drop in `RawVec::reserve/finish_grow`, `_realloc`, and `quick_xml::writer` shares.
- Status: pending
- Impact: TBD (record here after test)

2) Switch numeric formatting to zero-alloc (itoa/ryu)
- Hypothesis: avoid `to_string()` allocations in `as_cow_str` hot path.
- Change: in `xml_output.rs`, match on `BinXmlValue` for numbers and format via `itoa`/`ryu` into stack buffers, pass slices to quick-xml.
- Measurement: hyperfine; profile drop in `BinXmlValue::as_cow_str`, `SpecToString`, formatting functions.
- Status: pending
- Impact: TBD

3) Inline attribute storage (SmallVec)
- Hypothesis: reduce attribute Vec allocations for common small element cases.
- Change: replace `Vec<XmlAttribute>` with `SmallVec<[XmlAttribute; N]>` in `XmlElementBuilder` and `XmlElement`.
- Measurement: hyperfine; profile drop in `RawVec::*` and malloc/realloc around attribute handling, fewer `push_attr` cost.
- Status: pending
- Impact: TBD

4) Stream tokens directly to writer (bypass model Vec)
- Hypothesis: eliminate `create_record_model` allocations and extra traversal.
- Change: drive `BinXmlOutput` directly during token expansion with a small tag stack; remove intermediate `Vec<XmlModel>`.
- Measurement: hyperfine; profile drop in `create_record_model`, `XmlElementBuilder::finish`, and related allocs.
- Status: pending
- Impact: TBD

5) Batch attributes with `BytesStart::with_attributes`
- Hypothesis: fewer per-attribute calls reduce overhead in quick-xml layer.
- Change: construct attribute slice/iterator and use `with_attributes` instead of repeated `push_attribute`.
- Measurement: hyperfine; profile drop in `BytesStart::push_attr`.
- Status: pending
- Impact: TBD

6) Faster hash for string cache
- Hypothesis: reduce hashing overhead visible as `BuildHasher::hash_one`/`sip::Hasher::write`.
- Change: `hashbrown::HashMap` with `ahash::RandomState` in `StringCache`.
- Measurement: hyperfine; profile drop in hash functions; correctness verified by tests.
- Status: pending
- Impact: TBD

7) Non-functional toggles (validate allocator and indentation)
- a) Enable `--features fast-alloc` in release builds for prod binaries.
- b) Consider `--no-indent` for workloads where pretty printing isn’t required.
- Measurement: hyperfine; attribution in profile to allocator/Writer.
- Status: pending
- Impact: TBD

### Results ledger (fill as we go)

- Step 1: Preallocate writer buffer
  - Before: mean 515.4 ms ± 11.6 ms
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 2: itoa/ryu numeric formatting
  - Before: mean …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 3: SmallVec for attributes
  - Before: …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 4: Stream tokens direct-to-writer
  - Before: …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 5: with_attributes batching
  - Before: …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 6: faster hash for string cache
  - Before: …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …

- Step 7: allocator/indent toggles
  - Before: …
  - After: …
  - Δ time: …
  - Profile deltas: …
  - Notes: …


