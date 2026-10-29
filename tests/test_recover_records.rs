mod fixtures;

use evtx::{EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;
use std::path::Path;

/// (live, recovered) counts via the record iterator and the CLI's chunk renderer.
fn counts(path: impl AsRef<Path>, recover: bool) -> [(usize, usize); 2] {
    let settings = ParserSettings::new().recover_records(recover);
    let mut parser = EvtxParser::from_path(path)
        .unwrap()
        .with_configuration(settings);

    let records: Vec<_> = parser.records_json().filter_map(Result::ok).collect();
    let recovered = records.iter().filter(|r| r.recovered).count();

    let (mut ok, mut banners) = (0, 0);
    for chunk in parser.chunks_json_bytes(true) {
        let text = String::from_utf8(chunk.data).unwrap();
        banners += text.lines().filter(|l| l.ends_with("(recovered)")).count();
        ok += chunk
            .items
            .iter()
            .filter(|i| matches!(i, RenderedChunkItem::Record { .. }))
            .count();
    }

    [
        (records.len() - recovered, recovered),
        (ok - banners, banners),
    ]
}

#[test]
fn recovers_records_from_cleared_chunk() {
    // Chunk header reset (`free_space_offset` 512), old records still in the data area.
    let path = samples_dir().join("Microsoft-Windows-WorkFolders%4WHC.evtx");
    assert_eq!(counts(&path, false), [(0, 0); 2]);
    assert_eq!(counts(&path, true), [(0, 2); 2]);

    let mut parser = EvtxParser::from_path(&path)
        .unwrap()
        .with_configuration(ParserSettings::new().recover_records(true));
    let ids: Vec<_> = parser
        .records_json_value()
        .map(|r| r.unwrap())
        .map(|r| {
            (
                r.event_record_id,
                r.data["Event"]["System"]["EventID"].clone(),
            )
        })
        .collect();
    insta::assert_debug_snapshot!(ids);
}

#[test]
fn recovers_records_from_chunk_slack() {
    // Has corrupt live records: the walk resyncs past them and the 21 records it finds
    // before `free_space_offset` stay live, with or without recovery.
    let path = sample_with_a_bad_checksum_2();
    assert_eq!(counts(&path, false), [(1795, 0); 2]);
    assert_eq!(counts(&path, true), [(1795, 245); 2]);
}
