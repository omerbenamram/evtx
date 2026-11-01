#![no_main]

use evtx::mta::MtaFile;
use libfuzzer_sys::fuzz_target;

// A LocaleMetaData (`.MTA`) file.
fuzz_target!(|data: &[u8]| {
    let _ = MtaFile::from_bytes(data);
});
