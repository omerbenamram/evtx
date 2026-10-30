//! Crafted records that would make the renderers do far more work than any real record.

mod fixtures;

use std::error::Error;

use evtx::{EvtxParser, ParserSettings, RenderedChunkItem};
use fixtures::*;

const SIZE_ERROR: &str = "record renders to more than 16 MiB";
const DEPTH_ERROR: &str = "rendered elements nested too deeply";

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

fn assert_fails_everywhere(evtx: &[u8], error: &str) {
    let results = render_everywhere(evtx);
    assert_eq!(results.len(), 4);
    for result in results {
        let err = result.unwrap_err();
        assert!(err.contains(error), "{err}");
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
    assert_fails_everywhere(&fan_out(8, 64, 1), SIZE_ERROR);
    // Past the nesting the compiled lanes take, each instance doubles the output.
    assert_fails_everywhere(&fan_out(40, 2, 1), SIZE_ERROR);
    // JSON renders unnamed `Data` in an `EventData` as one array.
    assert_fails_everywhere(&fan_out_named(["EventData", "Data"], 8, 64, 1), SIZE_ERROR);
}

/// A record whose template uses its value, a 4,000-character string, 8,000 times between what
/// `open` and `close` write: 32 MB.
fn repeated_value(open: fn(&mut BinXml), close: fn(&mut BinXml)) -> Vec<u8> {
    synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |l| {
                    open(l);
                    for _ in 0..8000 {
                        l.substitution(0, 0x01);
                    }
                    close(l);
                },
                vec![value(0x01, |text| {
                    text.utf16(&"x".repeat(4000));
                })],
            )
            .eof();
    })
}

#[test]
fn repeated_values_are_capped() {
    // In an element, in an attribute, and after an element.
    let in_element = repeated_value(
        |l| {
            l.open("L", true).close_start();
        },
        |l| {
            l.end_element();
        },
    );
    assert_fails_everywhere(&in_element, SIZE_ERROR);
    let in_attribute = repeated_value(
        |l| {
            l.open_with_attributes("L", true).attribute("A");
        },
        |l| {
            l.close_empty();
        },
    );
    assert_fails_everywhere(&in_attribute, SIZE_ERROR);
    let after_element = repeated_value(
        |l| {
            l.open("L", true)
                .close_start()
                .open("E", true)
                .close_empty();
        },
        |l| {
            l.end_element();
        },
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
    // 4,096 places in an `EventData` hold 16 `Data` of 300 characters: 20 MB, which JSON
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
                        fragment.text_element("Data", &"x".repeat(300));
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
    // 8 instances of a template naming 1,000 elements with one long name: 64 MB.
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
            .template_instance(|root| shared_names(root, 1000, true), vec![level(7)])
            .eof();
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
    // One element with 3,000 attributes that share an 8,000-character name: 24 MB.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open_with_attributes("R", true)
                        .attribute(&"N".repeat(8000))
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

#[test]
fn programs_shared_by_many_instances_are_capped() {
    // 60 values each hold an instance of one template that renders to 800 KB of XML.
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("R", true).close_start();
                    for slot in 0..60 {
                        root.open("A", true)
                            .close_start()
                            .substitution(slot, 0x21)
                            .end_element();
                    }
                    root.end_element();
                },
                (0..60)
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
    });
    assert_fails_everywhere(&evtx, SIZE_ERROR);
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
