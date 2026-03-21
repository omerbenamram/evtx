mod fixtures;

use evtx::{EvtxParser, ParserSettings};
use fixtures::*;
use log::Level;
use std::path::Path;

/// Tests an .evtx file, asserting the number of parsed records matches `count`.
fn test_full_sample(path: impl AsRef<Path>, ok_count: usize, err_count: usize) {
    ensure_env_logger_initialized();
    let mut parser = EvtxParser::from_path(path).unwrap();

    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;

    for r in parser.records() {
        if let Ok(r) = r {
            actual_ok_count += 1;
            if log::log_enabled!(Level::Debug) {
                println!("{}", r.data);
            }
        } else {
            actual_err_count += 1;
        }
    }
    assert_eq!(
        actual_ok_count, ok_count,
        "XML: Failed to parse all expected records"
    );
    assert_eq!(actual_err_count, err_count, "XML: Expected errors");

    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;

    for r in parser.records_json() {
        if let Ok(r) = r {
            actual_ok_count += 1;
            if log::log_enabled!(Level::Debug) {
                println!("{}", r.data);
            }
        } else {
            actual_err_count += 1;
        }
    }
    assert_eq!(
        actual_ok_count, ok_count,
        "Failed to parse all records as JSON"
    );
    assert_eq!(actual_err_count, err_count, "XML: Expected errors");

    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;
    let seperate_json_attributes = ParserSettings::default().separate_json_attributes(true);
    parser = parser.with_configuration(seperate_json_attributes);

    for r in parser.records_json() {
        if let Ok(r) = r {
            actual_ok_count += 1;
            if log::log_enabled!(Level::Debug) {
                println!("{}", r.data);
            }
        } else {
            actual_err_count += 1;
        }
    }
    assert_eq!(
        actual_ok_count, ok_count,
        "Failed to parse all records as JSON"
    );
    assert_eq!(actual_err_count, err_count, "XML: Expected errors");
}

#[test]
// https://github.com/omerbenamram/evtx/issues/10
fn test_dirty_sample_single_threaded() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    let mut count = 0;
    for r in parser.records() {
        r.unwrap();
        count += 1;
    }
    assert_eq!(count, 14621, "Single threaded iteration failed");
}

#[test]
fn test_dirty_sample_parallel() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(8));

    let mut count = 0;

    for r in parser.records() {
        r.unwrap();
        count += 1;
    }

    assert_eq!(count, 14621, "Parallel iteration failed");
}

#[test]
fn test_parses_sample_with_irregular_boolean_values() {
    test_full_sample(sample_with_irregular_values(), 3028, 0);
}

