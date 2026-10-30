//! Crafted records that would make the renderers do far more work than any real record.

mod fixtures;

use std::error::Error;
use std::sync::Arc;

use evtx::mta::MtaFile;
use evtx::{EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;

const SIZE_ERROR: &str = "record renders to more than 16 MiB";
const NODES_ERROR: &str = "record renders more than 2^20 nodes";
const DEPTH_ERROR: &str = "rendered elements nested too deeply";
const CHUNK_ERROR: &str = "earlier records of the chunk spent its render budget";

/// A record whose template nests its root `depth` elements deep, around `uses` elements that
/// each hold its one value. The value is a fragment with another instance of the same
/// template, `instances` deep, then a string: it renders `uses.pow(instances)` times.
fn fan_out(instances: u32, uses: usize, depth: usize) -> Vec<u8> {
    fan_out_named(["R", "A"], instances, uses, depth)
}

/// As `fan_out`, with the root and the value's elements named `names`.
fn fan_out_named(names: [&str; 2], instances: u32, uses: usize, depth: usize) -> Vec<u8> {
    fn level<'v>(below: u32) -> Value<'v> {
        if below == 0 {
            value(0x01, |text| {
                text.utf16("x");
            })
        } else {
            value(0x21, move |fragment| {
                fragment
                    .fragment_header()
                    .template_ref(vec![level(below - 1)])
                    .eof();
            })
        }
    }
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    for _ in 0..depth {
                        root.open(names[0], true).close_start();
                    }
                    for _ in 0..uses {
                        root.open(names[1], true)
                            .close_start()
                            .substitution(0, 0x21)
                            .end_element();
                    }
                    for _ in 0..depth {
                        root.end_element();
                    }
                },
                vec![level(instances - 1)],
            )
            .eof();
    })
}

/// Record 1 of `evtx` through the records API and evtx_dump's chunk renderer, in XML and
/// JSON: the rendered size, or the error with its causes.
fn render_everywhere(evtx: &[u8]) -> Vec<Result<usize, String>> {
    // For each record here, the checks that stop a record as it renders keep its output under
    // twice the cap.
    render_passes(evtx, 32 << 20).concat()
}

/// Each record of `evtx` through the records API in XML and JSON, then evtx_dump's chunk
/// renderer in XML and JSON: the rendered size, or the error with its causes. The chunk
/// renderer's buffer keeps the capacity its records took, which must stay within `capacity`.
fn render_passes(evtx: &[u8], capacity: usize) -> [Vec<Result<usize, String>>; 4] {
    render_passes_with(evtx, capacity, ParserSettings::new())
}

/// As `render_passes`, with `settings`.
fn render_passes_with(
    evtx: &[u8],
    capacity: usize,
    settings: ParserSettings,
) -> [Vec<Result<usize, String>>; 4] {
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
            .with_configuration(settings.clone().num_threads(1))
    };
    let chunk_renderer = |chunks: &mut dyn Iterator<Item = evtx::RenderedChunk>| {
        let mut results = Vec::new();
        for chunk in chunks {
            assert!(
                chunk.data.capacity() <= capacity,
                "{} bytes",
                chunk.data.capacity()
            );
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
    };
    [
        (parser().records())
            .map(|r| r.map(|r| r.data.len()).map_err(|e| causes(&e)))
            .collect(),
        (parser().records_json())
            .map(|r| r.map(|r| r.data.len()).map_err(|e| causes(&e)))
            .collect(),
        chunk_renderer(&mut parser().chunks_xml_bytes(false)),
        chunk_renderer(&mut parser().chunks_json_bytes(false)),
    ]
}

fn assert_fails_everywhere(evtx: &[u8], error: &str) {
    assert_fails_as(evtx, Some(error), Some(error));
}

