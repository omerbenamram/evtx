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
    let parser = || {
        EvtxParser::from_path(sample_with_multiple_xml_fragments())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };
    let record_161 = |records: Vec<(u64, String)>| {
        records
            .into_iter()
            .find(|(id, _)| *id == 161)
            .expect("record 161 to render")
            .1
    };
    let records_api = |json: bool| {
        let mut parser = parser();
        let records: Vec<_> = if json {
            parser.records_json().map(|r| r.unwrap()).collect()
        } else {
            parser.records().map(|r| r.unwrap()).collect()
        };
        record_161(
            records
                .into_iter()
                .map(|record| (record.event_record_id, record.data))
                .collect(),
        )
    };

    let xml = records_api(false);
    insta::assert_snapshot!(xml);

    let json: Value = serde_json::from_str(&records_api(true)).unwrap();
    let event = &json["Event"];
    assert_eq!(
        event["EventData"]["SelectionData"],
        "Region Codes:US \nSKUs:6 \nGroup Cell Width:6\n"
    );
    assert_eq!(event["RenderingInfo"]["Task"], "TraceLayoutSelectionData");

    // evtx_dump's chunk renderer gives the same output.
    assert_eq!(
        record_161(rendered_records(parser().chunks_xml_bytes(false))),
        xml
    );
    let rendered = record_161(rendered_records(parser().chunks_json_bytes(false)));
    assert_eq!(serde_json::from_str::<Value>(&rendered).unwrap(), json);
}

/// A BinXML value that is neither a template instance nor a single element: evtx_dump's
/// chunk renderer builds it from the record bytes, and must render all of its elements.
#[test]
fn test_value_with_several_elements_renders_them_all() {
    ensure_env_logger_initialized();
    let mut binxml = BinXml::record();
    binxml.fragment_header().template_instance(
        |root| {
            root.open("Root", true)
                .close_start()
                .substitution(0, 0x21)
                .end_element();
        },
        &[(0x21, &|value: &mut BinXml| {
            value
                .fragment_header()
                .text_element("A", "1")
                .text_element("B", "2")
                .eof();
        })],
    );
    binxml.eof();
    let evtx = synthetic_evtx(&binxml.bytes);
    let parser = || {
        EvtxParser::from_buffer(evtx.clone())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };

    let xml = parser().records().next().unwrap().unwrap().data;
    insta::assert_snapshot!(xml, @r#"
    <?xml version="1.0" encoding="utf-8"?>
    <Root>
      <A>1</A>
      <B>2</B>
    </Root>
    "#);
    assert_eq!(
        rendered_records(parser().chunks_xml_bytes(false)),
        vec![(1, xml)]
    );

    let json = parser().records_json().next().unwrap().unwrap().data;
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value, serde_json::json!({"Root": {"A": "1", "B": "2"}}));
    let rendered = rendered_records(parser().chunks_json_bytes(false));
    assert_eq!(
        serde_json::from_str::<Value>(&rendered[0].1).unwrap(),
        value
    );
}

/// A BinXML value with no element in it (only EOF) renders nothing, in every output path.
#[test]
fn test_value_without_elements_renders_nothing() {
    ensure_env_logger_initialized();
    let mut binxml = BinXml::record();
    binxml.fragment_header().template_instance(
        |root| {
            root.open("Root", true)
                .close_start()
                .substitution(0, 0x21)
                .end_element();
        },
        &[(0x21, &|value: &mut BinXml| {
            value.fragment_header().eof();
        })],
    );
    binxml.eof();
    let evtx = synthetic_evtx(&binxml.bytes);
    let parser = || {
        EvtxParser::from_buffer(evtx.clone())
            .unwrap()
            .with_configuration(ParserSettings::new().num_threads(1))
    };

    let xml = parser().records().next().unwrap().unwrap().data;
    insta::assert_snapshot!(xml, @r#"
    <?xml version="1.0" encoding="utf-8"?>
    <Root>
    </Root>
    "#);
    assert_eq!(
        rendered_records(parser().chunks_xml_bytes(false)),
        vec![(1, xml)]
    );

    let json = parser().records_json().next().unwrap().unwrap().data;
    let value: Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value, serde_json::json!({"Root": null}));
    let rendered = rendered_records(parser().chunks_json_bytes(false));
    assert_eq!(
        serde_json::from_str::<Value>(&rendered[0].1).unwrap(),
        value
    );
}
