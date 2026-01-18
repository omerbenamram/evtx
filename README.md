<h1 align="center"><img style="padding:0;vertical-align:bottom;" height="32" width="32" src="/eventvwr.ico"/> EVTX</h1>
<div align="center">
 <p>
  <strong>
   A cross-platform parser for the Windows XML EventLog format
  </strong>

 </p>
</div>

<br />

<div align="center">
  <!-- Crates version -->
  <a href="https://crates.io/crates/evtx">
    <img src="https://img.shields.io/crates/v/evtx.svg?style=flat-square"
    alt="Crates.io version" />
  </a>
  <!-- Downloads -->
  <a href="https://crates.io/crates/evtx">
    <img src="https://img.shields.io/crates/d/evtx.svg?style=flat-square"
      alt="Download" />
  </a>
  <!-- docs.rs docs -->
  <a href="https://docs.rs/evtx">
    <img src="https://img.shields.io/badge/docs-latest-blue.svg?style=flat-square"
      alt="docs.rs docs" />
  </a>
   <a href="https://github.com/rust-secure-code/safety-dance/">
    <img src="https://img.shields.io/badge/unsafe-forbidden-success.svg"
      alt="safety-dance" />
  </a>
  <a href="https://github.com/omerbenamram/evtx/actions/workflows/test.yml">
    <img src="https://github.com/omerbenamram/evtx/actions/workflows/test.yml/badge.svg"
      alt="Build status" />
  </a>
</div>

</br>

## Features

 - 🔒 Implemented using 100% safe rust - and works on all platforms supported by rust (that have stdlib).
 - ⚡ Fast - see benchmarks below. It's faster than any other implementation by order(s) of magnitude!
 - 🚀 Multi-threaded.
 - ✨ Supports XML and JSON outputs, both being directly constructed from a shared intermediate representation (IR) (no xml2json conversion is performed!)
 - ⛏️ Supports some basic recovery of missing records/chunks!
 - 🐍 Python bindings are available as well at https://github.com/omerbenamram/pyevtx-rs (and at PyPi https://pypi.org/project/evtx/)

## Web-based Viewer (EVTX Web)

![EVTX Web Screenshot](/evtx_web_ui.png)

Prefer a zero-install option?  A fully-featured EVTX explorer runs right in your browser, powered by the same Rust core compiled to WebAssembly.

👉 **Try it now:** <https://omerbenamram.github.io/evtx/>

Everything happens locally – files never leave your machine.  Highlights:

* Drag-and-drop `.evtx` files (or click to browse) – handles very large logs!
* Blazing-fast parsing via WebAssembly and virtual-scroll rendering
* Faceted filters on level, provider, channel, Event ID, and dynamic `EventData` fields – all backed by DuckDB-WASM
* Full-text search, column management, and on-the-fly JSON/XML export of the filtered set
* Light/dark themes, keyboard navigation, and a Windows-style UI

The viewer is served statically from GitHub Pages; after the first load it works completely offline.

## Installation (associated binary utility):
  - Download latest executable release from https://github.com/omerbenamram/evtx/releases
    - Releases are automatically built for for Windows, macOS, and Linux. (64-bit executables only)
  - Build from sources using  `cargo install evtx`

# `evtx_dump` (Binary utility):
The main binary utility provided with this crate is `evtx_dump`, and it provides a quick way to convert `.evtx` files to
different output formats.

Some examples
  - `evtx_dump <evtx_file>` will dump contents of evtx records as xml.
  - `evtx_dump -o json <evtx_file>` will dump contents of evtx records as JSON.
  - `evtx_dump -f <output_file> -o json <input_file>` will dump contents of evtx records as JSON to a given file.
  - `cat <evtx_file> | evtx_dump -o jsonl -` will read the EVTX file from stdin (useful for piping/decompression).

