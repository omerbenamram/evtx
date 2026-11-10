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
