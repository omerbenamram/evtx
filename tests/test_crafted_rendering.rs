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
            // A reference to an entity XML doesn't define is its text.
            let element = format!("<{good} {good}=\"v\">&amp;{good};<?{good} data?></{good}>");
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

#[test]
fn the_named_form_keeps_data_it_has_no_name_for() {
    for in_template in [true, false] {
        // Beside a named `Data`: one with text and no name, one with an empty name, one with
        // another attribute, and one with nothing to write.
        let evtx = event_data(in_template, |parent, in_template| {
            named_data(parent, in_template, "A", "1");
            el(parent, in_template, "Data", "2");
            named_data(parent, in_template, "", "3");
            parent
                .open_with_attributes("Data", in_template)
                .attribute("Name")
                .text("B")
                .attribute("Type")
                .text("t")
                .close_start()
                .text("4")
                .end_element();
            parent.open("Data", in_template).close_empty();
            named_data(parent, in_template, "Data", "5");
        });
        let [json, separate] = json_and_separate(&evtx);
        assert_eq!(
            json,
            concat!(
                r##"{"Event":{"EventData":{"A":"1","Data":"2","Data_1":"3","##,
                r##""Data_2":{"#attributes":{"Name":"B","Type":"t"},"#text":"4"},"Data_3":"5"}}}"##
            )
        );
        assert_eq!(
            separate,
            concat!(
                r##"{"Event":{"EventData":{"A":"1","Data":"2","Data_1":"3","##,
                r##""Data_2_attributes":{"Name":"B","Type":"t"},"Data_2":"4","Data_3":"5"}}}"##
            )
        );
    }
}

