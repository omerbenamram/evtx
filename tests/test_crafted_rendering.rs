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
        let units = name.encode_utf16();
        (name.encode_utf16().count() as u16)
            .to_le_bytes()
            .into_iter()
            .chain(units.flat_map(u16::to_le_bytes))
            .chain([0, 0])
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
    let records: Vec<_> = parser.records_json().take(92).collect();
    // The 91 records of the first chunk, and the first of the second, which has its own table.
    for record in &records[..91] {
        let err = causes(record.as_ref().unwrap_err());
        assert!(
            err.contains(r#""Correla\"><x", the name at offset"#) && err.ends_with("XML name"),
            "{err}"
        );
    }
    assert!(records[91].is_ok());
}
