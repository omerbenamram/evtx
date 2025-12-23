#![allow(clippy::upper_case_acronyms)]

use anyhow::{Context, Result, bail, format_err};
use clap::{Arg, ArgAction, ArgMatches, Command};
use dialoguer::Confirm;
use indoc::indoc;

use encoding::all::encodings;
use encoding::types::Encoding;
use evtx::err::Result as EvtxResult;
use evtx::{EvtxParser, ParserSettings, SerializedEvtxRecord};
use log::Level;
use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tempfile::tempfile;

#[path = "evtx_dump/apply_wevt_cache.rs"]
mod apply_wevt_cache;
#[path = "evtx_dump/dump_template_instances.rs"]
mod dump_template_instances;
#[path = "evtx_dump/extract_wevt_templates.rs"]
mod extract_wevt_templates;

#[cfg(feature = "wevt_templates")]
#[path = "evtx_dump/wevt_cache.rs"]
mod wevt_cache;

#[cfg(all(not(target_env = "msvc"), feature = "fast-alloc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(not(target_env = "msvc"), feature = "fast-alloc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(target_env = "msvc", feature = "fast-alloc"))]
#[global_allocator]
static ALLOC: rpmalloc::RpMalloc = rpmalloc::RpMalloc;

#[derive(Copy, Clone, PartialOrd, PartialEq, Eq)]
pub enum EvtxOutputFormat {
    JSON,
    XML,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum JsonParserKind {
    /// Original JSON path: builds a full `serde_json::Value` per record.
    Legacy,
    /// Streaming JSON path: writes JSON directly to the output writer.
    Streaming,
}

struct EvtxDump {
    parser_settings: ParserSettings,
    input: PathBuf,
    show_record_number: bool,
    output_format: EvtxOutputFormat,
    output: Box<dyn Write>,
    json_parser: JsonParserKind,
    verbosity_level: Option<Level>,
    stop_after_error: bool,
    /// When set, only the specified events (offseted reltaive to file) will be outputted.
    ranges: Option<Ranges>,
    #[cfg(feature = "wevt_templates")]
    wevt_cache: Option<std::sync::Arc<wevt_cache::WevtCache>>,
}

impl EvtxDump {
    pub fn from_cli_matches(matches: &ArgMatches) -> Result<Self> {
        let input = PathBuf::from(
            matches
                .get_one::<String>("INPUT")
                .expect("This is a required argument"),
        );

        let output_format = match matches
            .get_one::<String>("output-format")
            .expect("has default")
            .as_str()
        {
            "xml" => EvtxOutputFormat::XML,
            "json" | "jsonl" => EvtxOutputFormat::JSON,
            _ => EvtxOutputFormat::XML,
        };

        let json_parser = match matches.get_one::<String>("json-parser").map(|s| s.as_str()) {
            Some("legacy") => JsonParserKind::Legacy,
            Some("streaming") | None => JsonParserKind::Streaming,
            _ => JsonParserKind::Streaming,
        };

        let no_indent = match (
            matches.get_flag("no-indent"),
            matches.get_one::<String>("output-format"),
        ) {
            // "jsonl" --> --no-indent
            (false, Some(fmt)) => fmt == "jsonl",
            (true, Some(fmt)) => {
                if fmt == "jsonl" {
                    eprintln!("no need to pass both `--no-indent` and `-o jsonl`");
                    true
                } else {
                    true
                }
            }
            (v, None) => v,
        };

        let separate_json_attrib_flag = matches.get_flag("separate-json-attributes");

        let no_show_record_number = match (
            matches.get_flag("no-show-record-number"),
            matches.get_one::<String>("output-format"),
        ) {
            // "jsonl" --> --no-show-record-number
            (false, Some(fmt)) => fmt == "jsonl",
            (true, Some(fmt)) => {
                if fmt == "jsonl" {
                    eprintln!("no need to pass both `--no-show-record-number` and `-o jsonl`");
                    true
                } else {
                    true
                }
            }
            (v, None) => v,
        };

        let num_threads: u32 = *matches.get_one("num-threads").expect("has default");

        let num_threads = match (cfg!(feature = "multithreading"), num_threads) {
            (true, number) => number,
            (false, _) => {
                eprintln!(
                    "turned on threads, but library was compiled without `multithreading` feature! using fallback sync iterator"
                );
                1
            }
        };

        let validate_checksums = matches.get_flag("validate-checksums");
        let stop_after_error = matches.get_flag("stop-after-one-error");

        let event_ranges = matches.get_one::<Ranges>("event-ranges").cloned();

        let verbosity_level = match matches.get_count("verbose") {
            0 => None,
            1 => Some(Level::Info),
            2 => Some(Level::Debug),
            3 => Some(Level::Trace),
            _ => {
                eprintln!("using more than  -vvv does not affect verbosity level");
                Some(Level::Trace)
            }
        };

        let ansi_codec = encodings()
            .iter()
            .find(|c| {
                c.name()
                    == matches
                        .get_one::<String>("ansi-codec")
                        .expect("has set default")
                        .as_str()
            })
            .expect("possible values are derived from `encodings()`");

        let output: Box<dyn Write> = if let Some(path) = matches.get_one::<String>("output-target")
        {
            Box::new(BufWriter::new(
                Self::create_output_file(path, !matches.get_flag("no-confirm-overwrite"))
                    .with_context(|| {
                        format!("An error occurred while creating output file at `{}`", path)
                    })?,
            ))
        } else {
            Box::new(BufWriter::new(io::stdout()))
        };

        #[cfg(feature = "wevt_templates")]
        let wevt_cache = matches
            .get_one::<String>("wevt-cache-index")
            .map(|p| wevt_cache::WevtCache::load(p).map(std::sync::Arc::new))
            .transpose()?;

        Ok(EvtxDump {
            parser_settings: ParserSettings::new()
                .num_threads(num_threads.try_into().expect("u32 -> usize"))
                .validate_checksums(validate_checksums)
                .separate_json_attributes(separate_json_attrib_flag)
                .indent(!no_indent)
                .ansi_codec(*ansi_codec),
            input,
            show_record_number: !no_show_record_number,
            output_format,
            output,
            json_parser,
            verbosity_level,
            stop_after_error,
            ranges: event_ranges,
            #[cfg(feature = "wevt_templates")]
            wevt_cache,
        })
    }

