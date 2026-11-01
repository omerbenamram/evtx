# Fuzzing evtx

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) targets for the parsers that read untrusted
input:

| Target | Input |
| --- | --- |
| `evtx_file` | A whole log, read with `EvtxParser`: each record as XML, then as a JSON value, with the records recovered from slack space too. |
| `evtx_chunk` | One chunk, read with `EvtxChunkData`: each record as XML, then as JSON. |
| `wevt_template` | A `WEVT_TEMPLATE` resource (a CRIM manifest), as `--wevt-cache` loads it, with each template in it rendered. |
| `mta` | A LocaleMetaData (`.MTA`) file. |

They need a nightly toolchain and cargo-fuzz (`cargo install cargo-fuzz`). Seed the corpora from
`samples/` (the `WEVT_TEMPLATE` seeds come from the DLLs in `samples/dlls`, which git-lfs keeps),
then fuzz a target:

```
python3 fuzz/seed_corpus.py
cargo +nightly fuzz run evtx_chunk -- -timeout=10
```

A crashing input is saved under `fuzz/artifacts/<target>/`, and
`cargo +nightly fuzz run <target> <file>` runs it again. CI runs every seed through every target
once, so that the targets keep building and the samples keep passing.
