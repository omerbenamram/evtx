#![allow(dead_code)]
use std::path::PathBuf;

use std::sync::Once;

static LOGGER_INIT: Once = Once::new();

// Rust runs the tests concurrently, so unless we synchronize logging access
// it will crash when attempting to run `cargo test` with some logging facilities.
#[cfg(test)]
pub fn ensure_env_logger_initialized() {
    use std::io::Write;

    LOGGER_INIT.call_once(|| {
        let mut builder = env_logger::Builder::from_default_env();
        builder
            .format(|buf, record| writeln!(buf, "[{}] - {}", record.level(), record.args()))
            .init();
    });
}

pub fn samples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("samples")
        .canonicalize()
        .unwrap()
}

pub fn regular_sample() -> PathBuf {
    samples_dir().join("security.evtx")
}

pub fn sample_with_irregular_values() -> PathBuf {
    samples_dir().join("sample-with-irregular-bool-values.evtx")
}

pub fn sample_with_a_bad_checksum() -> PathBuf {
    samples_dir()
        .join("2-vss_0-Microsoft-Windows-RemoteDesktopServices-RdpCoreTS%4Operational.evtx")
}

pub fn sample_with_a_bad_checksum_2() -> PathBuf {
    samples_dir().join(
        "2-vss_0-Microsoft-Windows-TerminalServices-RemoteConnectionManager%4Operational.evtx",
    )
}

pub fn sample_with_a_chunk_past_zeroes() -> PathBuf {
    samples_dir().join("2-vss_7-System.evtx")
}

pub fn sample_with_a_bad_chunk_magic() -> PathBuf {
    samples_dir().join("sample_with_a_bad_chunk_magic.evtx")
}

pub fn sample_binxml_with_incomplete_sid() -> PathBuf {
    samples_dir().join("Microsoft-Windows-HelloForBusiness%4Operational.evtx")
}

pub fn sample_binxml_with_incomplete_template() -> PathBuf {
    samples_dir().join("Microsoft-Windows-LanguagePackSetup%4Operational.evtx")
}

pub fn sample_with_multiple_xml_fragments() -> PathBuf {
    samples_dir()
        .join("E_Windows_system32_winevt_logs_Microsoft-Windows-Shell-Core%4Operational.evtx")
}

pub fn sample_with_binxml_as_substitution_tokens_and_pi_target() -> PathBuf {
    samples_dir().join("E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx")
}

pub fn sample_issue_65() -> PathBuf {
    samples_dir().join(
        "E_ShadowCopy6_windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx",
    )
}

pub fn sample_with_dependency_id_edge_case() -> PathBuf {
    samples_dir().join("Archive-ForwardedEvents-test.evtx")
}

pub fn sample_with_no_crc32() -> PathBuf {
    samples_dir().join("Application_no_crc32.evtx")
}

pub fn sample_with_invalid_flags_in_header() -> PathBuf {
    samples_dir().join("post-Security.evtx")
}