    /// Main entry point for `EvtxDump`
    pub fn run(&mut self) -> Result<()> {
        if let Err(err) = self.try_to_initialize_logging() {
            eprintln!("{:?}", err);
        }

        let mut parser = self
            .open_parser()
            .map(|parser| parser.with_configuration(self.parser_settings.clone()))?;

        match self.output_format {
            EvtxOutputFormat::XML => {
                #[cfg(feature = "wevt_templates")]
                if let Some(cache) = self.wevt_cache.clone() {
                    let iter = parser.serialized_records(move |record_res| {
                        record_res
                            .and_then(|record| render_record_xml_with_wevt_cache(record, &cache))
                    });
                    for record in iter {
                        self.dump_record(record)?
                    }
                } else {
                    for record in parser.records() {
                        self.dump_record(record)?
                    }
                }

                #[cfg(not(feature = "wevt_templates"))]
                for record in parser.records() {
                    self.dump_record(record)?
                }
            }
            EvtxOutputFormat::JSON => {
                match self.json_parser {
                    JsonParserKind::Streaming => {
                        #[cfg(feature = "wevt_templates")]
                        if let Some(cache) = self.wevt_cache.clone() {
                            let indent = self.parser_settings.should_indent();
                            let iter = parser.serialized_records(move |record_res| {
                                record_res.and_then(|record| {
                                    render_record_json_with_wevt_cache(record, &cache, indent, true)
                                })
                            });
                            for record in iter {
                                self.dump_record(record)?
                            }
                        } else {
                            for record in parser.records_json_stream() {
                                self.dump_record(record)?
                            }
                        }

                        #[cfg(not(feature = "wevt_templates"))]
                        for record in parser.records_json_stream() {
                            self.dump_record(record)?
                        }
                    }
                    JsonParserKind::Legacy => {
                        #[cfg(feature = "wevt_templates")]
                        if let Some(cache) = self.wevt_cache.clone() {
                            let indent = self.parser_settings.should_indent();
                            let iter = parser.serialized_records(move |record_res| {
                                record_res.and_then(|record| {
                                    render_record_json_with_wevt_cache(
                                        record, &cache, indent, false,
                                    )
                                })
                            });
                            for record in iter {
                                self.dump_record(record)?
                            }
                        } else {
                            for record in parser.records_json() {
                                self.dump_record(record)?
                            }
                        }

                        #[cfg(not(feature = "wevt_templates"))]
                        for record in parser.records_json() {
                            self.dump_record(record)?
                        }
                    }
                };
            }
        };

        Ok(())
    }

