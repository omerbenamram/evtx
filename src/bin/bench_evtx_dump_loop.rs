//! Loopable binary for profiling evtx_dump JSONL output end-to-end.

#[cfg(feature = "bench")]
mod imp {
    use evtx::{EvtxParser, ParserSettings};
    use std::env;
    use std::io::{self, Write};
    use std::path::PathBuf;

    fn parse_args() -> (PathBuf, usize, usize, bool) {
        let mut args = env::args().skip(1);
        let mut path = PathBuf::from("samples/security_big_sample.evtx");
        let mut loops: usize = 1;
        let mut threads: usize = 1;
        let mut stats = false;

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--file" => {
                    if let Some(p) = args.next() {
                        path = PathBuf::from(p);
                    }
                }
                "--loops" => {
                    if let Some(v) = args.next() {
                        loops = v.parse().unwrap_or(loops);
                    }
                }
                "--threads" => {
                    if let Some(v) = args.next() {
                        threads = v.parse().unwrap_or(threads);
                    }
                }
                "--stats" => {
                    stats = true;
                }
                _ => {}
            }
        }

        (path, loops, threads, stats)
    }

    pub fn run() -> Result<(), Box<dyn std::error::Error>> {
        let (path, loops, threads, want_stats) = parse_args();
        let mut sink = io::sink();

        for _ in 0..loops {
            let mut parser = EvtxParser::from_path(&path)?;
            let settings = ParserSettings::new()
                .num_threads(threads)
                .validate_checksums(false)
                .separate_json_attributes(false)
                .indent(false);
            parser = parser.with_configuration(settings);

            for record in parser.records_json() {
                match record {
                    Ok(rec) => {
                        sink.write_all(rec.data.as_bytes())?;
                        sink.write_all(b"\n")?;
                    }
                    Err(_err) => {
                        // Match evtx_dump behavior: non-fatal parse errors are skipped.
                    }
                }
            }
        }

        if want_stats {
            eprintln!("stats requested, but perf counters have been removed");
        }

        Ok(())
    }
}

#[cfg(feature = "bench")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    imp::run()
}

#[cfg(not(feature = "bench"))]
fn main() {
    eprintln!("`bench_evtx_dump_loop` requires `--features bench`");
}
