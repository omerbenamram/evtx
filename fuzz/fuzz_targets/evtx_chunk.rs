#![no_main]

use evtx::{EvtxChunkData, ParserSettings};
use libfuzzer_sys::fuzz_target;
use std::sync::Arc;

// One chunk: its header, string and template tables, and each record as XML, then as JSON.
fuzz_target!(|data: &[u8]| {
    let Ok(mut chunk_data) = EvtxChunkData::new(data.to_vec(), false) else {
        return;
    };
    let Ok(mut chunk) = chunk_data.parse(Arc::new(ParserSettings::new())) else {
        return;
    };
    for record in chunk.iter().flatten() {
        let _ = record.into_xml();
    }
    for record in chunk.iter().flatten() {
        let _ = record.into_json();
    }
});
