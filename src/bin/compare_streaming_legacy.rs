use evtx::{EvtxParser, ParserSettings};
use serde_json::Value;
use std::env;
use std::error::Error;
use std::path::PathBuf;

fn main() {
    if let Err(e) = run() {
        eprintln!("compare_streaming_legacy: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let (path, settings, max_records) = parse_args()?;

    let mut parser_legacy =
        EvtxParser::from_path(&path)?.with_configuration(settings.clone().indent(false));
    let mut parser_streaming =
        EvtxParser::from_path(&path)?.with_configuration(settings.clone().indent(false));

    let mut legacy_iter = parser_legacy.records_json();
    let mut streaming_iter = parser_streaming.records_json_stream();

    let mut index: usize = 0;

    loop {
        if let Some(limit) = max_records {
            if index >= limit {
                break;
            }
        }

        let legacy_next = legacy_iter.next();
        let streaming_next = streaming_iter.next();

        match (legacy_next, streaming_next) {
            (None, None) => break,
            (Some(_), None) => {
                eprintln!(
                    "Mismatch: legacy parser produced more records than streaming parser at index {}",
                    index
                );
                return Err("record count mismatch".into());
            }
            (None, Some(_)) => {
                eprintln!(
                    "Mismatch: streaming parser produced more records than legacy parser at index {}",
                    index
                );
                return Err("record count mismatch".into());
            }
            (Some(legacy_res), Some(streaming_res)) => {
                match (legacy_res, streaming_res) {
                    (Ok(legacy_record), Ok(streaming_record)) => {
                        let legacy_value: Value = serde_json::from_str(&legacy_record.data)?;
                        let streaming_value: Value = serde_json::from_str(&streaming_record.data)?;

                        if legacy_value != streaming_value {
                            eprintln!(
                                "JSON mismatch at record index {} (EventRecordId={}):",
                                index, legacy_record.event_record_id
                            );
                            eprintln!("Legacy JSON:");
                            eprintln!("{}", serde_json::to_string_pretty(&legacy_value)?);
                            eprintln!();
                            eprintln!("Streaming JSON:");
                            eprintln!("{}", serde_json::to_string_pretty(&streaming_value)?);
                            return Err("streaming JSON does not match legacy JSON".into());
                        }
                    }
                    (Err(legacy_err), Ok(streaming_record)) => {
                        eprintln!(
                            "Error mismatch at record index {}: legacy parser failed, streaming succeeded.",
                            index
                        );
                        eprintln!("Legacy error: {legacy_err}");
                        eprintln!(
                            "Streaming JSON record (EventRecordId={}):",
                            streaming_record.event_record_id
                        );
                        let streaming_value: Value = serde_json::from_str(&streaming_record.data)?;
                        eprintln!("{}", serde_json::to_string_pretty(&streaming_value)?);
                        return Err("legacy parser failed while streaming succeeded".into());
                    }
                    (Ok(legacy_record), Err(streaming_err)) => {
                        eprintln!(
                            "Error mismatch at record index {}: streaming parser failed, legacy succeeded.",
                            index
                        );
                        eprintln!("Streaming error: {streaming_err}");
                        eprintln!(
                            "Legacy JSON record (EventRecordId={}):",
                            legacy_record.event_record_id
                        );
                        let legacy_value: Value = serde_json::from_str(&legacy_record.data)?;
                        eprintln!("{}", serde_json::to_string_pretty(&legacy_value)?);
                        return Err("streaming parser failed while legacy succeeded".into());
                    }
                    (Err(legacy_err), Err(streaming_err)) => {
                        // Both failed for this record – treat as equivalent and continue.
                        eprintln!(
                            "Both parsers failed at record index {}.\n  Legacy error: {}\n  Streaming error: {}",
                            index, legacy_err, streaming_err
                        );
                    }
                }
            }
        }

        index += 1;
    }

    eprintln!(
        "Success: legacy and streaming JSON outputs match for {} records (path: {}).",
        index,
        path.display()
    );

    Ok(())
}

fn parse_args() -> Result<(PathBuf, ParserSettings, Option<usize>), Box<dyn Error>> {
    let mut args = env::args().skip(1);

    let mut separate_json_attributes = false;
    let mut validate_checksums = false;
    let mut num_threads: Option<usize> = None;
    let mut max_records: Option<usize> = None;
    let mut path: Option<PathBuf> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_usage();
                std::process::exit(0);
            }
            "-s" | "--separate-json-attributes" => {
                separate_json_attributes = true;
            }
            "-c" | "--validate-checksums" => {
                validate_checksums = true;
            }
            "-j" | "--num-threads" => {
                let value = args.next().ok_or("missing value for --num-threads")?;
                num_threads = Some(value.parse()?);
            }
            "-n" | "--max-records" => {
                let value = args.next().ok_or("missing value for --max-records")?;
                max_records = Some(value.parse()?);
            }
            _ if path.is_none() => {
                path = Some(PathBuf::from(arg));
            }
            _ => {
                return Err(format!("unknown argument: {arg}").into());
            }
        }
    }

    let path = path.ok_or("missing EVTX path\n\nUse --help for usage.")?;

    let mut settings = ParserSettings::new()
        .separate_json_attributes(separate_json_attributes)
        .validate_checksums(validate_checksums);

    if let Some(n) = num_threads {
        settings = settings.num_threads(n);
    }

    Ok((path, settings, max_records))
}

fn print_usage() {
    eprintln!(
        "Usage: compare_streaming_legacy [OPTIONS] <EVTX_PATH>

Compares legacy JSON and streaming JSON output for the given EVTX file and aborts
on the first mismatch, printing both JSON payloads for easy regression test creation.

Options:
  -s, --separate-json-attributes   Use separate_json_attributes=true
  -c, --validate-checksums         Validate chunk checksums
  -j, --num-threads <N>            Use N worker threads (0 = auto)
  -n, --max-records <N>            Only compare the first N records
  -h, --help                       Show this help message
"
    );
}