`evtx_dump` can be combined with [fd](https://github.com/sharkdp/fd) for convenient batch processing of files:
  - `fd -e evtx -x evtx_dump -o jsonl` will scan a folder and dump all evtx files to a single jsonlines file.
  - `fd -e evtx -x evtx_dump '{}' -f '{.}.xml'` will create an xml file next to each evtx file, for all files in folder recursively!
  - If the source of the file needs to be added to json, `xargs` (or `gxargs` on mac) and `jq` can be used: `fd -a -e evtx | xargs -I input sh -c "evtx_dump -o jsonl input | jq --arg path "input" '. + {path: \$path}'"`

**Note:** by default, `evtx_dump` will try to utilize multithreading, this means that the records may be returned out of order.

To force single threaded usage (which will also ensure order), `-t 1` can be passed.

## Offline template rendering (WEVT_TEMPLATE)

EVTX records can reference template definitions stored in provider binaries (EXE/DLL/SYS). `evtx_dump` can extract those templates into an offline cache and use them at render time.

**Note:** this functionality requires building `evtx_dump` with the Cargo feature `wevt_templates` (release binaries may already include it).

- Build a cache (single portable `.wevtcache` file):
  - `evtx_dump extract-wevt-templates --input <provider.dll> --output /tmp/wevt_cache.wevtcache --overwrite`
- Dump an EVTX file while using the cache (deterministic rule: only applies when a record fails due to an explicit missing/corrupt template GUID):
  - `evtx_dump --wevt-cache /tmp/wevt_cache.wevtcache <log.evtx>`

Debugging helpers:
- Dump a record’s `TemplateInstance` substitution values (JSONL):
  - `evtx_dump dump-template-instances --input <log.evtx> --record-id <ID> | head -n1`
- Render a specific template GUID with substitutions (XML to stdout):
  - `evtx_dump apply-wevt-cache --cache /tmp/wevt_cache.wevtcache --template-guid <GUID> --evtx <log.evtx> --record-id <ID>`

See [`docs/wevt_templates.md`](docs/wevt_templates.md) for details and background (issue #103).

## Example usage (as library):
```rust
use evtx::EvtxParser;
use std::path::PathBuf;

// Change this to a path of your .evtx sample.
let fp = PathBuf::from(format!("{}/samples/security.evtx", std::env::var("CARGO_MANIFEST_DIR").unwrap()));

let mut parser = EvtxParser::from_path(fp).unwrap();
for record in parser.records() {
    match record {
        Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
        Err(e) => eprintln!("{}", e),
    }
}
```

The parallel version is enabled when compiling with feature "multithreading" (enabled by default).

## Performance benchmarking

When using multithreading - `evtx` is significantly faster than any other parser available.
For single core performance, it is both the fastest and the only cross-platform parser than supports both xml and JSON outputs.

Performance was benched on my machine using `hyperfine` (statistical measurements tool).

I'm running tests on a 12-Core AMD Ryzen 3900X.

Bench run: **January 2026**.

System: **Arch Linux** (`Linux 6.17.9-arch1-1 x86_64`).

Benchmark commit: `e01782a`.

Libraries benched:

- `python-evtx`(https://github.com/williballenthin/python-evtx) - With CPython and PyPy
- `pyevtx-rs`(https://github.com/omerbenamram/pyevtx-rs) / `evtx`(https://pypi.org/project/evtx/) - Python bindings for this library
- `libevtx`(https://github.com/libyal/libevtx)
- `golang-evtx`(https://github.com/0xrawsec/golang-evtx.git) - only JSON (uses multithreading)
- `evtx`(https://github.com/Velocidex/evtx) - only JSON.
- `evtx` (This library)


|                  | evtx (1 thread)      | evtx (8 threads)      | evtx (24 threads)         | libevtx (C)          | velocidex/evtx (go)  | golang-evtx (uses multiprocessing) | pyevtx-rs (CPython 3.13.11) | python-evtx (CPython 3.13.11) | python-evtx (PyPy 7.3.19) |
|------------------|----------------------|-----------------------|---------------------------|----------------------|----------------------|------------------------------------|-----------------------------|------------------------------|--------------------------|
| 30MB evtx (XML)  | 275.9 ms ±   2.1 ms  | 96.9 ms ±   1.3 ms    | **79.5 ms ±   3.0 ms**    | 2.439 s ±   0.035 s  | No support           | No support                         | 0.367s (ran once)           | 2m41.075s (ran once)         | 40.096s (ran once)       |
| 30MB evtx (JSON) | 280.7 ms ±   1.2 ms  | 94.1 ms ±   1.5 ms    | **77.9 ms ±   5.5 ms**    | No support           | 5.467 s ±   0.038 s  | 1.344 s ±   0.005 s               | 0.398s (ran once)           | No support                    | No support               |

**Note**: numbers shown are `real-time` measurements (time it takes for invocation to complete). `user-time` measurements are higher when more using multithreading/multiprocessing, because of the synchronization overhead.

With 8 threads - `evtx` is more than **1600x** faster than `python-evtx` when dumping xml logs.

With maximum viable threads (number of logical cores) - `evtx` is about **14-17x** faster `golang-evtx`. Both implementations utilize similar multithreading strategies.

## Caveats

- Currently unimplemented:
   - CDATA nodes.
   - EVTHandle node type.

If the parser errors on any of these nodes, feel free to open an issue or drop me an email with a sample.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
