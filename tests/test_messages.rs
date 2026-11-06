//! Event messages from provider binaries, checked against the messages Windows wrote to an
//! `.MTA` file for the same log.

#[cfg(feature = "wevt_templates")]
mod messages {
    use std::path::Path;
    use std::sync::Arc;

    use evtx::mta::MtaFile;
    use evtx::wevt_templates::messages::MessageCatalog;
    use evtx::{EvtxParser, ParserSettings};

    /// The `RenderingInfo` of each record of `samples/mta/<sample>.evtx`.
    fn sample_rendering_info(sample: &str, settings: ParserSettings) -> Vec<serde_json::Value> {
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/mta");
        EvtxParser::from_path(samples.join(format!("{sample}.evtx")))
            .unwrap()
            .with_configuration(settings)
            .records_json_value()
            .map(|record| record.unwrap().data["Event"]["RenderingInfo"].clone())
            .collect()
    }

    fn rendering_info(settings: ParserSettings) -> Vec<serde_json::Value> {
        sample_rendering_info("security-sample", settings)
    }

    /// The `RenderingInfo` that the `.MTA` file of `samples/mta/<sample>.evtx` gives its records.
    fn mta_rendering_info(sample: &str) -> Vec<serde_json::Value> {
        let mta = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("samples/mta/LocaleMetaData/{sample}_1033.MTA"));
        let mta = Arc::new(MtaFile::from_path(mta).unwrap());
        sample_rendering_info(sample, ParserSettings::new().mta(Some(mta)))
    }

    #[test]
    fn security_messages_match_windows() {
        let dlls = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dlls");
        let catalog = MessageCatalog::from_directory(&dlls, None).unwrap();
        let ours = rendering_info(ParserSettings::new().messages(Some(Arc::new(catalog))));
        let windows = mta_rendering_info("security-sample");

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

    /// The Service Control Manager's events, which have qualifiers, get no opcode, and classic
    /// event sources get the standard task and opcode names, as Windows gives them.
    #[test]
    fn names_match_windows() {
        let dlls = Path::new(env!("CARGO_MANIFEST_DIR")).join("samples/dlls");
        let mut catalog = MessageCatalog::from_directory(&dlls, None).unwrap();
        // Its message file is missing, as `MtaTestClassicMissing`'s is.
        catalog.add_classic_source("MtaTestClassic", &[]).unwrap();
        let catalog = Arc::new(catalog);
        for (sample, events) in [("system-sample", 6), ("classic", 3)] {
            let settings = ParserSettings::new().messages(Some(catalog.clone()));
            let ours = sample_rendering_info(sample, settings);
            let windows = mta_rendering_info(sample);
            let mut compared = 0;
            for (ours, windows) in ours.iter().zip(&windows) {
                if ours.is_null() {
                    continue;
                }
                for field in ["Level", "Task", "Opcode", "Keywords"] {
                    assert_eq!(ours[field], windows[field], "{sample}: {field}");
                }
                compared += 1;
            }
            assert_eq!(compared, events, "{sample}");
        }
    }
}
