//! Checks evtx against `samples/windows/types.evtx.jsonl`, the XML Windows renders for each
//! event of `samples/windows/types.evtx` (see `samples/windows/README.md`).

use evtx::err::EvtxError;
use evtx::{EvtxParser, ParserSettings};
use std::collections::HashMap;

/// The `Data` elements of an XML record whose name `keep` selects, in order, as (name, text).
/// Windows quotes attributes with `'` and evtx with `"`.
fn named_data(xml: &str, keep: &impl Fn(&str) -> bool) -> Vec<(String, String)> {
    xml.split("<Data Name=")
        .skip(1)
        .filter_map(|data| {
            let quote = data.chars().next()?;
            let name = data[1..].split(quote).next()?;
            if !keep(name) {
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

/// Windows' XML for each event of `samples/windows/types.evtx`, by record ID.
fn windows_records() -> Vec<(u64, String)> {
    let jsonl = std::fs::read_to_string("samples/windows/types.evtx.jsonl").unwrap();
    jsonl
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .filter_map(|event| {
            Some((
                event["record_id"].as_u64()?,
                event["xml"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// evtx's XML for each record of `samples/windows/types.evtx`, or the error it failed with.
fn evtx_records() -> HashMap<u64, Result<String, String>> {
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
    evtx
}

/// Checks that each `Data` element of `samples/windows/types.evtx` that `keep` selects, by record
/// ID and name, renders as Windows renders it, and returns how many were compared.
fn compare_with_windows(keep: impl Fn(u64, &str) -> bool) -> usize {
    let evtx = evtx_records();
    let mut values = 0;
    for (record_id, xml) in &windows_records() {
        let keep = |name: &str| keep(*record_id, name);
        let expected = named_data(xml, &keep);
        if expected.is_empty() {
            continue;
        }
        let rendered = match &evtx[record_id] {
            Ok(rendered) => rendered,
            Err(err) => panic!("record {record_id} failed: {err}"),
        };
        assert_eq!(named_data(rendered, &keep), expected, "record {record_id}");
        values += expected.len();
    }
    values
}

#[test]
fn every_record_renders() {
    let evtx = evtx_records();
    let failed: Vec<_> = evtx.iter().filter(|(_, xml)| xml.is_err()).collect();
    assert!(failed.is_empty(), "{failed:?}");
    // One line per event, the two SID arrays Windows can't render (error 13) included.
    let jsonl = std::fs::read_to_string("samples/windows/types.evtx.jsonl").unwrap();
    assert_eq!(evtx.len(), jsonl.lines().count());
}

#[test]
fn timestamps_render_as_windows_renders_them() {
    let values = compare_with_windows(|_, name| name.ends_with("TIME") || name.ends_with("TIMEs"));
    // The FILETIME and SYSTEMTIME events, their array events, and the scalars event.
    assert!(values >= 30, "only {values} timestamps compared");
}

/// The value of the first attribute `name` (with its `=`) in `xml`, quoted either way.
fn attribute_value<'x>(xml: &'x str, name: &str) -> Option<&'x str> {
    let rest = &xml[xml.find(name)? + name.len()..];
    let quote = rest.chars().next()?;
    rest[1..].split(quote).next()
}

/// GUIDs render in braces and lowercase, as Windows renders them: event 1's values, event 26's
/// arrays (of three items, one and none), and each record's provider GUID and activity ID.
#[test]
fn guids_render_as_windows_renders_them() {
    assert_eq!(
        compare_with_windows(|_, name| name == "GUID" || name == "GUIDs"),
        8
    );
    let evtx = evtx_records();
    for (record_id, xml) in &windows_records() {
        let rendered = evtx[record_id].as_ref().unwrap();
        for name in ["Guid=", "ActivityID="] {
            assert!(attribute_value(xml, name).is_some(), "record {record_id}");
            assert_eq!(
                attribute_value(rendered, name),
                attribute_value(xml, name),
                "record {record_id}"
            );
        }
    }
}

#[test]
fn ansi_string_arrays_render_as_windows_renders_them() {
    // Arrays of three items (one of them empty), one item and none.
    assert_eq!(compare_with_windows(|_, name| name == "AnsiStrings"), 5);
}

/// Events 3 and 4 hold strings that are hard to render: controls, CR and LF, lone and reversed
/// surrogates, U+FFFE and U+FFFF, markup, a BOM, and counted strings with NULs. Characters XML
/// doesn't allow become `_`, as Windows writes them.
#[test]
fn text_renders_as_windows_renders_it() {
    assert_eq!(compare_with_windows(|_, name| name == "Text"), 26);
}

/// ANSI strings with C0 controls (event 5's record 37) get `_` for them too. The other ANSI
/// strings differ on purpose: evtx decodes windows-1252, and Windows widens each byte to the
/// code point of the same value.
#[test]
fn ansi_controls_render_as_windows_renders_them() {
    assert_eq!(
        compare_with_windows(|id, name| id == 37 && (name == "Ansi" || name == "Utf8")),
        2
    );
}

/// Event 11's UserData comes from its template: entity references, text the manifest compiler
/// made from character references, and a CDATA section. evtx renders it as Windows does, apart
/// from the double quotes evtx uses for attributes and a space Windows writes after the CDATA
/// section, which the event doesn't hold.
#[test]
fn user_data_renders_as_windows_renders_it() {
    let user_data = |xml: &str| {
        let start = xml.find("<UserData>").unwrap();
        let end = xml.find("</UserData>").unwrap();
        xml[start..end].to_owned()
    };
    let (record_id, windows) = windows_records()
        .into_iter()
        .find(|(_, xml)| xml.contains("<UserData>"))
        .unwrap();
    let windows = user_data(&windows)
        .replace(
            "xmlns='http://schemas.evtx.example/types'",
            "xmlns=\"http://schemas.evtx.example/types\"",
        )
        .replace("]]> </Cdata>", "]]></Cdata>");
    let evtx = evtx_records().remove(&record_id).unwrap().unwrap();
    assert_eq!(user_data(&evtx), windows);
}
