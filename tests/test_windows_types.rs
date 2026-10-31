//! Checks evtx against `samples/windows/types.evtx.jsonl`, the XML Windows renders for each
//! event of `samples/windows/types.evtx` (see `samples/windows/README.md`).

use evtx::err::EvtxError;
use evtx::{EvtxParser, ParserSettings};
use std::collections::HashMap;

/// The `Data` elements of an XML record whose name ends in `TIME` or `TIMEs`, in order, as
/// (name, text). Windows quotes attributes with `'` and evtx with `"`.
fn time_data(xml: &str) -> Vec<(String, String)> {
    xml.split("<Data Name=")
        .skip(1)
        .filter_map(|data| {
            let quote = data.chars().next()?;
            let name = data[1..].split(quote).next()?;
            if !(name.ends_with("TIME") || name.ends_with("TIMEs")) {
                return None;
            }
            let rest = &data[1 + name.len() + 1..];
            let text = match rest.strip_prefix('>') {
                Some(text) => text.split("</Data>").next()?,
                None => "", // `<Data Name='x'/>`
            };
            Some((name.to_owned(), text.to_owned()))
        })
        .collect()
}

#[test]
fn timestamps_render_as_windows_renders_them() {
    let jsonl = std::fs::read_to_string("samples/windows/types.evtx.jsonl").unwrap();
    let windows: Vec<(u64, String)> = jsonl
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter_map(|event| {
            Some((
                event["record_id"].as_u64()?,
                event["xml"].as_str()?.to_owned(),
            ))
        })
        .collect();

    let settings = ParserSettings::new().num_threads(1).indent(false);
    let mut parser = EvtxParser::from_path("samples/windows/types.evtx")
        .unwrap()
        .with_configuration(settings);
    let mut evtx = HashMap::new();
    for record in parser.records() {
        match record {
            Ok(record) => {
                evtx.insert(record.event_record_id, Ok(record.data));
            }
            Err(EvtxError::FailedToParseRecord { record_id, source }) => {
                evtx.insert(record_id, Err(source.to_string()));
            }
            Err(err) => panic!("{err}"),
        }
    }

    let mut values = 0;
    for (record_id, xml) in &windows {
        let expected = time_data(xml);
        if expected.is_empty() {
            continue;
        }
        let rendered = match &evtx[record_id] {
            Ok(rendered) => rendered,
            Err(err) => panic!("record {record_id} failed: {err}"),
        };
        assert_eq!(time_data(rendered), expected, "record {record_id}");
        values += expected.len();
    }
    // The FILETIME and SYSTEMTIME events, their array events, and the scalars event.
    assert!(values >= 30, "only {values} timestamps compared");
}
