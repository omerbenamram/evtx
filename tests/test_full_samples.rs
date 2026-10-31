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
    // Chunk 1 was caught while the log reused it: its 16 KiB from 0x4000 to 0x8000 are from
    // the log's next pass, where the walk reads records 462980 to 463011 (both passes wrote
    // 496-byte records). Record 463011 straddles 0x8000, so its BinXML runs into an old
    // record, and where the record after it should start is that old record's text: 2
    // errors. The walk resyncs on record 401576.
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
    let clean = include_bytes!("../samples/2-vss_7-System.evtx").to_vec();
    let parser = |bytes: &[u8]| {
        EvtxParser::from_buffer(bytes.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    // Records API (validated lane).
    let records = |bytes: &[u8]| -> Vec<(u64, String)> {
        parser(bytes)
            .records_json()
            .filter_map(Result::ok)
            .map(|r| (r.event_record_id, r.data))
            .collect()
    };
    // CLI chunk lane (raw bytes).
    let chunks = |bytes: &[u8], json: bool| -> Vec<Vec<u8>> {
        let mut parser = parser(bytes);
        if json {
            parser.chunks_json_bytes(false).map(|c| c.data).collect()
        } else {
            parser.chunks_xml_bytes(false).map(|c| c.data).collect()
        }
    };

    // Chunks render oldest first: chunk 0 holds records 1 to 131, then chunk 1 132 to 238. So
    // corrupting chunk 0 renders the corrupt name first, and corrupting chunk 1 renders it second.
    for (chunk, ids) in [(0, 1..=131), (1, 132..=238)] {
        // `Keywords` -> `Keyɷords` in the chunk's string table.
        let mut bytes = clean.clone();
        bytes[EVTX_FILE_HEADER_SIZE + chunk * EVTX_CHUNK_SIZE + 0x475] = 0x02;

        let mut corrupt = 0;
        for (record, expected) in records(&bytes).into_iter().zip(records(&clean)) {
            assert_eq!(record.0, expected.0);
            if ids.contains(&record.0) {
                corrupt += usize::from(record.1.contains("Keyɷords"));
            } else {
                assert_eq!(record, expected, "chunk {chunk}");
            }
        }
        assert!(corrupt > 0, "chunk {chunk}");

        for json in [true, false] {
            let (rendered, expected) = (chunks(&bytes, json), chunks(&clean, json));
            assert_eq!(rendered.len(), expected.len());
            for (i, (rendered, expected)) in rendered.iter().zip(&expected).enumerate() {
                if i == chunk {
                    assert!(String::from_utf8_lossy(rendered).contains("Keyɷords"));
                } else {
                    assert_eq!(rendered, expected, "chunk {chunk}, json: {json}");
                }
            }
        }
    }
}

#[test]
fn test_wrapped_logs_come_out_oldest_first() {
    // Both logs have wrapped around, so their oldest records are in the middle of the file, and
    // Windows renders them in this order.
    ensure_env_logger_initialized();
    for (path, first, count) in [
        (sample_issue_65(), 261, 459),
        (sample_with_a_stale_oldest_chunk(), 2032, 399),
    ] {
        let expected: Vec<u64> = (first..first + count).collect();
        let mut parser = EvtxParser::from_path(&path).unwrap();
        let ids: Vec<u64> = parser
            .records()
            .map(|record| record.unwrap().event_record_id)
            .collect();
        assert_eq!(ids, expected, "{}", path.display());
        let ids: Vec<u64> = EvtxParser::from_path(&path)
            .unwrap()
            .into_chunks()
            .flat_map(|chunk| {
                let mut chunk = chunk.unwrap();
                let settings = std::sync::Arc::new(ParserSettings::default());
                let mut chunk = chunk.parse(settings).unwrap();
                chunk
                    .iter()
                    .map(|record| record.unwrap().event_record_id)
                    .collect::<Vec<_>>()
            })
            .collect();
        assert_eq!(ids, expected, "{}", path.display());
    }
}
