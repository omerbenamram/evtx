//! Crafted records that Windows would not write, which must not render as something they
//! are not.

mod fixtures;

use fixtures::*;

/// A record with one name in each place a name can be: `element`, `attribute`, `entity` and
/// `target` in `<Event><element attribute="v">&entity;<?target data?></element></Event>`.
/// From a template, as plain tokens, or as plain tokens in a value of a template.
#[derive(Clone, Copy, Debug)]
enum Place {
    Template,
    Record,
    Value,
}

fn named([element, attribute, entity, target]: [&str; 4], place: Place) -> Vec<u8> {
    let event = |root: &mut BinXml, in_template: bool| {
        root.open("Event", in_template).close_start();
        root.open_with_attributes(element, in_template)
            .attribute(attribute)
            .text("v")
            .close_start()
            .entity_ref(entity)
            .processing_instruction(target, "data")
            .end_element();
        root.end_element();
    };
    synthetic_evtx(|record| {
        record.fragment_header();
        match place {
            Place::Template => {
                record.template_instance(|root| event(root, true), vec![]);
            }
            Place::Record => event(record, false),
            Place::Value => {
                record.template_instance(
                    |root| {
                        root.open("Outer", true)
                            .close_start()
                            .substitution(0, 0x21)
                            .end_element();
                    },
                    vec![value(0x21, |fragment| {
                        fragment.fragment_header();
                        event(fragment, false);
                        fragment.eof();
                    })],
                );
            }
        }
        record.eof();
    })
}

const PLACES: [Place; 3] = [Place::Template, Place::Record, Place::Value];

#[test]
fn names_that_are_not_xml_names_fail_the_record() {
    // Names are written as they are, so each of these would end its tag or its JSON key and
    // have its tail read as an attribute, an element or another key.
    for bad in [
        "",
        "a b",
        "a\"",
        "a'",
        "a>",
        "a<",
        "a=",
        "a/",
        "a&",
        "a;",
        "a?",
        "a\\",
        "a\n",
        "a\u{85}",
        "a\u{a0}b",
        "a\u{d7}",
        "a\u{ffff}",
        "\u{b7}a",
        "1a",
        "-a",
        r#"x":1,"y"#,
        r#"x y="1""#,
    ] {
        for place in PLACES {
            for position in 0..4 {
                let mut names = ["E", "A", "amp", "T"];
                names[position] = bad;
                for result in render_everywhere(&named(names, place)) {
                    let err = result.expect_err(bad);
                    assert!(
                        err.contains(&format!("{bad:?}, the name at offset"))
                            && err.ends_with("is not an XML name"),
                        "{bad:?} in {place:?}: {err}"
                    );
                }
            }
        }
    }
}

#[test]
fn xml_names_render() {
    for good in ["a", "_a", "a:b", "a-b.c_1", "é", "名前", "a·b", "𐐀"] {
        for place in PLACES {
            let [xml, json, chunk_xml, chunk_json] =
                render_everywhere(&named([good; 4], place)).map(Result::unwrap);
            let element = format!("<{good} {good}=\"v\">&{good};<?{good} data?></{good}>");
            assert!(xml.contains(&element), "{xml}");
            assert_eq!(chunk_xml.trim_end(), xml.trim_end());
            let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
            let event = match place {
                Place::Value => &parsed["Outer"]["Event"],
                _ => &parsed["Event"],
            };
            assert_eq!(event[good]["#attributes"][good], "v", "{json}");
            assert_eq!(chunk_json.trim_end(), json.trim_end());
        }
    }
}

/// `evtx` with the string-table name `from` of its first chunk replaced by `to`, of the same
/// length.
fn rename_in_string_table(mut evtx: Vec<u8>, from: &str, to: &str) -> Vec<u8> {
    let entry = |name: &str| -> Vec<u8> {
        let units: Vec<u16> = name.encode_utf16().collect();
        std::iter::once(units.len() as u16)
            .chain(units)
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect()
    };
    let (from, to) = (entry(from), entry(to));
    assert_eq!(from.len(), to.len());
    let chunk = EVTX_FILE_HEADER_SIZE..EVTX_FILE_HEADER_SIZE + EVTX_CHUNK_SIZE;
    let at = evtx[chunk.clone()]
        .windows(from.len())
        .position(|window| window == from)
        .expect("the name in the first chunk");
    evtx[chunk.start + at..][..to.len()].copy_from_slice(&to);
    evtx
}

