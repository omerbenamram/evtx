#[macro_use]
extern crate criterion;

extern crate evtx;

use bumpalo::Bump;
use criterion::{BenchmarkId, Criterion, Throughput, black_box};
use evtx::binxml::bench::{TreeBuildCache, build_tree_from_binxml_bytes_in_bump};
use evtx::{EvtxChunk, EvtxChunkData, ParserSettings};
use std::cell::RefCell;
use std::sync::Arc;

const EVTX_FILE_HEADER_SIZE: usize = 4096;
const EVTX_CHUNK_SIZE: usize = 65536;

fn bench_tree_build(c: &mut Criterion) {
    let evtx_file = include_bytes!("../../samples/security.evtx");
    let chunk_bytes =
        evtx_file[EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE].to_vec();
    let chunk_data = Box::leak(Box::new(
        EvtxChunkData::new(chunk_bytes, false).expect("chunk data"),
    ));
    let settings = Arc::new(ParserSettings::default());
    let chunk: &'static mut EvtxChunk<'static> = Box::leak(Box::new(
        chunk_data
            .parse(Arc::clone(&settings))
            .expect("chunk parse"),
    ));
    let record = {
        let mut record_iter = chunk.iter();
        record_iter.next().expect("record").expect("record ok")
    };
    let start = record.binxml_offset as usize;
    let end = start + record.binxml_size as usize;
    let bytes: &'static [u8] = &record.chunk.data[start..end];
    let chunk_ref = record.chunk;

    let bump_cold = RefCell::new(Bump::new());

    let mut group = c.benchmark_group("tree_build");
    group.throughput(Throughput::Bytes(bytes.len() as u64));

    group.bench_with_input(
        BenchmarkId::new("record_binxml_cold_cache", bytes.len()),
        &bytes,
        move |b, bytes| {
            b.iter(|| {
                {
                    let mut bump_mut = bump_cold.borrow_mut();
                    bump_mut.reset();
                }
                let bump_ref = bump_cold.borrow();
                let bytes = &bytes[..];
                let chunk = chunk_ref;
                let mut cache = TreeBuildCache::new(chunk_ref);
                let root =
                    build_tree_from_binxml_bytes_in_bump(bytes, chunk, &mut cache, &bump_ref)
                        .expect("build tree");
                black_box(root);
            });
        },
    );

    group.finish();
}

criterion_group!(benches, bench_tree_build);
criterion_main!(benches);
