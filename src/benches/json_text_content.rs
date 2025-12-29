#[macro_use]
extern crate criterion;

extern crate evtx;

use bumpalo::Bump;
use criterion::{BenchmarkId, Criterion, Throughput, black_box};
use evtx::binxml::bench::write_json_text_content;
use evtx::model::ir::{IrArena, Node, Text};
use evtx::Utf16LeSlice;
use utf16_simd::max_escaped_len;

struct Case {
    name: &'static str,
    utf16le: Vec<u8>,
}

fn utf16le_from_str(s: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len() * 2);
    for unit in s.encode_utf16() {
        out.extend_from_slice(&unit.to_le_bytes());
    }
    out
}

fn build_cases() -> Vec<Case> {
    vec![
        Case {
            name: "ascii",
            utf16le: utf16le_from_str("Hello &<>\"' World"),
        },
        Case {
            name: "long_ascii",
            utf16le: utf16le_from_str(
                "aaaaaaa&bbbbbbb&ccccccc<dddddd>eeeeee\"fffffff'gggggg",
            ),
        },
        Case {
            name: "mixed",
            utf16le: utf16le_from_str("Hello \u{20AC} \u{00E9} \u{1F600} World"),
        },
        Case {
            name: "win_path",
            utf16le: utf16le_from_str(r#"C:\Windows\System32\drivers\etc\hosts"#),
        },
        Case {
            name: "controls",
            utf16le: vec![
                0x00, 0x00, // NUL
                0x08, 0x00, // backspace
                0x0A, 0x00, // newline
                0x1F, 0x00, // unit separator
                b'"', 0x00, b'\\', 0x00,
            ],
        },
    ]
}

fn bench_json_text_content(c: &mut Criterion) {
    let cases = build_cases();
    let mut group = c.benchmark_group("json_text_content");

    for case in cases {
        let num_units = case.utf16le.len() / 2;
        group.throughput(Throughput::Bytes(case.utf16le.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("write_json_text_content", case.name),
            &case,
            |b, case| {
                let slice = Utf16LeSlice::new(&case.utf16le, num_units);
                let nodes = vec![Node::Text(Text::Utf16(slice))];

                let bump = Bump::new();
                let arena = IrArena::new_in(&bump);

                let max_len = max_escaped_len(num_units, false);
                let mut out = Vec::with_capacity(max_len);

                b.iter(|| {
                    out.clear();
                    write_json_text_content(&mut out, &arena, &nodes).unwrap();
                    black_box(out.len());
                });
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_json_text_content);
criterion_main!(benches);
