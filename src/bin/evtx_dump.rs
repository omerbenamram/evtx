use clap::{App, AppSettings, Arg, ArgMatches};
use dialoguer::Confirmation;

use encoding::all::encodings;
use encoding::types::Encoding;
use evtx::err::{dump_err_with_backtrace, Error};
use evtx::{EvtxParser, ParserSettings, SerializedEvtxRecord};
use log::Level;
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::exit;

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
    // It's ok to rely on interior mutability here,
    // since there is only one code flow writing to output which is trivial to verify.
    output: RefCell<Box<Write>>,
    verbosity_level: Option<Level>,
    backtraces: bool,
}

/// Tries to write a line to a given target, aborts program if fails.
macro_rules! try_writeln {
    ($($arg:tt)*) => (
        match writeln!($($arg)*) {
            Ok(_) => {},
            Err(e) => {
                eprintln!("{}", &e);
                exit(1)
            }
        }
    );
}

/// Simple error  macro for use inside of internal errors in `EvtxDump`
macro_rules! err {
    ($($tt:tt)*) => { Err(Box::<std::error::Error>::from(format!($($tt)*))) }
}

impl EvtxDump {
    pub fn from_cli_matches(matches: &ArgMatches) -> Self {
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
            .and_then(|value| Some(value.parse::<usize>().expect("used validator")));

        let num_threads = match (cfg!(feature = "multithreading"), num_threads) {
            (true, Some(number)) => number,
            (true, None) => 0,
            (false, _) => {
                eprintln!("turned on threads, but library was compiled without `multithreading` feature! using fallback sync iterator");
                1
            }
        };

        let validate_checksums = matches.is_present("validate-checksums");
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

        let backtraces = matches.is_present("backtraces");

        let ansi_codec = encodings()
            .iter()
            .find(|c| c.name() == matches.value_of("ansi-codec").expect("has set default"))
            .expect("possible values are derived from `encodings()`");

        let output: Box<Write> = if let Some(path) = matches.value_of("output-target") {
            match Self::create_output_file(path, !matches.is_present("no-confirm-overwrite")) {
                Ok(f) => Box::new(f),
                Err(e) => {
                    eprintln!(
                        "An error occurred while creating output file at `{}` - `{}`",
                        path, e
                    );
                    exit(1)
                }
            }
        } else {
            Box::new(io::stdout())
        };

        EvtxDump {
            parser_settings: ParserSettings::new()
                .num_threads(num_threads)
                .validate_checksums(validate_checksums)
                .indent(!no_indent)
                .ansi_codec(*ansi_codec),
            input,
            show_record_number: !no_show_record_number,
            output_format,
            output: RefCell::new(output),
            verbosity_level,
            backtraces,
        }
    }

    /// Main entry point for `EvtxDump`
    pub fn run(&self) {
        self.try_to_initialize_logging();

        let mut parser = match EvtxParser::from_path(&self.input) {
            Ok(parser) => parser.with_configuration(self.parser_settings.clone()),
            Err(e) => {
                eprintln!(
                    "Failed to open file {}.\n\tcaused by: {}",
                    self.input.display(),
                    &e
                );
                exit(1)
            }
        };

        match self.output_format {
            EvtxOutputFormat::XML => {
                for record in parser.records() {
                    self.dump_record(record)
                }
            }
            EvtxOutputFormat::JSON => {
                for record in parser.records_json() {
                    self.dump_record(record)
                }
            }
        }
    }

    /// If `prompt` is passed, will display a confirmation prompt before overwriting files.
    fn create_output_file(
        path: impl AsRef<Path>,
        prompt: bool,
    ) -> Result<File, Box<std::error::Error>> {
        let p = path.as_ref();

        if p.is_dir() {
            return err!(
                "There is a directory at {}, refusing to overwrite",
                p.display()
            );
        }

        if p.exists() {
            if prompt {
                match Confirmation::new()
                    .with_text(&format!(
                        "Are you sure you want to override output file at {}",
                        p.display()
                    ))
                    .default(false)
                    .interact()
                {
                    Ok(true) => Ok(File::create(p)?),
                    Ok(false) => err!("Cancelled"),
                    Err(e) => err!(
                        "Failed to write confirmation prompt to term caused by\n{}",
                        e
                    ),
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
                    if parent.exists() {
                        Ok(File::create(p)?)
                    } else {
                        fs::create_dir_all(parent)?;
                        Ok(File::create(p)?)
                    }
                }
                None => err!("Output file cannot be root."),
            }
        }
    }

    fn dump_record(&self, record: Result<SerializedEvtxRecord, Error>) {
        match record {
            Ok(r) => {
                if self.show_record_number {
                    try_writeln!(self.output.borrow_mut(), "Record {}", r.event_record_id);
                }
                try_writeln!(self.output.borrow_mut(), "{}", r.data);
            }
            Err(e) => {
                if self.backtraces {
                    dump_err_with_backtrace(&e)
                } else {
                    eprintln!("{}", &e);
                }
            }
        }
    }

    fn try_to_initialize_logging(&self) {
        if let Some(level) = self.verbosity_level {
            match simple_logger::init_with_level(level) {
                Ok(_) => {}
                Err(e) => eprintln!("Failed to initialize logging: {}", e),
            };
        }
    }
}

fn is_a_non_negative_number(value: String) -> Result<(), String> {
    match value.parse::<usize>() {
        Ok(_) => Ok(()),
        Err(_) => Err("Expected value to be a positive number.".to_owned()),
    }
}

fn main() {
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
                .long_help("\
                    Sets the output format:
                        \"xml\"   - prints XML output.
                        \"json\"  - prints
                        \"jsonl\" - same as json with --no-indent --dont-show-record-number 
                "),
        )
        .arg(
            Arg::with_name("output-target")
                .long("--output")
                .short("-f")
                .takes_value(true)
                .help("Writes output to the file specified instead of stdout, errors will still be printed to stderr.\
                       Will ask for confirmation before overwriting files, to allow overwriting, pass `--no-confirm-overwrite`\
                       Will create parent directories if needed."),
        )
        .arg(
            Arg::with_name("no-confirm-overwrite")
                .long("--no-confirm-overwrite")
                .takes_value(false)
                .help("When set, will not ask for confirmation before overwriting files, useful for automation"),
        )
        .arg(
            Arg::with_name("validate-checksums")
                .long("--validate-checksums")
                .takes_value(false)
                .help("When set, chunks with invalid checksums will not be parsed. \
                Usually dirty files have bad checksums, so using this flag will result in fewer records."),
        )
        .arg(
            Arg::with_name("no-indent")
                .long("--no-indent")
                .takes_value(false)
                .help("When set, output will not be indented."),
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
        .arg(Arg::with_name("verbose").short("-v").multiple(true).takes_value(false)
            .help("-v - info, -vv - debug, -vvv - trace.\
             trace output is only available in debug builds, as it is extremely verbose"))
        .arg(
            Arg::with_name("backtraces")
                .long("--backtraces")
                .takes_value(false)
                .help("If set, a backtrace will be printed with some errors if available"))
        .get_matches();

    let app = EvtxDump::from_cli_matches(&matches);
    app.run();
}