#[test]
fn test_dirty_sample_with_a_bad_checksum() {
    test_full_sample(sample_with_a_bad_checksum(), 1910, 4)
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_2() {
    // TODO: investigate 2 failing records
    test_full_sample(sample_with_a_bad_checksum_2(), 1774, 2)
}

#[test]
fn test_dirty_sample_with_a_chunk_past_zeros() {
    test_full_sample(sample_with_a_chunk_past_zeroes(), 1160, 0)
}

#[test]
fn test_dirty_sample_with_a_bad_chunk_magic() {
    test_full_sample(sample_with_a_bad_chunk_magic(), 270, 2)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_token() {
    // Contains an unparsable record
    test_full_sample(sample_binxml_with_incomplete_sid(), 6, 0)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_template() {
    // Contains an unparsable record
    test_full_sample(sample_binxml_with_incomplete_template(), 17, 0)
}

#[test]
fn test_sample_with_multiple_xml_fragments() {
    test_full_sample(sample_with_multiple_xml_fragments(), 1146, 0)
}

#[test]
fn test_issue_65() {
    test_full_sample(sample_issue_65(), 459, 0)
}

#[test]
fn test_sample_with_binxml_as_substitution_tokens_and_pi_target() {
    test_full_sample(
        sample_with_binxml_as_substitution_tokens_and_pi_target(),
        340,
        0,
    )
}

#[test]
fn test_sample_with_dependency_identifier_edge_case() {
    test_full_sample(sample_with_dependency_id_edge_case(), 653, 0)
}

#[test]
fn test_sample_with_no_crc32() {
    test_full_sample(sample_with_no_crc32(), 17, 0)
}

#[test]
fn test_sample_with_invalid_flags_in_header() {
    test_full_sample(sample_with_invalid_flags_in_header(), 126, 0)
}

#[test]
fn test_sample_with_zero_data_size_event() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/sample-with-zero-data-size-event.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    let mut count = 0;
    for r in parser.records() {
        if let Err(e) = r {
            assert_eq!(
                e.to_string(),
                "Invalid EVTX record data size, should be equals or greater than 28, found `0`"
            );
        }
        count += 1;
    }
    assert_eq!(count, 336, "Single threaded iteration failed");
}

#[test]
fn test_compiled_xml_matches_ir_xml_for_all_samples() {
    ensure_env_logger_initialized();
    let samples = [
        "security.evtx",
        "system.evtx",
        "new-user-security.evtx",
        "Application.evtx",
        "sysmon.evtx",
        "Security_short_selected.evtx",
        "Security_with_size_t.evtx",
        "E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx",
        "E_Windows_system32_winevt_logs_Microsoft-Windows-Shell-Core%4Operational.evtx",
        "Archive-ForwardedEvents-test.evtx",
    ];

    for sample in &samples {
        let path = samples_dir().join(sample);
        if !path.exists() {
            continue;
        }

        // IR path: use the explicit IR iterator pipeline.
        let mut parser_ir = EvtxParser::from_path(&path)
            .unwrap()
            .with_configuration(ParserSettings::new().indent(true));
        let ir_records: Vec<_> = parser_ir
            .serialized_records(|record| record.and_then(|record| record.into_xml()))
            .collect();

        // Compiled raw XML path: for_each_xml_record uses compiled templates
        let mut parser_compiled = EvtxParser::from_path(&path)
            .unwrap()
            .with_configuration(ParserSettings::new().indent(true));
        let mut compiled_records: Vec<(u64, String)> = Vec::new();
        let mut compiled_errors: Vec<String> = Vec::new();
        parser_compiled
            .for_each_xml_record(
                |record_id, _ts, xml_bytes| {
                    compiled_records
                        .push((record_id, String::from_utf8_lossy(xml_bytes).into_owned()));
                },
                |err| {
                    compiled_errors.push(format!("{:?}", err));
                    Ok(())
                },
            )
            .unwrap();

        // Count IR successes and errors for comparison
        let ir_ok: Vec<_> = ir_records.iter().filter(|r| r.is_ok()).collect();

        if ir_ok.len() != compiled_records.len() {
            eprintln!(
                "{}: errors: {:?}",
                sample,
                &compiled_errors[..compiled_errors.len().min(5)]
            );
        }
        assert_eq!(
            ir_ok.len(),
            compiled_records.len(),
            "{}: record count mismatch (IR ok={}, compiled={}, errors={})",
            sample,
            ir_ok.len(),
            compiled_records.len(),
            compiled_errors.len()
        );

        for (i, (ir, (compiled_id, compiled_xml))) in
            ir_ok.iter().zip(compiled_records.iter()).enumerate()
        {
            let ir_rec = ir.as_ref().unwrap();
            assert_eq!(
                ir_rec.data, *compiled_xml,
                "{}: record {} (id={}) XML mismatch",
                sample, i, compiled_id
            );
        }
    }
}

#[test]
fn test_records_matches_for_each_xml_record() {
    ensure_env_logger_initialized();
    let path = samples_dir().join("security.evtx");
    if !path.exists() {
        return;
    }

    let mut parser_records = EvtxParser::from_path(&path)
        .unwrap()
        .with_configuration(ParserSettings::new().indent(true));
    let records_api: Vec<_> = parser_records.records().collect();

    let mut parser_stream = EvtxParser::from_path(&path)
        .unwrap()
        .with_configuration(ParserSettings::new().indent(true));
    let mut stream_ok: Vec<(u64, String)> = Vec::new();
    let mut stream_err: Vec<String> = Vec::new();
    parser_stream
        .for_each_xml_record(
            |record_id, _ts, xml_bytes| {
                stream_ok.push((record_id, String::from_utf8_lossy(xml_bytes).into_owned()));
            },
            |err| {
                stream_err.push(err.to_string());
                Ok(())
            },
        )
        .unwrap();

    let records_ok: Vec<(u64, String)> = records_api
        .iter()
        .filter_map(|record| {
            record
                .as_ref()
                .ok()
                .map(|record| (record.event_record_id, record.data.clone()))
        })
        .collect();
    let records_err_count = records_api.iter().filter(|record| record.is_err()).count();

    assert_eq!(records_err_count, stream_err.len(), "error count mismatch");
    assert_eq!(records_ok.len(), stream_ok.len(), "success count mismatch");

    for (idx, (left, right)) in records_ok.iter().zip(stream_ok.iter()).enumerate() {
        assert_eq!(left.0, right.0, "record id mismatch at {}", idx);
        assert_eq!(left.1, right.1, "xml mismatch at {}", idx);
    }
}
