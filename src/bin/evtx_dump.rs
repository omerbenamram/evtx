extern crate evtx;

use clap::{App, Arg, ArgMatches};

use evtx::evtx_parser::EvtxOutputFormat;
use evtx::{EvtxParser, ParserSettings};

fn is_a_non_negative_number(value: String) -> Result<(), String> {
    match value.parse::<usize>() {
        Ok(_) => Ok(()),
        Err(_) => Err("Expected value to be a positive number.".to_owned()),
    }
}

fn parser_configuration_from_cli_matches(matches: &ArgMatches) -> ParserSettings {
    let output_format = match matches.value_of("output-format").unwrap_or_default() {
        "xml" => EvtxOutputFormat::XML,
        "json" => EvtxOutputFormat::JSON,
        _ => EvtxOutputFormat::XML,
    };

    let threads: bool = matches.is_present("threads");
    let num_threads = matches
        .value_of("num-threads")
        .and_then(|value| Some(value.parse::<usize>().expect("used validator")));

    let num_threads = match (threads, cfg!(feature = "multithreading"), num_threads) {
        (true, true, Some(number)) => number,
        (true, true, None) => 0,
        (true, false, _) => {
            eprintln!("turned on threads, but library was compiled without `multithreading` feature! using fallback sync iterator");
            1
        }
        (false, _, _) => 1,
    };

    ParserSettings::new()
        .output_format(output_format)
        .num_threads(num_threads)
}

fn main() {
    let matches = App::new("EVTX Parser")
        .version("0.1")
        .author("Omer B. <omerbenamram@gmail.com>")
        .about("Utility to parse EVTX files")
        .arg(Arg::with_name("INPUT").required(true).index(1))
        .arg(
            Arg::with_name("threads")
                .short("t")
                .long("threads")
                .help("enable uses of multi-threading")
                .takes_value(false),
        )
        .arg(
            Arg::with_name("num-threads")
                .requires("threads")
                .default_value("0")
                .validator(is_a_non_negative_number)
                .help("Sets the number of worker threads, defaults to number of CPU cores. Only works with `-t`"),
        )
        .arg(
            Arg::with_name("output-format")
                .short("-o")
                .possible_values(&["json", "xml"])
                .default_value("xml")
                .help("sets the output format"),
        )
        // TODO: replace `env_logger` with something nicer for the CLI.
//        .arg(Arg::with_name("verbose").short("-v").multiple(true).max_values(3).help("1 - info, 2 - debug, 3 - trace"))
        .get_matches();

    let fp = matches
        .value_of("input")
        .expect("This is a required argument");

    let configuration = parser_configuration_from_cli_matches(&matches);

    let mut parser = EvtxParser::from_path(fp)
        .expect(&format!("Failed to load evtx file located at {}", fp))
        .with_configuration(configuration);

    for record in parser.records() {
        match record {
            Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
            Err(e) => eprintln!("{}", e),
        }
    }
}
