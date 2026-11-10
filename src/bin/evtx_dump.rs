#![allow(clippy::upper_case_acronyms)]

use anyhow::{Context, Result, bail, format_err};
use clap::{Arg, ArgAction, ArgMatches, Command};
use dialoguer::Confirm;
use indoc::indoc;

use evtx::carve::CarvedEvtx;
use evtx::{AnsiCodec, EvtxParser, ParserSettings, RenderedChunk, RenderedChunkItem};
use log::Level;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
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
#[path = "evtx_dump/info.rs"]
mod info;

fn load_wevt_cache_from_wevtcache_file(
    path: &Path,
) -> Result<std::sync::Arc<evtx::wevt_templates::WevtCache>> {
    use evtx::wevt_templates::wevtcache::{EntryKind, WevtCacheReader};

    let mut cache = evtx::wevt_templates::WevtCache::new();
    let mut reader = WevtCacheReader::open(path)?;
    while let Some((kind, bytes)) = reader.next_entry()? {
        match kind {
            EntryKind::Crim => {
                cache.add_wevt_blob(bytes).map_err(|e| format_err!("{e}"))?;
            }
        }
    }

    Ok(std::sync::Arc::new(cache))
}

#[cfg(all(not(target_env = "msvc"), feature = "fast-alloc"))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(not(target_env = "msvc"), feature = "fast-alloc"))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(target_env = "msvc", feature = "fast-alloc"))]
#[global_allocator]
static ALLOC: rpmalloc::RpMalloc = rpmalloc::RpMalloc;

// rpmalloc calls advapi32's OpenProcessToken, LookupPrivilegeValueA and AdjustTokenPrivileges, to
// use large pages, without linking advapi32. The windows-sys 0.59 import library that other
// dependencies used to bring in happened to provide them.
#[cfg(all(target_env = "msvc", feature = "fast-alloc"))]
#[link(name = "advapi32")]
unsafe extern "system" {}

#[derive(Copy, Clone, PartialOrd, PartialEq, Eq)]
pub enum EvtxOutputFormat {
    JSON,
    XML,
}

struct EvtxDump {
    parser_settings: ParserSettings,
    input: PathBuf,
    show_record_number: bool,
    output_format: EvtxOutputFormat,
    /// Where the records go: stdout, until `open_output` has created the `-f` file.
    output: Box<dyn Write>,
    /// The `-f` file, until it has been created.
    output_file: Option<PathBuf>,
    verbosity_level: Option<Level>,
    stop_after_error: bool,
    /// When set, only the records whose event record IDs are in these ranges are output.
    ranges: Option<Ranges>,
    /// Whether the input is raw data to carve chunks from, rather than an EVTX file.
    carve: bool,
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

        let no_indent = flag_or_jsonl(matches, "no-indent");

        let separate_json_attrib_flag = matches.get_flag("separate-json-attributes");

        let no_show_record_number = flag_or_jsonl(matches, "no-show-record-number");

        let num_threads: u32 = *matches.get_one("num-threads").expect("has default");

