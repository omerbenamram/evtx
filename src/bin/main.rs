extern crate evtx;

use clap::App;
use clap::Arg;

use evtx::EvtxParser;

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
    let par_iter = parser.records();

    par_iter.for_each(|r| match r {
        Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
        Err(e) => eprintln!("{}", e),
    });
}