/// Record 1 of `evtx` fails with `xml` in XML and `json` in JSON, in both APIs. `None` for a
/// format that renders it, within the cap.
fn assert_fails_as(evtx: &[u8], xml: Option<&str>, json: Option<&str>) {
    let results = render_everywhere(evtx);
    assert_eq!(results.len(), 4);
    for (result, error) in results.into_iter().zip([xml, json, xml, json]) {
        match error {
            Some(error) => {
                let err = result.unwrap_err();
                assert!(err.contains(error), "{err}");
            }
            None => assert!(result.unwrap() <= 16 << 20),
        }
    }
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
                vec![value(0x21, |fragment| {
                    fragment
                        .fragment_header()
                        .template_instance(
                            |d| {
                                d.open("D", true)
                                    .close_start()
                                    .substitution(0, 0x01)
                                    .end_element();
                            },
                            vec![value(0x01, |text| {
                                text.utf16("x");
                            })],
                        )
                        .eof();
                })],
            )
            .eof();
    });
    let (xml, json) = render_record(&evtx, 1);
    // The compiled lanes used to indent both as the last one.
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
fn fan_out_is_capped() {
    // 64 uses, 8 instances deep: the string would render 64^8 times, and preparing the
    // records API's compiled lanes took 64^7 steps.
    assert_fails_everywhere(&fan_out(8, 64, 1), NODES_ERROR);
    // Past the nesting the compiled lanes take, each instance doubles the output. XML indents
    // it, so it reaches 16 MiB first.
    assert_fails_as(&fan_out(40, 2, 1), Some(SIZE_ERROR), Some(NODES_ERROR));
    // JSON renders unnamed `Data` in an `EventData` as one array.
    assert_fails_everywhere(&fan_out_named(["EventData", "Data"], 8, 64, 1), NODES_ERROR);
}

/// A record whose template uses its value, an element with `attributes` attributes of `values`
/// NULL values each, `uses` times. The value holds another instance of the template, `instances`
/// deep: the element is visited `uses.pow(instances)` times, and renders to a few bytes.
fn null_attributes(instances: u32, uses: usize, attributes: usize, values: usize) -> Vec<u8> {
    fn level<'v>(below: u32, attributes: usize, values: usize) -> Value<'v> {
        value(0x21, move |fragment| {
            fragment.fragment_header();
            if below == 0 {
                fragment.open_with_attributes("X", false).attribute("A");
                for attribute in 0..attributes {
                    if attribute > 0 {
                        fragment.reattribute();
                    }
                    for _ in 0..values {
                        fragment.raw(&[0x05, 0x00]);
                    }
                }
                fragment.close_empty();
            } else {
                fragment.template_ref(vec![level(below - 1, attributes, values)]);
            }
            fragment.eof();
        })
    }
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    for _ in 0..uses {
                        root.substitution(0, 0x21);
                    }
                    root.end_element();
                },
                vec![level(instances - 1, attributes, values)],
            )
            .eof();
    })
}

#[test]
fn visits_are_capped() {
    // 64^4 visits of 6,500 attributes that render nothing kept the walkers busy for a minute.
    assert_fails_everywhere(&null_attributes(4, 64, 6500, 1), NODES_ERROR);
    // 64^3 visits of one attribute of 6,500 values: 1.7 billion values read.
    assert_fails_everywhere(&null_attributes(3, 64, 1, 6500), NODES_ERROR);
}

/// A record whose template uses its value, a string of `chars` characters, `uses` times between
/// what `open` and `close` write.
fn repeated_value(
    uses: usize,
    chars: usize,
    open: fn(&mut BinXml),
    close: fn(&mut BinXml),
) -> Vec<u8> {
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |l| {
                    open(l);
                    for _ in 0..uses {
                        l.substitution(0, 0x01);
                    }
                    close(l);
                },
                vec![value(0x01, |text| {
                    text.utf16(&"x".repeat(chars));
                })],
            )
            .eof();
    })
}

fn open_l(l: &mut BinXml) {
    l.open("L", true).close_start();
}

fn close_l(l: &mut BinXml) {
    l.end_element();
}

