extern crate evtx_rs;

use clap::App;
use clap::Arg;

use evtx_rs::evtx::EvtxParser;

fn main() {
    env_logger::init();
    let matches = App::new("EVTX Parser")
        .version("0.1")
        .author("Omer B. <omerbenamram@gmail.com>")
        .about("Utility to parse EVTX files")
        .arg(
            Arg::with_name("input")
                .short("i")
                .long("input")
                .value_name("INPUT")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    let fp = matches
        .value_of("input")
        .expect("This is a required argument");

    let parser = EvtxParser::from_path(fp).unwrap();
    for record in parser.records() {
        match record {
            Ok(r) => println!("{}", r.data),
            Err(e) => eprintln!("{}", e),
        }
    }
}
