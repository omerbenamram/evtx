#![feature(test)]
extern crate test;
use memchr::memmem;
use std::collections::VecDeque;
use test::Bencher;


#[bench]
fn bench_regex_bytes(b: &mut Bencher) {
    let blob = include_bytes!("../samples/security.evtx");
    b.iter(|| {
        memmem::find_iter(blob, &[0x2a, 0x2a, 0x00, 0x00])
            .collect::<VecDeque<usize>>()
    });
}