        let num_threads = match (cfg!(feature = "multithreading"), num_threads) {
            (true, number) => number,
            // The default (0) and `-t 1` ask for nothing a build without threads lacks.
            (false, 0 | 1) => 1,
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

        let ansi_codec = *matches
            .get_one::<AnsiCodec>("ansi-codec")
            .expect("has default");

        let wevt_cache_path = matches.get_one::<String>("wevt-cache").map(Path::new);
        let mta_path = matches.get_one::<PathBuf>("mta").map(PathBuf::as_path);

        // What can be checked about the output file is checked (and asked) now, before any
        // long work. The file itself is created once the input has been opened (see
        // `open_output`).
        let output_file = matches
            .get_one::<String>("output-target")
            .map(PathBuf::from);
        if let Some(output_file) = &output_file {
            let read = (!Self::is_stdin_input(&input)).then_some(input.as_path());
            Self::check_output_file(
                output_file,
                !matches.get_flag("no-confirm-overwrite"),
                read.into_iter().chain(wevt_cache_path).chain(mta_path),
            )?;
        }

        let wevt_cache = wevt_cache_path
            .map(load_wevt_cache_from_wevtcache_file)
            .transpose()?;

        let mta = mta_path
            .map(|path| evtx::mta::MtaFile::from_path(path).map(std::sync::Arc::new))
            .transpose()?;

        if mta.is_some() && (matches.get_flag("carve") || matches.get_flag("recover-records")) {
            eprintln!(
                "warning: --mta with --carve or --recover-records: an event's message is found by \
                 its record's position and EventRecordID. A carved or recovered record that is \
                 not an event of the log the .MTA file was saved with can get another event's \
                 message."
            );
        }

        let parser_settings = ParserSettings::new()
            .num_threads(num_threads.try_into().expect("u32 -> usize"))
            .validate_checksums(validate_checksums)
            .separate_json_attributes(separate_json_attrib_flag)
            .indent(!no_indent)
            .ansi_codec(ansi_codec)
            .recover_records(matches.get_flag("recover-records"))
            .wevt_cache(wevt_cache)
            .mta(mta);

        Ok(EvtxDump {
            parser_settings,
            input,
            show_record_number: !no_show_record_number,
            output_format,
            output: Box::new(BufWriter::new(io::stdout())),
            output_file,
            verbosity_level,
            stop_after_error,
            ranges: event_ranges,
            carve: matches.get_flag("carve"),
        })
    }

    /// Main entry point for `EvtxDump`
    pub fn run(&mut self) -> Result<()> {
        if let Err(err) = self.try_to_initialize_logging() {
            eprintln!("{:?}", err);
        }

        let input = self.open_input()?;
        if self.carve {
            let carved = CarvedEvtx::new(input)
                .with_context(|| format!("Failed to carve chunks from {}", self.input.display()))?;
            eprintln!(
                "Carved {} chunks from {}",
                carved.chunk_offsets().len(),
                self.input.display()
            );
            if carved.unreadable_bytes() > 0 {
                eprintln!(
                    "{} bytes of {} could not be read and were skipped",
                    carved.unreadable_bytes(),
                    self.input.display()
                );
            }
            // Not an empty log: nothing here to dump, which a script should hear about.
            if carved.chunk_offsets().is_empty() {
                bail!("No EVTX chunks were found in {}", self.input.display());
            }
            for (number, offset) in carved.chunk_offsets().iter().enumerate() {
                log::info!("chunk {number}: offset {offset} ({offset:#x})");
            }
            let parser = EvtxParser::from_read_seek(carved).with_context(|| {
                format!(
                    "Failed to read the chunks carved from {}",
                    self.input.display()
                )
            })?;
            return self.dump(parser);
        }
        let parser = if Self::is_stdin_input(&self.input) {
            EvtxParser::from_read_seek(input).context("Failed to parse EVTX data from stdin")
        } else {
            EvtxParser::from_read_seek(input)
                .with_context(|| format!("Failed to open evtx file at: {}", self.input.display()))
        }?;
        self.dump(parser)
    }

    /// Create the `-f` output file. This comes after the input, the template cache and the
    /// `.MTA` file have been opened, so that a run that can't start leaves an existing file as
    /// it was: creating it empties it.
    fn open_output(&mut self) -> Result<()> {
        let Some(path) = self.output_file.take() else {
            return Ok(());
        };
        let file = Self::create_output_file(&path).with_context(|| {
            format!(
                "An error occurred while creating output file at `{}`",
                path.display()
            )
        })?;
        self.output = Box::new(BufWriter::new(file));
        Ok(())
    }

    fn dump<T: Read + Seek>(&mut self, parser: EvtxParser<T>) -> Result<()> {
        self.open_output()?;
        let mut parser = parser.with_configuration(self.parser_settings.clone());
        let header = parser.header();
        // With logging on, the parser's own warning says this.
        if !header.validate_checksum() && self.verbosity_level.is_none() {
            eprintln!(
                "warning: the file header's CRC32 is invalid (stored {:#010x}, computed {:#010x}); reading the file anyway",
                header.checksum, header.computed_checksum
            );
        }

        match self.output_format {
            EvtxOutputFormat::XML => {
                for chunk in parser.chunks_xml_bytes(self.show_record_number) {
                    self.dump_chunk(chunk)?
                }
            }
            EvtxOutputFormat::JSON => {
                for chunk in parser.chunks_json_bytes(self.show_record_number) {
                    self.dump_chunk(chunk)?
                }
            }
        };

        // Dropping the buffer flushes it too, but ignores a failed write.
        self.output.flush()?;
        Ok(())
    }

