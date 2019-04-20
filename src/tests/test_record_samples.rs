use crate::{ensure_env_logger_initialized, EvtxParser, ParserSettings};
use pretty_assertions::assert_eq;

#[test]
fn test_event_xml_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    assert_eq!(
        first_record
            .data
            .lines()
            .map(|l| l.trim())
            .collect::<String>(),
        include_str!("../../samples/security_event_1.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}

#[test]
fn test_event_json_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    assert_eq!(
        first_record
            .data
            .lines()
            .map(|l| l.trim())
            .collect::<String>(),
        include_str!("../../samples/security_event_1.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}

#[test]
fn test_event_json_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    assert_eq!(
        first_record
            .data
            .lines()
            .map(|l| l.trim())
            .collect::<String>(),
        include_str!("../../samples/event_with_eventdata.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}

#[test]
fn test_event_xml_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    assert_eq!(
        first_record
            .data
            .lines()
            .map(|l| l.trim())
            .collect::<String>(),
        include_str!("../../samples/event_with_eventdata.xml")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}
