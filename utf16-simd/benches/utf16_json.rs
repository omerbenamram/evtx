use criterion::{black_box, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use sonic_rs::format::{CompactFormatter, Formatter};
use std::mem::MaybeUninit;

use utf16_simd::{
    Scratch, escape_json_utf16le, escape_json_utf16le_into, escape_json_utf16le_scalar,
    max_escaped_len,
};

struct Case {
    name: &'static str,
    utf16le: Vec<u8>,
    units: Vec<u16>,
}

fn utf16le_from_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn units_from_utf16le(bytes: &[u8]) -> Vec<u16> {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        units.push(u16::from_le_bytes([chunk[0], chunk[1]]));
    }
    units
}

fn build_cases() -> Vec<Case> {
    let mut cases = Vec::new();

    let ascii = utf16le_from_str("Hello &<>\"' World");
    cases.push(Case {
        name: "ascii",
        units: units_from_utf16le(&ascii),
        utf16le: ascii,
    });

    let long_ascii = utf16le_from_str("aaaaaaa&bbbbbbb&ccccccc<dddddd>eeeeee\"fffffff'gggggg");
    cases.push(Case {
        name: "long_ascii",
        units: units_from_utf16le(&long_ascii),
        utf16le: long_ascii,
    });

    let mixed = utf16le_from_str("Hello \u{20AC} \u{00E9} \u{1F600} World");
    cases.push(Case {
        name: "mixed",
        units: units_from_utf16le(&mixed),
        utf16le: mixed,
    });

    let win_path = utf16le_from_str(r#"C:\Windows\System32\drivers\etc\hosts"#);
    cases.push(Case {
        name: "win_path",
        units: units_from_utf16le(&win_path),
        utf16le: win_path,
    });

    let controls = vec![
        0x00, 0x00, // NUL
        0x08, 0x00, // backspace
        0x0A, 0x00, // newline
        0x1F, 0x00, // unit separator
        b'"', 0x00,
        b'\\', 0x00,
    ];
    cases.push(Case {
        name: "controls",
        units: units_from_utf16le(&controls),
        utf16le: controls,
    });

    let mut dirty_block_str = String::new();
    for _ in 0..1000 {
        dirty_block_str.push_str("aaaa\"bbb");
    }
    let dirty_block = utf16le_from_str(&dirty_block_str);
    cases.push(Case {
        name: "dirty_block_repetitive",
        units: units_from_utf16le(&dirty_block),
        utf16le: dirty_block,
    });

    cases
}

fn bench_utf16_json(c: &mut Criterion) {
    let cases = build_cases();
    let mut group = c.benchmark_group("utf16_json");

    for case in cases {
        let num_units = case.units.len();
        let max_len = max_escaped_len(num_units, true);
        group.throughput(Throughput::Bytes(case.utf16le.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("scalar_direct", case.name),
            &case,
            |b, case| {
                let mut out = vec![MaybeUninit::uninit(); max_len];
                b.iter(|| {
                    let len =
                        escape_json_utf16le_scalar(&case.utf16le, num_units, &mut out, true);
                    black_box(len);
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("vec_zeroed", case.name),
            &case,
            |b, case| {
                let mut out = Vec::with_capacity(max_len);
                b.iter(|| {
                    escape_json_utf16le_into(&case.utf16le, num_units, &mut out, true);
                    black_box(out.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("scratch_reuse", case.name),
            &case,
            |b, case| {
                let mut scratch = Scratch::new();
                b.iter(|| {
                    let out = scratch.escape_json_utf16le(&case.utf16le, num_units, true);
                    black_box(out.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("write_vec", case.name),
            &case,
            |b, case| {
                let mut scratch = Scratch::new();
                let mut out: Vec<u8> = Vec::new();
                b.iter(|| {
                    out.clear();
                    scratch
                        .write_json_utf16le_to(&mut out, &case.utf16le, num_units, true)
                        .unwrap();
                    black_box(out.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("utf8_sonic", case.name),
            &case,
            |b, case| {
                let mut fmt = CompactFormatter;
                let mut out = Vec::with_capacity(max_len);
                b.iter(|| {
                    let s = String::from_utf16_lossy(&case.units);
                    out.clear();
                    fmt.write_string_fast(&mut out, &s, true).unwrap();
                    black_box(out.len());
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("simd_direct", case.name),
            &case,
            |b, case| {
                let mut out = vec![MaybeUninit::uninit(); max_len];
                b.iter(|| {
                    let len = escape_json_utf16le(&case.utf16le, num_units, &mut out, true);
                    black_box(len);
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_utf16_json);
criterion_main!(benches);
