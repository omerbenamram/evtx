//! Checks `evtx::mta` against Windows. `samples/mta/windows/*.jsonl` holds what wevtapi returned
//! for every event of the archived logs in `samples/mta` (see `samples/mta/README.md`).

mod fixtures;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use evtx::mta::{MtaEvent, MtaFile, RenderingInfo};
use evtx::{EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;
use serde::Deserialize;
use serde_json::Value;

fn mta_dir() -> PathBuf {
    samples_dir().join("mta")
}

fn mta_path(log: &str, lcid: u32) -> PathBuf {
    mta_dir()
        .join("LocaleMetaData")
        .join(format!("{log}_{lcid}.MTA"))
}

fn load_mta(log: &str, lcid: u32) -> Arc<MtaFile> {
    Arc::new(MtaFile::from_path(mta_path(log, lcid)).expect("fixture parses"))
}

/// One line of `samples/mta/windows/*.jsonl`: what `scripts/mta/MtaProbe.cs` got from wevtapi.
/// Strings keep the NULs Windows returned; `*_error` is the Win32 error, 0 on success.
#[derive(Debug, Deserialize)]
struct WindowsEvent {
    index: usize,
    provider: String,
    event_record_id: String,
    qualifiers: Option<String>,
    task: Option<String>,
    /// Set when Windows found no metadata for the publisher, which then has no strings.
    metadata_error: u32,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    message_error: u32,
    #[serde(default)]
    level_name: Option<String>,
    #[serde(default)]
    task_name: Option<String>,
    #[serde(default)]
    opcode_name: Option<String>,
    /// NUL-separated.
    #[serde(default)]
    keyword_names: Option<String>,
}

/// An event's strings, from Windows or from our `RenderingInfo`.
#[derive(Debug, Clone, Default, PartialEq)]
struct Strings {
    message: Option<String>,
    level: Option<String>,
    task: Option<String>,
    opcode: Option<String>,
    keywords: Vec<String>,
}

impl WindowsEvent {
    fn strings(&self) -> Option<Strings> {
        if self.metadata_error != 0 {
            return None;
        }
        let name = |s: &Option<String>| {
            Some(s.as_deref()?.trim_end_matches('\0'))
                .filter(|s| !s.is_empty())
                .map(String::from)
        };
        Some(Strings {
            message: name(&self.message).filter(|_| self.message_error == 0),
            level: name(&self.level_name),
            task: name(&self.task_name),
            opcode: name(&self.opcode_name),
            keywords: (self.keyword_names.as_deref().unwrap_or_default())
                .split('\0')
                .filter(|name| !name.is_empty())
                .map(String::from)
                .collect(),
        })
    }

    fn number(value: &Option<String>) -> u64 {
        value.as_deref().map_or(0, |v| v.parse().unwrap())
    }
}

fn rendered_strings(event: &Value) -> Option<Strings> {
    let info = event["Event"].get("RenderingInfo")?;
    let text = |key| info.get(key).map(|v| v.as_str().unwrap().to_string());
    // Repeated `Keyword` elements become `Keyword`, `Keyword_1`, ...
    let keywords = info.get("Keywords").map_or(Vec::new(), |keywords| {
        let keywords = keywords.as_object().unwrap().values();
        keywords.map(|v| v.as_str().unwrap().to_string()).collect()
    });
    Some(Strings {
        message: text("Message"),
        level: text("Level"),
        task: text("Task"),
        opcode: text("Opcode"),
        keywords,
    })
}

fn windows_events(path: &Path) -> Vec<WindowsEvent> {
    let text = std::fs::read_to_string(path).unwrap();
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// Every record of `evtx` as JSON, rendered with `mta` through the record API and through the
/// chunk renderer that `evtx_dump` uses, which must agree.
fn render(evtx: &Path, mta: Arc<MtaFile>) -> Vec<Value> {
    let settings = ParserSettings::new().indent(false).mta(Some(mta));
    let mut parser = EvtxParser::from_path(evtx)
        .unwrap()
        .with_configuration(settings);
    let records: Vec<Value> = parser
        .records_json_value()
        .map(|record| record.unwrap().data)
        .collect();
    let mut chunked = Vec::new();
    for chunk in parser.chunks_json_bytes(false) {
        for item in &chunk.items {
            assert!(matches!(item, RenderedChunkItem::Record { .. }), "{item:?}");
        }
        for line in chunk.data.split(|&b| b == b'\n').filter(|l| !l.is_empty()) {
            chunked.push(serde_json::from_slice::<Value>(line).unwrap());
        }
    }
    assert_eq!(records, chunked, "{}", evtx.display());
    records
}

#[test]
fn rendering_matches_windows() {
    let mut events = 0;
    let mut opcode_fallbacks = 0;
    let mut merged_messages = 0;
    let mut files: Vec<_> = std::fs::read_dir(mta_dir().join("windows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    for path in files {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let (log, lcid) = stem.rsplit_once('_').unwrap();
        let windows = windows_events(&path);
        let ours = render(
            &mta_dir().join(format!("{log}.evtx")),
            load_mta(log, lcid.parse().unwrap()),
        );
        assert_eq!(ours.len(), windows.len(), "{stem}: event count");
        let mut seen_record_ids = HashSet::new();
        for (position, (event, windows)) in ours.iter().zip(&windows).enumerate() {
            let context = format!(
                "{stem}, event {position} ({} {})",
                windows.provider, windows.event_record_id
            );
            assert_eq!(windows.index, position, "{context}");
            let event_record_id = &event["Event"]["System"]["EventRecordID"];
            assert_eq!(event_record_id.to_string(), windows.event_record_id);
            let first_with_record_id = seen_record_ids.insert(event_record_id.as_u64().unwrap());
            events += 1;

            let (Some(mut expected), Some(ours)) = (windows.strings(), rendered_strings(event))
            else {
                assert_eq!(
                    rendered_strings(event),
                    windows.strings(),
                    "{context}: only one side found the publisher"
                );
                continue;
            };
            // Where we deliberately differ from Windows (see `MtaFile::rendering_info`). The
            // strings themselves are checked by the tests below.
            let task = WindowsEvent::number(&windows.task);
            let qualifiers = WindowsEvent::number(&windows.qualifiers);
            if expected.opcode.is_none() && ours.opcode.is_some() && task != 0 && qualifiers == 0 {
                expected.opcode.clone_from(&ours.opcode);
                opcode_fallbacks += 1;
            }
            if !first_with_record_id && expected.message != ours.message {
                expected.message.clone_from(&ours.message);
                merged_messages += 1;
            }
            assert_eq!(ours, expected, "{context}");
        }
    }
    assert_eq!(events, 869);
    assert_eq!(opcode_fallbacks, 726);
    assert_eq!(merged_messages, 5);
}

/// Opcode names Windows leaves empty because the archive has them only as opcodes of no task.
#[test]
fn opcode_falls_back_to_the_name_outside_the_task() {
    let opcode = |log: &str, record: usize| {
        let events = render(&mta_dir().join(format!("{log}.evtx")), load_mta(log, 1033));
        let info = &events[record]["Event"]["RenderingInfo"];
        info["Opcode"].as_str().map(String::from)
    };
    // 4624, Logon task, opcode 0: the default publisher's `Info`, as live Windows shows it.
    assert_eq!(opcode("security-sample", 0).as_deref(), Some("Info"));
    // MtaTest-Provider: `win:Start` in the Connect task, and a global opcode in task 300.
    assert_eq!(opcode("custom", 0).as_deref(), Some("Start"));
    assert_eq!(opcode("custom", 127).as_deref(), Some("Global Opcode"));
    // The task-specific name still wins in its own task.
    assert_eq!(
        opcode("custom", 125).as_deref(),
        Some("Handshake (task-specific)")
    );
}

/// `custom-merged` exports two channels whose record IDs 1-5 overlap. Windows gives the Admin
/// events the messages of the Operational events with the same IDs.
#[test]
fn merged_log_messages_follow_record_position() {
    let events = render(
        &mta_dir().join("custom-merged.evtx"),
        load_mta("custom-merged", 1033),
    );
    let admin: Vec<_> = events
        .iter()
        .filter(|event| event["Event"]["System"]["Channel"] == "MtaTest-Provider/Admin")
        .map(|event| {
            let system = &event["Event"]["System"];
            let message = event["Event"]["RenderingInfo"]["Message"].as_str().unwrap();
            (
                system["EventRecordID"].as_u64().unwrap(),
                message.to_string(),
            )
        })
        .collect();
    let expected: Vec<_> = (1..=5)
        .map(|n| (n, format!("Admin channel error: admin error {n}")))
        .collect();
    assert_eq!(admin, expected);
}

/// dev-joss's export from PR 284, with Event Viewer's CSV of the same events.
#[test]
fn messages_match_event_viewer_csv() {
    let csv = std::fs::read_to_string(mta_dir().join("setup-export.csv")).unwrap();
    // Columns: level, date and time, source, event ID, task category, message. The CSV is
    // newest first.
    let mut expected: Vec<_> = csv
        .lines()
        .skip(1)
        .map(|row| {
            let fields: Vec<_> = row.splitn(6, ',').collect();
            (
                fields[3].parse::<u64>().unwrap(),
                fields[5].trim().to_string(),
            )
        })
        .collect();
    expected.reverse();
    let events = render(
        &mta_dir().join("setup-export.evtx"),
        load_mta("setup-export", 1033),
    );
    let ours: Vec<_> = events
        .iter()
        .map(|event| {
            let event_id = event["Event"]["System"]["EventID"].as_u64().unwrap();
            let message = event["Event"]["RenderingInfo"]["Message"].as_str().unwrap();
            (event_id, message.trim().to_string())
        })
        .collect();
    assert_eq!(ours.len(), 239);
    assert_eq!(ours, expected);
}

/// Every record of `evtx`, as JSON or XML.
fn records(evtx: &Path, settings: ParserSettings, json: bool) -> Vec<String> {
    let mut parser = EvtxParser::from_path(evtx)
        .unwrap()
        .with_configuration(settings);
    let records: Box<dyn Iterator<Item = _>> = if json {
        Box::new(parser.records_json())
    } else {
        Box::new(parser.records())
    };
    records.map(|record| record.unwrap().data).collect()
}

/// Forwarded events already carry Windows' own `RenderingInfo`, which is kept as it is.
#[test]
fn existing_rendering_info_is_kept() {
    let path = samples_dir().join("Archive-ForwardedEvents-test.evtx");
    let plain = records(&path, ParserSettings::new(), false);
    let mta = load_mta("security-sample", 1033);
    let with_mta = records(&path, ParserSettings::new().mta(Some(mta)), false);
    assert!(plain[0].contains("<RenderingInfo"));
    assert_eq!(plain, with_mta);
}

#[test]
fn records_from_publishers_missing_from_the_archive_are_unchanged() {
    // The runner had no Sysmon publisher, so the archive has no strings for these events.
    let path = mta_dir().join("sysmon-sample.evtx");
    let mta = load_mta("sysmon-sample", 1033);
    let with_mta = records(&path, ParserSettings::new().mta(Some(mta)), true);
    assert_eq!(with_mta.len(), 20);
    assert_eq!(records(&path, ParserSettings::new(), true), with_mta);
}

#[test]
fn lookup_api() {
    let mta = load_mta("custom", 1031);
    let event = MtaEvent {
        provider: "mtatest-provider",
        record_number: 126,
        event_record_id: 126,
        level: 3,
        task: 1,
        opcode: 10,
        keywords: 0x8000_0000_0000_0005,
        ..MtaEvent::default()
    };
    // The runner had no German winmeta strings, so standard names stay English.
    assert_eq!(
        mta.rendering_info(&event),
        Some(RenderingInfo {
            message: Some("Handshake <alpha & <beta>> & „Anführungszeichen“ Größe ÄÖÜ äöü ß"),
            level: Some("Warning"),
            task: Some("Verbinden"),
            opcode: Some("Handshake (aufgabenspezifisch)"),
            keywords: vec!["Netzwerk", "Sicherheit"],
        })
    );
    let publisher = mta.publisher("MtaTest-Provider").unwrap();
    assert_eq!(publisher.name(), "MtaTest-Provider");
    assert_eq!(publisher.level(16), Some("Schwerwiegend (eigene Stufe)"));
    assert_eq!(publisher.task(300), Some("Große Aufgabe 300"));
    assert_eq!(
        publisher.opcode(10, 1),
        Some("Handshake (aufgabenspezifisch)")
    );
    assert_eq!(publisher.opcode(10, 0), None);
    assert_eq!(publisher.opcode(11, 0), Some("Globaler Opcode"));
    assert_eq!(publisher.keyword(47), Some("Hohes Bit 47"));
    assert_eq!(mta.event_count(), 137);
    let names: Vec<_> = mta.publishers().map(|p| p.name()).collect();
    assert_eq!(names, ["DEFAULT_PUBLISHER", "MtaTest-Provider"]);
    assert!(mta.rendering_info(&MtaEvent::default()).is_none());
}

#[test]
fn rendered_xml_and_json() {
    let parser = || {
        let settings = ParserSettings::new().mta(Some(load_mta("custom", 1033)));
        EvtxParser::from_path(mta_dir().join("custom.evtx"))
            .unwrap()
            .with_configuration(settings)
    };
    let xml = parser().records().nth(125).unwrap().unwrap().data;
    insta::assert_snapshot!("mta_xml", xml);
    let json = parser()
        .records_json_value()
        .nth(125)
        .unwrap()
        .unwrap()
        .data;
    insta::assert_json_snapshot!("mta_json", json);
}

#[test]
fn cli_mta_flag() {
    let _lock = CLI_TEST_LOCK.lock().unwrap();
    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["-o", "jsonl", "--mta"])
        .arg(mta_path("classic", 1033))
        .arg(mta_dir().join("classic.evtx"))
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8(output.stdout).unwrap();
    let first: Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(
        first["Event"]["RenderingInfo"]["Message"],
        "Classic information message."
    );

    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .arg("--mta")
        .arg(mta_dir().join("classic.evtx"))
        .arg(mta_dir().join("classic.evtx"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("not an MTA file"), "{stderr}");
}
