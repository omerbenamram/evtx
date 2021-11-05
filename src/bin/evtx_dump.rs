#![allow(clippy::upper_case_acronyms)]

use anyhow::{bail, format_err, Context, Result};
use clap::{App, AppSettings, Arg, ArgMatches};
use dialoguer::Confirm;
use indoc::indoc;

use encoding::all::encodings;
use encoding::types::Encoding;
use evtx::err::Result as EvtxResult;
use evtx::{EvtxParser, ParserSettings, SerializedEvtxRecord};
use log::Level;
use std::fs::{self, File};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

#[cfg(all(feature = "fast-alloc", not(windows)))]
use jemallocator::Jemalloc;

#[cfg(all(feature = "fast-alloc", not(windows)))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[cfg(all(feature = "fast-alloc", windows))]
#[global_allocator]
static ALLOC: rpmalloc::RpMalloc = rpmalloc::RpMalloc;

#[derive(Copy, Clone, PartialOrd, PartialEq)]
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
                if self.show_record_number {
                    writeln!(self.output, "Record {}", r.event_record_id)?;
                }
                writeln!(self.output, "{}", r.data)?;
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

fn is_a_non_negative_number(value: String) -> Result<(), String> {
    match value.parse::<usize>() {
        Ok(_) => Ok(()),
        Err(_) => Err("Expected value to be a positive number.".to_owned()),
    }
}

fn main() -> Result<()> {
    let matches = App::new("EVTX Parser")
        .version(env!("CARGO_PKG_VERSION"))
        .setting(AppSettings::ColoredHelp)
        .setting(AppSettings::DeriveDisplayOrder)
        .author("Omer B. <omerbenamram@gmail.com>")
        .about("Utility to parse EVTX files")
        .arg(Arg::with_name("INPUT").required(true))
        .arg(
            Arg::with_name("num-threads")
                .short("-t")
                .long("--threads")
                .default_value("0")
                .validator(is_a_non_negative_number)
                .help("Sets the number of worker threads, defaults to number of CPU cores."),
        )
        .arg(
            Arg::with_name("output-format")
                .short("-o")
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
            Arg::with_name("output-target")
                .long("--output")
                .short("-f")
                .takes_value(true)
                .help(indoc!("Writes output to the file specified instead of stdout, errors will still be printed to stderr.
                       Will ask for confirmation before overwriting files, to allow overwriting, pass `--no-confirm-overwrite`
                       Will create parent directories if needed.")),
        )
        .arg(
            Arg::with_name("no-confirm-overwrite")
                .long("--no-confirm-overwrite")
                .takes_value(false)
                .help(indoc!("When set, will not ask for confirmation before overwriting files, useful for automation")),
        )
        .arg(
            Arg::with_name("validate-checksums")
                .long("--validate-checksums")
                .takes_value(false)
                .help(indoc!("When set, chunks with invalid checksums will not be parsed. \
                Usually dirty files have bad checksums, so using this flag will result in fewer records.")),
        )
        .arg(
            Arg::with_name("no-indent")
                .long("--no-indent")
                .takes_value(false)
                .help("When set, output will not be indented."),
        )
        .arg(
            Arg::with_name("separate-json-attributes")
                .long("--separate-json-attributes")
                .takes_value(false)
                .help("If outputting JSON, XML Element's attributes will be stored in a separate object named '<ELEMENTNAME>_attributes', with <ELEMENTNAME> containing the value of the node."),
        )
        .arg(
            Arg::with_name("no-show-record-number")
                .long("--dont-show-record-number")
                .takes_value(false)
                .help("When set, `Record <id>` will not be printed."),
        )
        .arg(
            Arg::with_name("ansi-codec")
                .long("--ansi-codec")
                .possible_values(&encodings().iter()
                    .filter(|&e| e.raw_decoder().is_ascii_compatible())
                    .map(|e| e.name())
                    .collect::<Vec<&'static str>>())
                .default_value(encoding::all::WINDOWS_1252.name())
                .help("When set, controls the codec of ansi encoded strings the file."),
        )
        .arg(
            Arg::with_name("stop-after-one-error")
                .long("--stop-after-one-error")
                .takes_value(false)
                .help("When set, will exit after any failure of reading a record. Useful for debugging."),
        )
        .arg(Arg::with_name("verbose")
            .short("-v")
            .multiple(true)
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
