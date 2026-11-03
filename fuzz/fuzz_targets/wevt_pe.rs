#![no_main]

use evtx::wevt_templates::{WevtCache, extract_wevt_template_resources};
use libfuzzer_sys::fuzz_target;

// A PE file (a provider DLL or EXE), as `evtx_dump extract-wevt-templates` reads it: its resource
// directory, walked for `WEVT_TEMPLATE` resources, each then loaded into a cache.
fuzz_target!(|data: &[u8]| {
    let Ok(resources) = extract_wevt_template_resources(data) else {
        return;
    };
    let mut cache = WevtCache::new();
    for resource in resources {
        let _ = cache.add_wevt_blob(resource.data);
    }
});
