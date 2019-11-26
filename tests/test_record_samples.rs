mod fixtures;
use fixtures::*;

use evtx::{EvtxParser, ParserSettings};
use pretty_assertions::assert_eq;

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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/security_event_1.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/security_event_1.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_eventdata.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_eventdata.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_text_and_attributes.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_text_and_attributes.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    println!("{}", first_record.data);

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_template_as_substitution.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    println!("{}", record.data);

    assert_eq!(
        record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_entity_ref.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    println!("{}", record.data);

    assert_eq!(
        record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_entity_ref_2.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    println!("{}", record.data);

    assert_eq!(
        record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_multiple_nodes_same_name.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
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

    assert_eq!(
        first_record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/application_event_1_separate_attributes.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}