#[test]
fn a_named_data_with_an_empty_attribute_has_a_key_of_its_own() {
    // An attribute with no text isn't written, so the `Data` is one of the named form, whose
    // key an element can have too.
    for in_template in [true, false] {
        let evtx = event_data(in_template, |parent, in_template| {
            parent
                .open_with_attributes("Data", in_template)
                .attribute("Name")
                .text("A")
                .attribute("Type")
                .text("")
                .close_start()
                .text("1")
                .end_element();
            el(parent, in_template, "A", "2");
        });
        for json in json_and_separate(&evtx) {
            assert_eq!(json, r#"{"Event":{"EventData":{"A":"1","A_1":"2"}}}"#);
        }
    }
}

#[test]
fn a_numeric_array_of_one_item_is_a_number_as_the_items_of_longer_ones_are() {
    // `<X>%0</X><Y a="%0"/>` for an array of `count` items of 1, each `size` bytes.
    let record = |value_type: u8, size: usize, count: usize| {
        let evtx = synthetic_evtx(|record| {
            record.fragment_header().template_instance(
                |root| {
                    root.open("Event", true).close_start();
                    root.open("X", true)
                        .close_start()
                        .substitution(0, value_type)
                        .end_element();
                    root.open_with_attributes("Y", true)
                        .attribute("a")
                        .substitution(0, value_type)
                        .close_empty();
                    root.end_element();
                },
                vec![value(value_type, |items| {
                    for _ in 0..count {
                        let mut item = vec![0; size];
                        item[0] = 1;
                        items.raw(&item);
                    }
                })],
            );
            record.eof();
        });
        let [_, json, _, chunk_json] = render_everywhere(&evtx).map(Result::unwrap);
        assert_eq!(chunk_json.trim_end(), json);
        json
    };
    let y = |a: &str| format!(r##"{{"#attributes":{{"a":{a}}}}}"##);
    let one = |item: &str| format!(r#"{{"Event":{{"X":{item},"Y":{}}}}}"#, y(item));
    let two = |item: &str| {
        format!(
            r#"{{"Event":{{"X":{item},"X_1":{item},"Y":{},"Y_1":{}}}}}"#,
            y(item),
            y(item)
        )
    };
    // Int8 to UInt64, then Bool.
    let sizes = [1, 1, 2, 2, 4, 4, 8, 8];
    for (value_type, size) in (0x83..=0x8a).zip(sizes) {
        assert_eq!(record(value_type, size, 1), one("1"));
        assert_eq!(record(value_type, size, 2), two("1"));
    }
    assert_eq!(record(0x8d, 4, 1), one("true"));
    assert_eq!(record(0x8d, 4, 2), two("true"));
    // A HexInt32 is a string, alone or in an array.
    assert_eq!(record(0x94, 4, 1), one(r#""0x1""#));
    assert_eq!(record(0x94, 4, 2), two(r#""0x1""#));
}

/// `<Event><A>%0</A><B>%1</B></Event>` with a value of type `first_type` and bytes `first`,
/// then the UInt32 7.
fn value_then_uint32(first_type: u8, first: &[u8]) -> Vec<u8> {
    synthetic_evtx(|record| {
        record.fragment_header().template_instance(
            |root| {
                root.open("Event", true).close_start();
                for (id, (name, value_type)) in
                    [("A", first_type), ("B", 0x08)].into_iter().enumerate()
                {
                    root.open(name, true)
                        .close_start()
                        .substitution(id as u16, value_type)
                        .end_element();
                }
                root.end_element();
            },
            vec![
                value(first_type, |v| {
                    v.raw(first);
                }),
                value(0x08, |v| {
                    v.raw(&7u32.to_le_bytes());
                }),
            ],
        );
        record.eof();
    })
}

/// The record's XML and JSON, which the records API and the chunk renderer agree on.
fn xml_and_json(evtx: &[u8]) -> (String, String) {
    let [xml, json, chunk_xml, chunk_json] = render_everywhere(evtx).map(Result::unwrap);
    assert_eq!(chunk_xml.trim_end(), xml.trim_end());
    assert_eq!(chunk_json.trim_end(), json);
    let xml = xml.split_once('\n').unwrap().1;
    (xml.split_whitespace().collect(), json)
}

#[test]
fn a_value_that_needs_more_bytes_than_it_has_fails_the_record() {
    // A UInt32 of two bytes would take two of the next value's.
    for rendered in render_everywhere(&value_then_uint32(0x08, &[1, 0])) {
        let err = rendered.unwrap_err();
        assert!(
            err.contains("buffer too small for template value")
                && err.contains("need 4 bytes, have 2"),
            "{err}"
        );
    }
}

#[test]
fn a_value_with_bytes_to_spare_leaves_the_next_where_it_is() {
    let (xml, json) = xml_and_json(&value_then_uint32(0x08, &[1, 0, 0, 0, 9, 9]));
    assert_eq!(xml, "<Event><A>1</A><B>7</B></Event>");
    assert_eq!(json, r#"{"Event":{"A":1,"B":7}}"#);
}

#[test]
fn a_value_with_no_bytes_is_no_value() {
    let (xml, json) = xml_and_json(&value_then_uint32(0x08, &[]));
    assert_eq!(xml, "<Event><A></A><B>7</B></Event>");
    assert_eq!(json, r#"{"Event":{"A":null,"B":7}}"#);
}

#[test]
fn a_string_array_ends_with_its_own_bytes() {
    // `a`, NUL, `b` and no NUL: the last string doesn't run on into the next value.
    let (xml, json) = xml_and_json(&value_then_uint32(0x81, &[b'a', 0, 0, 0, b'b', 0]));
    assert_eq!(xml, "<Event><A>a</A><A>b</A><B>7</B></Event>");
    assert_eq!(json, r#"{"Event":{"A":"a","A_1":"b","B":7}}"#);
}

#[test]
fn xml_has_no_character_it_does_not_allow_wherever_text_is() {
    for in_template in [true, false] {
        let evtx = event(in_template, |root, in_template| {
            root.open("A", in_template)
                .close_start()
                .cdata("a\u{1}b\u{ffff}c\td")
                .char_ref(0x41)
                .char_ref(0x1)
                .char_ref(0xffff)
                .char_ref(0xd800)
                .entity_ref("amp")
                .entity_ref("foo")
                .processing_instruction("p", "x?>y\u{2}z??>")
                .end_element();
        });
        let [xml, json, chunk_xml, chunk_json] = render_everywhere(&evtx).map(Result::unwrap);
        assert_eq!(chunk_xml.trim_end(), xml.trim_end());
        assert_eq!(chunk_json.trim_end(), json);
        assert!(
            xml.contains("<A><![CDATA[a_b_c\td]]>&#65;___&amp;&amp;foo;<?p x? >y_z?? >?></A>"),
            "{xml}"
        );
        // And it parses, to the text and the instruction a reader should see.
        let parsed = roxmltree::Document::parse(&xml).unwrap();
        let a = parsed.descendants().find(|n| n.has_tag_name("A")).unwrap();
        let text: String = a.children().filter_map(|n| n.text()).collect();
        assert_eq!(text, "a_b_c\tdA___&&foo;");
        let pi = a.children().find_map(|n| n.pi()).unwrap();
        assert_eq!((pi.target, pi.value), ("p", Some("x? >y_z?? >")));
        // JSON has escapes for every character, and U+FFFD for a surrogate.
        assert_eq!(
            json,
            "{\"Event\":{\"A\":\"a\\u0001b\u{ffff}c\\tdA\\u0001\u{ffff}\u{fffd}&&foo;\"}}"
        );
    }
}

#[test]
fn a_processing_instruction_named_xml_fails_the_record_in_xml() {
    for target in ["xml", "XmL"] {
        let evtx = event(true, |root, in_template| {
            root.open("A", in_template)
                .close_start()
                .processing_instruction(target, "version=\"1.0\"")
                .end_element();
        });
        let [xml, json, chunk_xml, chunk_json] = render_everywhere(&evtx);
        for xml in [xml, chunk_xml] {
            assert!(xml.unwrap_err().contains("which XML reserves"));
        }
        // JSON has no processing instructions.
        assert_eq!(chunk_json.unwrap().trim_end(), json.unwrap());
    }
}

#[test]
fn a_binxml_value_that_builds_nothing_is_null() {
    // A value of type BinXML that is only the end of its stream, or only fragment headers, is
    // nothing, as one with no bytes is. The compiled XML took it for an element and broke
    // the line for it: `<Outer>\n  </Outer>`, where the tree walker wrote `<Outer></Outer>`.
    let streams: [&[u8]; 6] = [
        &[],
        &[0x00],
        &[0x0f, 1, 1, 0],
        &[0x0f, 1, 1, 0, 0x00],
        &[0x0f, 1, 1, 0, 0x0f, 1, 1, 0],
        &[0x0f, 1, 1, 0, 0x0f, 1, 1, 0, 0x00],
    ];
    // One substitution to a template, so that the template is one the renderers compile.
    let shapes: [(&dyn Fn(&mut BinXml), &str, &str); 4] = [
        (
            &|outer| {
                outer
                    .open("Outer", true)
                    .close_start()
                    .substitution(0, 0x21);
            },
            "<Event>\n  <Outer></Outer>\n</Event>",
            r#"{"Event":{"Outer":null}}"#,
        ),
        (
            &|outer| {
                outer
                    .open("Outer", true)
                    .close_start()
                    .optional_substitution(0, 0x21);
            },
            "<Event>\n  <Outer></Outer>\n</Event>",
            r#"{"Event":{"Outer":null}}"#,
        ),
        // An element that depends on a NULL value is left out.
        (
            &|outer| {
                outer
                    .open_depending("Outer", 0)
                    .close_start()
                    .substitution(0, 0x21);
            },
            "<Event></Event>",
            r#"{"Event":null}"#,
        ),
        // So is one that only depends on it.
        (
            &|outer| {
                outer.open_depending("Outer", 0).close_start().text("x");
            },
            "<Event></Event>",
            r#"{"Event":null}"#,
        ),
    ];
    for stream in streams {
        for (shape, (outer, expected_xml, expected_json)) in shapes.iter().enumerate() {
            let evtx = synthetic_evtx(|record| {
                record.fragment_header();
                record.template_instance(
                    |root| {
                        root.open("Event", true).close_start();
                        outer(root);
                        root.end_element().end_element();
                    },
                    vec![value(0x21, |fragment| {
                        fragment.raw(stream);
                    })],
                );
                record.eof();
            });
            let at = format!("{stream:?} in shape {shape}");
            let [xml, json, chunk_xml, chunk_json] = render_everywhere(&evtx).map(Result::unwrap);
            assert_eq!(
                xml.split_once('\n').unwrap().1.trim_end(),
                *expected_xml,
                "{at}"
            );
            assert_eq!(chunk_xml.trim_end(), xml.trim_end(), "{at}");
            assert_eq!(json, *expected_json, "{at}");
            assert_eq!(chunk_json.trim_end(), json, "{at}");
        }
    }
}

#[test]
fn a_binxml_value_with_an_element_in_an_attribute_fails_the_record() {
    // An attribute can't hold an element. XML output failed the record; JSON output took the
    // value for nothing and wrote `"Outer":null`.
    for nested_instance in [false, true] {
        let evtx = synthetic_evtx(|record| {
            record.fragment_header();
            record.template_instance(
                |root| {
                    root.open("Event", true).close_start();
                    root.open_with_attributes("Outer", true)
                        .attribute("K")
                        .substitution(0, 0x21)
                        .close_start()
                        .end_element();
                    root.end_element();
                },
                vec![value(0x21, |fragment| {
                    fragment.fragment_header();
                    if nested_instance {
                        fragment.template_instance(
                            |inner| {
                                inner.open("In", true).close_empty();
                            },
                            vec![],
                        );
                    } else {
                        fragment.open("In", false).close_empty();
                    }
                    fragment.eof();
                })],
            );
            record.eof();
        });
        for (renderer, rendered) in render_everywhere(&evtx).into_iter().enumerate() {
            let at = format!("renderer {renderer}, nested instance: {nested_instance}");
            let err = rendered.expect_err(&at);
            assert!(
                err.contains("element node inside attribute value"),
                "{at}: {err}"
            );
        }
    }
}

/// A record whose template writes its first value, a string, and has a second value of type
/// BinXML that the template never uses.
fn unused_binxml_value(stream: impl Fn(&mut BinXml)) -> Vec<u8> {
    synthetic_evtx(|record| {
        record.fragment_header();
        record.template_instance(
            |root| {
                root.open("Event", true).close_start();
                root.open("A", true)
                    .close_start()
                    .substitution(0, 0x01)
                    .end_element();
                root.end_element();
            },
            vec![
                value(0x01, |text| {
                    text.raw(&[b'o', 0, b'k', 0]);
                }),
                value(0x21, |fragment| stream(fragment)),
            ],
        );
        record.eof();
    })
}

#[test]
fn a_binxml_value_the_template_does_not_use_is_still_read() {
    // The records API builds every BinXML value, so one that can't be read fails the record.
    // The chunk renderer read only the values its template writes, and rendered the record.
    let bad: [&[u8]; 3] = [
        // An element whose name is out of the chunk.
        &[
            0x0f, 1, 1, 0, 0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff,
        ],
        // A template instance that is cut off.
        &[0x0f, 1, 1, 0, 0x0c, 0x01],
        // Not a token.
        &[0xff, 0xff],
    ];
    for stream in bad {
        let evtx = unused_binxml_value(|fragment| {
            fragment.raw(stream);
        });
        let [first, rest @ ..] = render_everywhere(&evtx).map(Result::unwrap_err);
        for err in rest {
            assert_eq!(err, first, "{stream:?}");
        }
    }

    // One that can be read changes nothing.
    let evtx = unused_binxml_value(|fragment| {
        fragment.fragment_header();
        fragment.open("In", false).close_empty();
        fragment.eof();
    });
    let (xml, json) = xml_and_json(&evtx);
    assert_eq!(xml, "<Event><A>ok</A></Event>");
    assert_eq!(json, r#"{"Event":{"A":"ok"}}"#);
}
