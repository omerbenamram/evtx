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

    // Test JSON parser (streaming IR renderer)
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
        "Streaming JSON: Failed to parse all expected records"
    );
    assert_eq!(
        actual_err_count, err_count,
        "Streaming JSON: Expected errors"
    );

    // Test JSON parser with separate_json_attributes
    let mut actual_ok_count = 0;
    let mut actual_err_count = 0;
    let separate_json_attributes = ParserSettings::default().separate_json_attributes(true);
    parser = parser.with_configuration(separate_json_attributes);

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
        "Streaming JSON (separate attributes): Failed to parse all expected records"
    );
    assert_eq!(
        actual_err_count, err_count,
        "Streaming JSON (separate attributes): Expected errors"
    );
}

// (Removed: streaming vs legacy JSON parity tests. The library now only exposes the streaming
// IR JSON renderer.)

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
