//! Checks evtx's XML against Windows' own rendering of the samples. For a selection of their
//! records, in `samples/windows/samples.jsonl` (see `samples/windows/README.md`), the two must be
//! the same XML. They are compared as trees, so quoting, entity forms and the XML declaration
//! don't count, and the only differences allowed are the ones `known` lists.
//!
//! To compare every record of every sample instead, set `EVTX_WINDOWS_XML` to the `samples/`
//! folder of the `windows-xml` workflow's artifact. With `EVTX_WINDOWS_XML_SAVE=1` as well, the
//! test writes a new `samples/windows/samples.jsonl` from it.

mod fixtures;

use evtx::err::EvtxError;
use evtx::{EvtxParser, ParserSettings};
use fixtures::samples_dir;
use roxmltree::{Document, Node};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// One of Windows' renderings: a sample, relative to `samples/`, a record ID and its XML.
struct WindowsRecord {
    sample: String,
    record_id: u64,
    xml: String,
}

/// The records in `samples/windows/samples.jsonl`, or every record Windows rendered in the
/// artifact folder `EVTX_WINDOWS_XML` names.
fn windows_records() -> Vec<WindowsRecord> {
    match std::env::var_os("EVTX_WINDOWS_XML") {
        Some(folder) => artifact_records(Path::new(&folder)),
        None => std::fs::read_to_string(samples_dir().join("windows/samples.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                let line: serde_json::Value = serde_json::from_str(line).unwrap();
                WindowsRecord {
                    sample: line["sample"].as_str().unwrap().to_owned(),
                    record_id: line["record_id"].as_u64().unwrap(),
                    xml: line["xml"].as_str().unwrap().to_owned(),
                }
            })
            .collect(),
    }
}

/// Every record Windows rendered in an artifact's `samples/` folder, which has a
/// `<sample>.jsonl` for each sample, in its folder under `samples/`.
fn artifact_records(folder: &Path) -> Vec<WindowsRecord> {
    let mut records = vec![];
    let mut folders = vec![folder.to_path_buf()];
    while let Some(dir) = folders.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            let relative = path.strip_prefix(folder).unwrap().to_string_lossy();
            let Some(sample) = relative.strip_suffix(".jsonl") else {
                continue;
            };
            let sample = sample.replace('\\', "/");
            if !samples_dir().join(&sample).is_file() {
                continue;
            }
            for line in std::fs::read_to_string(&path).unwrap().lines() {
                let line: serde_json::Value = serde_json::from_str(line).unwrap();
                // Windows can't read some samples at all (`query_error` instead of `xml`).
                let (Some(record_id), Some(xml)) =
                    (line["record_id"].as_u64(), line["xml"].as_str())
                else {
                    continue;
                };
                records.push(WindowsRecord {
                    sample: sample.clone(),
                    record_id,
                    xml: xml.to_owned(),
                });
            }
        }
    }
    records
}

/// evtx's XML for each record of `sample`, by record ID, or the error it failed with.
fn evtx_records(sample: &str) -> HashMap<u64, Result<String, String>> {
    let settings = ParserSettings::new().num_threads(1).indent(false);
    let mut parser = EvtxParser::from_path(samples_dir().join(sample))
        .unwrap()
        .with_configuration(settings);
    let mut records = HashMap::new();
    for record in parser.records() {
        match record {
            Ok(record) => {
                records
                    .entry(record.event_record_id)
                    .or_insert(Ok(record.data));
            }
            Err(EvtxError::FailedToParseRecord { record_id, source }) => {
                records.entry(record_id).or_insert(Err(source.to_string()));
            }
            // A chunk or a record header that can't be read: Windows can't read such logs.
            Err(_) => {}
        }
    }
    records
}

/// Where evtx's XML and Windows' differ: an element's text, one of its attributes (`path` ends
/// with `@name`) or its child elements (`/*`), with each side's value, `None` where missing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Difference {
    path: String,
    evtx: Option<String>,
    windows: Option<String>,
}

/// Why evtx renders `difference` differently from Windows, if it is one of the differences evtx
/// keeps on purpose.
fn known(difference: &Difference) -> Option<&'static str> {
    let (evtx, windows) = (difference.evtx.as_deref(), difference.windows.as_deref());
    match (evtx, windows) {
        // On purpose (#289): a boolean stored as neither 0 nor 1 renders as the stored number,
        // so that it isn't lost. Windows renders it `true`.
        (Some(evtx), Some("true")) if evtx.parse::<i64>().is_ok_and(|n| n != 0 && n != 1) => {
            Some(RAW_BOOLEAN)
        }
        // evtx leaves out an attribute whose value is empty. Windows does too when the value is
        // an empty substitution (`<Correlation/>`), but writes the attribute when its value is
        // an empty literal in the template (`EventSourceName=''` in two records of
        // 2-system-Security-dirty.evtx).
        (None, Some("")) if difference.path.contains('@') => Some(EMPTY_ATTRIBUTE),
        _ => None,
    }
}

const RAW_BOOLEAN: &str = "a boolean that is neither 0 nor 1 renders as stored";
const EMPTY_ATTRIBUTE: &str = "an attribute whose value is empty is left out";