#[test]
fn repeated_values_are_capped() {
    // A 4,500-character string 8,000 times, 36 MB: in an element, in an attribute, and after an
    // element.
    assert_fails_everywhere(&repeated_value(8000, 4500, open_l, close_l), SIZE_ERROR);
    let in_attribute = repeated_value(
        8000,
        4500,
        |l| {
            l.open_with_attributes("L", true).attribute("A");
        },
        |l| {
            l.close_empty();
        },
    );
    assert_fails_everywhere(&in_attribute, SIZE_ERROR);
    let after_element = repeated_value(
        8000,
        4500,
        |l| {
            l.open("L", true)
                .close_start()
                .open("E", true)
                .close_empty();
        },
        close_l,
    );
    assert_fails_everywhere(&after_element, SIZE_ERROR);
    // 50 places hold an element that holds a 1,000-character string 1,000 times: 50 MB.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    for _ in 0..50 {
                        root.substitution(0, 0x21);
                    }
                    root.end_element();
                },
                vec![value(0x21, |fragment| {
                    fragment
                        .fragment_header()
                        .template_instance(
                            |l| {
                                l.open("L", true).close_start();
                                for _ in 0..1000 {
                                    l.substitution(0, 0x01);
                                }
                                l.end_element();
                            },
                            vec![value(0x01, |text| {
                                text.utf16(&"x".repeat(1000));
                            })],
                        )
                        .eof();
                })],
            )
            .eof();
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
    // 4,096 places in an `EventData` hold 16 `Data` of 600 characters: 40 MB, which JSON
    // renders as one array.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("E", true).close_start();
                    root.open("EventData", true).close_start();
                    for _ in 0..4096 {
                        root.substitution(0, 0x21);
                    }
                    root.end_element().end_element();
                },
                vec![value(0x21, |fragment| {
                    fragment.fragment_header();
                    for _ in 0..16 {
                        fragment.text_element("Data", &"x".repeat(600));
                    }
                    fragment.eof();
                })],
            )
            .eof();
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
}

/// A template of `elements` empty elements that share one 4,000-character name, and then,
/// `with_value`, an `<A>` that holds its value.
fn shared_names(root: &mut BinXml, elements: usize, with_value: bool) {
    root.open("R", true).close_start();
    root.open(&"N".repeat(4000), true).close_empty();
    for _ in 1..elements {
        root.reopen(true).close_empty();
    }
    if with_value {
        root.open("A", true)
            .close_start()
            .substitution(0, 0x21)
            .end_element();
    }
    root.end_element();
}

#[test]
fn shared_names_are_capped() {
    // 9 instances of a template naming 1,000 elements with one long name: 72 MB.
    fn level<'v>(below: u32) -> Value<'v> {
        value(0x21, move |fragment| {
            fragment.fragment_header();
            if below == 0 {
                fragment.text_element("Z", "z");
            } else {
                fragment.template_ref(vec![level(below - 1)]);
            }
            fragment.eof();
        })
    }
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(|root| shared_names(root, 1000, true), vec![level(8)])
            .eof();
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
    // One element with 3,000 attributes that share an 11,500-character name: 35 MB.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open_with_attributes("R", true)
                        .attribute(&"N".repeat(11500))
                        .text("x");
                    for _ in 1..3000 {
                        root.reattribute().text("x");
                    }
                    root.close_empty();
                },
                vec![],
            )
            .eof();
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
}

/// Each pass through a chunk of `evtx`'s: how many records render before its records start to
/// fail with `CHUNK_ERROR`, and the bytes those render.
fn rendered_before_chunk_error(evtx: &[u8]) -> Vec<(usize, usize)> {
    render_passes(evtx, 128 << 20)
        .into_iter()
        .map(|pass| {
            let first_error = pass.iter().position(|r| r.is_err()).unwrap();
            let chunk_errors = &pass[first_error..];
            assert!(chunk_errors.len() > 1);
            for err in chunk_errors {
                let err = err.as_ref().unwrap_err();
                assert!(err.contains(CHUNK_ERROR), "{err}");
            }
            let rendered = &pass[..first_error];
            (
                rendered.len(),
                rendered.iter().map(|r| r.as_ref().unwrap()).sum(),
            )
        })
        .collect()
}