#[test]
fn a_string_table_name_that_is_not_an_xml_name_fails_the_records_that_use_it() {
    // Every record of the sample has a `Correlation` element, named in its chunk's table.
    let sample = std::fs::read(regular_sample()).unwrap();
    let evtx = rename_in_string_table(sample, "Correlation", "Correla\"><x");
    let mut parser = evtx::EvtxParser::from_buffer(evtx).unwrap();
    let xml: Vec<_> = parser.records().collect();
    let json: Vec<_> = parser.records_json().collect();
    // The 91 records of the first chunk fail, and the others render: each chunk has its own
    // table.
    for records in [xml, json] {
        let failed: Vec<String> = records
            .iter()
            .filter_map(|record| record.as_ref().err().map(|err| causes(err)))
            .collect();
        assert_eq!((failed.len(), records.len()), (91, 2261));
        for err in failed {
            assert!(
                err.contains(r#""Correla\"><x", the name at offset"#) && err.ends_with("XML name"),
                "{err}"
            );
        }
    }
}

/// A record `<Event><EventData>…</EventData></Event>` whose children `children` writes, in a
/// template or as plain tokens.
fn event_data(in_template: bool, children: impl Fn(&mut BinXml, bool)) -> Vec<u8> {
    event(in_template, |root, in_template| {
        root.open("EventData", in_template).close_start();
        children(root, in_template);
        root.end_element();
    })
}

/// `<Data Name="name">text</Data>`.
fn named_data(parent: &mut BinXml, in_template: bool, name: &str, text: &str) {
    parent
        .open_with_attributes("Data", in_template)
        .attribute("Name")
        .text(name)
        .close_start()
        .text(text)
        .end_element();
}

/// `<name>text</name>`.
fn el(parent: &mut BinXml, in_template: bool, name: &str, text: &str) {
    parent
        .open(name, in_template)
        .close_start()
        .text(text)
        .end_element();
}

/// The members of `Event.EventData` in each JSON rendering of `evtx`, which agree.
fn event_data_members(evtx: &[u8]) -> serde_json::Map<String, serde_json::Value> {
    let [_, json, _, chunk_json] = render_everywhere(evtx).map(Result::unwrap);
    assert_eq!(chunk_json.trim_end(), json.trim_end());
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    parsed["Event"]["EventData"].as_object().unwrap().clone()
}

#[test]
fn json_keys_of_one_object_never_repeat() {
    // An element and a named `Data` under each of three names, of which a number after the
    // first gives the other two.
    let names = ["A", "A_1", "A_1_1"];
    let kinds: Vec<(bool, &str)> = names
        .iter()
        .flat_map(|name| [(false, *name), (true, *name)])
        .collect();
    let mut children: Vec<Vec<(bool, &str)>> = vec![vec![]];
    for at in 0.. {
        if children[at].len() == 4 {
            break;
        }
        for kind in &kinds {
            let mut longer = children[at].clone();
            longer.push(*kind);
            children.push(longer);
        }
    }
    for children in &children[1..] {
        let members = [true, false].map(|in_template| {
            event_data_members(&event_data(in_template, |parent, in_template| {
                for (at, (named, name)) in children.iter().enumerate() {
                    let text = at.to_string();
                    if *named {
                        named_data(parent, in_template, name, &text);
                    } else {
                        el(parent, in_template, name, &text);
                    }
                }
            }))
        });
        assert_eq!(members[0], members[1], "{children:?}");
        // A key written twice would lose the value of the first. Every child's text is
        // there, alone or in the array that named `Data` of one name gather into.
        let mut texts: Vec<usize> = members[0]
            .values()
            .flat_map(|value| match value {
                serde_json::Value::Array(values) => values.clone(),
                value => vec![value.clone()],
            })
            .map(|text| text.as_str().unwrap().parse().unwrap())
            .collect();
        texts.sort_unstable();
        assert!(
            texts.iter().copied().eq(0..children.len()),
            "{children:?}: {:?}",
            members[0]
        );
    }
}

#[test]
fn a_repeated_json_key_takes_the_next_free_number() {
    let keys = |children: &[(bool, &str)]| -> Vec<String> {
        let members = event_data_members(&event_data(true, |parent, in_template| {
            for (named, name) in children {
                if *named {
                    named_data(parent, in_template, name, "v");
                } else {
                    el(parent, in_template, name, "v");
                }
            }
        }));
        members.keys().cloned().collect()
    };
    let (element, data) = (false, true);
    // What a log Windows wrote has: one name, repeated.
    assert_eq!(
        keys(&[(element, "A"), (element, "A"), (element, "A")]),
        ["A", "A_1", "A_2"]
    );
    // The number a repeat would take is a name of its own.
    assert_eq!(
        keys(&[(element, "A"), (element, "A"), (element, "A_1")]),
        ["A", "A_1", "A_1_1"]
    );
    assert_eq!(
        keys(&[(element, "A"), (element, "A_1"), (element, "A")]),
        ["A", "A_1", "A_2"]
    );
    // A named `Data` and an element share the object.
    assert_eq!(
        keys(&[(data, "A"), (element, "A"), (data, "A"), (data, "A_1")]),
        ["A", "A_1", "A_2", "A_1_1"]
    );
}

/// A record whose `<Event>` holds what `children` writes, in a template or as plain tokens.
fn event(in_template: bool, children: impl Fn(&mut BinXml, bool)) -> Vec<u8> {
    let event = |root: &mut BinXml| {
        root.open("Event", in_template).close_start();
        children(root, in_template);
        root.end_element();
    };
    synthetic_evtx(|record| {
        record.fragment_header();
        if in_template {
            record.template_instance(event, vec![]);
        } else {
            event(record);
        }
        record.eof();
    })
}

#[test]
fn a_data_named_as_the_attributes_key_keeps_the_attributes() {
    for in_template in [true, false] {
        let evtx = event(in_template, |root, in_template| {
            root.open_with_attributes("EventData", in_template)
                .attribute("Name")
                .text("n")
                .close_start();
            named_data(root, in_template, "#attributes", "1");
            named_data(root, in_template, "x", "2");
            root.end_element();
        });
        let [_, json, _, chunk_json] = render_everywhere(&evtx).map(Result::unwrap);
        assert_eq!(
            json,
            r##"{"Event":{"EventData":{"#attributes":{"Name":"n"},"#attributes_1":"1","x":"2"}}}"##
        );
        assert_eq!(chunk_json.trim_end(), json);
    }
}

/// The record's JSON from the records API and the chunk renderer, which agree, and with
/// attributes apart, which no template is compiled for.
fn json_and_separate(evtx: &[u8]) -> [String; 2] {
    let [_, json, _, chunk_json] = render_everywhere(evtx).map(Result::unwrap);
    assert_eq!(chunk_json.trim_end(), json);
    let settings = evtx::ParserSettings::new()
        .num_threads(1)
        .separate_json_attributes(true);
    let mut parser = evtx::EvtxParser::from_buffer(evtx.to_vec())
        .unwrap()
        .with_configuration(settings);
    [json, parser.records_json().next().unwrap().unwrap().data]
}

#[test]
fn separate_attributes_keys_never_repeat() {
    for in_template in [true, false] {
        // The second `A` is `A_1`, so the element `A_1` is `A_1_1`, and the element
        // `A_1_attributes`, which is the key of the second `A`'s attributes, takes a number.
        let evtx = event_data(in_template, |parent, in_template| {
            for name in ["A", "A", "A_1", "A_1_attributes"] {
                parent
                    .open_with_attributes(name, in_template)
                    .attribute("k")
                    .text("a")
                    .close_start()
                    .text("v")
                    .end_element();
            }
        });
        let [_, json] = json_and_separate(&evtx);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let keys: Vec<_> = parsed["Event"]["EventData"]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(
            keys,
            [
                "A_attributes",
                "A",
                "A_1_attributes",
                "A_1",
                "A_1_1_attributes",
                "A_1_1",
                "A_1_attributes_1_attributes",
                "A_1_attributes_1"
            ]
        );
    }
}

#[test]
fn data_names_that_render_alike_get_keys_of_their_own() {
    // An unpaired surrogate renders as U+FFFD, whichever it is.
    for in_template in [true, false] {
        let evtx = event_data(in_template, |parent, in_template| {
            for surrogate in [0xd800u16, 0xd801] {
                parent
                    .open_with_attributes("Data", in_template)
                    .attribute("Name")
                    .raw(&[0x05, 0x01, 1, 0])
                    .raw(&surrogate.to_le_bytes())
                    .close_start()
                    .text("v")
                    .end_element();
            }
        });
        for json in json_and_separate(&evtx) {
            assert_eq!(
                json,
                "{\"Event\":{\"EventData\":{\"\u{fffd}\":\"v\",\"\u{fffd}_1\":\"v\"}}}"
            );
        }
    }
}

#[test]
fn a_key_is_not_numbered_after_a_member_that_was_left_out() {
    // `<A>` depends on a value that is NULL, so the `Data` named `A` is the only `A`.
    let evtx = synthetic_evtx(|record| {
        record.fragment_header().template_instance(
            |root| {
                root.open("Event", true).close_start();
                root.open("EventData", true).close_start();
                root.open_depending("A", 0)
                    .close_start()
                    .optional_substitution(0, 0x01)
                    .end_element();
                named_data(root, true, "A", "v");
                root.end_element().end_element();
            },
            vec![value(0x00, |_| {})],
        );
        record.eof();
    });
    for json in json_and_separate(&evtx) {
        assert_eq!(json, r#"{"Event":{"EventData":{"A":"v"}}}"#);
    }
}

#[test]
fn a_name_takes_its_next_number_that_is_free() {
    let members = event_data_members(&event_data(true, |parent, in_template| {
        for name in ["A", "A_1", "A", "A_1"] {
            el(parent, in_template, name, "v");
        }
    }));
    let keys: Vec<_> = members.keys().collect();
    assert_eq!(keys, ["A", "A_1", "A_2", "A_1_1"]);
}
