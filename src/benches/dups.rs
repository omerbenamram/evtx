#[macro_use]
extern crate criterion;
extern crate evtx;

use criterion::{Criterion, Throughput};
use evtx::{JsonStreamOutput, ParserSettings};
use evtx::model::xml::{XmlElementBuilder, XmlElement};

struct Sink;
impl std::io::Write for Sink {
	fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> { Ok(buf.len()) }
	fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn build_elem<'a>(arena: &'a bumpalo::Bump, name: &'a str) -> XmlElement<'a> {
	let mut b = XmlElementBuilder::new_in(arena);
	let bin = evtx::binxml::name::BinXmlName { str: name.to_string() };
	b.name(std::borrow::Cow::Owned(bin));
	b.finish().unwrap()
}

fn bench_dups(c: &mut Criterion) {
	let arena = bumpalo::Bump::new();
	let settings = ParserSettings::default();
	let mut group = c.benchmark_group("json_stream_dups");
	let siblings: Vec<XmlElement> = (0..200).map(|i| {
		let name = if i % 3 == 0 { "Field" } else { "Field" };
		build_elem(&arena, name)
	}).collect();
	group.throughput(Throughput::Elements(siblings.len() as u64));
	group.bench_function("dup_keys_200", |b| {
		b.iter(|| {
			let sink = Sink;
			let mut out = JsonStreamOutput::with_writer(sink, &settings);
			let _ = out.visit_start_of_stream();
			for e in siblings.iter() {
				let _ = out.visit_open_start_element(e);
				let _ = out.visit_close_element(e);
			}
			let _ = out.visit_end_of_stream();
		});
	});
	group.finish();
}

criterion_group!(benches, bench_dups);
criterion_main!(benches);