#[test]
fn chunks_are_capped() {
    // Records share their chunk's templates: each of these 40 instances of a template that names
    // 1,000 elements with one long name renders 8 MB of XML, 320 MB in all.
    let evtx = synthetic_evtx_records(40, |i, record| {
        record.fragment_header();
        if i == 0 {
            record.template_instance(|root| shared_names(root, 1000, false), vec![]);
        } else {
            record.template_ref(vec![]);
        }
        record.eof();
    });
    for (rendered, bytes) in rendered_before_chunk_error(&evtx) {
        assert!(rendered >= 4, "{rendered}");
        assert!(bytes <= 64 << 20, "{bytes}");
    }
    // Records that fail on the node cap count too, and so do the nodes of records that render
    // little: each of these visits an element of 2,000 NULL attributes 64^3 times.
    fn level<'v>(below: u32) -> Value<'v> {
        if below == 0 {
            value(0x00, |_| {})
        } else {
            value(0x21, move |fragment| {
                fragment
                    .fragment_header()
                    .template_ref(vec![level(below - 1)])
                    .eof();
            })
        }
    }
    let evtx = synthetic_evtx_records(10, |i, record| {
        record.fragment_header();
        if i == 0 {
            record.template_instance(
                |root| {
                    root.open("R", true).close_start();
                    root.open_with_attributes("X", true)
                        .attribute("A")
                        .raw(&[0x05, 0x00]);
                    for _ in 1..2000 {
                        root.reattribute().raw(&[0x05, 0x00]);
                    }
                    root.close_empty();
                    for _ in 0..64 {
                        root.substitution(0, 0x21);
                    }
                    root.end_element();
                },
                vec![level(3)],
            );
        } else {
            record.template_ref(vec![level(3)]);
        }
        record.eof();
    });
    for pass in render_passes(&evtx, 32 << 20) {
        assert_eq!(pass.len(), 10);
        for (i, result) in pass.iter().enumerate() {
            let err = result.as_ref().unwrap_err();
            let error = if i < 4 { NODES_ERROR } else { CHUNK_ERROR };
            assert!(err.contains(error), "record {}: {err}", i + 1);
        }
    }
    // A compiled template's fragment renders through the walkers, whose nodes count too: each of
    // these records visits 270,000 nodes in its `<F>`.
    let evtx = synthetic_evtx_records(16, |_, record| {
        record
            .fragment_header()
            .template_instance(
                |e| {
                    e.open("E", true)
                        .close_start()
                        .substitution(0, 0x21)
                        .end_element();
                },
                vec![value(0x21, |fragment| {
                    fragment
                        .fragment_header()
                        .open("F", false)
                        .close_start()
                        .template_instance(
                            |root| {
                                root.open("R", true).close_start();
                                for _ in 0..64 {
                                    root.substitution(0, 0x21);
                                }
                                root.end_element();
                            },
                            vec![level(2)],
                        )
                        .end_element()
                        .eof();
                })],
            )
            .eof();
    });
    for (rendered, _) in rendered_before_chunk_error(&evtx) {
        assert_eq!(rendered, 12);
    }
    // So do records that the compiled lanes fail on the size cap.
    let evtx = synthetic_evtx_records(4, |_, record| shared_program(record));
    for pass in render_passes(&evtx, 128 << 20) {
        for (i, result) in pass.iter().enumerate() {
            let err = result.as_ref().unwrap_err();
            let error = if i < 3 { SIZE_ERROR } else { CHUNK_ERROR };
            assert!(err.contains(error), "record {}: {err}", i + 1);
        }
    }
}

#[test]
fn each_pass_over_a_chunk_has_its_budget() {
    // As in `chunks_are_capped`: 8 MB of XML a record.
    let evtx = synthetic_evtx_records(10, |i, record| {
        record.fragment_header();
        if i == 0 {
            record.template_instance(|root| shared_names(root, 1000, false), vec![]);
        } else {
            record.template_ref(vec![]);
        }
        record.eof();
    });
    let mut parser = EvtxParser::from_buffer(evtx).unwrap();
    let mut data = parser.chunks().next().unwrap().unwrap();
    let mut chunk = data.parse(Arc::new(ParserSettings::new())).unwrap();
    let mut rendered = || {
        (chunk.iter())
            .map(|record| record.and_then(|record| record.into_xml()))
            .filter(Result::is_ok)
            .count()
    };
    assert_eq!([rendered(), rendered()], [7, 7]);
}

/// Opens an `<Event>` whose `<System>` names its provider, `P`.
fn open_event(root: &mut BinXml) {
    root.open("Event", true).close_start();
    root.open("System", true).close_start();
    (root.open_with_attributes("Provider", true))
        .attribute("Name")
        .text("P")
        .close_empty();
    root.end_element();
}