    /// The input file, or a copy of stdin.
    fn open_input(&self) -> Result<File> {
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
            Ok(tmp)
        } else {
            File::open(&self.input)
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

    /// Refuse an output file that this run can't or mustn't write, without touching it.
    /// `inputs` are the files the run reads. If `prompt` is passed, will display a
    /// confirmation prompt before overwriting a file.
    fn check_output_file<'a>(
        p: &Path,
        prompt: bool,
        inputs: impl Iterator<Item = &'a Path>,
    ) -> Result<()> {
        if p.is_dir() {
            bail!(
                "There is a directory at {}, refusing to overwrite",
                p.display()
            );
        }
        if p.parent().is_none() {
            bail!("Output file cannot be root.");
        }
        if !p.exists() {
            return Ok(());
        }
        // Creating the output empties it, under any of its names (a link, another spelling).
        for input in inputs {
            if same_file::is_same_file(input, p).unwrap_or(false) {
                bail!(
                    "The output file {} is {}, which this run reads: refusing to overwrite it",
                    p.display(),
                    input.display()
                );
            }
        }
        if prompt {
            match Confirm::new()
                .with_prompt(format!(
                    "Are you sure you want to override output file at {}",
                    p.display()
                ))
                .default(false)
                .interact()
            {
                Ok(true) => {}
                Ok(false) => bail!("Cancelled"),
                Err(_e) => bail!("Failed to display confirmation prompt"),
            }
        }
        Ok(())
    }

    /// Create (or empty) an output file that `check_output_file` has accepted.
    fn create_output_file(p: &Path) -> Result<File> {
        if let Some(parent) = p.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }
        Ok(File::create(p)?)
    }

    /// Write one rendered chunk: a single `write_all` on the common path, or a
    /// per-record walk when filtering/numbering/error handling requires it.
    fn dump_chunk(&mut self, chunk: RenderedChunk) -> Result<()> {
        let plain = self.ranges.is_none();
        let has_errors = chunk
            .items
            .iter()
            .any(|item| matches!(item, RenderedChunkItem::Failed(_)));

        if plain && !has_errors {
            self.output.write_all(&chunk.data)?;
            return Ok(());
        }

        let mut start = 0usize;
        for item in chunk.items {
            match item {
                RenderedChunkItem::Record {
                    event_record_id,
                    end,
                } => {
                    let range_filter = if let Some(ranges) = &self.ranges {
                        ranges.contains(&(event_record_id as usize))
                    } else {
                        true
                    };

                    if range_filter {
                        self.output.write_all(&chunk.data[start..end])?;
                    }
                    start = end;
                }
                // This error is non fatal.
                RenderedChunkItem::Failed(e) => {
                    eprintln!(
                        "{:?}",
                        anyhow::Error::from(e).context("Failed to dump the next record.")
                    );

                    if self.stop_after_error {
                        bail!("Stopped after error");
                    }
                }
            }
        }

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

/// `-o jsonl` implies `flag`; warn when both are passed.
fn flag_or_jsonl(matches: &ArgMatches, flag: &str) -> bool {
    let set = matches.get_flag(flag);
    let jsonl = matches
        .get_one::<String>("output-format")
        .is_some_and(|fmt| fmt == "jsonl");
    if set && jsonl {
        eprintln!("no need to pass both `--{flag}` and `-o jsonl`");
    }
    set || jsonl
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
            let (start, stop) = range.split_once('-').unwrap_or((range, range));
            match (start.parse::<usize>(), stop.parse::<usize>()) {
                (Ok(start), Ok(stop)) => res.push(start..=stop),
                _ => bail!("Expected range to be a positive number, got: {}", range),
            }
        }

        Ok(Ranges(res))
    }
}

