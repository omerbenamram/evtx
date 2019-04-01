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
        .arg(
            Arg::with_name("threads")
                .short("t")
                .long("threads")
                .takes_value(false),
        )
        .get_matches();

    let fp = matches
        .value_of("input")
        .expect("This is a required argument");

    let threads: bool = matches.is_present("threads");

    let parser = EvtxParser::from_path(fp).unwrap();

    if threads && !cfg!(feature = "multithreading") {
        eprintln!("turned on threads, but library was compiled without `multithreading` feature! using fallback sync iterator");
    };

    let iter = if threads {
        #[cfg(feature = "multithreading")]
        {
            parser.parallel_records()
        }
        #[cfg(not(feature = "multithreading"))]
        {
            parser.records()
        }
    } else {
        parser.records()
    };

    iter.for_each(|r| match r {
        Ok(r) => println!("Record {}\n{}", r.event_record_id, r.data),
        Err(e) => eprintln!("{}", e),
    });
}
