use crate::{ensure_env_logger_initialized, EvtxParser, ParserSettings};

/// Tests an .evtx file, asserting the number of parsed records matches `count`.
macro_rules! test_full_sample {
    ($path: expr, $count: expr) => {{
        use log::Level;

        ensure_env_logger_initialized();
        let evtx_file = include_bytes!($path);

        let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

        let mut count = 0;

        for r in parser.records() {
            if r.is_ok() {
                count += 1;
                if log::log_enabled!(Level::Debug) {
                    println!("{}", r.unwrap().data);
                }
            }
        }
        assert_eq!(count, $count);
    };};
}

#[test]
// https://github.com/omerbenamram/evtx/issues/10
fn test_dirty_sample_single_threaded() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    let mut count = 0;
    for r in parser.records() {
        r.unwrap();
        count += 1;
    }
    assert_eq!(count, 14621, "Single threaded iteration failed");
}

#[test]
fn test_dirty_sample_parallel() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-system-Security-dirty.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(8));

    let mut count = 0;

    for r in parser.records() {
        r.unwrap();
        count += 1;
    }

    assert_eq!(count, 14621, "Parallel iteration failed");
}

#[test]
fn test_parses_sample_with_irregular_boolean_values() {
    ensure_env_logger_initialized();
    // This sample contains boolean values which are not zero or one.
    let evtx_file = include_bytes!("../../samples/sample-with-irregular-bool-values.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    for r in parser.records() {
        r.unwrap();
    }
}

#[test]
fn test_dirty_sample_with_a_bad_checksum() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../../samples/2-vss_0-Microsoft-Windows-RemoteDesktopServices-RdpCoreTS%4Operational.evtx"
    );

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    let mut count = 0;

    for r in parser.records() {
        if r.is_ok() {
            count += 1;
        }
    }

    assert_eq!(count, 1910);
}

#[test]
fn test_dirty_sample_with_a_bad_checksum_2() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../../samples/2-vss_0-Microsoft-Windows-TerminalServices-RemoteConnectionManager%4Operational.evtx");

    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec()).unwrap();

    let mut count = 0;

    for r in parser.records() {
        if r.is_ok() {
            count += 1;
        }
    }

    assert_eq!(count, 1774);
}

#[test]
fn test_dirty_sample_with_a_chunk_past_zeros() {
    test_full_sample!("../../samples/2-vss_7-System.evtx", 1170)
}

#[test]
fn test_dirty_sample_with_a_bad_chunk_magic() {
    test_full_sample!(
        "../../samples/2-vss_7-Microsoft-Windows-AppXDeployment%4Operational.evtx",
        270
    )
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_token() {
    test_full_sample!(
        "../../samples/Microsoft-Windows-HelloForBusiness%4Operational.evtx",
        6
    )
}

#[test]
fn test_dirty_sample_binxml_with_incomplete_template() {
    test_full_sample!(
        "../../samples/Microsoft-Windows-LanguagePackSetup%4Operational.evtx",
        17
    )
}