fn parse_ansi_codec(label: &str) -> Result<AnsiCodec, String> {
    let codec = encoding_rs::Encoding::for_label(label.as_bytes()).ok_or_else(|| {
        format!(
            "unknown encoding `{label}`; expected an Encoding Standard label such as windows-1252"
        )
    })?;
    if !codec.is_ascii_compatible() {
        return Err(format!(
            "`{label}` is not an ASCII-compatible encoding; ANSI strings cannot be UTF-16"
        ));
    }
    Ok(codec)
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

#[test]
fn test_parse_ansi_codec() {
    assert_eq!(
        parse_ansi_codec("windows-1252").unwrap().name(),
        "windows-1252"
    );
    assert_eq!(parse_ansi_codec("cp1252").unwrap().name(), "windows-1252");
    assert_eq!(
        parse_ansi_codec("WINDOWS-1251").unwrap().name(),
        "windows-1251"
    );
    assert!(parse_ansi_codec("utf-16le").is_err());
    assert!(parse_ansi_codec("cp437").is_err());
    assert!(parse_ansi_codec("no-such-encoding").is_err());
}

fn main() -> Result<()> {
    match run_cli() {
        // A reader that stops early, such as `head`, closes the pipe: that ends the output, as
        // it does for other command-line tools, rather than being an error.
        Err(error) if is_broken_pipe(&error) => Ok(()),
        result => result,
    }
}

fn is_broken_pipe(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
    })
}

fn run_cli() -> Result<()> {
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
                .help(indoc!("When set, only the records with these event record IDs (`EventRecordID`) are output.
                For example:
                    --events=1 outputs the record whose ID is 1.
                    --events=0-10,20-30 outputs the records with IDs 0 to 10 and 20 to 30.
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
            Arg::new("recover-records")
                .long("recover-records")
                .action(ArgAction::SetTrue)
                .help("Also carve deleted/stale records from unallocated chunk space (cleared chunks, chunk slack)."),
        )
        .arg(
            Arg::new("carve")
                .long("carve")
                .action(ArgAction::SetTrue)
                .help(indoc!("Treats INPUT as raw data (a disk image, unallocated space, a memory dump, a damaged log) and
                parses the EVTX chunks found anywhere in it, skipping exact copies. Prints how many it found;
                with -v, where each one is. Combine with --recover-records to also carve their slack.")),
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
                .value_parser(parse_ansi_codec)
                .default_value(encoding_rs::WINDOWS_1252.name())
                .help("Code page for ANSI strings. An Encoding Standard label such as windows-1252 (the default), windows-1251, or gbk."),
        );

    let matches = cmd
        // Offline WEVT template cache: fallback for records whose embedded EVTX templates are
        // missing/corrupt (common in carved/dirty logs).
        .arg(
            Arg::new("wevt-cache")
                .long("wevt-cache")
                .value_name("WEVTCACHE")
                .help("Path to a WEVT template cache file (`.wevtcache`). When set, evtx_dump will try to render records using this cache if the embedded EVTX template expansion fails."),
        )
        .arg(
            Arg::new("mta")
                .long("mta")
                .value_name("MTA")
                .value_parser(clap::value_parser!(PathBuf))
                .help("Path to an .MTA file (LocaleMetaData\\<log>_<LCID>.MTA, written next to logs saved with display information). Adds each event's localized message, level, task, opcode and keyword names as a RenderingInfo element. It must be the file saved with this log: an event's message is found by the record's position and EventRecordID, which a record of another log (or one found by --carve or --recover-records) can have too."),
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
        .subcommand(extract_wevt_templates::command())
        .subcommand(dump_template_instances::command())
        .subcommand(apply_wevt_cache::command())
        .subcommand(info::command())
        .get_matches();

    match matches.subcommand() {
        Some(("extract-wevt-templates", m)) => return extract_wevt_templates::run(m),
        Some(("dump-template-instances", m)) => return dump_template_instances::run(m),
        Some(("apply-wevt-cache", m)) => return apply_wevt_cache::run(m),
        Some(("info", m)) => return info::run(m),
        _ => {}
    }

    if matches.get_one::<String>("INPUT").is_none() {
        bail!("Missing INPUT. Provide an EVTX file path, or use a subcommand (try `--help`).");
    }

    EvtxDump::from_cli_matches(&matches)?.run()?;

    Ok(())
}