    fn open_parser(&self) -> Result<EvtxParser<File>> {
        if Self::is_stdin_input(&self.input) {
            let mut tmp =
                tempfile().context("Failed to create temporary file for stdin buffering")?;

            let mut stdin = io::stdin().lock();
            let bytes_copied =
                io::copy(&mut stdin, &mut tmp).context("Failed to read EVTX data from stdin")?;

            if bytes_copied == 0 {
                bail!("No input received on stdin");
            }

            tmp.seek(SeekFrom::Start(0))
                .context("Failed to rewind stdin buffer")?;

            EvtxParser::from_read_seek(tmp).context("Failed to parse EVTX data from stdin")
        } else {
            EvtxParser::from_path(&self.input)
                .with_context(|| format!("Failed to open evtx file at: {}", &self.input.display()))
        }
    }

    fn is_stdin_input(path: &Path) -> bool {
        if path.as_os_str() == "-" {
            return true;
        }

        // Common Unix aliases for stdin.
        // Note: we intentionally accept these even if stdin is seekable (e.g. redirected from a file),
        // since the intent is explicit and buffering is still correct.
        path == Path::new("/dev/stdin")
            || path == Path::new("/dev/fd/0")
            || path == Path::new("/proc/self/fd/0")
    }

    /// If `prompt` is passed, will display a confirmation prompt before overwriting files.
    fn create_output_file(path: impl AsRef<Path>, prompt: bool) -> Result<File> {
        let p = path.as_ref();

        if p.is_dir() {
            bail!(
                "There is a directory at {}, refusing to overwrite",
                p.display()
            );
        }

        if p.exists() {
            if prompt {
                match Confirm::new()
                    .with_prompt(format!(
                        "Are you sure you want to override output file at {}",
                        p.display()
                    ))
                    .default(false)
                    .interact()
                {
                    Ok(true) => Ok(File::create(p)?),
                    Ok(false) => bail!("Cancelled"),
                    Err(_e) => bail!("Failed to display confirmation prompt"),
                }
            } else {
                Ok(File::create(p)?)
            }
        } else {
            // Ok to assume p is not an existing directory
            match p.parent() {
                Some(parent) =>
                // Parent exist
                {
                    if !parent.exists() {
                        fs::create_dir_all(parent)?;
                    }

                    Ok(File::create(p)?)
                }
                None => bail!("Output file cannot be root."),
            }
        }
    }

