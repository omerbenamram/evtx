<h1 align="center">EVTX</h1>
<div align="center">
 <strong>
   A parser for the Windows EVTX format
 </strong>
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
  <a href="https://dev.azure.com/benamram/evtx/_build/latest?definitionId=1&branchName=master">
    <img src="https://dev.azure.com/benamram/dfir/_apis/build/status/omerbenamram.evtx?branchName=master"
      alt="Build status" />
  </a>
</div>

</br>

Python bindings are available as well at https://github.com/omerbenamram/pyevtx-rs (and at PyPi https://pypi.org/project/evtx/)

## Features

 - Implemented using 100% safe rust - and works on all platforms supported by rust (that have stdlib).
 - Multi-threaded.
 - Supports XML and JSON outputs, both being zero-copy and independent of each other (JSON documents are being built directly from the inner representation of the binary xml token tree, no xml2json is performed!)
 - Supports some basic recovery of missing records/chunks!

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

`evtx_dump` can be combined with [fd](https://github.com/sharkdp/fd) for convinient batch processing of files:
  - `fd -e evtx -x evtx_dump -o jsonl` will scan a folder and dump all evtx files to a single jsonlines file.
  - `fd -e evtx -x evtx_dump -f "{.}.xml` will create an xml file next to each evtx file, for all files in folder recursively!
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

## Benchmarking

Initial benchmarking I've performed indicate that this implementation is probably the fastest available 🍺.

I'm using a real world, 30MB sample which contains ~62K records.

This is benchmarked on my 2017 MBP.

Comparison with other libraries:

- python-evtx (https://github.com/williballenthin/python-evtx)
    
    With CPython this is quite slow 
    
    ```
    time -- python3 ~/Workspace/python-evtx/scripts/evtx_dump.py ./samples/security_big_sample.evtx > /dev/null                                                                      Mon Apr  1 19:41:16 2019
          363.83 real       356.26 user         2.17 sys
    ```
    
    With PyPy (tested with pypy3.5, 7.0.0), it's taking just less than a minute (a 6x improvement!)
    ```
    time -- pypy3 ~/Workspace/python-evtx/scripts/evtx_dump.py ./samples/security_big_sample.evtx > /dev/null                                                                      Mon Apr  1 19:41:16 2019
          59.30 real        58.10 user         0.51 sys
    ```
    
- libevtx (https://github.com/libyal/libevtx)
   
   This library is written in C, so I initially expected it to be faster than my implementation.

   It clocks in about 6x faster than PyPy.
   
   ```
   time -- ~/Workspace/libevtx/dist/bin/evtxexport -f xml ./samples/security_big_sample.evtx > /dev/null
          11.30 real        10.77 user         0.41 sys
   ```
    
   Note: libevtx does have multi-threading support planned (according to the readme),
   but isn't implemented as of writing this (April 2019).
   
- evtx (this library!)
    
    When using a single thread, this implementation is about 2.8x faster than C
    ```
    time -- ./target/release/main -t 1 ./samples/security_big_sample.evtx > /dev/null                                                                                     
            4.04 real         3.90 user         0.11 sys
    ```
    
    With multi-threading enabled, it blazes through the file in just 1.3 seconds:
    ```
    time -- ./target/release/main ./samples/security_big_sample.evtx > /dev/null                                                                                 
            1.30 real         6.10 user         0.29 sys
    ```
   
## Caveats

- Currently unimplemented:
   - PI/cdata nodes.
   - entity/character refs.
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
