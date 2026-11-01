#![no_main]

use evtx::{EvtxParser, ParserSettings};
use libfuzzer_sys::fuzz_target;

// A whole log: its file header and the walk over its chunks, with each record as XML, then as a
// JSON value with the records recovered from the chunks' slack space too.
fuzz_target!(|data: &[u8]| {
    let Ok(mut parser) = EvtxParser::from_buffer(data.to_vec()) else {
        return;
    };
    parser.records().for_each(drop);
    let settings = ParserSettings::new()
        .recover_records(true)
        .separate_json_attributes(true);
    let mut parser = parser.with_configuration(settings);
    parser.records_json_value().for_each(drop);
});
