use crate::SerializedEvtxRecord;

use crate::model::deserialized::BinXMLDeserializedTokens;

#[derive(Debug, Clone)]
struct TemplateInstanceInfo {
    /// Normalized GUID (lowercased, braces stripped) if we can resolve it.
    guid: Option<String>,
    substitutions: Vec<String>,
}

fn extract_template_guid_from_error(err: &crate::err::EvtxError) -> Option<String> {
    use crate::err::{DeserializationError, EvtxError};
    match err {
        EvtxError::FailedToParseRecord { source, .. } => extract_template_guid_from_error(source),
        EvtxError::DeserializationError(DeserializationError::FailedToDeserializeTemplate {
            template_id,
            ..
        }) => Some(template_id.to_string()),
        _ => None,
    }
}

fn binxml_value_to_string_lossy(value: &crate::binxml::value_variant::BinXmlValue<'_>) -> String {
    use crate::binxml::value_variant::BinXmlValue;
    match value {
        BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => String::new(),
        _ => value.as_cow_str().into_owned(),
    }
}

fn substitutions_from_template_instance<'a>(
    chunk: &'a crate::EvtxChunk<'a>,
    tpl: &crate::model::deserialized::BinXmlTemplateRef,
) -> Vec<String> {
    tpl.substitutions
        .iter()
        .map(|span| match span.decode(chunk) {
            Ok(v) => binxml_value_to_string_lossy(&v),
            Err(_) => String::new(),
        })
        .collect()
}

/// Resolve the GUID for a record `TemplateInstance` so we can strictly match it against the
/// template GUID from a deserialization error (`FailedToDeserializeTemplate { template_id: GUID }`).
///
/// Resolution order (deterministic):
/// 1. **Inline**: `tpl.template_guid` when the record embeds a template definition header inline in
///    the `TemplateInstance`.
/// 2. **Cached**: lookup `tpl.template_def_offset` in the chunk `template_table` and read the cached
///    template definition header GUID.
/// 3. **Direct**: read and validate a `TemplateDefinitionHeader` directly from the chunk bytes at
///    `tpl.template_def_offset` (bounds + plausible header + BinXML fragment header).
///
/// If none of the above succeeds, returns `None`. In that case WEVT-cache rendering will not be
/// attempted because we cannot prove which substitution array matches the error GUID.
fn resolve_template_guid_from_record<'a>(
    record: &crate::EvtxRecord<'a>,
    tpl: &crate::model::deserialized::BinXmlTemplateRef,
) -> Option<String> {
    if let Some(g) = tpl.template_guid.as_ref() {
        return Some(g.to_string());
    }

    // Prefer the fully-parsed/cached template table (fast path).
    if let Some(def) = record
        .chunk
        .template_table
        .get_template(tpl.template_def_offset)
    {
        return Some(def.header.guid.to_string());
    }

    // Finally: validate and read the template definition header directly from the chunk bytes at
    // `template_def_offset`.
    crate::binxml::tokens::try_read_template_definition_header_at(
        record.chunk.data,
        tpl.template_def_offset,
    )
    .ok()
    .map(|h| h.guid.to_string())
}

fn collect_template_instances<'a>(record: &crate::EvtxRecord<'a>) -> Vec<TemplateInstanceInfo> {
    let mut out = Vec::new();

    for t in &record.tokens {
        let BinXMLDeserializedTokens::TemplateInstance(tpl) = t else {
            continue;
        };

        let guid =
            resolve_template_guid_from_record(record, tpl).map(|g| super::normalize_guid(&g));
        let substitutions = substitutions_from_template_instance(record.chunk, tpl);

        out.push(TemplateInstanceInfo {
            guid,
            substitutions,
        });
    }

    out
}

