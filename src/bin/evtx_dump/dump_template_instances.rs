use anyhow::{Context, Result};
use clap::{Arg, ArgMatches, Command};
use indoc::indoc;

pub fn command() -> Command {
    Command::new("dump-template-instances")
        .about("Dump BinXML TemplateInstance substitution arrays from EVTX records (JSONL)")
        .long_about(indoc!(
            r#"
            Dump BinXML TemplateInstance substitution arrays from EVTX records as JSONL.

            This is useful for offline rendering workflows where you have a template cache
            (from `extract-wevt-templates`) and need the record's substitution values array.
        "#
        ))
        .arg(
            Arg::new("input")
                .long("input")
                .short('i')
                .required(true)
                .value_name("EVTX")
                .help("Input EVTX file path."),
        )
        .arg(
            Arg::new("record-id")
                .long("record-id")
                .value_parser(clap::value_parser!(u64).range(0..))
                .value_name("ID")
                .help("Only dump template instances for the specified event record id."),
        )
        .arg(
            Arg::new("template-instance-index")
                .long("template-instance-index")
                .value_parser(clap::value_parser!(usize))
                .default_value("0")
                .value_name("N")
                .help("When a record contains multiple TemplateInstance tokens, select which one to dump (default: 0)."),
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
        anyhow::bail!(
            "This subcommand requires building with Cargo feature `wevt_templates`.\n\
             Example:\n\
               cargo run --features wevt_templates --bin evtx_dump -- dump-template-instances ..."
        );
    }
}

#[cfg(feature = "wevt_templates")]
mod imp {
    use super::*;
    use evtx::{EvtxParser, ParserSettings};
    use serde::Serialize;
    use serde_json::Value as JsonValue;
    use std::path::PathBuf;

    #[derive(Debug, Serialize)]
    struct DumpTemplateInstanceOutputLine {
        source: String,
        record_id: u64,
        timestamp: String,
        template_instance_index: usize,
        template_id: u32,
        template_def_offset: u32,
        template_guid: Option<String>,
        substitutions: Vec<JsonValue>,
    }

    fn binxml_value_to_json_lossy(
        value: &evtx::binxml::value_variant::BinXmlValue<'_>,
    ) -> JsonValue {
        use evtx::binxml::value_variant::BinXmlValue;
        match value {
            BinXmlValue::EvtHandle => JsonValue::Object(
                [(
                    "type".to_string(),
                    JsonValue::String("EvtHandle".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
            BinXmlValue::BinXmlType(_) => JsonValue::Object(
                [(
                    "type".to_string(),
                    JsonValue::String("BinXmlType".to_string()),
                )]
                .into_iter()
                .collect(),
            ),
            BinXmlValue::EvtXml => JsonValue::Object(
                [("type".to_string(), JsonValue::String("EvtXml".to_string()))]
                    .into_iter()
                    .collect(),
            ),
            other => JsonValue::from(other),
        }
    }

    pub(super) fn run_impl(matches: &ArgMatches) -> Result<()> {
        use evtx::model::deserialized::BinXMLDeserializedTokens;

        let input = PathBuf::from(matches.get_one::<String>("input").expect("required"));
        let record_id_filter = matches.get_one::<u64>("record-id").copied();
        let template_instance_index: usize = *matches
            .get_one::<usize>("template-instance-index")
            .expect("has default");

        let settings = ParserSettings::default();
        let mut parser = EvtxParser::from_path(&input)
            .with_context(|| format!("Failed to open evtx file at: {}", input.display()))?
            .with_configuration(settings.clone());

        let source = input.to_string_lossy().to_string();

        for chunk_res in parser.chunks() {
            let mut chunk_data = match chunk_res {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    continue;
                }
            };

            let mut chunk = match chunk_data.parse(std::sync::Arc::new(settings.clone())) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("{e}");
                    continue;
                }
            };

            for record_res in chunk.iter() {
                let record = match record_res {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{e}");
                        continue;
                    }
                };

                if record_id_filter.is_some_and(|want| record.event_record_id != want) {
                    continue;
                }

                let mut instances = vec![];
                for t in &record.tokens {
                    if let BinXMLDeserializedTokens::TemplateInstance(tpl) = t {
                        instances.push(tpl);
                    }
                }

                let Some(tpl) = instances.get(template_instance_index) else {
                    continue;
                };

                let mut substitutions = Vec::with_capacity(tpl.substitution_array.len());
                for s in &tpl.substitution_array {
                    match s {
                        BinXMLDeserializedTokens::Value(v) => {
                            substitutions.push(binxml_value_to_json_lossy(v))
                        }
                        other => substitutions.push(JsonValue::String(format!("{other:?}"))),
                    }
                }

                let line = DumpTemplateInstanceOutputLine {
                    source: source.clone(),
                    record_id: record.event_record_id,
                    timestamp: record.timestamp.to_rfc3339(),
                    template_instance_index,
                    template_id: tpl.template_id,
                    template_def_offset: tpl.template_def_offset,
                    template_guid: tpl.template_guid.as_ref().map(|g| g.to_string()),
                    substitutions,
                };

                println!("{}", serde_json::to_string(&line)?);

                if record_id_filter.is_some() {
                    // Found the record we wanted; keep going anyway in case there are duplicates? No.
                    return Ok(());
                }
            }
        }

        Ok(())
    }
}

#[cfg(feature = "wevt_templates")]
use imp::run_impl;
