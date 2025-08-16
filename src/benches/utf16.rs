#[macro_use]
extern crate criterion;
extern crate evtx;

use criterion::{black_box, Criterion};
use evtx::utf16_opt::decode_utf16_trim;
use std::char::decode_utf16;

fn baseline_decode(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len());
    for r in decode_utf16(units.iter().copied()) {
        match r {
            Ok(ch) => {
                if ch == '\0' { break; }
                out.push(ch);
            }
            Err(_) => break,
        }
    }
    out.trim_end().to_owned()
}

fn bench_utf16_decode(c: &mut Criterion) {
    // ASCII-like
    let ascii: Vec<u16> = "The quick brown fox jumps over the lazy dog    \0".encode_utf16().collect();
    // Mixed BMP
    let mixed: Vec<u16> = "Δοκιμή κειμένου με ελληνικούς χαρακτήρες    \0".encode_utf16().collect();
    // Longer
    let long: Vec<u16> = "A".repeat(4096).chars().collect::<Vec<char>>().into_iter().map(|ch| ch as u16).chain(std::iter::once(0)).collect();

    c.bench_function("utf16_opt::decode_utf16_trim ascii", |b| {
        b.iter(|| decode_utf16_trim(black_box(&ascii)).unwrap())
    });
    c.bench_function("baseline decode_utf16 ascii", |b| {
        b.iter(|| baseline_decode(black_box(&ascii)))
    });

    c.bench_function("utf16_opt::decode_utf16_trim mixed", |b| {
        b.iter(|| decode_utf16_trim(black_box(&mixed)).unwrap())
    });
    c.bench_function("baseline decode_utf16 mixed", |b| {
        b.iter(|| baseline_decode(black_box(&mixed)))
    });

    c.bench_function("utf16_opt::decode_utf16_trim long", |b| {
        b.iter(|| decode_utf16_trim(black_box(&long)).unwrap())
    });
    c.bench_function("baseline decode_utf16 long", |b| {
        b.iter(|| baseline_decode(black_box(&long)))
    });
}

criterion_group!(benches, bench_utf16_decode);
criterion_main!(benches);


