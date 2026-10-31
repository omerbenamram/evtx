mod fixtures;
use fixtures::*;

use evtx::{EvtxParser, ParserSettings};
use serde_json::Value;

#[test]
fn test_event_xml_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_json_sample() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_xml_sample_with_event_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/2-system-Security-dirty.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_json_sample_with_event_data_with_attributes_and_text() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/system.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_xml_sample_with_event_data_with_attributes_and_text() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/system.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_xml_sample_with_user_data() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(first_record.data);
}

#[test]
fn test_event_xml_sample_with_entity_ref() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_xml_sample_with_entity_ref_2() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 25)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_xml_sample_with_string_with_trailing_spaces() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 2260)
        .expect("record to parse correctly");

    insta::assert_snapshot!(record.data);
}

#[test]
fn test_event_json_with_size_t() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/Security_with_size_t.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 196)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_nodes_same_name() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_sample_with_separate_json_attributes() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/Application.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_data_elements() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/MSExchange_Management_wec.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(ParserSettings::new().num_threads(1));

    let first_record_xml = parser
        .records()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    insta::assert_snapshot!(&first_record_xml.data);

    let first_record = parser
        .records_json()
        .next()
        .expect("to have records")
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&first_record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_with_multiple_nodes_same_name_separate() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!(
        "../samples/E_Windows_system32_winevt_logs_Microsoft-Windows-CAPI2%4Operational.evtx"
    );
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .find(|record| record.event_record_id == 28)
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_multiple_empty_data_nodes_not_ignored() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/issue_201.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .next()
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

#[test]
fn test_event_json_missing_string_cache_entry() {
    ensure_env_logger_initialized();
    let evtx_file = include_bytes!("../samples/security_bad_string_cache.evtx");
    let mut parser = EvtxParser::from_buffer(evtx_file.to_vec())
        .unwrap()
        .with_configuration(
            ParserSettings::new()
                .num_threads(1)
                .separate_json_attributes(true),
        );

    let record = parser
        .records_json()
        .filter_map(|record| record.ok())
        .next()
        .expect("record to parse correctly");

    let value: Value = serde_json::from_str(&record.data).expect("to parse correctly");
    insta::assert_json_snapshot!(&value);
}

/// Record 161's event data is one BinXML value holding a nested template instance followed by
/// a `RenderingInfo` element. Windows renders both; so must every output path.
#[test]
fn test_event_with_rendering_info_after_nested_template() {
    ensure_env_logger_initialized();
    let evtx = std::fs::read(sample_with_multiple_xml_fragments()).unwrap();
    let (xml, json) = render_record(&evtx, 161);
    insta::assert_snapshot!(xml);

    let json: Value = serde_json::from_str(&json).unwrap();
    let event = &json["Event"];
    assert_eq!(
        event["EventData"]["SelectionData"],
        "Region Codes:US \nSKUs:6 \nGroup Cell Width:6\n"
    );
    assert_eq!(event["RenderingInfo"]["Task"], "TraceLayoutSelectionData");
}

/// A BinXML value that is neither a template instance nor a single element: evtx_dump's
/// chunk renderer builds it from the record bytes, and must render all of its elements.
#[test]
fn test_value_with_several_elements_renders_them_all() {
    ensure_env_logger_initialized();
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("Root", true)
                        .close_start()
                        .substitution(0, 0x21)
                        .end_element();
                },
                0x21,
                |value| {
                    value
                        .fragment_header()
                        .text_element("A", "1")
                        .text_element("B", "2")
                        .eof();
                },
            )
            .eof();
    });
    let (xml, json) = render_record(&evtx, 1);
    insta::assert_snapshot!(xml, @r#"
    <?xml version="1.0" encoding="utf-8"?>
    <Root>
      <A>1</A>
      <B>2</B>
    </Root>
    "#);
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(json, serde_json::json!({"Root": {"A": "1", "B": "2"}}));
}

/// CDATA sections and character references render as XML writes them, and JSON gets their
/// text. A `]]>` inside a CDATA section splits it in two, so the XML stays well-formed.
#[test]
fn test_cdata_and_character_references_render() {
    ensure_env_logger_initialized();
    let evtx = synthetic_evtx(|record| {
        record
            .fragment_header()
            .template_instance(
                |root| {
                    root.open("Root", true).close_start();
                    root.open("CharRef", true)
                        .close_start()
                        .char_ref(0x41)
                        .char_ref(0x263a)
                        .end_element();
                    root.open("Cdata", true)
                        .close_start()
                        .cdata("x < y ]]> & z")
                        .end_element();
                    root.open("Value", true)
                        .close_start()
                        .substitution(0, 0x01)
                        .end_element();
                    root.end_element();
                },
                0x01,
                |value| {
                    value.raw(&[b'v', 0]);
                },
            )
            .eof();
    });
    let (xml, json) = render_record(&evtx, 1);
    insta::assert_snapshot!(xml, @r#"
    <?xml version="1.0" encoding="utf-8"?>
    <Root>
      <CharRef>&#65;&#9786;</CharRef>
      <Cdata><![CDATA[x < y ]]]]><![CDATA[> & z]]></Cdata>
      <Value>v</Value>
    </Root>
    "#);
    let json: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"Root": {"CharRef": "A\u{263a}", "Cdata": "x < y ]]> & z", "Value": "v"}})
    );
}

/// Record 17's last value is BinXML with no element in it (an EOF token, then zeroes): it
/// renders nothing, in every output path.
#[test]
fn test_value_without_elements_renders_nothing() {
    ensure_env_logger_initialized();
    let evtx = std::fs::read(sample_binxml_with_incomplete_template()).unwrap();
    let (xml, json) = render_record(&evtx, 17);
    assert!(xml.trim_end().ends_with("  </System>\n</Event>"), "{xml}");
    let json: Value = serde_json::from_str(&json).unwrap();
    let event = json["Event"].as_object().unwrap();
    assert_eq!(event.keys().collect::<Vec<_>>(), ["#attributes", "System"]);
}