/// Settings with an `.MTA` file that names level 0 of `P` with 16 Mi characters: the
/// `RenderingInfo` of an event of `P` passes the cap on its own. The JSON writer reserves six
/// bytes a character for a string, so the chunk renderer's buffer reaches 96 MiB.
fn long_level_name() -> ParserSettings {
    let mta = mta_naming_level("P", &"L".repeat(16 << 20));
    ParserSettings::new().mta(Some(Arc::new(MtaFile::from_bytes(&mta).unwrap())))
}

#[test]
fn rendering_info_counts_toward_the_caps() {
    let evtx = synthetic_evtx_records(5, |i, record| {
        record.fragment_header();
        if i == 0 {
            record.template_instance(
                |event| {
                    open_event(event);
                    event.end_element();
                },
                vec![],
            );
        } else {
            record.template_ref(vec![]);
        }
        record.eof();
    });
    for pass in render_passes_with(&evtx, 100 << 20, long_level_name()) {
        assert_eq!(pass.len(), 5);
        for (i, result) in pass.iter().enumerate() {
            let err = result.as_ref().unwrap_err();
            let error = if i < 3 { SIZE_ERROR } else { CHUNK_ERROR };
            assert!(err.contains(error), "record {}: {err}", i + 1);
        }
    }
}

/// A record whose 100 values each hold an instance of one template that renders to 800 KB of
/// XML. Both templates compile.
fn shared_program(record: &mut BinXml) {
    record
        .fragment_header()
        .template_instance(
            |root| {
                root.open("R", true).close_start();
                for slot in 0..100 {
                    root.open("A", true)
                        .close_start()
                        .substitution(slot, 0x21)
                        .end_element();
                }
                root.end_element();
            },
            (0..100)
                .map(|slot| {
                    value(0x21, move |fragment| {
                        fragment.fragment_header();
                        if slot == 0 {
                            fragment.template_instance(
                                |shared| shared_names(shared, 100, false),
                                vec![],
                            );
                        } else {
                            fragment.template_ref(vec![]);
                        }
                        fragment.eof();
                    })
                })
                .collect(),
        )
        .eof();
}

#[test]
fn programs_shared_by_many_instances_are_capped() {
    assert_fails_everywhere(&synthetic_evtx(shared_program), SIZE_ERROR);
}

/// A record whose template is an event of `P` that holds `a` `<A>` elements and then `b` `<B>`
/// elements, each around a value. Each `<A>`'s holds an instance of a template of one element
/// with 2,000 NULL attributes, and each `<B>`'s an instance of `shared_names`' of 100 elements:
/// 800 KB of XML.
fn visits_then_output(a: u16, b: u16) -> Vec<u8> {
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    open_event(root);
                    for slot in 0..a + b {
                        match slot {
                            0 => root.open("A", true),
                            _ if slot == a => root.open("B", true),
                            _ => root.reopen(true),
                        }
                        .close_start()
                        .substitution(slot, 0x21)
                        .end_element();
                    }
                    root.end_element();
                },
                (0..a + b)
                    .map(|slot| {
                        value(0x21, move |fragment| {
                            fragment.fragment_header();
                            if slot == 0 {
                                fragment.template_instance(
                                    |x| {
                                        (x.open_with_attributes("X", true).attribute("A"))
                                            .raw(&[0x05, 0x00]);
                                        for _ in 1..2000 {
                                            x.reattribute().raw(&[0x05, 0x00]);
                                        }
                                        x.close_empty();
                                    },
                                    vec![],
                                );
                            } else if slot == a {
                                fragment.template_instance(
                                    |shared| shared_names(shared, 100, false),
                                    vec![],
                                );
                            } else {
                                fragment.template_ref(vec![]);
                            }
                            fragment.eof();
                        })
                    })
                    .collect(),
            )
            .eof();
    })
}

#[test]
fn size_errors_are_not_rendered_again_from_the_tree() {
    // The compiled lanes don't count nodes, and fail this record on its `<B>`s: 40 MB of XML,
    // 20 MB of JSON. Rendering it again from its tree would first build the 1.2 million
    // attributes of its `<A>`s, and then fail on their visits.
    assert_fails_everywhere(&visits_then_output(600, 50), SIZE_ERROR);
    // Nor is a record that its `RenderingInfo` takes past the cap. The records API renders a
    // record from its tree when it has an `.MTA` file, so there this one fails on the visits.
    let passes = render_passes_with(&visits_then_output(600, 0), 100 << 20, long_level_name());
    for (pass, error) in passes
        .iter()
        .zip([NODES_ERROR, NODES_ERROR, SIZE_ERROR, SIZE_ERROR])
    {
        let err = pass[0].as_ref().unwrap_err();
        assert!(err.contains(error), "{err}");
    }
}

