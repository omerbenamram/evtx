# EVTX

This is a parser for the Windows EVTX format.

Note that it is complete as in the sense that it successfully parses a wide variety of samples, but I've yet to implement the full specification.

This uses almost 100% safe rust, the only exception being memory mapping input files to gain seek ergonomics.
But otherwise the entire parser is safe!

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

## Benchmarking

Initial benchmarking that I've performed indicates that this implementation is relatively fast.

It crunches through a 30MB .evtx file (around 62K records) in around 4 seconds.

## License

Licensed under either of

 * Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
 * MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any
additional terms or conditions.
