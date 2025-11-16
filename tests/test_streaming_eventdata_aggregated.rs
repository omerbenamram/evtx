mod fixtures;

use evtx::{EvtxParser, ParserSettings};
use fixtures::ensure_env_logger_initialized;
use serde_json::Value;

/// Regression test for aggregated `<EventData><Data>...</Data>...</EventData>`
/// handling in the streaming JSON parser. The legacy JSON parser produces
/// `Event.EventData.Data.#text` as an array when there are multiple unnamed
/// `<Data>` elements; the streaming parser must match this behaviour.
#[test]
fn test_streaming_multiple_data_elements_matches_legacy() {
    ensure_env_logger_initialized();

    let evtx_file = include_bytes!("../samples/MSExchange_Management_wec.evtx");

    // Legacy JSON parser.
    let mut parser_legacy = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    // Streaming JSON parser.
    let mut parser_streaming = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let legacy_record = parser_legacy
        .records_json()
        .next()
        .expect("to have records")
        .expect("legacy record to parse correctly");

    let streaming_record = parser_streaming
        .records_json_stream()
        .next()
        .expect("to have records")
        .expect("streaming record to parse correctly");

    let legacy_value: Value =
        serde_json::from_str(&legacy_record.data).expect("legacy JSON should be valid");
    let streaming_value: Value =
        serde_json::from_str(&streaming_record.data).expect("streaming JSON should be valid");

    // Full event equality – this also checks the aggregated `EventData/Data`
    // structure for regressions.
    assert_eq!(
        legacy_value, streaming_value,
        "streaming JSON must match legacy JSON for multiple <Data> elements"
    );

    let legacy_text = &legacy_value["Event"]["EventData"]["Data"]["#text"];
    let streaming_text = &streaming_value["Event"]["EventData"]["Data"]["#text"];

    assert!(
        legacy_text.is_array(),
        "legacy parser should expose Event.EventData.Data.#text as an array when multiple unnamed <Data> elements exist"
    );
    assert_eq!(
        legacy_text, streaming_text,
        "streaming parser must match legacy Event.EventData.Data.#text array semantics"
    );
}