    fn dump_record(&mut self, record: EvtxResult<SerializedEvtxRecord<String>>) -> Result<()> {
        match record.with_context(|| "Failed to dump the next record.") {
            Ok(r) => {
                let range_filter = if let Some(ranges) = &self.ranges {
                    ranges.contains(&(r.event_record_id as usize))
                } else {
                    true
                };

                if range_filter {
                    if self.show_record_number {
                        writeln!(self.output, "Record {}", r.event_record_id)?;
                    }
                    writeln!(self.output, "{}", r.data)?;
                }
            }
            // This error is non fatal.
            Err(e) => {
                eprintln!("{:?}", format_err!(e));

                if self.stop_after_error {
                    std::process::exit(1);
                }
            }
        };

        Ok(())
    }

    fn try_to_initialize_logging(&self) -> Result<()> {
        if let Some(level) = self.verbosity_level {
            simplelog::WriteLogger::init(
                level.to_level_filter(),
                simplelog::Config::default(),
                io::stderr(),
            )
            .with_context(|| "Failed to initialize logging")?;
        }

        Ok(())
    }
}

#[derive(Clone)]
struct Ranges(Vec<RangeInclusive<usize>>);

impl Ranges {
    fn contains(&self, number: &usize) -> bool {
        self.0.iter().any(|r| r.contains(number))
    }
}

impl FromStr for Ranges {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut res = vec![];

        for range in s.split(',') {
            if range.contains('-') {
                let numbers = range.split('-').collect::<Vec<_>>();
                let (rstart, rstop) = (numbers.first(), numbers.get(1));

                // verify rstart, rstop are numbers
                if let (Some(rstart), Some(rstop)) = (rstart, rstop) {
                    if rstart.parse::<usize>().is_err() || rstop.parse::<usize>().is_err() {
                        bail!("Expected range to be a positive number, got: {}", range);
                    }
                } else {
                    bail!("Expected range to be a positive number, got: {}", range);
                }

                if numbers.len() != 2 {
                    bail!(
                        "Expected either a single number or range of numbers, but got: {}",
                        range
                    );
                }

                if rstart.is_none() || rstop.is_none() {
                    bail!(
                        "Expected range to be in the form of `start-stop`, got `{}`",
                        range
                    );
                }

                res.push(
                    rstart.unwrap().parse::<usize>().unwrap()
                        ..=rstop.unwrap().parse::<usize>().unwrap(),
                );
            } else {
                match range.parse::<usize>() {
                    Ok(r) => res.push(r..=r),
                    Err(_) => bail!("Expected range to be a positive number, got: {}", range),
                };
            }
        }

        Ok(Ranges(res))
    }
}

fn matches_ranges(value: &str) -> Result<Ranges, String> {
    Ranges::from_str(value).map_err(|e| e.to_string())
}

#[test]
fn test_ranges() {
    assert!(matches_ranges("1-2,3,4-5,6-7,8-9").is_ok());
    assert!(matches_ranges("1").is_ok());
    assert!(matches_ranges("1-").is_err());
    assert!(matches_ranges("-2").is_err());
}

