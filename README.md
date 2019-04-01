[![Build Status](https://travis-ci.org/omerbenamram/evtx.svg?branch=master)](https://travis-ci.org/omerbenamram/evtx)
![crates.io](https://img.shields.io/crates/v/evtx.svg)
# EVTX

This is a parser for the Windows EVTX format.

Note that it is complete as in the sense that it successfully parses a wide variety of samples, but I've yet to implement the full specification.

This parser is implemented using 100% safe rust, and should work on recent (i'm testing against 1.34) versions of rust.

## Example usage:
```rust
    use evtx::EvtxParser;
    
    fn main() {
        let parser = EvtxParser::from_path(fp).unwrap();
        for record in parser.records() {
            match record {
                Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
                Err(e) => eprintln!("{}", e),
            }
        }
    }
```

For parallel iteration (uses rayon):

```rust
    use evtx::EvtxParser;
    
    fn main() {
        let parser = EvtxParser::from_path(fp).unwrap();
        for record in parser.parallel_records() {
            match record {
                Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
                Err(e) => eprintln!("{}", e),
            }
        }
    }
```

The parallel version is enabled when compiling with feature "multithreading" (enabled by default).

## Benchmarking

Initial benchmarking that I've performed indicates that this implementation probably the fastest available online 🍺.

I'm using a real world, 30MB sample which contains ~62K records.

This is benchmarked on my 2017 MBP.

Comparison with other libraries:

- python-evtx (https://github.com/williballenthin/python-evtx)
    
    With CPython this is quite slow 
    
    ```
    time -- python3 ~/Workspace/python-evtx/scripts/evtx_dump.py ./samples/security_big_sample.evtx > /dev/null                                                                      Mon Apr  1 19:41:16 2019
          363.83 real       356.26 user         2.17 sys
    ```
    
    With PyPy (tested with pypy3.5, 7.0.0), it's taking about just less than a minute (a 6x improvement!)
    ```
    time -- pypy3 ~/Workspace/python-evtx/scripts/evtx_dump.py ./samples/security_big_sample.evtx > /dev/null                                                                      Mon Apr  1 19:41:16 2019
          59.30 real        58.10 user         0.51 sys
    ```
    
- libevtx (https://github.com/libyal/libevtx)
   
   This library is written in C, so I initially expected it to be faster than my implementation originally.
   It clocks in about 6x than PyPy, which is pretty good.
   
   ```
   time -- ~/Workspace/libevtx/dist/bin/evtxexport -f xml ./samples/security_big_sample.evtx > /dev/null
          11.30 real        10.77 user         0.41 sys
   ```
    
   Note: libevtx does have multi-threading support planned (according to the readme),
   but isn't implemented as of writing this (April 2019).
   
- evtx (this library!)
    
    When using a single thread, this implementation is about 2x faster than C
    ```
    time -- ./target/release/main --input ./samples/security_big_sample.evtx > /dev/null                                                                                     516ms  Mon Apr  1 19:53:59 2019
            4.65 real         4.53 user         0.10 sys
    ```
    
    With multi-threading enabled, it blazes through the file in just 1.5 seconds:
    ```
    time -- ./target/release/main -t --input ./samples/security_big_sample.evtx > /dev/null                                                                                 4661ms  Mon Apr  1 19:54:14 2019
            1.51 real         7.50 user         0.26 sys
    ```
   

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
