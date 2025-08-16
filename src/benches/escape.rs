#[macro_use]
extern crate criterion;
extern crate evtx;

use criterion::{BatchSize, Criterion, Throughput};

fn bench_escape(c: &mut Criterion) {
	let cases: Vec<(&'static str, Vec<u8>)> = vec![
		("ascii_short", b"HelloWorld1234567890".to_vec()),
		("ascii_long", vec![b'a'; 4096]),
		("mixed", {
			let mut v = Vec::with_capacity(512);
			v.extend_from_slice(b"path \\\\"quoted\" name\nnew\tline");
			v
		}),
		("controls", (0u8..32).collect()),
		("quotes", vec![b'"'; 2048]),
	];
	let mut group = c.benchmark_group("escape_json_ascii");
	for (name, data) in cases.into_iter() {
		group.throughput(Throughput::Bytes(data.len() as u64));
		group.bench_function(name, |b| {
			b.iter_batched(
				|| String::with_capacity(data.len() * 2),
				|mut s| {
					let _ = evtx::utils::escape::escape_json_ascii(&data, &mut s);
					criterion::black_box(&s);
				},
				BatchSize::SmallInput,
			)
		});
	}
	group.finish();
}

criterion_group!(benches, bench_escape);
criterion_main!(benches);