fn main() -> Result<()> {
    let all_encoings = encodings()
        .iter()
        .filter(|&e| e.raw_decoder().is_ascii_compatible())
        .map(|e| e.name())
        .collect::<Vec<&'static str>>();

    let cmd = Command::new("EVTX Parser")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Omer B. <omerbenamram@gmail.com>")
        .about("Utility to parse EVTX files")
        .arg(
            Arg::new("INPUT")
                .required(false)
                .help("Input EVTX file path, or '-' to read from stdin. Required unless using a subcommand."),
        )
        .arg(
            Arg::new("num-threads")
                .short('t')
                .long("threads")
                .default_value("0")
                .value_parser(clap::value_parser!(u32).range(0..))
                .help("Sets the number of worker threads, defaults to number of CPU cores."),
        )
        .arg(
            Arg::new("output-format")
                .short('o')
                .long("format")
                .value_parser(["json", "xml", "jsonl"])
                .default_value("xml")
                .help("Sets the output format")
                .long_help(indoc!(
                r#"Sets the output format:
                     "xml"   - prints XML output.
                     "json"  - prints JSON output.
                     "jsonl" - (jsonlines) same as json with --no-indent --dont-show-record-number
                "#)),
        )
        .arg(
            Arg::new("json-parser")
                .long("json-parser")
                .value_parser(["legacy", "streaming"])
                .default_value("streaming")
                .help("Select JSON parser implementation: legacy (tree-based) or streaming"),
        )
        .arg(
            Arg::new("output-target")
                .long("output")
                .short('f')
                .action(ArgAction::Set)
                .help(indoc!("Writes output to the file specified instead of stdout, errors will still be printed to stderr.
                       Will ask for confirmation before overwriting files, to allow overwriting, pass `--no-confirm-overwrite`
                       Will create parent directories if needed.")),
        )
        .arg(
            Arg::new("no-confirm-overwrite")
                .long("no-confirm-overwrite")
                .action(ArgAction::SetTrue)
                .help(indoc!("When set, will not ask for confirmation before overwriting files, useful for automation")),
        )
        .arg(
            Arg::new("event-ranges")
                .long("events")
                .action(ArgAction::Set)
                .value_parser(matches_ranges)
                .help(indoc!("When set, only the specified events (offseted reltaive to file) will be outputted.
                For example:
                    --events=1 will output the first event.
                    --events=0-10,20-30 will output events 0-10 and 20-30.
                ")),
        )
        .arg(
            Arg::new("validate-checksums")
                .long("validate-checksums")
                .action(ArgAction::SetTrue)
                .help(indoc!("When set, chunks with invalid checksums will not be parsed. \
                Usually dirty files have bad checksums, so using this flag will result in fewer records.")),
        )
        .arg(
            Arg::new("no-indent")
                .long("no-indent")
                .action(ArgAction::SetTrue)
                .help("When set, output will not be indented."),
        )
        .arg(
            Arg::new("separate-json-attributes")
                .long("separate-json-attributes")
                .action(ArgAction::SetTrue)
                .help("If outputting JSON, XML Element's attributes will be stored in a separate object named '<ELEMENTNAME>_attributes', with <ELEMENTNAME> containing the value of the node."),
        )
        .arg(
            Arg::new("no-show-record-number")
                .long("dont-show-record-number")
                .action(ArgAction::SetTrue)
                .help("When set, `Record <id>` will not be printed."),
        )
        .arg(
            Arg::new("ansi-codec")
                .long("ansi-codec")
                .value_parser(all_encoings)
                .default_value(encoding::all::WINDOWS_1252.name())
                .help("When set, controls the codec of ansi encoded strings the file."),
        );

    // Optional: when provided, use an offline WEVT template cache as a fallback for records
    // whose embedded EVTX templates are missing/corrupt (common in carved/dirty logs).
    #[cfg(feature = "wevt_templates")]
    let cmd = cmd.arg(
        Arg::new("wevt-cache-index")
            .long("wevt-cache-index")
            .value_name("INDEX_JSONL")
            .help("Path to a WEVT template cache index JSONL (from `extract-wevt-templates`). When set, evtx_dump will try to render records using this cache if the embedded EVTX template expansion fails."),
    );

    let matches = cmd
        .arg(
            Arg::new("stop-after-one-error")
                .long("stop-after-one-error")
                .action(ArgAction::SetTrue)
                .help("When set, will exit after any failure of reading a record. Useful for debugging."),
        )
        .arg(Arg::new("verbose")
            .short('v')
            .action(ArgAction::Count)
            .help(indoc!(r#"
            Sets debug prints level for the application:
                -v   - info
                -vv  - debug
                -vvv - trace
            NOTE: trace output is only available in debug builds, as it is extremely verbose."#))
        )
        .subcommand(extract_wevt_templates::command())
        .subcommand(dump_template_instances::command())
        .subcommand(apply_wevt_cache::command())
        .get_matches();

    if let Some(("extract-wevt-templates", sub_matches)) = matches.subcommand() {
        return extract_wevt_templates::run(sub_matches);
    }

    if let Some(("dump-template-instances", sub_matches)) = matches.subcommand() {
        return dump_template_instances::run(sub_matches);
    }

    if let Some(("apply-wevt-cache", sub_matches)) = matches.subcommand() {
        return apply_wevt_cache::run(sub_matches);
    }

    if matches.get_one::<String>("INPUT").is_none() {
        bail!("Missing INPUT. Provide an EVTX file path, or use a subcommand (try `--help`).");
    }

    EvtxDump::from_cli_matches(&matches)?.run()?;

    Ok(())
}

#[cfg(feature = "wevt_templates")]
fn extract_template_guid_from_error(err: &evtx::err::EvtxError) -> Option<String> {
    use evtx::err::{DeserializationError, EvtxError};
    match err {
        EvtxError::FailedToParseRecord { source, .. } => extract_template_guid_from_error(source),
        EvtxError::DeserializationError(DeserializationError::FailedToDeserializeTemplate {
            template_id,
            ..
        }) => Some(template_id.to_string()),
        _ => None,
    }
}

#[cfg(feature = "wevt_templates")]
fn binxml_value_to_string_lossy(value: &evtx::binxml::value_variant::BinXmlValue<'_>) -> String {
    use evtx::binxml::value_variant::BinXmlValue;
    match value {
        BinXmlValue::EvtHandle | BinXmlValue::BinXmlType(_) | BinXmlValue::EvtXml => String::new(),
        _ => value.as_cow_str().into_owned(),
    }
}

#[cfg(feature = "wevt_templates")]
fn substitutions_from_template_instance<'a>(
    tpl: &evtx::model::deserialized::BinXmlTemplateRef<'a>,
) -> Vec<String> {
    use evtx::model::deserialized::BinXMLDeserializedTokens;
    tpl.substitution_array
        .iter()
        .map(|t| match t {
            BinXMLDeserializedTokens::Value(v) => binxml_value_to_string_lossy(v),
            _ => String::new(),
        })
        .collect()
}

#[cfg(feature = "wevt_templates")]
fn resolve_template_guid_from_record<'a>(
    record: &evtx::EvtxRecord<'a>,
    tpl: &evtx::model::deserialized::BinXmlTemplateRef<'a>,
) -> Option<String> {
    if let Some(g) = tpl.template_guid.as_ref() {
        return Some(g.to_string());
    }

    record
        .chunk
        .template_table
        .get_template(tpl.template_def_offset)
        .map(|def| def.header.guid.to_string())
}

#[cfg(feature = "wevt_templates")]
struct TemplateInstanceInfo {
    /// Normalized GUID (lowercased, braces stripped) if we can resolve it.
    guid: Option<String>,
    substitutions: Vec<String>,
}

#[cfg(feature = "wevt_templates")]
fn collect_template_instances<'a>(record: &evtx::EvtxRecord<'a>) -> Vec<TemplateInstanceInfo> {
    use evtx::model::deserialized::BinXMLDeserializedTokens;
    let mut out = Vec::new();

    for t in &record.tokens {
        let BinXMLDeserializedTokens::TemplateInstance(tpl) = t else {
            continue;
        };

        let guid =
            resolve_template_guid_from_record(record, tpl).map(|g| wevt_cache::normalize_guid(&g));
        let substitutions = substitutions_from_template_instance(tpl);

        out.push(TemplateInstanceInfo {
            guid,
            substitutions,
        });
    }

    out
}

#[cfg(feature = "wevt_templates")]
fn select_template_instance_for_guid<'a>(
    instances: &'a [TemplateInstanceInfo],
    guid: &str,
) -> Option<&'a TemplateInstanceInfo> {
    let want = wevt_cache::normalize_guid(guid);

    match instances.len() {
        0 => None,
        1 => Some(&instances[0]),
        _ => {
            let matches: Vec<&TemplateInstanceInfo> = instances
                .iter()
                .filter(|i| i.guid.as_ref().is_some_and(|g| g == &want))
                .collect();

            if matches.len() == 1 {
                Some(matches[0])
            } else {
                None
            }
        }
    }
}

#[cfg(feature = "wevt_templates")]
/// Render a record as XML, using the EVTX’s embedded templates first.
///
/// If rendering fails *specifically because a template definition cannot be deserialized* and the
/// error contains a concrete template GUID, we will deterministically attempt to render the record
/// using the provided offline WEVT cache:
/// - We only use the cache when the error is `FailedToDeserializeTemplate { template_id: GUID }`.
/// - We only proceed when we can unambiguously select the matching `TemplateInstance` substitution
///   array for that GUID (single instance, or a unique GUID match among multiple instances).
/// - Otherwise we return the original error unchanged.
fn render_record_xml_with_wevt_cache<'a>(
    record: evtx::EvtxRecord<'a>,
    cache: &std::sync::Arc<wevt_cache::WevtCache>,
) -> evtx::err::Result<SerializedEvtxRecord<String>> {
    let record_id = record.event_record_id;
    let timestamp = record.timestamp;
    let instances = collect_template_instances(&record);

    match record.into_xml() {
        Ok(r) => Ok(r),
        Err(e) => {
            // Deterministic rule: only attempt cache rendering when the failure explicitly
            // indicates a template GUID (i.e. template deserialization failure).
            let Some(guid) = extract_template_guid_from_error(&e) else {
                return Err(e);
            };

            let Some(tpl) = select_template_instance_for_guid(&instances, &guid) else {
                return Err(e);
            };
            let subs = &tpl.substitutions;

            match cache.render_by_template_guid(&guid, subs) {
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
                    eprintln!(
                        "wevt-cache render failed for record {} template_guid={}: {render_err}",
                        record_id, guid
                    );
                    Err(e)
                }
            }
        }
    }
}

#[cfg(feature = "wevt_templates")]
/// Render a record as JSON, using the EVTX’s embedded templates first.
///
/// This follows the same deterministic WEVT-cache rule as `render_record_xml_with_wevt_cache`:
/// only on an explicit template-GUID deserialization failure and only with an unambiguous
/// `TemplateInstance` substitution array.
///
/// When the cache is used, the JSON output is a synthetic object that contains the rendered XML
/// fragment under `xml` (and includes metadata fields like `template_guid` and `record_id`).
fn render_record_json_with_wevt_cache<'a>(
    record: evtx::EvtxRecord<'a>,
    cache: &std::sync::Arc<wevt_cache::WevtCache>,
    indent: bool,
    use_streaming_json: bool,
) -> evtx::err::Result<SerializedEvtxRecord<String>> {
    let record_id = record.event_record_id;
    let timestamp = record.timestamp;
    let instances = collect_template_instances(&record);

    let normal = if use_streaming_json {
        record.into_json_stream()
    } else {
        record.into_json()
    };

    match normal {
        Ok(r) => Ok(r),
        Err(e) => {
            let Some(guid) = extract_template_guid_from_error(&e) else {
                return Err(e);
            };
            let Some(tpl) = select_template_instance_for_guid(&instances, &guid) else {
                return Err(e);
            };
            let subs = &tpl.substitutions;

            match cache.render_by_template_guid(&guid, subs) {
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
                            .map_err(evtx::err::SerializationError::from)?
                    } else {
                        serde_json::to_string(&v).map_err(evtx::err::SerializationError::from)?
                    };

                    Ok(SerializedEvtxRecord {
                        event_record_id: record_id,
                        timestamp,
                        data,
                    })
                }
                Err(render_err) => {
                    eprintln!(
                        "wevt-cache render failed for record {} template_guid={}: {render_err}",
                        record_id, guid
                    );
                    Err(e)
                }
            }
        }
    }
}
