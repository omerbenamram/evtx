#![no_main]

use evtx::wevt_templates::{WevtCache, extract_temp_templates_from_wevt_blob, render_temp_to_xml};
use libfuzzer_sys::fuzz_target;

// A `WEVT_TEMPLATE` resource (a CRIM manifest), as `--wevt-cache` loads it, with each template
// in it rendered.
fuzz_target!(|data: &[u8]| {
    let Ok(templates) = extract_temp_templates_from_wevt_blob(data) else {
        return;
    };
    for template in templates {
        let start = template.temp_offset as usize;
        let size = template.temp_size as usize;
        if let Some(temp) = data.get(start..).and_then(|rest| rest.get(..size)) {
            let _ = render_temp_to_xml(temp, encoding_rs::WINDOWS_1252);
        }
    }
    let _ = WevtCache::new().add_wevt_blob(data.to_vec());
});
