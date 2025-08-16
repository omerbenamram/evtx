#[macro_use]
extern crate criterion;
extern crate evtx;

use criterion::{Criterion, Throughput};
use evtx::binxml::value_variant::BinXmlValue;
use evtx::model::deserialized::BinXMLDeserializedTokens as T;
use evtx::xml_output::BinXmlOutput;
use evtx::{EvtxChunk, ParserSettings};

struct SinkVisitor;
impl BinXmlOutput for SinkVisitor {
	fn visit_start_of_stream(&mut self) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_end_of_stream(&mut self) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_open_start_element(&mut self, _e: &evtx::model::xml::XmlElement) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_close_element(&mut self, _e: &evtx::model::xml::XmlElement) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_characters(&mut self, _v: std::borrow::Cow<BinXmlValue>) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_entity_reference(&mut self, _e: &evtx::binxml::name::BinXmlName) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_cdata_section(&mut self) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_character_reference(&mut self, _c: std::borrow::Cow<str>) -> evtx::err::SerializationResult<()> { Ok(()) }
	fn visit_processing_instruction(&mut self, _p: &evtx::model::xml::BinXmlPI) -> evtx::err::SerializationResult<()> { Ok(()) }
}

fn canned_tokens<'a>() -> Vec<T<'a>> {
	use evtx::binxml::name::BinXmlNameRef;
	let name = BinXmlNameRef { offset: 0 }; // will be missed; expand_string_ref will fallback, but we won't call it in this bench path
	let mut v = Vec::new();
	for _ in 0..500 {
		v.push(T::OpenStartElement(evtx::model::deserialized::BinXMLOpenStartElement { data_size: 0, name }));
		v.push(T::CloseStartElement);
		v.push(T::Value(BinXmlValue::StringType("abc".to_string())));
		v.push(T::CloseElement);
	}
	v
}

fn bench_expander(c: &mut Criterion) {
	let settings = ParserSettings::default();
	let data = include_bytes!("../../samples/security.evtx");
	let chunk = EvtxChunk::new(&data[..evtx::EVTX_CHUNK_SIZE], 0, settings.clone()).unwrap();
	let toks = canned_tokens();
	let mut group = c.benchmark_group("stream_expand_token");
	group.throughput(Throughput::Elements(toks.len() as u64));
	group.bench_function("expand_500_simple", |b| {
		b.iter(|| {
			let mut v = SinkVisitor;
			let _ = evtx::binxml::assemble::parse_tokens_streaming(&toks, &chunk, &mut v);
		});
	});
	group.finish();
}

criterion_group!(benches, bench_expander);
criterion_main!(benches);