/// A template that nests `<Data>` `depth` elements deep (their indentation is in its compiled
/// program) and gives it an array of `items` empty strings.
fn deep_array(depth: usize, items: usize) -> Vec<u8> {
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    for _ in 1..depth {
                        root.reopen(true).close_start();
                    }
                    root.open("Data", true)
                        .close_start()
                        .substitution(0, 0x81)
                        .end_element();
                    for _ in 0..depth {
                        root.end_element();
                    }
                },
                vec![value(0x81, move |array| {
                    for _ in 0..items {
                        array.raw(&[0, 0]);
                    }
                })],
            )
            .eof();
    })
}

#[test]
fn array_items_are_capped() {
    // A 59 KB record: each of its 26,000 items repeats 1,022 spaces of indentation twice, 54 MB
    // in all. JSON doesn't indent.
    assert_fails_as(&deep_array(511, 26000), Some(SIZE_ERROR), None);
}

/// A record whose template holds `uses + 1` values: `uses` instances of a template of 124
/// elements that share a 4,000-character name, then a chain of 8 instances of a template that
/// ends with 110 more of them, after the value it holds.
fn output_after_instances(uses: u16) -> Vec<u8> {
    let name = "N".repeat(4000);
    let name = &name;
    fn link<'v>(below: u32) -> Value<'v> {
        if below == 0 {
            value(0x01, |text| {
                text.utf16("x");
            })
        } else {
            value(0x21, move |fragment| {
                fragment
                    .fragment_header()
                    .template_ref(vec![link(below - 1)])
                    .eof();
            })
        }
    }
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("Q", true).close_start();
                    for slot in 0..=uses {
                        root.open("P", true)
                            .close_start()
                            .substitution(slot, 0x21)
                            .end_element();
                    }
                    root.end_element();
                },
                (0..=uses)
                    .map(|slot| {
                        value(0x21, move |fragment| {
                            fragment.fragment_header();
                            if slot == 0 {
                                fragment.template_instance(
                                    |u| {
                                        u.open("U", true).close_start();
                                        u.open(name, true).close_empty();
                                        for _ in 1..124 {
                                            u.reopen(true).close_empty();
                                        }
                                        u.end_element();
                                    },
                                    vec![],
                                );
                            } else if slot < uses {
                                fragment.template_ref(vec![]);
                            } else {
                                fragment.template_instance(
                                    |t| {
                                        t.open("T", true).close_start();
                                        t.open("A", true)
                                            .close_start()
                                            .substitution(0, 0x21)
                                            .end_element();
                                        t.open(name, true).close_empty();
                                        for _ in 1..110 {
                                            t.reopen(true).close_empty();
                                        }
                                        t.end_element();
                                    },
                                    vec![link(7)],
                                );
                            }
                            fragment.eof();
                        })
                    })
                    .collect(),
            )
            .eof();
    })
}

#[test]
fn output_after_the_last_instance_is_capped() {
    // 16 instances of 1 MB take the output to just under 16 MiB, then each instance of the
    // chain writes 880 KB more once the one it holds is rendered: 23 MB. JSON writes each name
    // once, 11 MB.
    assert_fails_as(&output_after_instances(16), Some(SIZE_ERROR), None);
}

/// A template that nests `depth` elements that share a name of `chars` CJK characters.
fn long_named_nest(depth: usize, chars: usize) -> Vec<u8> {
    let name = "\u{4e2d}".repeat(chars);
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open(&name, true).close_start();
                    for _ in 2..depth {
                        root.reopen(true).close_start();
                    }
                    root.reopen(true).close_empty();
                    for _ in 1..depth {
                        root.end_element();
                    }
                },
                vec![],
            )
            .eof();
    })
}

