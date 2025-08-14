use evtx::{EvtxParser, ParserSettings};
use serde_json::Value;

fn main() {
    let evtx_path = std::env::args().nth(1).unwrap_or_else(|| {
        format!(
            "{}/samples/security.evtx",
            std::env::var("CARGO_MANIFEST_DIR").unwrap()
        )
    });

    let mut parser = EvtxParser::from_path(&evtx_path)
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_json = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let mut parser_stream = EvtxParser::from_path(&evtx_path)
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_stream = parser_stream
        .records_json_stream()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    println!("Non-streaming:\n{}", first_json.data);
    println!("Streaming:\n{}", first_stream.data);

    let v1: Value = serde_json::from_str(&first_json.data).unwrap();
    let v2: Value = serde_json::from_str(&first_stream.data).unwrap();
    if v1 != v2 {
        eprintln!("Mismatch");
        // Also show XML for context
        let mut parser_xml = EvtxParser::from_path(&evtx_path)
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1));
        let first_xml = parser_xml
            .records()
            .next()
            .expect("to have records")
            .expect("record to parse correctly");
        println!("XML:\n{}", first_xml.data);
        std::process::exit(1);
    }
}
