//! Crafted records that would make the renderers do far more work than any real record.

mod fixtures;

use std::error::Error;

use evtx::{EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;

/// A record whose template nests its root `depth` elements deep, around `uses` elements that
/// each hold its one value. The value is a fragment with another instance of the same
/// template, `instances` deep, then a string: it renders `uses.pow(instances)` times.
fn fan_out(instances: u32, uses: usize, depth: usize) -> Vec<u8> {
    fn value_type(below: u32) -> u8 {
        if below == 0 { 0x01 } else { 0x21 }
    }
    fn value(binxml: &mut BinXml, below: u32) {
        if below == 0 {
            binxml.utf16("x");
        } else {
            binxml
                .fragment_header()
                .template_ref(value_type(below - 1), |inner| value(inner, below - 1))
                .eof();
        }
    }
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    for _ in 0..depth {
                        root.open("R", true).close_start();
                    }
                    for _ in 0..uses {
                        root.open("A", true)
                            .close_start()
                            .substitution(0, 0x21)
                            .end_element();
                    }
                    for _ in 0..depth {
                        root.end_element();
                    }
                },
                value_type(instances - 1),
                |inner| value(inner, instances - 1),
            )
            .eof();
    })
}

/// Record 1 of `evtx` through the records API and evtx_dump's chunk renderer, in XML and
/// JSON: the rendered size, or the error with its causes.
fn render_everywhere(evtx: &[u8]) -> Vec<Result<usize, String>> {
    fn causes(mut err: &dyn Error) -> String {
        let mut text = err.to_string();
        while let Some(source) = err.source() {
            text = format!("{text}: {source}");
            err = source;
        }
        text
    }
    let parser = || {
        EvtxParser::from_buffer(evtx.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    let mut results: Vec<_> = (parser().records())
        .map(|r| r.map(|r| r.data.len()).map_err(|e| causes(&e)))
        .collect();
    results
        .extend((parser().records_json()).map(|r| r.map(|r| r.data.len()).map_err(|e| causes(&e))));
    for chunk in parser()
        .chunks_xml_bytes(false)
        .chain(parser().chunks_json_bytes(false))
    {
        let mut start = 0;
        for item in &chunk.items {
            results.push(match item {
                RenderedChunkItem::Record { end, .. } => {
                    let size = end - start;
                    start = *end;
                    Ok(size)
                }
                RenderedChunkItem::Failed(err) => Err(causes(err)),
            });
        }
    }
    results
}

#[test]
fn small_fan_out_renders() {
    let (xml, json) = render_record(&fan_out(3, 2, 1), 1);
    assert_eq!(xml.matches("<A>x</A>").count(), 8);
    assert_eq!(json.matches(r#":"x""#).count(), 8);
}

#[test]
fn value_used_at_two_depths_renders_at_each() {
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    root.open("A", true)
                        .close_start()
                        .substitution(0, 0x21)
                        .end_element();
                    root.open("B", true).close_start();
                    root.open("C", true)
                        .close_start()
                        .substitution(0, 0x21)
                        .end_element();
                    root.end_element().end_element();
                },
                0x21,
                |value| {
                    value
                        .fragment_header()
                        .template_instance(
                            |d| {
                                d.open("D", true)
                                    .close_start()
                                    .substitution(0, 0x01)
                                    .end_element();
                            },
                            0x01,
                            |text| {
                                text.utf16("x");
                            },
                        )
                        .eof();
                },
            )
            .eof();
    });
    let (xml, json) = render_record(&evtx, 1);
    // The compiled lanes used to indent both by the last place.
    assert_eq!(
        xml.trim_end(),
        r#"<?xml version="1.0" encoding="utf-8"?>
<R>
  <A>
    <D>x</D>
  </A>
  <B>
    <C>
      <D>x</D>
    </C>
  </B>
</R>"#
    );
    assert_eq!(json, r#"{"R":{"A":{"D":"x"},"B":{"C":{"D":"x"}}}}"#);
}

#[test]
fn output_is_capped() {
    // 64 uses, 8 instances deep: the string would render 64^8 times, and preparing the
    // compiled lanes took 64^7 steps.
    for result in render_everywhere(&fan_out(8, 64, 1)) {
        let err = result.unwrap_err();
        assert!(err.contains("record renders to more than 16 MiB"), "{err}");
    }
    // Past the nesting the compiled lanes take, each instance doubles the output.
    for result in render_everywhere(&fan_out(40, 2, 1)) {
        let err = result.unwrap_err();
        assert!(err.contains("record renders to more than 16 MiB"), "{err}");
    }
}

#[test]
fn depth_is_capped() {
    // Each instance nests 101 elements: 1,212 levels would overflow the stack of the walkers.
    for result in render_everywhere(&fan_out(12, 1, 100)) {
        let err = result.unwrap_err();
        assert!(err.contains("rendered elements nested too deeply"), "{err}");
    }
}