#[test]
fn closing_tags_are_capped() {
    // 348 elements whose name renders to 48 KB, too long to compile: opening them takes 16 MB
    // and closing them 16 MB more. JSON writes each name once.
    assert_fails_as(&long_named_nest(348, 16000), Some(SIZE_ERROR), None);
}

#[test]
fn the_last_node_is_capped() {
    // A 2,397-character string 7,000 times in an element: the output is under 16 MiB before
    // the last one, and 1,800 bytes over after it.
    assert_fails_everywhere(&repeated_value(7000, 2397, open_l, close_l), SIZE_ERROR);
}

/// A template that nests `outer` elements around two uses of its value, a template that
/// nests 300 elements around an empty one: `outer + 301` levels.
fn nested(outer: usize) -> Vec<u8> {
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    for _ in 0..outer {
                        root.open("R", true).close_start();
                    }
                    root.substitution(0, 0x21).substitution(0, 0x21);
                    for _ in 0..outer {
                        root.end_element();
                    }
                },
                vec![value(0x21, |fragment| {
                    fragment
                        .fragment_header()
                        .template_instance(
                            |inner| {
                                for _ in 0..300 {
                                    inner.open("I", true).close_start();
                                }
                                inner.open("E", true).close_empty();
                                for _ in 0..300 {
                                    inner.end_element();
                                }
                            },
                            vec![],
                        )
                        .eof();
                })],
            )
            .eof();
    })
}

#[test]
fn depth_is_capped() {
    // As in one stream, 512 elements can nest around an empty one, in both formats.
    let (xml, json) = render_record(&nested(212), 1);
    assert_eq!(xml.matches("<E>").count(), 2);
    assert_eq!(json.matches(r#""E":null"#).count(), 2);
    assert_fails_everywhere(&nested(213), DEPTH_ERROR);
    // Each instance nests 101 elements: 1,212 levels would overflow the stack of the walkers.
    assert_fails_everywhere(&fan_out(12, 1, 100), DEPTH_ERROR);
}

#[test]
fn many_children_of_one_name_get_distinct_keys() {
    // 4,096 places hold a fragment of 16 `<D/>`, then one more: 65,537 children named D.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    for _ in 0..4096 {
                        root.substitution(0, 0x21);
                    }
                    root.open("D", true).close_empty().end_element();
                },
                vec![value(0x21, |fragment| {
                    fragment.fragment_header();
                    for _ in 0..16 {
                        fragment.open("D", false).close_empty();
                    }
                    fragment.eof();
                })],
            )
            .eof();
    });
    let (xml, json) = render_record(&evtx, 1);
    assert_eq!(xml.matches("<D>").count(), 65537);
    assert!(
        json.ends_with(r#""D_65536":null}}"#),
        "{}",
        &json[json.len() - 100..]
    );
}

#[test]
fn children_of_many_names_get_distinct_keys() {
    // Each of 20 names twice, more names than real elements have: in the template, where a
    // value adds two more, and in a value.
    let names: Vec<String> = (0..20).map(|i| format!("N{i}")).collect();
    let children = |parent: &mut BinXml, in_template_definition| {
        for name in names.iter().chain(&names) {
            parent.open(name, in_template_definition).close_empty();
        }
    };
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    root.open("T", true).close_start();
                    children(root, true);
                    root.substitution(0, 0x21).end_element();
                    root.open("V", true)
                        .close_start()
                        .substitution(1, 0x21)
                        .end_element();
                    root.end_element();
                },
                vec![
                    value(0x21, |fragment| {
                        fragment.fragment_header();
                        fragment.open("N0", false).close_empty();
                        fragment.open("N19", false).close_empty();
                        fragment.eof();
                    }),
                    value(0x21, |fragment| {
                        fragment.fragment_header();
                        children(fragment, false);
                        fragment.eof();
                    }),
                ],
            )
            .eof();
    });
    let (_, json) = render_record(&evtx, 1);
    let members: Vec<String> = (names.iter().map(|name| format!(r#""{name}":null"#)))
        .chain(names.iter().map(|name| format!(r#""{name}_1":null"#)))
        .collect();
    let members = members.join(",");
    assert!(
        json.contains(&format!(r#""T":{{{members},"N0_2":null,"N19_2":null}}"#)),
        "{json}"
    );
    assert!(json.contains(&format!(r#""V":{{{members}}}"#)), "{json}");
}
