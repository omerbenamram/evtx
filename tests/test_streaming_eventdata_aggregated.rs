mod fixtures;

use evtx::{EvtxParser, ParserSettings};
use fixtures::ensure_env_logger_initialized;
use serde_json::Value;

/// Regression test for aggregated `<EventData><Data>...</Data>...</EventData>`
/// handling in the JSON renderer.
///
/// When there are multiple unnamed `<Data>` elements, we expect
/// `Event.EventData.Data.#text` to be an array.
#[test]
fn test_json_multiple_data_elements_are_aggregated() {
    ensure_env_logger_initialized();

    let evtx_file = include_bytes!("../samples/MSExchange_Management_wec.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("JSON should be valid");
    let text = &value["Event"]["EventData"]["Data"]["#text"];
    assert!(
        text.is_array(),
        "expected Event.EventData.Data.#text to be an array when multiple unnamed <Data> elements exist"
    );
    assert!(
        text.as_array().is_some_and(|a| a.len() > 1),
        "expected multiple aggregated Data items"
    );
}