/// The differences between evtx's element and Windows', in their names, attributes, text and
/// child elements, with `path` the path to their parent.
fn differences(evtx: Node, windows: Node, path: &str, found: &mut Vec<Difference>) {
    let path = format!("{path}/{}", windows.tag_name().name());
    let mut differ = |path: String, evtx: Option<&str>, windows: Option<&str>| {
        found.push(Difference {
            path,
            evtx: evtx.map(str::to_owned),
            windows: windows.map(str::to_owned),
        })
    };
    if evtx.tag_name() != windows.tag_name() {
        let name = |node: Node| format!("{:?}", node.tag_name());
        differ(path, Some(&name(evtx)), Some(&name(windows)));
        return;
    }
    let (ea, wa) = (attributes(evtx), attributes(windows));
    let names: BTreeSet<_> = ea.keys().chain(wa.keys()).collect();
    for name in names {
        let (e, w) = (ea.get(name), wa.get(name));
        if e != w {
            differ(
                format!("{path}@{name}"),
                e.map(|e| &e[..]),
                w.map(|w| &w[..]),
            );
        }
    }
    let (et, wt) = (text(evtx), text(windows));
    if et != wt {
        differ(path.clone(), Some(&et), Some(&wt));
    }
    let (ec, wc) = (child_elements(evtx), child_elements(windows));
    if ec.len() != wc.len() {
        let names = |nodes: &[Node]| {
            let names: Vec<_> = nodes.iter().map(|n| n.tag_name().name()).collect();
            names.join(",")
        };
        differ(format!("{path}/*"), Some(&names(&ec)), Some(&names(&wc)));
    }
    for (e, w) in ec.into_iter().zip(wc) {
        differences(e, w, &path, found);
    }
}

fn attributes(node: Node) -> BTreeMap<String, String> {
    node.attributes()
        .map(|a| (a.name().to_owned(), a.value().to_owned()))
        .collect()
}

/// An element's own text, with its CDATA.
fn text(node: Node) -> String {
    node.children()
        .filter(Node::is_text)
        .filter_map(|text| text.text())
        .collect()
}

fn child_elements<'a, 'input>(node: Node<'a, 'input>) -> Vec<Node<'a, 'input>> {
    node.children().filter(Node::is_element).collect()
}

/// The event a record is: its provider, event ID and version, as Windows renders them.
fn event_of(windows: &Document) -> [String; 3] {
    let system = windows
        .root_element()
        .children()
        .find(|n| n.has_tag_name("System"));
    let field = |name: &str| system.and_then(|s| s.children().find(|n| n.has_tag_name(name)));
    [
        field("Provider").and_then(|n| n.attribute("Name")),
        field("EventID").and_then(|n| n.text()),
        field("Version").and_then(|n| n.text()),
    ]
    .map(|field| field.unwrap_or_default().to_owned())
}

#[test]
fn records_render_as_windows_renders_them() {
    let windows = windows_records();
    let mut by_sample: BTreeMap<&str, Vec<&WindowsRecord>> = BTreeMap::new();
    for record in &windows {
        by_sample.entry(&record.sample).or_default().push(record);
    }

    let mut compared = 0;
    let mut failures = vec![];
    let mut seen = BTreeSet::new();
    // For `EVTX_WINDOWS_XML_SAVE`: the first record of each event in each sample, and of each
    // known difference.
    let mut selection = vec![];
    let mut selected = BTreeSet::new();
    for (sample, records) in by_sample {
        let evtx = evtx_records(sample);
        for record in records {
            let id = record.record_id;
            let rendered = match evtx.get(&id) {
                Some(Ok(rendered)) => rendered,
                Some(Err(error)) => {
                    failures.push(format!("{sample} record {id}: evtx failed: {error}"));
                    continue;
                }
                None => {
                    failures.push(format!("{sample} record {id}: not in evtx's output"));
                    continue;
                }
            };
            let evtx = match Document::parse(rendered) {
                Ok(evtx) => evtx,
                Err(error) => {
                    failures.push(format!("{sample} record {id}: evtx's XML: {error}"));
                    continue;
                }
            };
            let windows = Document::parse(&record.xml).unwrap();
            let mut found = vec![];
            differences(evtx.root_element(), windows.root_element(), "", &mut found);
            let mut keys = vec![format!("{:?}", event_of(&windows))];
            for difference in found {
                match known(&difference) {
                    Some(reason) => {
                        seen.insert(reason);
                        keys.push(format!("{difference:?}"));
                    }
                    None => failures.push(format!("{sample} record {id}: {difference:?}")),
                }
            }
            let mut new = false;
            for key in keys {
                new |= selected.insert((sample, key));
            }
            if new {
                selection.push(record);
            }
            compared += 1;
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences in {compared} records:\n{}",
        failures.len(),
        failures.join("\n")
    );
    // A kept difference that no longer happens: take it out of `known`.
    for reason in [RAW_BOOLEAN, EMPTY_ATTRIBUTE] {
        assert!(seen.contains(reason), "no longer seen: {reason}");
    }
    assert!(compared >= 600, "only {compared} records compared");

    if std::env::var_os("EVTX_WINDOWS_XML").is_some()
        && std::env::var_os("EVTX_WINDOWS_XML_SAVE").is_some()
    {
        let mut jsonl = String::new();
        for record in selection {
            let line = serde_json::json!({
                "sample": record.sample,
                "record_id": record.record_id,
                "xml": record.xml,
            });
            jsonl.push_str(&line.to_string());
            jsonl.push('\n');
        }
        std::fs::write(samples_dir().join("windows/samples.jsonl"), jsonl).unwrap();
    }
}
