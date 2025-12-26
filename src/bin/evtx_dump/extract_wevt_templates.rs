use anyhow::{Context, Result, bail};
use clap::{Arg, ArgAction, ArgMatches, Command};
use indoc::indoc;

pub fn command() -> Command {
    let cmd = Command::new("extract-wevt-templates")
        .about("Extract WEVT_TEMPLATE resources from PE files (EXE/DLL)")
        .long_about(indoc!(r#"
            Extract WEVT_TEMPLATE resources from PE files (EXE/DLL).

            This is intended to support building an offline cache of EVTX templates
            (see issue #103), without committing to any database format yet.

            Note: this subcommand is included in the default `evtx_dump` build.
        "#))
        .arg(
            Arg::new("input")
                .long("input")
                .short('i')
                .action(ArgAction::Append)
                .value_name("PATH")
                .help("Input PE path (file or directory). Can be passed multiple times."),
        )
        .arg(
            Arg::new("glob")
                .long("glob")
                .action(ArgAction::Append)
                .value_name("PATTERN")
                .help("Glob pattern to expand into input paths (cross-platform). Can be passed multiple times."),
        )
        .arg(
            Arg::new("recursive")
                .long("recursive")
                .short('r')
                .action(ArgAction::SetTrue)
                .help("When an input path is a directory (or a glob matches a directory), recurse into it."),
        )
        .arg(
            Arg::new("extensions")
                .long("extensions")
                .value_name("EXTS")
                .default_value("exe,dll,sys")
                .help("Comma-separated list of allowed file extensions when walking directories (default: exe,dll,sys)."),
        )
        .arg(
            Arg::new("output-dir")
                .long("output-dir")
                .short('o')
                .required(true)
                .value_name("DIR")
                .help("Directory to write extracted resources into."),
        )
        .arg(
            Arg::new("overwrite")
                .long("overwrite")
                .action(ArgAction::SetTrue)
                .help("Overwrite output files if they already exist."),
        );

    #[cfg(feature = "wevt_templates")]
    let cmd = cmd
        .arg(
            Arg::new("split-ttbl")
                .long("split-ttbl")
                .action(ArgAction::SetTrue)
                .help("Also split extracted WEVT_TEMPLATE blobs into TTBL/TEMP entries and write each TEMP to <output-dir>/temp/."),
        )
        .arg(
            Arg::new("dump-temp-xml")
                .long("dump-temp-xml")
                .action(ArgAction::SetTrue)
                .help("Also render each TEMP BinXML fragment to XML and write to <output-dir>/temp_xml/."),
        )
        .arg(
            Arg::new("dump-events")
                .long("dump-events")
                .action(ArgAction::SetTrue)
                .help("Dump EVNT event definitions (including template_offset join keys) as JSONL."),
        )
        .arg(
            Arg::new("dump-items")
                .long("dump-items")
                .action(ArgAction::SetTrue)
                .help("Dump TEMP template item descriptors/names as JSONL."),
        );

    cmd
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
              cargo run --bin evtx_dump -- extract-wevt-templates ..."
        );
    }
}

#[cfg(feature = "wevt_templates")]
mod imp {
    use super::*;
    use serde::Serialize;
    use std::fs;
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, Serialize)]
    #[serde(untagged)]
    enum ResourceIdJson {
        Id(u32),
        Name(String),
    }

    #[derive(Debug, Serialize)]
    struct ExtractWevtTemplatesOutputLine {
        source: String,
        resource: ResourceIdJson,
        lang_id: u32,
        output_path: String,
        size: usize,
    }

    #[derive(Debug, Serialize)]
    struct ExtractWevtTempOutputLine {
        source: String,
        resource: ResourceIdJson,
        lang_id: u32,
        ttbl_offset: u32,
        temp_offset: u32,
        temp_size: u32,
        item_descriptor_count: u32,
        item_name_count: u32,
        template_items_offset: u32,
        event_type: u32,
        guid: String,
        output_path: String,
    }

    #[derive(Debug, Serialize)]
    struct ExtractWevtTempXmlOutputLine {
        source: String,
        resource: ResourceIdJson,
        lang_id: u32,
        temp_index: usize,
        guid: String,
        output_path: String,
    }

    #[derive(Debug, Serialize)]
    struct ExtractWevtEventOutputLine {
        source: String,
        resource: ResourceIdJson,
        lang_id: u32,
        provider_guid: String,
        event_index: usize,
        event_id: u16,
        version: u8,
        channel: u8,
        level: u8,
        opcode: u8,
        task: u16,
        keywords: u64,
        message_identifier: u32,
        template_offset: Option<u32>,
        template_guid: Option<String>,
    }

    #[derive(Debug, Serialize)]
    struct ExtractWevtTemplateItemOutputLine {
        source: String,
        resource: ResourceIdJson,
        lang_id: u32,
        ttbl_offset: u32,
        template_offset: u32,
        template_guid: String,
        item_index: usize,
        name: Option<String>,
        input_type: u8,
        output_type: u8,
        count: u16,
        length: u16,
        name_offset: u32,
        unknown1: u32,
        unknown3: u16,
        unknown4: u32,
    }

    pub(super) fn run_impl(matches: &ArgMatches) -> Result<()> {
        use evtx::wevt_templates::{ResourceIdentifier, extract_wevt_template_resources};

        use evtx::wevt_templates::manifest::CrimManifest;
        use evtx::wevt_templates::render_template_definition_to_xml;
        use std::collections::HashSet;

        let output_dir = PathBuf::from(
            matches
                .get_one::<String>("output-dir")
                .expect("required argument"),
        );
        fs::create_dir_all(&output_dir).with_context(|| {
            format!(
                "failed to create output dir `{}`",
                output_dir.to_string_lossy()
            )
        })?;

        let overwrite = matches.get_flag("overwrite");
        let recursive = matches.get_flag("recursive");

        let split_ttbl = matches.get_flag("split-ttbl");

        let dump_temp_xml = matches.get_flag("dump-temp-xml");

        let dump_events = matches.get_flag("dump-events");

        let dump_items = matches.get_flag("dump-items");

        let allowed_exts: HashSet<String> = matches
            .get_one::<String>("extensions")
            .expect("has default")
            .split(',')
            .map(|s| s.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|s| !s.is_empty())
            .collect();

        let mut inputs: Vec<PathBuf> = vec![];

        if let Some(paths) = matches.get_many::<String>("input") {
            inputs.extend(paths.map(PathBuf::from));
        }

        if let Some(patterns) = matches.get_many::<String>("glob") {
            for pat in patterns {
                for entry in
                    glob::glob(pat).with_context(|| format!("invalid glob pattern `{pat}`"))?
                {
                    match entry {
                        Ok(p) => inputs.push(p),
                        Err(e) => eprintln!("glob entry error: {e}"),
                    }
                }
            }
        }

        if inputs.is_empty() {
            bail!("No inputs provided. Use --input and/or --glob.");
        }

        // Expand directories (optionally recursively) and filter by extension.
        let mut files = vec![];
        let mut seen = HashSet::<PathBuf>::new();

        for input in inputs {
            collect_input_paths(&input, recursive, &allowed_exts, &mut seen, &mut files)?;
        }

        // Keep output stable-ish.
        files.sort();

        let mut error_count = 0usize;
        let mut extracted_count = 0usize;

        for path in files {
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(e) => {
                    error_count += 1;
                    eprintln!("failed to read `{}`: {e}", path.to_string_lossy());
                    continue;
                }
            };

            let resources = match extract_wevt_template_resources(&bytes) {
                Ok(r) => r,
                Err(e) => {
                    error_count += 1;
                    eprintln!(
                        "failed to extract WEVT_TEMPLATE from `{}`: {e}",
                        path.to_string_lossy()
                    );
                    continue;
                }
            };

            if resources.is_empty() {
                continue;
            }

            let source_str = path.to_string_lossy().to_string();
            let source_hash = evtx::checksum_ieee(source_str.as_bytes());

            for res in resources {
                let resource_id_str = match &res.resource {
                    ResourceIdentifier::Id(id) => format!("id_{id}"),
                    ResourceIdentifier::Name(name) => format!("name_{}", sanitize_component(name)),
                };

                let out_name = format!(
                    "{base}.{hash:08x}.wevt_template.{res_id}.lang_{lang}.bin",
                    base = path
                        .file_name()
                        .map(|s| s.to_string_lossy())
                        .unwrap_or_else(|| "unknown".into()),
                    hash = source_hash,
                    res_id = resource_id_str,
                    lang = res.lang_id
                );

                let out_path = output_dir.join(out_name);

                if out_path.exists() && !overwrite {
                    continue;
                }

                if let Err(e) = fs::write(&out_path, &res.data) {
                    error_count += 1;
                    eprintln!("failed to write `{}`: {e}", out_path.to_string_lossy());
                    continue;
                }

                extracted_count += 1;

                let resource_json = match &res.resource {
                    ResourceIdentifier::Id(id) => ResourceIdJson::Id(*id),
                    ResourceIdentifier::Name(name) => ResourceIdJson::Name(name.clone()),
                };

                let line = ExtractWevtTemplatesOutputLine {
                    source: source_str.clone(),
                    resource: resource_json,
                    lang_id: res.lang_id,
                    output_path: out_path.to_string_lossy().to_string(),
                    size: res.data.len(),
                };

                println!("{}", serde_json::to_string(&line)?);

                if split_ttbl || dump_temp_xml || dump_events || dump_items {
                    let templates_dir = output_dir.join("temp");
                    let templates_xml_dir = output_dir.join("temp_xml");

                    if split_ttbl {
                        fs::create_dir_all(&templates_dir).with_context(|| {
                            format!(
                                "failed to create TEMP output dir `{}`",
                                templates_dir.to_string_lossy()
                            )
                        })?;
                    }

                    if dump_temp_xml {
                        fs::create_dir_all(&templates_xml_dir).with_context(|| {
                            format!(
                                "failed to create TEMP XML output dir `{}`",
                                templates_xml_dir.to_string_lossy()
                            )
                        })?;
                    }

                    let manifest = match CrimManifest::parse(&res.data) {
                        Ok(m) => m,
                        Err(e) => {
                            error_count += 1;
                            eprintln!(
                                "failed to parse CRIM/WEVT manifest in `{}`: {e}",
                                source_str
                            );
                            continue;
                        }
                    };

                    let resource_json_for_records = match &res.resource {
                        ResourceIdentifier::Id(id) => ResourceIdJson::Id(*id),
                        ResourceIdentifier::Name(name) => ResourceIdJson::Name(name.clone()),
                    };

                    if dump_events {
                        for provider in &manifest.providers {
                            let provider_guid = format!("{}", provider.guid);
                            if let Some(evnt) = provider.wevt.elements.events.as_ref() {
                                for (event_index, ev) in evnt.events.iter().enumerate() {
                                    let template_guid = ev
                                        .template_offset
                                        .and_then(|off| provider.template_by_offset(off))
                                        .map(|t| format!("{}", t.guid));

                                    let line = ExtractWevtEventOutputLine {
                                        source: source_str.clone(),
                                        resource: resource_json_for_records.clone(),
                                        lang_id: res.lang_id,
                                        provider_guid: provider_guid.clone(),
                                        event_index,
                                        event_id: ev.identifier,
                                        version: ev.version,
                                        channel: ev.channel,
                                        level: ev.level,
                                        opcode: ev.opcode,
                                        task: ev.task,
                                        keywords: ev.keywords,
                                        message_identifier: ev.message_identifier,
                                        template_offset: ev.template_offset,
                                        template_guid,
                                    };

                                    println!("{}", serde_json::to_string(&line)?);
                                }
                            }
                        }
                    }

                    if dump_items {
                        for provider in &manifest.providers {
                            if let Some(ttbl) = provider.wevt.elements.templates.as_ref() {
                                for tpl in &ttbl.templates {
                                    let template_guid = format!("{}", tpl.guid);
                                    for (item_index, item) in tpl.items.iter().enumerate() {
                                        let line = ExtractWevtTemplateItemOutputLine {
                                            source: source_str.clone(),
                                            resource: resource_json_for_records.clone(),
                                            lang_id: res.lang_id,
                                            ttbl_offset: ttbl.offset,
                                            template_offset: tpl.offset,
                                            template_guid: template_guid.clone(),
                                            item_index,
                                            name: item.name.clone(),
                                            input_type: item.input_type,
                                            output_type: item.output_type,
                                            count: item.count,
                                            length: item.length,
                                            name_offset: item.name_offset,
                                            unknown1: item.unknown1,
                                            unknown3: item.unknown3,
                                            unknown4: item.unknown4,
                                        };
                                        println!("{}", serde_json::to_string(&line)?);
                                    }
                                }
                            }
                        }
                    }

                    // Keep ordering stable-ish by sorting by template offset.
                    let mut templates: Vec<(
                        u32,
                        &evtx::wevt_templates::manifest::TemplateDefinition<'_>,
                    )> = vec![];
                    for provider in &manifest.providers {
                        if let Some(ttbl) = provider.wevt.elements.templates.as_ref() {
                            for tpl in &ttbl.templates {
                                templates.push((ttbl.offset, tpl));
                            }
                        }
                    }
                    templates.sort_by_key(|(ttbl_off, tpl)| (tpl.offset, *ttbl_off));

                    for (idx, (ttbl_offset, tpl)) in templates.iter().enumerate() {
                        let temp_off = tpl.offset as usize;
                        let temp_end = temp_off.saturating_add(tpl.size as usize);
                        if temp_end > res.data.len() {
                            error_count += 1;
                            eprintln!(
                                "TEMP slice out of bounds for `{}` (temp_offset={}, temp_size={})",
                                source_str, tpl.offset, tpl.size
                            );
                            continue;
                        }

                        let temp_bytes = &res.data[temp_off..temp_end];

                        let guid_display = format!("{}", tpl.guid);
                        let guid_file = sanitize_component(&guid_display);

                        if split_ttbl {
                            let out_name = format!(
                                "{base}.{hash:08x}.wevt_template.{res_id}.lang_{lang}.temp_{idx:04}.{guid}.bin",
                                base = path
                                    .file_name()
                                    .map(|s| s.to_string_lossy())
                                    .unwrap_or_else(|| "unknown".into()),
                                hash = source_hash,
                                res_id = resource_id_str,
                                lang = res.lang_id,
                                idx = idx,
                                guid = guid_file,
                            );

                            let temp_path = templates_dir.join(out_name);
                            if overwrite || !temp_path.exists() {
                                match fs::write(&temp_path, temp_bytes) {
                                    Ok(()) => {}
                                    Err(e) => {
                                        error_count += 1;
                                        eprintln!(
                                            "failed to write `{}`: {e}",
                                            temp_path.to_string_lossy()
                                        );
                                        continue;
                                    }
                                }
                            }

                            let temp_line = ExtractWevtTempOutputLine {
                                source: source_str.clone(),
                                resource: match &res.resource {
                                    ResourceIdentifier::Id(id) => ResourceIdJson::Id(*id),
                                    ResourceIdentifier::Name(name) => {
                                        ResourceIdJson::Name(name.clone())
                                    }
                                },
                                lang_id: res.lang_id,
                                ttbl_offset: *ttbl_offset,
                                temp_offset: tpl.offset,
                                temp_size: tpl.size,
                                item_descriptor_count: tpl.item_descriptor_count,
                                item_name_count: tpl.item_name_count,
                                template_items_offset: tpl.template_items_offset,
                                event_type: tpl.event_type,
                                guid: guid_display.clone(),
                                output_path: temp_path.to_string_lossy().to_string(),
                            };

                            println!("{}", serde_json::to_string(&temp_line)?);
                        }

                        if dump_temp_xml {
                            let xml_name = format!(
                                "{base}.{hash:08x}.wevt_template.{res_id}.lang_{lang}.temp_{idx:04}.{guid}.xml",
                                base = path
                                    .file_name()
                                    .map(|s| s.to_string_lossy())
                                    .unwrap_or_else(|| "unknown".into()),
                                hash = source_hash,
                                res_id = resource_id_str,
                                lang = res.lang_id,
                                idx = idx,
                                guid = guid_file,
                            );
                            let xml_path = templates_xml_dir.join(xml_name);

                            if overwrite || !xml_path.exists() {
                                match render_template_definition_to_xml(
                                    tpl,
                                    encoding::all::WINDOWS_1252,
                                ) {
                                    Ok(xml) => {
                                        if let Err(e) = fs::write(&xml_path, xml.as_bytes()) {
                                            error_count += 1;
                                            eprintln!(
                                                "failed to write `{}`: {e}",
                                                xml_path.to_string_lossy()
                                            );
                                            continue;
                                        }
                                    }
                                    Err(e) => {
                                        error_count += 1;
                                        eprintln!(
                                            "failed to render TEMP XML for `{}` (temp_index={}, guid={}): {e}",
                                            source_str, idx, guid_display
                                        );
                                        continue;
                                    }
                                }
                            }

                            let xml_line = ExtractWevtTempXmlOutputLine {
                                source: source_str.clone(),
                                resource: match &res.resource {
                                    ResourceIdentifier::Id(id) => ResourceIdJson::Id(*id),
                                    ResourceIdentifier::Name(name) => {
                                        ResourceIdJson::Name(name.clone())
                                    }
                                },
                                lang_id: res.lang_id,
                                temp_index: idx,
                                guid: guid_display,
                                output_path: xml_path.to_string_lossy().to_string(),
                            };
                            println!("{}", serde_json::to_string(&xml_line)?);
                        }
                    }
                }
            }
        }

        if error_count > 0 {
            bail!(
                "extract-wevt-templates completed with {error_count} error(s) (extracted {extracted_count} resource blob(s))"
            );
        }

        eprintln!("extracted {extracted_count} resource blob(s)");
        Ok(())
    }

    fn collect_input_paths(
        input: &Path,
        recursive: bool,
        allowed_exts: &std::collections::HashSet<String>,
        seen: &mut std::collections::HashSet<PathBuf>,
        out_files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        use std::collections::VecDeque;

        if !input.exists() {
            return Ok(());
        }

        if input.is_file() {
            // For explicit files (or glob matches that are files), do not apply extension filtering.
            // Users often point to unusual extensions (e.g. `services.exe` renamed to `.gif`).
            let p = input.to_path_buf();
            if seen.insert(p.clone()) {
                out_files.push(p);
            }
            return Ok(());
        }

        if input.is_dir() {
            if !recursive {
                // Directory input without recursion is ambiguous; ignore silently.
                return Ok(());
            }

            let mut queue = VecDeque::new();
            queue.push_back(input.to_path_buf());

            while let Some(dir) = queue.pop_front() {
                let entries = match fs::read_dir(&dir) {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                for entry in entries.flatten() {
                    let p = entry.path();
                    if p.is_dir() {
                        queue.push_back(p);
                    } else if p.is_file()
                        && should_keep_file(&p, allowed_exts)
                        && seen.insert(p.clone())
                    {
                        out_files.push(p);
                    }
                }
            }
        }

        Ok(())
    }

    fn should_keep_file(path: &Path, allowed_exts: &std::collections::HashSet<String>) -> bool {
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            return false;
        };
        allowed_exts.contains(&ext.to_ascii_lowercase())
    }

    fn sanitize_component(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        let mut last_underscore = false;
        for ch in s.chars() {
            let keep = ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' || ch == '_';
            if keep {
                out.push(ch);
                last_underscore = false;
            } else if !last_underscore {
                out.push('_');
                last_underscore = true;
            }
        }
        if out.is_empty() {
            "unnamed".to_string()
        } else {
            out
        }
    }
}

#[cfg(feature = "wevt_templates")]
use imp::run_impl;
