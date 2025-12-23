use super::types::{WevtTempTemplateHeader, WevtTempTemplateRef};

/// Research-only parser for `TTBL`/`TEMP` structures within a `WEVT_TEMPLATE` resource blob.
///
/// Many real-world blobs contain multiple `TTBL` sections. This function finds all parseable
/// `TTBL` sections and returns references to all `TEMP` entries contained within them.
///
/// This uses the CRIM/WEVT provider element directory to locate `TTBL` elements, and then parses
/// the `TTBL`/`TEMP` structures.
pub fn extract_temp_templates_from_wevt_blob(
    blob: &[u8],
) -> Result<Vec<WevtTempTemplateRef>, super::manifest::WevtManifestError> {
    let mut out = Vec::new();

    let manifest = super::manifest::CrimManifest::parse(blob)?;

    for provider in &manifest.providers {
        let Some(ttbl) = provider.wevt.elements.templates.as_ref() else {
            continue;
        };
        for tpl in &ttbl.templates {
            out.push(WevtTempTemplateRef {
                ttbl_offset: ttbl.offset,
                temp_offset: tpl.offset,
                temp_size: tpl.size,
                header: WevtTempTemplateHeader {
                    item_descriptor_count: tpl.item_descriptor_count,
                    item_name_count: tpl.item_name_count,
                    template_items_offset: tpl.template_items_offset,
                    event_type: tpl.event_type,
                    guid: tpl.guid.clone(),
                },
            });
        }
    }

    Ok(out)
}


