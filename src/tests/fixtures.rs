use std::path::PathBuf;

pub fn samples_dir() -> PathBuf {
    PathBuf::from(file!())
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("samples")
        .canonicalize()
        .unwrap()
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
