#[macro_use]
extern crate criterion;
extern crate evtx;

use criterion::{BatchSize, Criterion, Throughput};
use evtx::JsonWriter;

struct Sink;
impl std::io::Write for Sink {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { Ok(buf.len()) }
	fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn bench_escape(c: &mut Criterion) {
	let ascii_short = "HelloWorld1234567890".to_string();
	let ascii_long = "a".repeat(4096);
	let mixed = {
		let mut s = String::from("path \\");
		s.push('"');
		s.push_str("quoted");
		s.push('"');
		s.push_str(" name\nnew\tline");
		s
	};
	let controls: String = (0u8..32).map(|b| b as char).collect();
	let quotes = "\"".repeat(2048);

	let cases: Vec<(&'static str, String)> = vec![
		("ascii_short", ascii_short),
		("ascii_long", ascii_long),
		("mixed", mixed),
		("controls", controls),
		("quotes", quotes),
	];
	let mut group = c.benchmark_group("escape_json_writer_quoted_str");
	for (name, s) in cases.into_iter() {
		group.throughput(Throughput::Bytes(s.len() as u64));
		group.bench_function(name, |b| {
			b.iter_batched(
				|| Sink,
				|mut sink| {
					let mut w = JsonWriter::new(&mut sink);
					w.write_quoted_str(&s).unwrap();
					criterion::black_box(&sink);
				},
				BatchSize::SmallInput,
			)
		});
	}
	group.finish();
}

criterion_group!(benches, bench_escape);
criterion_main!(benches);