fn select_template_instance_for_guid<'a>(
    instances: &'a [TemplateInstanceInfo],
    guid: &str,
) -> Option<&'a TemplateInstanceInfo> {
    let want = super::normalize_guid(guid);

    let mut matches = instances
        .iter()
        .filter(|i| i.guid.as_ref().is_some_and(|g| g == &want));

    let first = matches.next()?;
    if matches.next().is_some() {
        None
    } else {
        Some(first)
    }
}

impl crate::EvtxRecord<'_> {
    /// Render a record as XML, using the EVTX’s embedded templates first.
    ///
    /// If rendering fails *specifically because a template definition cannot be deserialized* and
    /// the error contains a concrete template GUID, this will deterministically attempt to render
    /// the record using the provided offline WEVT cache:
    /// - We only use the cache when the error is `FailedToDeserializeTemplate { template_id: GUID }`.
    /// - We only proceed when we can unambiguously select the matching `TemplateInstance`
    ///   substitution array for that GUID.
    /// - Otherwise we return the original error unchanged.
    ///
    /// Note: When the cache is used, the returned `data` is the rendered *template XML fragment*,
    /// not the full EVTX `<Event>` wrapper.
    pub fn into_xml_with_wevt_cache(
        self,
        cache: &super::WevtCache,
    ) -> crate::err::Result<SerializedEvtxRecord<String>> {
        let record_id = self.event_record_id;
        let timestamp = self.timestamp;
        let ansi_codec = self.settings.get_ansi_codec();

        let instances = collect_template_instances(&self);

        match self.into_xml() {
            Ok(r) => Ok(r),
            Err(e) => {
                let Some(guid) = extract_template_guid_from_error(&e) else {
                    return Err(e);
                };

                let Some(tpl) = select_template_instance_for_guid(&instances, &guid) else {
                    return Err(e);
                };
                let subs = &tpl.substitutions;

                match cache.render_by_template_guid_with_ansi_codec(&guid, subs, ansi_codec) {
                    Ok(xml_fragment) => {
                        log::info!(
                            "wevt-cache used: record_id={} template_guid={}",
                            record_id,
                            guid
                        );
                        Ok(SerializedEvtxRecord {
                            event_record_id: record_id,
                            timestamp,
                            data: xml_fragment,
                        })
                    }
                    Err(render_err) => {
                        log::warn!(
                            "wevt-cache render failed for record {} template_guid={}: {render_err}",
                            record_id,
                            guid
                        );
                        Err(e)
                    }
                }
            }
        }
    }

    /// Render a record as JSON, using the EVTX’s embedded templates first.
    ///
    /// This follows the same deterministic WEVT-cache rule as `into_xml_with_wevt_cache`:
    /// only on an explicit template-GUID deserialization failure and only with an unambiguous
    /// `TemplateInstance` substitution array.
    ///
    /// When the cache is used, the JSON output is a synthetic object that contains the rendered XML
    /// fragment under `xml` (and includes metadata fields like `template_guid` and `record_id`).
    pub fn into_json_with_wevt_cache(
        self,
        cache: &super::WevtCache,
    ) -> crate::err::Result<SerializedEvtxRecord<String>> {
        let record_id = self.event_record_id;
        let timestamp = self.timestamp;
        let indent = self.settings.should_indent();
        let ansi_codec = self.settings.get_ansi_codec();

        let instances = collect_template_instances(&self);

        match self.into_json() {
            Ok(r) => Ok(r),
            Err(e) => {
                let Some(guid) = extract_template_guid_from_error(&e) else {
                    return Err(e);
                };
                let Some(tpl) = select_template_instance_for_guid(&instances, &guid) else {
                    return Err(e);
                };
                let subs = &tpl.substitutions;

                match cache.render_by_template_guid_with_ansi_codec(&guid, subs, ansi_codec) {
                    Ok(xml_fragment) => {
                        log::info!(
                            "wevt-cache used: record_id={} template_guid={}",
                            record_id,
                            guid
                        );
                        let v = serde_json::json!({
                            "_wevt_cache_used": true,
                            "template_guid": guid,
                            "record_id": record_id,
                            "timestamp": timestamp.to_rfc3339(),
                            "xml": xml_fragment,
                        });

                        let data = if indent {
                            serde_json::to_string_pretty(&v)
                                .map_err(crate::err::SerializationError::from)?
                        } else {
                            serde_json::to_string(&v)
                                .map_err(crate::err::SerializationError::from)?
                        };

                        Ok(SerializedEvtxRecord {
                            event_record_id: record_id,
                            timestamp,
                            data,
                        })
                    }
                    Err(render_err) => {
                        log::warn!(
                            "wevt-cache render failed for record {} template_guid={}: {render_err}",
                            record_id,
                            guid
                        );
                        Err(e)
                    }
                }
            }
        }
    }

    /// Like `into_json_with_wevt_cache`, but the "normal path" uses `into_json_stream()` instead of
    /// building a full `serde_json::Value` per record.
    pub fn into_json_stream_with_wevt_cache(
        self,
        cache: &super::WevtCache,
    ) -> crate::err::Result<SerializedEvtxRecord<String>> {
        let record_id = self.event_record_id;
        let timestamp = self.timestamp;
        let indent = self.settings.should_indent();
        let ansi_codec = self.settings.get_ansi_codec();

        let instances = collect_template_instances(&self);

        match self.into_json_stream() {
            Ok(r) => Ok(r),
            Err(e) => {
                let Some(guid) = extract_template_guid_from_error(&e) else {
                    return Err(e);
                };
                let Some(tpl) = select_template_instance_for_guid(&instances, &guid) else {
                    return Err(e);
                };
                let subs = &tpl.substitutions;

                match cache.render_by_template_guid_with_ansi_codec(&guid, subs, ansi_codec) {
                    Ok(xml_fragment) => {
                        log::info!(
                            "wevt-cache used: record_id={} template_guid={}",
                            record_id,
                            guid
                        );
                        let v = serde_json::json!({
                            "_wevt_cache_used": true,
                            "template_guid": guid,
                            "record_id": record_id,
                            "timestamp": timestamp.to_rfc3339(),
                            "xml": xml_fragment,
                        });

                        let data = if indent {
                            serde_json::to_string_pretty(&v)
                                .map_err(crate::err::SerializationError::from)?
                        } else {
                            serde_json::to_string(&v)
                                .map_err(crate::err::SerializationError::from)?
                        };

                        Ok(SerializedEvtxRecord {
                            event_record_id: record_id,
                            timestamp,
                            data,
                        })
                    }
                    Err(render_err) => {
                        log::warn!(
                            "wevt-cache render failed for record {} template_guid={}: {render_err}",
                            record_id,
                            guid
                        );
                        Err(e)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_template_instance_for_guid_requires_match_even_when_single_instance() {
        let want = "{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}";
        let other = "{11111111-2222-3333-4444-555555555555}";

        let instances = vec![TemplateInstanceInfo {
            guid: Some(super::super::normalize_guid(want)),
            substitutions: vec![],
        }];

        assert!(
            select_template_instance_for_guid(&instances, other).is_none(),
            "single instance must not be selected when GUID mismatches"
        );
        assert!(
            select_template_instance_for_guid(&instances, want).is_some(),
            "single instance should be selected when GUID matches"
        );
    }

    #[test]
    fn select_template_instance_for_guid_requires_unique_match() {
        let want = "{aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}";
        let g = super::super::normalize_guid(want);

        let instances = vec![
            TemplateInstanceInfo {
                guid: Some(g.clone()),
                substitutions: vec!["a".to_string()],
            },
            TemplateInstanceInfo {
                guid: Some(g),
                substitutions: vec!["b".to_string()],
            },
        ];

        assert!(
            select_template_instance_for_guid(&instances, want).is_none(),
            "ambiguous GUID match must be rejected"
        );
    }
}
