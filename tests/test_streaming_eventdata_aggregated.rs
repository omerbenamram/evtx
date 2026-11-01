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

/// The JSON of every record in `path`, by record ID, from the records API with `settings` and
/// from the chunk renderer `evtx_dump` uses.
fn json_records(path: &std::path::Path, settings: ParserSettings) -> Vec<(u64, String)> {
    let mut parser = EvtxParser::from_path(path)
        .unwrap()
        .with_configuration(settings.num_threads(1));
    let mut records: Vec<(u64, String)> = parser
        .records_json()
        .filter_map(Result::ok)
        .map(|record| (record.event_record_id, record.data))
        .collect();
    let chunks: Vec<_> = parser.chunks_json_bytes(false).collect();
    for chunk in &chunks {
        for (id, data) in chunk.records() {
            records.push((id, String::from_utf8(data.to_vec()).unwrap()));
        }
    }
    records
}

/// An array substitution in a named `Data` repeats the element once per value, as in Windows'
/// XML. In JSON its values become one array, like the `#text` of unnamed `Data`.
#[test]
fn test_json_array_substitutions_become_arrays() {
    ensure_env_logger_initialized();
    let windows = fixtures::samples_dir().join("windows");

    // Windows' XML of each event: the Data names it repeats, and how often.
    let mut repeated = std::collections::HashMap::new();
    let jsonl = std::fs::read_to_string(windows.join("types.evtx.jsonl")).unwrap();
    for line in jsonl.lines() {
        let event: Value = serde_json::from_str(line).unwrap();
        // Windows can't render the SID arrays with items.
        let Some(xml) = event["xml"].as_str() else {
            continue;
        };
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for data in xml.split("<Data Name='").skip(1) {
            let name = data.split('\'').next().unwrap();
            *counts.entry(name.to_string()).or_default() += 1;
        }
        counts.retain(|_, count| *count > 1);
        if !counts.is_empty() {
            repeated.insert(event["record_id"].as_u64().unwrap(), counts);
        }
    }
    assert_eq!(
        repeated.len(),
        19,
        "the sample has an array event for each type"
    );

    for separate in [false, true] {
        let settings = ParserSettings::new().separate_json_attributes(separate);
        let records = json_records(&windows.join("types.evtx"), settings);
        let mut checked = 0;
        for (id, data) in &records {
            let Some(counts) = repeated.get(id) else {
                continue;
            };
            let json: Value = serde_json::from_str(data).unwrap();
            for (name, count) in counts {
                let values = &json["Event"]["EventData"][name.as_str()];
                assert_eq!(
                    values.as_array().map(Vec::len),
                    Some(*count),
                    "record {id}, {name}: {data}"
                );
                // One key, which JSON readers don't collapse.
                assert_eq!(data.matches(&format!("\"{name}\":")).count(), 1, "{data}");
            }
            checked += 1;
        }
        // Both APIs render the array events: all 19 once evtx reads ANSI string arrays.
        assert!(checked >= 2 * 18, "{checked} renderings checked");
    }
}

/// Some templates repeat a `Data` name further on: event 4648 in this sample names both the
/// subject's and the target's logon GUID `LogonGuid` (the other samples call the second
/// `TargetLogonGuid`). The second is numbered like other repeated keys, so that JSON readers
/// keep both.
#[test]
fn test_json_repeated_data_names_are_numbered() {
    ensure_env_logger_initialized();
    let path = fixtures::samples_dir().join("Security_with_size_t.evtx");
    for separate in [false, true] {
        let settings = ParserSettings::new().separate_json_attributes(separate);
        let records = json_records(&path, settings);
        let (_, data) = records.iter().find(|(id, _)| *id == 4).unwrap();
        let json: Value = serde_json::from_str(data).unwrap();
        let event_data = &json["Event"]["EventData"];
        assert_eq!(json["Event"]["System"]["EventID"], 4648);
        assert_eq!(event_data["SubjectLogonId"], "0x3e7");
        assert!(event_data["LogonGuid"].is_string(), "{data}");
        assert!(event_data["LogonGuid_1"].is_string(), "{data}");
        assert_eq!(data.matches("\"LogonGuid\":").count(), 1, "{data}");
        assert_eq!(
            records
                .iter()
                .filter(|(_, data)| data.contains("\"LogonGuid_1\":"))
                .count(),
            2 * 92,
            "every 4648 event in the sample, from both APIs"
        );
    }
}

/// A template that repeats a `Data` name itself, twice in a row and once further on: the two in
/// a row are an array, as an array substitution's are, and the later one is numbered. Templates
/// that repeat a name in a row render the slower way, from their tree, in both APIs.
#[test]
fn test_json_template_repeating_a_data_name() {
    let evtx = fixtures::synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("Event", true).close_start();
                    root.open("EventData", true).close_start();
                    for (slot, name) in ["A", "A", "B", "A"].into_iter().enumerate() {
                        root.open_with_attributes("Data", true)
                            .attribute("Name")
                            .text(name)
                            .close_start()
                            .substitution(slot as u16, 0x01)
                            .end_element();
                    }
                    root.end_element();
                    root.end_element();
                },
                (1..=4)
                    .map(|n| {
                        fixtures::value(0x01, move |text| {
                            text.utf16(&n.to_string());
                        })
                    })
                    .collect(),
            )
            .eof();
    });
    let (xml, json) = fixtures::render_record(&evtx, 1);
    assert_eq!(xml.matches("<Data Name=\"A\">").count(), 3, "{xml}");
    assert!(
        json.contains(r#""EventData":{"A":["1","2"],"B":"3","A_1":"4"}"#),
        "{json}"
    );
}
