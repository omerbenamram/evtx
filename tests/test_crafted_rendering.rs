//! Crafted records that Windows would not write, which must not render as something they
//! are not.

mod fixtures;

use evtx::{EvtxParser, ParserSettings};
use fixtures::*;

/// `err` and what caused it.
fn causes(err: &dyn std::error::Error) -> String {
    match err.source() {
        Some(source) => format!("{err}: {}", causes(source)),
        None => err.to_string(),
    }
}

/// What each way of rendering makes of the one record of `evtx`: the records API and the
/// chunk renderer, in XML and in JSON.
fn render_everywhere(evtx: &[u8]) -> Vec<Result<String, String>> {
    let parser = || {
        EvtxParser::from_buffer(evtx.to_vec())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    let chunks = |chunks: &mut dyn Iterator<Item = evtx::RenderedChunk>| {
        let chunk = chunks.next().unwrap();
        match &chunk.items[..] {
            [evtx::RenderedChunkItem::Record { end, .. }] => {
                Ok(String::from_utf8(chunk.data[..*end].to_vec()).unwrap())
            }
            [evtx::RenderedChunkItem::Failed(err)] => Err(causes(err)),
            items => panic!("{} items", items.len()),
        }
    };
    let record = |record: Option<evtx::err::Result<evtx::SerializedEvtxRecord<String>>>| {
        record.unwrap().map(|r| r.data).map_err(|e| causes(&e))
    };
    vec![
        record(parser().records().next()),
        record(parser().records_json().next()),
        chunks(&mut parser().chunks_xml_bytes(false)),
        chunks(&mut parser().chunks_json_bytes(false)),
    ]
}

/// A record `<Event><name attribute="v">text</name></Event>`, from a template or as plain
/// tokens.
fn named(element: &str, attribute: &str, in_template: bool) -> Vec<u8> {
    let event = |root: &mut BinXml| {
        root.open("Event", in_template).close_start();
        root.open_with_attributes(element, in_template)
            .attribute(attribute)
            .text("v")
            .close_start()
            .text("text")
            .end_element();
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
        "a\\",
        "a\n",
        "a\u{85}",
        "a\u{a0}b",
        "1a",
        "-a",
        r#"x":1,"y"#,
        r#"x y="1""#,
    ] {
        for in_template in [true, false] {
            for evtx in [named(bad, "A", in_template), named("E", bad, in_template)] {
                for result in render_everywhere(&evtx) {
                    let err = result.expect_err(bad);
                    assert!(err.contains("not an XML name"), "{bad:?}: {err}");
                }
            }
        }
    }
}

#[test]
fn xml_names_render() {
    for good in ["a", "_a", "a:b", "a-b.c_1", "é", "名前", "a·b"] {
        for in_template in [true, false] {
            let results = render_everywhere(&named(good, good, in_template));
            let xml = results[0].as_ref().unwrap();
            assert!(xml.contains(&format!("<{good} {good}=\"v\">")), "{xml}");
            let json: serde_json::Value =
                serde_json::from_str(results[1].as_ref().unwrap()).unwrap();
            assert_eq!(json["Event"][good]["#text"], "text");
            assert_eq!(json["Event"][good]["#attributes"][good], "v");
        }
    }
}
