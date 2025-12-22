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
#[cfg(feature = "wevt_templates")]
use serde::Serialize;
use std::fs::{self, File};
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tempfile::tempfile;

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
                for record in parser.records() {
                    self.dump_record(record)?
                }
            }
            EvtxOutputFormat::JSON => {
                match self.json_parser {
                    JsonParserKind::Streaming => {
                        for record in parser.records_json_stream() {
                            self.dump_record(record)?
                        }
                    }
                    JsonParserKind::Legacy => {
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

    let matches = Command::new("EVTX Parser")
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
        )
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
        .subcommand({
            let cmd = Command::new("extract-wevt-templates")
                .about("Extract WEVT_TEMPLATE resources from PE files (EXE/DLL)")
                .long_about(indoc!(r#"
                    Extract WEVT_TEMPLATE resources from PE files (EXE/DLL).

                    This is intended to support building an offline cache of EVTX templates
                    (see issue #103), without committing to any database format yet.

                    NOTE: this subcommand is gated behind the `wevt_templates` Cargo feature.
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
        })
        .get_matches();

    if let Some(("extract-wevt-templates", sub_matches)) = matches.subcommand() {
        return run_extract_wevt_templates(sub_matches);
    }

    if matches.get_one::<String>("INPUT").is_none() {
        bail!("Missing INPUT. Provide an EVTX file path, or use a subcommand (try `--help`).");
    }

    EvtxDump::from_cli_matches(&matches)?.run()?;

    Ok(())
}

#[cfg(feature = "wevt_templates")]
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
enum ResourceIdJson {
    Id(u32),
    Name(String),
}

#[cfg(feature = "wevt_templates")]
#[derive(Debug, Serialize)]
struct ExtractWevtTemplatesOutputLine {
    source: String,
    resource: ResourceIdJson,
    lang_id: u32,
    output_path: String,
    size: usize,
}

#[cfg(feature = "wevt_templates")]
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

#[cfg(feature = "wevt_templates")]
#[derive(Debug, Serialize)]
struct ExtractWevtTempXmlOutputLine {
    source: String,
    resource: ResourceIdJson,
    lang_id: u32,
    temp_index: usize,
    guid: String,
    output_path: String,
}

#[cfg(feature = "wevt_templates")]
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

#[cfg(feature = "wevt_templates")]
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

fn run_extract_wevt_templates(matches: &ArgMatches) -> Result<()> {
    #[cfg(feature = "wevt_templates")]
    {
        run_extract_wevt_templates_impl(matches)
    }

    #[cfg(not(feature = "wevt_templates"))]
    {
        let _ = matches;
        bail!(
            "This subcommand requires building with Cargo feature `wevt_templates`.\n\
             Example:\n\
               cargo run --features wevt_templates --bin evtx_dump -- extract-wevt-templates ..."
        );
    }
}

#[cfg(feature = "wevt_templates")]
fn run_extract_wevt_templates_impl(matches: &ArgMatches) -> Result<()> {
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
            for entry in glob::glob(pat).with_context(|| format!("invalid glob pattern `{pat}`"))? {
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

#[cfg(feature = "wevt_templates")]
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

#[cfg(feature = "wevt_templates")]
fn should_keep_file(path: &Path, allowed_exts: &std::collections::HashSet<String>) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    allowed_exts.contains(&ext.to_ascii_lowercase())
}

#[cfg(feature = "wevt_templates")]
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
