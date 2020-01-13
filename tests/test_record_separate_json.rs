mod fixtures;
use fixtures::*;

use evtx::{EvtxParser, ParserSettings};
use pretty_assertions::assert_eq;

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

    println!("{}", record.data);

    assert_eq!(
        record.data.lines().map(str::trim).collect::<String>(),
        include_str!("../samples/event_with_multiple_nodes_same_name_separate_attr.json")
            .lines()
            .map(str::trim)
            .collect::<String>()
    );
}
