mod fixtures;

use evtx::{EvtxParser, MtaFile};
use fixtures::*;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone)]
struct CsvRow {
    time: String,
    source: String,
    event_id: u32,
    message: String,
}

fn load_csv_rows(path: impl AsRef<Path>) -> Vec<CsvRow> {
    let contents = std::fs::read_to_string(path).expect("failed to read MTA csv");
    let mut rows = Vec::new();

    for (line_no, line) in contents.lines().enumerate() {
        if line_no == 0 {
            continue;
        }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }

        let mut parts = line.splitn(6, ',');
        let _level = parts.next();
        let time = parts
            .next()
            .expect("missing csv time")
            .trim();
        let source = parts
            .next()
            .expect("missing csv source")
            .trim();
        let event_id = parts
            .next()
            .and_then(|value| value.trim().parse::<u32>().ok())
            .expect("missing csv event id");
        let _task_category = parts.next();
        let message = parts.next().unwrap_or("").trim();

        rows.push(CsvRow {
            time: time.to_string(),
            source: source.to_string(),
            event_id,
            message: message.to_string(),
        });
    }

    rows
}


#[test]
fn test_mta_localized_messages_match_csv() {
    ensure_env_logger_initialized();

    let mta = Arc::new(MtaFile::from_path(mta_test_mta()).expect("failed to load MTA file"));
    let mut csv_rows = load_csv_rows(mta_test_csv());
    // CSV is newest-to-oldest; EVTX iterates oldest-to-newest.
    csv_rows.reverse();

    let mut row_index = 0usize;

    let mut parser = EvtxParser::from_path(mta_test_evtx()).expect("failed to open MTA evtx");

    for chunk in parser.chunks() {
        let mut chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => continue,
        };
        let settings = Arc::new(Default::default());
        let mut chunk = match chunk.parse(Arc::clone(&settings)) {
            Ok(chunk) => chunk,
            Err(_) => continue,
        };

        for record in chunk.iter() {
            let record = match record {
                Ok(record) => record,
                Err(_) => continue,
            };

            let Some(row) = csv_rows.get(row_index) else {
                panic!("found more records than csv rows (record_id={})", record.event_record_id);
            };
            let message = mta
                .message_for_evtx_record(&record)
                .unwrap_or("")
                .trim()
                .to_string();
            let expected = row.message.trim();
            if message != expected {
                panic!(
                    "message mismatch at row {} (record_id={} event_id={} source={} time={}): expected={} got={}",
                    row_index + 1,
                    record.event_record_id,
                    row.event_id,
                    row.source,
                    row.time,
                    expected,
                    message
                );
            }
            row_index += 1;
        }
    }

    if row_index != csv_rows.len() {
        panic!(
            "csv rows were not matched (matched={}, total={})",
            row_index,
            csv_rows.len()
        );
    }
}
