extern crate evtx;

use clap::{App, Arg, ArgMatches};

use evtx::{EvtxParser, ParserSettings};

#[derive(Copy, Clone, PartialOrd, PartialEq)]
pub enum EvtxOutputFormat {
    JSON,
    XML,
}

struct EvtxDumpConfig {
    parser_settings: ParserSettings,
    output_format: EvtxOutputFormat,
}

impl EvtxDumpConfig {
    pub fn from_cli_matches(matches: &ArgMatches) -> Self {
        let output_format = match matches.value_of("output-format").unwrap_or_default() {
            "xml" => EvtxOutputFormat::XML,
            "json" => EvtxOutputFormat::JSON,
            _ => EvtxOutputFormat::XML,
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

        EvtxDumpConfig {
            parser_settings: ParserSettings::new()
                .num_threads(num_threads)
                .validate_checksums(validate_checksums),
            output_format,
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
        .version("0.1")
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
                .possible_values(&["json", "xml"])
                .default_value("xml")
                .help("sets the output format"),
        )
        .arg(
            Arg::with_name("validate-checksums")
                .long("--validate-checksums")
                .takes_value(false)
                .help("When set, chunks with invalid checksums will not be parsed. \
                Usually dirty files have bad checksums, so using this flag will result in fewer records."),
        )
        // TODO: replace `env_logger` with something nicer for the CLI.
        //        .arg(Arg::with_name("verbose").short("-v").multiple(true).max_values(3).help("1 - info, 2 - debug, 3 - trace"))
        .get_matches();

    let fp = matches
        .value_of("INPUT")
        .expect("This is a required argument");

    let config = EvtxDumpConfig::from_cli_matches(&matches);

    let mut parser = EvtxParser::from_path(fp)
        .unwrap_or_else(|_| panic!("Failed to load evtx file located at {}", fp))
        .with_configuration(config.parser_settings.clone());

    match config.output_format {
        EvtxOutputFormat::XML => {
            for record in parser.records() {
                match record {
                    Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
                    Err(e) => eprintln!("{}", e),
                }
            }
        }
        EvtxOutputFormat::JSON => {
            for record in parser.records_json() {
                match record {
                    Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
                    Err(e) => eprintln!("{}", e),
                }
            }
        }
    };
}
