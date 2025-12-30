use anyhow::{Context, Result, bail, format_err};
use clap::{Arg, ArgMatches, Command};
use indoc::indoc;

pub fn command() -> Command {
    Command::new("apply-wevt-cache")
        .about("Render a WEVT template using an offline cache + substitution values")
        .long_about(indoc!(r#"
            Render a WEVT template using an offline cache + substitution values.

            Inputs:
            - A cache index JSONL (stdout from `extract-wevt-templates`).
            - A template selector: either --template-guid, or (provider_guid,event_id,version).
            - Substitution values: either extracted from an EVTX record (--evtx + --record-id),
              or provided as a JSON array (--substitutions / --substitutions-file).
        "#))
        .arg(
            Arg::new("cache-index")
                .long("cache-index")
                .required(true)
                .value_name("PATH")
                .help("Path to cache index JSONL (stdout from `extract-wevt-templates`)."),
        )
        .arg(
            Arg::new("template-guid")
                .long("template-guid")
                .value_name("GUID")
                .help("Template GUID to render."),
        )
        .arg(
            Arg::new("provider-guid")
                .long("provider-guid")
                .value_name("GUID")
                .help("Provider GUID (used to resolve template GUID via the cache index)."),
        )
        .arg(
            Arg::new("event-id")
                .long("event-id")
                .value_parser(clap::value_parser!(u16).range(0..))
                .value_name("ID")
                .help("Event ID (used to resolve template GUID via the cache index)."),
        )
        .arg(
            Arg::new("version")
                .long("version")
                .value_parser(clap::value_parser!(u8).range(0..))
                .value_name("V")
                .help("Event version (used to resolve template GUID via the cache index)."),
        )
        .arg(
            Arg::new("evtx")
                .long("evtx")
                .value_name("PATH")
                .help("EVTX file to extract substitution values from (TemplateInstance)."),
        )
        .arg(
            Arg::new("record-id")
                .long("record-id")
                .value_parser(clap::value_parser!(u64).range(0..))
                .value_name("ID")
                .help("Event record id to extract substitution values from."),
        )
        .arg(
            Arg::new("template-instance-index")
                .long("template-instance-index")
                .value_parser(clap::value_parser!(usize))
                .default_value("0")
                .value_name("N")
                .help("When a record contains multiple TemplateInstance tokens, select which one to use (default: 0)."),
        )
        .arg(
            Arg::new("substitutions")
                .long("substitutions")
                .value_name("JSON")
                .help("Substitution values as a JSON array (strings/numbers)."),
        )
        .arg(
            Arg::new("substitutions-file")
                .long("substitutions-file")
                .value_name("PATH")
                .help("Path to a JSON file containing a substitution values array."),
        )
        .arg(
            Arg::new("output")
                .long("output")
                .short('o')
                .value_name("PATH")
                .help("Write rendered XML to this path (default: stdout)."),
        )
}

pub fn run(matches: &ArgMatches) -> Result<()> {
    #[cfg(feature = "wevt_templates")]
    {
        run_impl(matches)
    }

    #[cfg(not(feature = "wevt_templates"))]
    {
        let _ = matches;
        bail!(
            "This subcommand requires building `evtx_dump` with template support enabled.\n\
             Example:\n\
               cargo run --bin evtx_dump -- apply-wevt-cache ..."
        );
    }
}

#[cfg(feature = "wevt_templates")]
mod imp {
    use super::*;
    use evtx::EvtxParser;
    use evtx::ParserSettings;
    use evtx::binxml::value_variant::BinXmlValue;
    use evtx::wevt_templates::manifest::CrimManifest;
    use evtx::wevt_templates::render_template_definition_to_xml_with_values;
    use serde_json::Value as JsonValue;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    struct ResourceKey {
        source: String,
        resource: String,
        lang_id: u32,
    }

    #[derive(Debug, Default)]
    struct CacheIndex {
        crim_paths: Vec<String>,
        event_to_template_guid: std::collections::HashMap<(String, u16, u8), String>,
    }

    fn normalize_guid(s: &str) -> String {
        evtx::wevt_templates::normalize_guid(s)
    }

    fn parse_resource_id(v: &JsonValue) -> Option<String> {
        match v {
            JsonValue::Number(n) => n.as_u64().map(|id| format!("id:{id}")),
            JsonValue::String(s) => Some(format!("name:{s}")),
            _ => None,
        }
    }

    fn load_cache_index(path: &Path) -> Result<CacheIndex> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read cache index `{}`", path.display()))?;
        let mut out = CacheIndex::default();

        // Also map (source,resource,lang) -> CRIM output path.
        let mut crim_by_key: std::collections::HashMap<ResourceKey, String> =
            std::collections::HashMap::new();

        fn resolve_output_path(index_path: &Path, output_path: &str) -> String {
            let p = Path::new(output_path);
            if p.is_absolute() {
                return output_path.to_string();
            }
            let base = index_path.parent().unwrap_or_else(|| Path::new("."));
            base.join(p).to_string_lossy().to_string()
        }

        for (line_no, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: JsonValue = serde_json::from_str(line)
                .with_context(|| format!("invalid JSONL at {}:{}", path.display(), line_no + 1))?;

            let source = v
                .get("source")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let resource = v.get("resource").and_then(parse_resource_id);
            let lang_id = v
                .get("lang_id")
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok());

            // ExtractWevtTemplatesOutputLine: has output_path + size, but no guid/provider_guid/template_guid.
            if v.get("output_path").and_then(|p| p.as_str()).is_some()
                && v.get("size").is_some()
                && v.get("guid").is_none()
                && v.get("provider_guid").is_none()
                && v.get("template_guid").is_none()
            {
                if let (Some(source), Some(resource), Some(lang_id)) = (source, resource, lang_id) {
                    let key = ResourceKey {
                        source,
                        resource,
                        lang_id,
                    };
                    if let Some(p) = v.get("output_path").and_then(|p| p.as_str()) {
                        crim_by_key.insert(key, resolve_output_path(path, p));
                    }
                }
                continue;
            }

            // ExtractWevtEventOutputLine: has provider_guid/event_id/version/template_guid.
            let template_guid = v
                .get("template_guid")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty());

            if let (Some(provider_guid), Some(event_id), Some(version), Some(template_guid)) = (
                v.get("provider_guid").and_then(|v| v.as_str()),
                v.get("event_id")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u16::try_from(n).ok()),
                v.get("version")
                    .and_then(|v| v.as_u64())
                    .and_then(|n| u8::try_from(n).ok()),
                template_guid,
            ) {
                out.event_to_template_guid.insert(
                    (normalize_guid(provider_guid), event_id, version),
                    normalize_guid(template_guid),
                );
            }
        }

        out.crim_paths = crim_by_key.values().cloned().collect();
        Ok(out)
    }

    fn values_from_json_array<'a>(
        v: &JsonValue,
        bump: &'a bumpalo::Bump,
    ) -> Result<Vec<BinXmlValue<'a>>> {
        let Some(arr) = v.as_array() else {
            bail!("substitutions JSON must be an array");
        };
        Ok(arr
            .iter()
            .map(|v| match v {
                JsonValue::Null => BinXmlValue::NullType,
                JsonValue::Bool(b) => BinXmlValue::BoolType(*b),
                JsonValue::Number(n) => {
                    if let Some(i) = n.as_i64() {
                        BinXmlValue::Int64Type(i)
                    } else if let Some(u) = n.as_u64() {
                        BinXmlValue::UInt64Type(u)
                    } else if let Some(f) = n.as_f64() {
                        BinXmlValue::Real64Type(f)
                    } else {
                        BinXmlValue::NullType
                    }
                }
                JsonValue::String(s) => BinXmlValue::AnsiStringType(bump.alloc_str(s)),
                other => BinXmlValue::AnsiStringType(bump.alloc_str(&other.to_string())),
            })
            .collect::<Vec<_>>())
    }

    pub(super) fn run_impl(matches: &ArgMatches) -> Result<()> {
        let cache_index_path =
            PathBuf::from(matches.get_one::<String>("cache-index").expect("required"));
        let cache = load_cache_index(&cache_index_path)?;

        // Resolve substitutions.
        let template_instance_index: usize = *matches
            .get_one::<usize>("template-instance-index")
            .expect("has default");

        let evtx_subs = if let (Some(evtx_path), Some(record_id)) = (
            matches.get_one::<String>("evtx").map(PathBuf::from),
            matches.get_one::<u64>("record-id").copied(),
        ) {
            Some((evtx_path, record_id))
        } else {
            None
        };

        let json_subs = if evtx_subs.is_none() {
            if let Some(s) = matches.get_one::<String>("substitutions") {
                Some(
                    serde_json::from_str::<JsonValue>(s)
                        .context("failed to parse --substitutions as JSON")?,
                )
            } else if let Some(p) = matches.get_one::<String>("substitutions-file") {
                let text = fs::read_to_string(p)
                    .with_context(|| format!("failed to read substitutions file `{p}`"))?;
                Some(
                    serde_json::from_str::<JsonValue>(&text)
                        .context("failed to parse substitutions file as JSON")?,
                )
            } else {
                None
            }
        } else {
            None
        };

        if evtx_subs.is_none() && json_subs.is_none() {
            bail!(
                "Must provide substitutions via --evtx+--record-id or --substitutions/--substitutions-file"
            );
        }

        // Resolve template guid.
        let template_guid = if let Some(g) = matches.get_one::<String>("template-guid") {
            normalize_guid(g)
        } else if let (Some(provider_guid), Some(event_id), Some(version)) = (
            matches.get_one::<String>("provider-guid"),
            matches.get_one::<u16>("event-id").copied(),
            matches.get_one::<u8>("version").copied(),
        ) {
            let key = (normalize_guid(provider_guid), event_id, version);
            cache
                .event_to_template_guid
                .get(&key)
                .cloned()
                .ok_or_else(|| {
                    format_err!(
                        "no template_guid found in cache index for provider_guid={provider_guid} event_id={event_id} version={version}"
                    )
                })?
        } else {
            bail!(
                "Must provide either --template-guid or (--provider-guid, --event-id, --version)"
            );
        };

        // Locate the template definition in one of the CRIM blobs.
        let mut template_crim_bytes: Option<Vec<u8>> = None;
        for crim_path in &cache.crim_paths {
            let bytes = match fs::read(crim_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let manifest = match CrimManifest::parse(&bytes) {
                Ok(m) => m,
                Err(_) => continue,
            };
            let mut found = false;
            for provider in &manifest.providers {
                if let Some(ttbl) = provider.wevt.elements.templates.as_ref()
                    && ttbl
                        .templates
                        .iter()
                        .any(|tpl| normalize_guid(&tpl.guid.to_string()) == template_guid)
                {
                    found = true;
                    break;
                }
            }
            if found {
                template_crim_bytes = Some(bytes);
                break;
            }
        }

        let template_crim_bytes = template_crim_bytes.ok_or_else(|| {
            format_err!(
                "template GUID `{}` not found in any CRIM blobs referenced by `{}`",
                template_guid,
                cache_index_path.display()
            )
        })?;

        let manifest = CrimManifest::parse(&template_crim_bytes)
            .context("failed to parse selected CRIM blob")?;
        let tpl = manifest
            .providers
            .iter()
            .find_map(|provider| {
                provider.wevt.elements.templates.as_ref().and_then(|ttbl| {
                    ttbl.templates
                        .iter()
                        .find(|tpl| normalize_guid(&tpl.guid.to_string()) == template_guid)
                })
            })
            .ok_or_else(|| {
                format_err!(
                    "template GUID `{}` not found in selected CRIM blob (unexpected)",
                    template_guid
                )
            })?;

        let xml = if let Some((evtx_path, record_id)) = evtx_subs {
            let settings = ParserSettings::default();
            let mut parser = EvtxParser::from_path(&evtx_path)
                .with_context(|| format!("Failed to open evtx file at: {}", evtx_path.display()))?
                .with_configuration(settings.clone());

            for chunk_res in parser.chunks() {
                let mut chunk_data = chunk_res?;
                let mut chunk = chunk_data.parse(std::sync::Arc::new(settings.clone()))?;
                for record_res in chunk.iter() {
                    let record = record_res?;
                    if record.event_record_id != record_id {
                        continue;
                    }

                    let instances = record.template_instances()?;
                    let instance = instances.get(template_instance_index).ok_or_else(|| {
                        format_err!(
                            "record {} has no TemplateInstance at index {}",
                            record.event_record_id,
                            template_instance_index
                        )
                    })?;
                    let xml = render_template_definition_to_xml_with_values(
                        tpl,
                        &instance.values,
                        encoding::all::WINDOWS_1252,
                        &record.chunk.arena,
                    )?;
                    // Found and rendered; stop searching.
                    if let Some(out_path) = matches.get_one::<String>("output") {
                        fs::write(out_path, xml.as_bytes())
                            .with_context(|| format!("failed to write output `{out_path}`"))?;
                    } else {
                        print!("{xml}");
                    }
                    return Ok(());
                }
            }

            bail!("record_id {record_id} not found in {}", evtx_path.display());
        } else {
            let json = json_subs.expect("checked above");
            let bump = bumpalo::Bump::new();
            let values = values_from_json_array(&json, &bump)?;
            render_template_definition_to_xml_with_values(
                tpl,
                &values,
                encoding::all::WINDOWS_1252,
                &bump,
            )?
        };

        if let Some(out_path) = matches.get_one::<String>("output") {
            fs::write(out_path, xml.as_bytes())
                .with_context(|| format!("failed to write output `{out_path}`"))?;
        } else {
            print!("{xml}");
        }

        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::io::Write;

        #[test]
        fn normalize_guid_strips_braces_and_is_case_insensitive() {
            let braced = "{12345678-1234-1234-1234-123456789ABC}";
            let unbraced = "12345678-1234-1234-1234-123456789abc";

            assert_eq!(normalize_guid(braced), unbraced);
            assert_eq!(normalize_guid(unbraced), unbraced);
        }

        #[test]
        fn load_cache_index_normalizes_provider_and_template_guids() -> Result<()> {
            let mut f = tempfile::NamedTempFile::new().context("tempfile")?;
            writeln!(
                f,
                r#"{{"provider_guid":"{{AAAAAAAA-AAAA-AAAA-AAAA-AAAAAAAAAAAA}}","event_id":1,"version":2,"template_guid":"{{BBBBBBBB-BBBB-BBBB-BBBB-BBBBBBBBBBBB}}"}}"#
            )
            .context("write jsonl")?;

            let cache = load_cache_index(f.path()).context("load_cache_index")?;

            let key = (
                normalize_guid("aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"),
                1u16,
                2u8,
            );
            assert_eq!(
                cache.event_to_template_guid.get(&key).map(|s| s.as_str()),
                Some("bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
            );
            Ok(())
        }
    }
}

#[cfg(feature = "wevt_templates")]
use imp::run_impl;
