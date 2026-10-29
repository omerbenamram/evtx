mod fixtures;
use fixtures::*;

use evtx::{EvtxParser, ParserSettings};
use serde_json::Value;

#[test]
fn test_event_xml_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_json_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_xml_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_json_sample_with_event_data_with_attributes_and_text() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/system.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_xml_sample_with_event_data_with_attributes_and_text() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/system.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_xml_sample_with_user_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_xml_sample_with_entity_ref() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_xml_sample_with_entity_ref_2() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 25)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_xml_sample_with_string_with_trailing_spaces() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 2260)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_json_with_size_t() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/Security_with_size_t.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 196)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_nodes_same_name() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_sample_with_separate_json_attributes() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/Application.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_data_elements() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/MSExchange_Management_wec.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record_xml = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(&first_record_xml.data);

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_nodes_same_name_separate() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_multiple_empty_data_nodes_not_ignored() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/issue_201.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .next()
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_missing_string_cache_entry() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security_bad_string_cache.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .next()
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

/// Record 161's event data is one BinXML value holding a nested template instance followed by
/// a `RenderingInfo` element. Windows renders both; so must every output path.
#[test]
fn test_event_with_rendering_info_after_nested_template() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-Shell-Core%4Operational.evtx"
    );
    let parser = || {
        EvtxParser::from_buffer(evtx_file.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    let find = |records: &mut dyn Iterator<
        Item = evtx::err::Result<evtx::SerializedEvtxRecord<String>>,
    >| {
        records
            .filter_map(|record| record.ok())
            .find(|record| record.event_record_id == 161)
            .expect("record to parse correctly")
            .data
    };

    let xml = find(&mut parser().records());
    insta::assert_snapshot!(xml);

    let json: Value = serde_json::from_str(&find(&mut parser().records_json())).unwrap();
    let event = &json["Event"];
    assert_eq!(
        event["EventData"]["SelectionData"],
        "Region Codes:US \nSKUs:6 \nGroup Cell Width:6\n"
    );
    assert_eq!(event["RenderingInfo"]["Task"], "TraceLayoutSelectionData");

    // evtx_dump's chunk renderer gives the same bytes.
    for (records_api, json) in [(xml, false), (json.to_string(), true)] {
        let mut parser = parser();
        let chunks: Box<dyn Iterator<Item = evtx::RenderedChunk>> = if json {
            Box::new(parser.chunks_json_bytes(false))
        } else {
            Box::new(parser.chunks_xml_bytes(false))
        };
        let rendered = chunks
            .flat_map(|chunk| {
                let mut start = 0;
                let mut records = Vec::new();
                for item in chunk.items {
                    if let evtx::RenderedChunkItem::Record {
                        event_record_id,
                        end,
                    } = item
                    {
                        records.push((event_record_id, chunk.data[start..end - 1].to_vec()));
                        start = end;
                    }
                }
                records
            })
            .find(|(id, _)| *id == 161)
            .map(|(_, data)| String::from_utf8(data).unwrap())
            .expect("evtx_dump renders record 161");
        if json {
            let rendered: Value = serde_json::from_str(&rendered).unwrap();
            assert_eq!(rendered.to_string(), records_api);
        } else {
            assert_eq!(rendered, records_api);
        }
    }
}
