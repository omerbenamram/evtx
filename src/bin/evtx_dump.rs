#![allow(clippy::upper_case_acronyms)]

use anyhow::{bail, format_err, Context, Result};
use clap::{AppSettings, Arg, ArgMatches, Command};
use dialoguer::Confirm;
use indoc::indoc;

use encoding::all::encodings;
use encoding::types::Encoding;
use evtx::err::Result as EvtxResult;
use evtx::{EvtxParser, ParserSettings, SerializedEvtxRecord};
use log::Level;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::ops::RangeInclusive;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[cfg(all(feature = "fast-alloc", not(windows)))]
use jemallocator::Jemalloc;

#[cfg(all(feature = "fast-alloc", not(windows)))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(feature = "fast-alloc", windows))]
#[global_allocator]
static ALLOC: rpmalloc::RpMalloc = rpmalloc::RpMalloc;

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
    output: Box<dyn Write>,
    verbosity_level: Option<Level>,
    stop_after_error: bool,
    /// When set, only the specified events (offseted reltaive to file) will be outputted.
    ranges: Option<Ranges>,
}

impl EvtxDump {
    pub fn from_cli_matches(matches: &ArgMatches) -> Result<Self> {
        let input = PathBuf::from(
            matches
                .value_of("INPUT")
                .expect("This is a required argument"),
        );

        let output_format = match matches.value_of("output-format").unwrap_or_default() {
            "xml" => EvtxOutputFormat::XML,
            "json" | "jsonl" => EvtxOutputFormat::JSON,
            _ => EvtxOutputFormat::XML,
        };

        let no_indent = match (
            matches.is_present("no-indent"),
            matches.value_of("output-format"),
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

        let separate_json_attrib_flag = matches.is_present("separate-json-attributes");

        let no_show_record_number = match (
            matches.is_present("no-show-record-number"),
            matches.value_of("output-format"),
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

        let num_threads = matches
            .value_of("num-threads")
            .map(|value| value.parse::<usize>().expect("used validator"));

        let num_threads = match (cfg!(feature = "multithreading"), num_threads) {
            (true, Some(number)) => number,
            (true, None) => 0,
            (false, _) => {
                eprintln!("turned on threads, but library was compiled without `multithreading` feature! using fallback sync iterator");
                1
            }
        };

        let validate_checksums = matches.is_present("validate-checksums");
        let stop_after_error = matches.is_present("stop-after-one-error");

        let event_ranges = matches
            .value_of("event-ranges")
            .map(|s| Ranges::from_str(s).expect("used validator"));

        let verbosity_level = match matches.occurrences_of("verbose") {
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
            .find(|c| c.name() == matches.value_of("ansi-codec").expect("has set default"))
            .expect("possible values are derived from `encodings()`");

        let output: Box<dyn Write> = if let Some(path) = matches.value_of("output-target") {
            Box::new(BufWriter::new(
                Self::create_output_file(path, !matches.is_present("no-confirm-overwrite"))
                    .with_context(|| {
                        format!("An error occurred while creating output file at `{}`", path)
                    })?,
            ))
        } else {
            Box::new(BufWriter::new(io::stdout()))
        };

        Ok(EvtxDump {
            parser_settings: ParserSettings::new()
                .num_threads(num_threads)
                .validate_checksums(validate_checksums)
                .separate_json_attributes(separate_json_attrib_flag)
                .indent(!no_indent)
                .ansi_codec(*ansi_codec),
            input,
            show_record_number: !no_show_record_number,
            output_format,
            output,
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

        let mut parser = EvtxParser::from_path(&self.input)
            .with_context(|| format!("Failed to open evtx file at: {}", &self.input.display()))
            .map(|parser| parser.with_configuration(self.parser_settings.clone()))?;

        match self.output_format {
            EvtxOutputFormat::XML => {
                for record in parser.records() {
                    self.dump_record(record)?
                }
            }
            EvtxOutputFormat::JSON => {
                for record in parser.records_json() {
                    self.dump_record(record)?
                }
            }
        };

        Ok(())
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
                    .with_prompt(&format!(
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

fn is_a_non_negative_number(value: &str) -> Result<(), String> {
    match value.to_string().parse::<usize>() {
        Ok(_) => Ok(()),
        Err(_) => Err("Expected value to be a positive number.".to_owned()),
    }
}

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

fn matches_ranges(value: &str) -> Result<(), String> {
    Ranges::from_str(value)
        .map_err(|e| e.to_string())
        .map(|_| ())
}

#[test]
fn test_ranges() {
    assert!(matches_ranges("1-2,3,4-5,6-7,8-9").is_ok());
    assert!(matches_ranges("1").is_ok());
    assert!(matches_ranges("1-").is_err());
    assert!(matches_ranges("-2").is_err());
}

fn main() -> Result<()> {
    let matches = Command::new("EVTX Parser")
        .version(env!("CARGO_PKG_VERSION"))
        .setting(AppSettings::DeriveDisplayOrder)
        .author("Omer B. <omerbenamram@gmail.com>")
        .about("Utility to parse EVTX files")
        .arg(Arg::new("INPUT").required(true))
        .arg(
            Arg::new("num-threads")
                .short('t')
                .long("--threads")
                .default_value("0")
                .validator(is_a_non_negative_number)
                .help("Sets the number of worker threads, defaults to number of CPU cores."),
        )
        .arg(
            Arg::new("output-format")
                .short('o')
                .long("--format")
                .possible_values(&["json", "xml", "jsonl"])
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
                .long("--output")
                .short('f')
                .takes_value(true)
                .help(indoc!("Writes output to the file specified instead of stdout, errors will still be printed to stderr.
                       Will ask for confirmation before overwriting files, to allow overwriting, pass `--no-confirm-overwrite`
                       Will create parent directories if needed.")),
        )
        .arg(
            Arg::new("no-confirm-overwrite")
                .long("--no-confirm-overwrite")
                .takes_value(false)
                .help(indoc!("When set, will not ask for confirmation before overwriting files, useful for automation")),
        )
        .arg(
            Arg::with_name("event-ranges")
                .long("--events")
                .takes_value(true)
                .validator(matches_ranges)
                .help(indoc!("When set, only the specified events (offseted reltaive to file) will be outputted.
                For example:
                    --events=1 will output the first event.
                    --events=0-10,20-30 will output events 0-10 and 20-30.
                ")),
        )
        .arg(
            Arg::with_name("validate-checksums")
                .long("--validate-checksums")
                .takes_value(false)
                .help(indoc!("When set, chunks with invalid checksums will not be parsed. \
                Usually dirty files have bad checksums, so using this flag will result in fewer records.")),
        )
        .arg(
            Arg::new("no-indent")
                .long("--no-indent")
                .takes_value(false)
                .help("When set, output will not be indented."),
        )
        .arg(
            Arg::new("separate-json-attributes")
                .long("--separate-json-attributes")
                .takes_value(false)
                .help("If outputting JSON, XML Element's attributes will be stored in a separate object named '<ELEMENTNAME>_attributes', with <ELEMENTNAME> containing the value of the node."),
        )
        .arg(
            Arg::new("no-show-record-number")
                .long("--dont-show-record-number")
                .takes_value(false)
                .help("When set, `Record <id>` will not be printed."),
        )
        .arg(
            Arg::new("ansi-codec")
                .long("--ansi-codec")
                .possible_values(&encodings().iter()
                    .filter(|&e| e.raw_decoder().is_ascii_compatible())
                    .map(|e| e.name())
                    .collect::<Vec<&'static str>>())
                .default_value(encoding::all::WINDOWS_1252.name())
                .help("When set, controls the codec of ansi encoded strings the file."),
        )
        .arg(
            Arg::new("stop-after-one-error")
                .long("--stop-after-one-error")
                .takes_value(false)
                .help("When set, will exit after any failure of reading a record. Useful for debugging."),
        )
        .arg(Arg::new("verbose")
            .short('v')
            .multiple_occurrences(true)
            .takes_value(false)
            .help(indoc!(r#"
            Sets debug prints level for the application:
                -v   - info
                -vv  - debug
                -vvv - trace
            NOTE: trace output is only available in debug builds, as it is extremely verbose."#))
        ).get_matches();

    EvtxDump::from_cli_matches(&matches)?.run()?;

    Ok(())
}
