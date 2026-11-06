//! Event messages from provider binaries, checked against the messages Windows wrote to an
//! `.MTA` file for the same log.

#[cfg(feature = "wevt_templates")]
mod messages {
    use std::path::Path;
    use std::sync::Arc;

    use evtx::mta::MtaFile;
    use evtx::wevt_templates::messages::MessageCatalog;
    use evtx::{EvtxParser, ParserSettings};

    fn rendering_info(settings: ParserSettings) -> Vec<serde_json::Value> {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/mta");
        EvtxParser::from_path(samples.join("security-sample.evtx"))
            .unwrap()
            .with_configuration(settings)
            .records_json_value()
            .map(|record| record.unwrap().data["Event"]["RenderingInfo"].clone())
            .collect()
    }

    #[test]
    fn security_messages_match_windows() {
        let dlls = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dlls");
        let catalog = MessageCatalog::from_directory(&dlls, None).unwrap();
        let ours = rendering_info(ParserSettings::new().messages(Some(Arc::new(catalog))));

        let mta = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/mta/LocaleMetaData/security-sample_1033.MTA");
        let windows = rendering_info(
            ParserSettings::new().mta(Some(Arc::new(MtaFile::from_path(mta).unwrap()))),
        );

        let mut same = 0;
        let mut differ = Vec::new();
        for (ours, windows) in ours.iter().zip(&windows) {
            for field in ["Level", "Task", "Opcode", "Keywords"] {
                if !ours[field].is_null() {
                    assert_eq!(ours[field], windows[field], "{field}");
                }
            }
            match ours["Message"].as_str() {
                Some(message) if Some(message) == windows["Message"].as_str() => same += 1,
                Some(message) => differ.push(message.to_string()),
                None => {}
            }
        }
        assert_eq!(same, 56);
        // The others name `%%1793` ("<value not set>"), whose string is in `msobjs.dll`, which
        // the samples don't have.
        assert_eq!(differ.len(), 3);
        assert!(differ.iter().all(|m| m.contains("%%1793")));
    }

    #[test]
    fn mta_strings_win() {
        let dlls = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dlls");
        let mta = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples/mta/LocaleMetaData/security-sample_1033.MTA");
        let mta = Arc::new(MtaFile::from_path(mta).unwrap());
        let both = rendering_info(ParserSettings::new().mta(Some(mta.clone())).messages(Some(
            Arc::new(MessageCatalog::from_directory(&dlls, None).unwrap()),
        )));
        assert_eq!(both, rendering_info(ParserSettings::new().mta(Some(mta))));
    }
}
