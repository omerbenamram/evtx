mod fixtures;

use evtx::{EvtxParser, ParserSettings};
use fixtures::*;
use log::Level;
use std::path::Path;

/// Tests an .evtx file using the streaming JSON parser, asserting the number of parsed records matches `count`.
fn test_full_sample_streaming(path: impl AsRef<Path>, ok_count: usize, err_count: usize) {
    ensure_env_logger_initialized();
    let mut parser = EvtxParser::from_path(path).unwrap();

    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;

    // Test streaming JSON parser
    for r in parser.records_json_stream() {
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
        "Streaming JSON: Failed to parse all expected records"
    );
    assert_eq!(
        actual_err_count, err_count,
        "Streaming JSON: Expected errors"
    );

    // Test streaming JSON parser with separate_json_attributes
    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;
    let separate_json_attributes = ParserSettings::default().separate_json_attributes(true);
    parser = parser.with_configuration(separate_json_attributes);

    for r in parser.records_json_stream() {
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
        "Streaming JSON (separate attributes): Failed to parse all expected records"
    );
    assert_eq!(
        actual_err_count, err_count,
        "Streaming JSON (separate attributes): Expected errors"
    );
}

/// Compare streaming JSON output with regular JSON output to ensure they produce equivalent results
fn test_streaming_equivalent_to_regular(path: impl AsRef<Path>) {
    ensure_env_logger_initialized();

    // Parse with regular JSON parser
    let mut parser_regular = EvtxParser::from_path(&path).unwrap();
    let mut regular_results: Vec<String> = Vec::new();
    for record in parser_regular.records_json().flatten() {
        regular_results.push(record.data);
    }

    // Parse with streaming JSON parser
    let mut parser_streaming = EvtxParser::from_path(&path).unwrap();
    let mut streaming_results: Vec<String> = Vec::new();
    for record in parser_streaming.records_json_stream().flatten() {
        streaming_results.push(record.data);
    }

    // Compare counts
    assert_eq!(
        regular_results.len(),
        streaming_results.len(),
        "Streaming parser should produce same number of records as regular parser"
    );

    // Compare JSON values (parse and compare as Value to handle formatting differences)
    use serde_json::Value;
    for (i, (regular, streaming)) in regular_results
        .iter()
        .zip(streaming_results.iter())
        .enumerate()
    {
        let regular_value: Value = serde_json::from_str(regular)
            .unwrap_or_else(|e| panic!("Regular JSON should be valid at record {i}: {e}"));
        let streaming_value: Value = serde_json::from_str(streaming)
            .unwrap_or_else(|e| panic!("Streaming JSON should be valid at record {i}: {e}"));

        if regular_value != streaming_value {
            eprintln!(
                "Regular JSON record {}:\n{}\nStreaming JSON record {}:\n{}",
                i,
                serde_json::to_string_pretty(&regular_value).unwrap(),
                i,
                serde_json::to_string_pretty(&streaming_value).unwrap()
            );
        }

        assert_eq!(
            regular_value, streaming_value,
            "Streaming parser should produce equivalent JSON to regular parser at record {}",
            i
        );
    }
}

#[test]
fn test_streaming_equivalent_to_regular_security() {
    test_streaming_equivalent_to_regular(regular_sample());
}

#[test]
fn test_streaming_equivalent_to_regular_system() {
    test_streaming_equivalent_to_regular(samples_dir().join("system.evtx"));
}

#[test]
fn test_parses_sample_with_irregular_boolean_values_streaming() {
    test_full_sample_streaming(sample_with_irregular_values(), 3028, 0);
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_streaming() {
    test_full_sample_streaming(sample_with_a_bad_checksum(), 1910, 4)
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_2_streaming() {
    // TODO: investigate 2 failing records
    test_full_sample_streaming(sample_with_a_bad_checksum_2(), 1774, 2)
}

#[test]
fn test_dirty_sample_with_a_chunk_past_zeros_streaming() {
    test_full_sample_streaming(sample_with_a_chunk_past_zeroes(), 1160, 0)
}

#[test]
fn test_dirty_sample_with_a_bad_chunk_magic_streaming() {
    test_full_sample_streaming(sample_with_a_bad_chunk_magic(), 270, 2)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_token_streaming() {
    // Contains an unparsable record
    test_full_sample_streaming(sample_binxml_with_incomplete_sid(), 6, 0)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_template_streaming() {
    // Contains an unparsable record
    test_full_sample_streaming(sample_binxml_with_incomplete_template(), 17, 0)
}

#[test]
fn test_sample_with_multiple_xml_fragments_streaming() {
    test_full_sample_streaming(sample_with_multiple_xml_fragments(), 1146, 0)
}

#[test]
fn test_issue_65_streaming() {
    test_full_sample_streaming(sample_issue_65(), 459, 0)
}

#[test]
fn test_sample_with_binxml_as_substitution_tokens_and_pi_target_streaming() {
    test_full_sample_streaming(
        sample_with_binxml_as_substitution_tokens_and_pi_target(),
        340,
        0,
    )
}

#[test]
fn test_sample_with_dependency_identifier_edge_case_streaming() {
    test_full_sample_streaming(sample_with_dependency_id_edge_case(), 653, 0)
}

#[test]
fn test_sample_with_no_crc32_streaming() {
    test_full_sample_streaming(sample_with_no_crc32(), 17, 0)
}

#[test]
fn test_sample_with_invalid_flags_in_header_streaming() {
    test_full_sample_streaming(sample_with_invalid_flags_in_header(), 126, 0)
}
