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

/// Issue #289: bool slots holding values other than 0/1 keep the raw integer.
#[test]
fn test_irregular_boolean_values_are_preserved() {
    let mut parser = EvtxParser::from_path(sample_with_irregular_values()).unwrap();
    let xml = parser
        .records()
        .map(Result::unwrap)
        .find(|r| r.event_record_id == 141)
        .unwrap();
    assert!(
        xml.data.contains(r#"<Data Name="IsMachine">16</Data>"#),
        "{}",
        xml.data
    );

    let json = parser
        .records_json_value()
        .map(Result::unwrap)
        .find(|r| r.event_record_id == 148)
        .unwrap();
    let data = &json.data["Event"]["EventData"];
    assert_eq!(data["IsMachine"], true);
    assert_eq!(data["IsBackgroundProcessing"], 65536);
    assert_eq!(data["IsAsyncProcessing"], 8);

    // The CLI renders through the compiled raw-bytes path, which formats
    // values without decoding them first; check it agrees.
    let xml: Vec<u8> = parser
        .chunks_xml_bytes(false)
        .flat_map(|c| c.data)
        .collect();
    let xml = String::from_utf8(xml).unwrap();
    assert!(xml.contains(r#"<Data Name="IsMachine">16</Data>"#));
    let json: Vec<u8> = parser
        .chunks_json_bytes(false)
        .flat_map(|c| c.data)
        .collect();
    let json = String::from_utf8(json).unwrap();
    assert!(json.contains(r#""IsBackgroundProcessing":65536"#));
}

#[test]
fn test_dirty_sample_with_a_bad_checksum() {
    test_full_sample(sample_with_a_bad_checksum(), 1910, 4)
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_2() {
    // TODO: investigate 2 failing records
    test_full_sample(sample_with_a_bad_checksum_2(), 1795, 2)
}

#[test]
fn test_dirty_sample_with_a_chunk_past_zeros() {
    test_full_sample(sample_with_a_chunk_past_zeroes(), 1160, 0)
}

#[test]
fn test_dirty_sample_with_a_bad_chunk_magic() {
    test_full_sample(sample_with_a_bad_chunk_magic(), 301, 3)
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
    // The corrupt record does not hide the 114 live records after it.
    assert_eq!(count, 450, "Single threaded iteration failed");
}

/// Chunks share compiled templates, but a template body can reference a name
/// stored elsewhere in its chunk. Corrupting that name in one chunk must not
/// leak into the other chunk, whichever chunk is rendered first.
#[test]
fn test_shared_template_names_do_not_leak_across_chunks() {
    let mut bytes = include_bytes!("../samples/2-vss_7-System.evtx").to_vec();
    // `Keywords` -> `Keyɷords` in chunk 1's string table.
    bytes[EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE + 0x475] = 0x02;
    let mut swapped = bytes.clone();
    swapped[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + 2 * EVTX_CHUNK_SIZE]
        .rotate_left(EVTX_CHUNK_SIZE);

    let parser = |bytes: &[u8]| {
        EvtxParser::from_buffer(bytes.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    // Records API (validated lane).
    let by_id = |bytes: &[u8]| {
        parser(bytes)
            .records_json()
            .filter_map(Result::ok)
            .map(|r| (r.event_record_id, r.data))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let normal = by_id(&bytes);
    assert!(normal.values().any(|r| r.contains("Keyɷords")));
    assert_eq!(normal, by_id(&swapped));

    // CLI chunk lane (raw bytes): chunk 0 and 1 trade places.
    for json in [true, false] {
        let chunks = |bytes: &[u8]| {
            let mut parser = parser(bytes);
            let chunks: Vec<_> = if json {
                parser.chunks_json_bytes(false).map(|c| c.data).collect()
            } else {
                parser.chunks_xml_bytes(false).map(|c| c.data).collect()
            };
            chunks
        };
        let mut moved = chunks(&swapped);
        moved.swap(0, 1);
        assert_eq!(chunks(&bytes), moved, "json: {json}");
    }
}
