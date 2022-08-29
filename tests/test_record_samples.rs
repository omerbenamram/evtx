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

    insta::assert_display_snapshot!(first_record.data);
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

    insta::assert_display_snapshot!(first_record.data);
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

    insta::assert_display_snapshot!(first_record.data);
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

    insta::assert_display_snapshot!(first_record.data);
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

    insta::assert_display_snapshot!(record.data);
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

    insta::assert_display_snapshot!(record.data);
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

    insta::assert_display_snapshot!(&first_record_xml.data);

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
