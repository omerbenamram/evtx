mod fixtures;

use evtx::err::{DeserializationError, EvtxError};
use evtx::{EvtxChunkData, EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;
use std::path::Path;
use std::sync::Arc;

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

#[test]
fn resyncs_on_records_the_chunk_header_describes() {
    // Chunks 0 and 13 end with a 16 KiB block from the log's next pass over them. Past the
    // corrupt record where that block starts, its records have IDs outside the chunk header's
    // range, so they are stale.
    let path = sample_with_a_bad_checksum();
    assert_eq!(counts(&path, false), [(1910, 0); 2]);
    assert_eq!(counts(&path, true), [(1910, 56); 2]);
}

#[test]
fn resyncs_past_zeroed_records() {
    // A zeroed 16 KiB block inside chunk 7's live area took records 47641 to 47674; the walk
    // picks up again at 47675.
    let path = sample_with_a_bad_chunk_magic();
    assert_eq!(counts(&path, false), [(301, 0); 2]);
    assert_eq!(counts(&path, true), [(301, 50); 2]);
}

/// security.evtx with its second record's header changed by `damage`, and where that record
/// starts in the file.
fn with_a_damaged_second_record(damage: impl FnOnce(&mut [u8])) -> (Vec<u8>, usize) {
    let mut evtx = std::fs::read(regular_sample()).unwrap();
    let first = 4096 + 512;
    let size = u32::from_le_bytes(evtx[first + 4..first + 8].try_into().unwrap());
    let second = first + size as usize;
    damage(&mut evtx[second..second + 24]);
    (evtx, second)
}

#[test]
fn record_errors_say_where_the_record_is() {
    let (evtx, in_file) = with_a_damaged_second_record(|header| header[..4].fill(0xff));
    let offset = (in_file - 4096) as u64;

    let mut parser = EvtxParser::from_buffer(evtx).unwrap();
    let errors: Vec<_> = parser.records().filter_map(Result::err).collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    let EvtxError::FailedToReadRecordAt {
        chunk: Some(0),
        offset: at,
        source,
    } = &errors[0]
    else {
        panic!("{:?}", errors[0]);
    };
    assert_eq!(*at, offset);
    assert!(matches!(
        **source,
        EvtxError::DeserializationError(DeserializationError::InvalidEvtxRecordHeaderMagic { .. })
    ));
    assert_eq!(
        errors[0].to_string(),
        format!(
            "Failed to read the record at offset {offset:#x} of chunk 0 (file offset {in_file:#x})"
        )
    );

    // A chunk that the parser read knows its number when it is parsed on its own.
    let mut data = parser.chunks().next().unwrap().unwrap();
    assert_eq!(data.number, Some(0));
    let mut chunk = data.parse(Arc::new(ParserSettings::new())).unwrap();
    let error = chunk.iter().find_map(Result::err).unwrap();
    assert_eq!(error.to_string(), errors[0].to_string());
    drop(chunk);

    // A chunk built from its bytes doesn't.
    let mut data = EvtxChunkData::new(data.data, false).unwrap();
    assert_eq!(data.number, None);
    let mut chunk = data.parse(Arc::new(ParserSettings::new())).unwrap();
    let error = chunk.iter().find_map(Result::err).unwrap();
    assert_eq!(
        error.to_string(),
        format!("Failed to read the record at offset {offset:#x} of its chunk")
    );
}

#[test]
fn record_timestamp_errors_name_the_field() {
    let (evtx, _) =
        with_a_damaged_second_record(|header| header[16..24].copy_from_slice(&[0xff; 8]));
    let mut parser = EvtxParser::from_buffer(evtx).unwrap();
    let error = parser.records().find_map(Result::err).unwrap();
    let EvtxError::FailedToReadRecordAt { source, .. } = &error else {
        panic!("{error:?}");
    };
    let EvtxError::DeserializationError(
        error @ DeserializationError::InvalidRecordTimestamp { filetime: u64::MAX },
    ) = &**source
    else {
        panic!("{source:?}");
    };
    assert_eq!(
        error.to_string(),
        "The record header's timestamp (FILETIME 0xffffffffffffffff) is out of range"
    );
}

#[test]
fn record_errors_name_the_chunk_in_the_file() {
    // The walk starts at the chunk with the oldest records, but errors name chunks by their
    // place in the file: the zeroed block is in chunk 7, at file offset 0x73060.
    for threads in [1, 4] {
        let settings = ParserSettings::new().num_threads(threads);
        let mut parser = EvtxParser::from_path(sample_with_a_bad_chunk_magic())
            .unwrap()
            .with_configuration(settings);
        let errors: Vec<_> = parser
            .records()
            .filter_map(Result::err)
            .map(|error| error.to_string())
            .filter(|error| error.starts_with("Failed to read the record"))
            .collect();
        assert_eq!(
            errors,
            ["Failed to read the record at offset 0x2060 of chunk 7 (file offset 0x73060)"]
        );
    }

    // So do the chunks that `chunks()` returns, in record order, each parsed on its own.
    let mut parser = EvtxParser::from_path(sample_with_a_bad_chunk_magic()).unwrap();
    let settings = Arc::new(ParserSettings::new());
    let mut errors = vec![];
    for data in parser.chunks() {
        let Ok(mut data) = data else { continue };
        let Ok(mut chunk) = data.parse(Arc::clone(&settings)) else {
            continue;
        };
        errors.extend(
            chunk
                .iter()
                .filter_map(Result::err)
                .map(|error| error.to_string())
                .filter(|error| error.starts_with("Failed to read the record")),
        );
    }
    assert_eq!(
        errors,
        ["Failed to read the record at offset 0x2060 of chunk 7 (file offset 0x73060)"]
    );
}
