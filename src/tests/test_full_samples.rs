use crate::tests::fixtures::*;
use crate::{ensure_env_logger_initialized, EvtxParser, ParserSettings};
use log::Level;
use std::path::Path;

/// Tests an .evtx file, asserting the number of parsed records matches `count`.
fn test_full_sample(path: impl AsRef<Path>, count: usize) {
    ensure_env_logger_initialized();
    let mut parser = EvtxParser::from_path(path).unwrap();

    let mut real_count = 0;

    for r in parser.records() {
        if r.is_ok() {
            real_count += 1;
            if log::log_enabled!(Level::Debug) {
                println!("{}", r.unwrap().data);
            }
        }
    }
    assert_eq!(real_count, count, "Failed to parse all records as XML");

    let mut real_count = 0;
    for r in parser.records_json() {
        if r.is_ok() {
            real_count += 1;
            if log::log_enabled!(Level::Debug) {
                println!("{}", r.unwrap().data);
            }
        }
    }
    assert_eq!(real_count, count, "Failed to parse all records as JSON");
}

#[test]
// https://github.com/omerbenamram/evtx/issues/10
fn test_dirty_sample_single_threaded() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");

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
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");

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
    test_full_sample(sample_with_irregular_values(), 3028);
}

#[test]
fn test_dirty_sample_with_a_bad_checksum() {
    test_full_sample(sample_with_a_bad_checksum(), 1910)
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_2() {
    test_full_sample(sample_with_a_bad_checksum_2(), 1774)
}

#[test]
fn test_dirty_sample_with_a_chunk_past_zeros() {
    test_full_sample(sample_with_a_chunk_past_zeroes(), 1160)
}

#[test]
fn test_dirty_sample_with_a_bad_chunk_magic() {
    test_full_sample(sample_with_a_bad_chunk_magic(), 270)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_token() {
    test_full_sample(sample_binxml_with_incomplete_sid(), 6)
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_template() {
    test_full_sample(sample_binxml_with_incomplete_template(), 17)
}

#[test]
fn test_sample_with_multiple_xml_fragments() {
    test_full_sample(sample_with_multiple_xml_fragments(), 1146)
}

#[test]
fn test_sample_issue_25() {
    test_full_sample(sample_with_issue_25(), 1146)
}
