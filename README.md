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
 - ✨ Supports XML and JSON outputs, both being directly constructed from the token tree and independent of each other (no xml2json conversion is performed!)
 - ⛏️ Supports some basic recovery of missing records/chunks!
 - 🐍 Python bindings are available as well at https://github.com/omerbenamram/pyevtx-rs (and at PyPi https://pypi.org/project/evtx/)

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

`evtx_dump` can be combined with [fd](https://github.com/sharkdp/fd) for convenient batch processing of files:
  - `fd -e evtx -x evtx_dump -o jsonl` will scan a folder and dump all evtx files to a single jsonlines file.
  - `fd -e evtx -x evtx_dump '{}' -f '{.}.xml` will create an xml file next to each evtx file, for all files in folder recursively!
  - If the source of the file needs to be added to json, `xargs` (or `gxargs` on mac) and `jq` can be used: `fd -a -e evtx | xargs -I input sh -c "evtx_dump -o jsonl input | jq --arg path "input" '. + {path: \$path}'"`
  
**Note:** by default, `evtx_dump` will try to utilize multithreading, this means that the records may be returned out of order.

To force single threaded usage (which will also ensure order), `-t 1` can be passed.

## Example usage (as library):
```rust
use evtx::EvtxParser;
use std::path::PathBuf;

fn main() {
    // Change this to a path of your .evtx sample. 
    let fp = PathBuf::from(format!("{}/samples/security.evtx", std::env::var("CARGO_MANIFEST_DIR").unwrap())); 
    
    let mut parser = EvtxParser::from_path(fp).unwrap();
    for record in parser.records() {
        match record {
            Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
            Err(e) => eprintln!("{}", e),
        }
    }
}
```

The parallel version is enabled when compiling with feature "multithreading" (enabled by default).

## Performance benchmarking

When using multithreading - `evtx` is significantly faster than any other parser available.
For single core performance, it is both the fastest and the only cross-platform parser than supports both xml and JSON outputs.

Performance was benched on my machine using `hyperfine` (statistical measurements tool).

I'm running tests on a 12-Core AMD Ryzen 3900X.

Tests are running under WSL2, on a linux filesystem (so there shouldn't be any overhead incurred from reading windows mounts).

Libraries benched:

- `python-evtx`(https://github.com/williballenthin/python-evtx) - With CPython and PyPy
- `libevtx`(https://github.com/libyal/libevtx)
- `golang-evtx`(https://github.com/0xrawsec/golang-evtx.git) - only JSON (uses multithreading)
- `evtx`(https://github.com/Velocidex/evtx) - only JSON.
- `evtx` (This library)


|                  | evtx (1 thread)      | evtx (8 threads)      | evtx (24 threads)         | libevtx (C)          | velocidex/evtx (go)  | golang-evtx (uses multiprocessing) | python-evtx (CPython 3.7.6) | python-evtx (PyPy 7.3.0) |
|------------------|----------------------|-----------------------|---------------------------|----------------------|----------------------|------------------------------------|-----------------------------|--------------------------|
| 30MB evtx (XML)  | 1.155 s  ±   0.008 s | 277.4 ms  ±    5.8 ms | **177.1 ms  ±    4.5 ms** | 4.509 s  ±   0.100 s | No support           | No support                         | 4m11.046s (ran once)        | 1m12.828s (ran once)     |
| 30MB evtx (JSON) | 1.631 s  ±   0.006 s | 341.6 ms  ±    7.3 ms | **207.2 ms  ±    7.2 ms** | No support           | 5.587 s  ±   0.086 s | 2.216 s  ±   0.027 s               | No support                  | No support               |

**Note**: numbers shown are `real-time` measurements (time it takes for invocation to complete). `user-time` measurements are higher when more using multithreading/multiprocessing, because of the synchronization overhead.

With 8 threads - `evtx` is more than **650x** faster than `python-evtx` when dumping xml logs.

With maximum viable threads (number of logical cores) - `evtx` is about **8-10x** faster `golang-evtx`. Both implementations utilize similar multithreading strategies.

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
