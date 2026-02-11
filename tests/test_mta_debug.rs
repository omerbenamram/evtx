mod fixtures;

use evtx::binxml::value_variant::BinXmlValue;
use evtx::model::ir::{Node, Text};
use evtx::{EvtxParser, MtaFile};
use fixtures::*;
use std::sync::Arc;

/// Extract `<EventRecordID>` from the IR tree (Event > System > EventRecordID).
///
/// This is the XML-embedded record ID, which *may* differ from the binary
/// record header's `event_record_id`.
fn extract_xml_event_record_id(tree: &evtx::model::ir::IrTree<'_>) -> Option<u64> {
    let root = tree.root_element();
    let arena = tree.arena();

    // Find the System child element.
    let system_id = root.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "System").then_some(*id)
        }
        _ => None,
    })?;

    let system = arena.get(system_id)?;

    // Find the EventRecordID child element.
    let erid_el_id = system.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "EventRecordID").then_some(*id)
        }
        _ => None,
    })?;

    let erid_el = arena.get(erid_el_id)?;

    // Extract the numeric value from the first child node.
    erid_el.children.iter().find_map(|node| match node {
        Node::Text(Text::Utf8(s)) => s.parse::<u64>().ok(),
        Node::Text(Text::Utf16(s)) => s.to_string().ok()?.parse::<u64>().ok(),
        Node::Value(BinXmlValue::UInt64Type(v)) => Some(*v),
        Node::Value(BinXmlValue::UInt32Type(v)) => Some(*v as u64),
        Node::Value(BinXmlValue::Int64Type(v)) => u64::try_from(*v).ok(),
        Node::Value(BinXmlValue::Int32Type(v)) => u64::try_from(*v).ok(),
        _ => None,
    })
}

/// Extract `<EventID>` from the IR tree (Event > System > EventID).
fn extract_xml_event_id(tree: &evtx::model::ir::IrTree<'_>) -> Option<u16> {
    let root = tree.root_element();
    let arena = tree.arena();

    let system_id = root.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "System").then_some(*id)
        }
        _ => None,
    })?;

    let system = arena.get(system_id)?;

    let event_id_el_id = system.children.iter().find_map(|node| match node {
        Node::Element(id) => {
            let el = arena.get(*id)?;
            (el.name.as_str() == "EventID").then_some(*id)
        }
        _ => None,
    })?;

    let event_id_el = arena.get(event_id_el_id)?;

    event_id_el.children.iter().find_map(|node| match node {
        Node::Text(Text::Utf8(s)) => s.parse::<u16>().ok(),
        Node::Text(Text::Utf16(s)) => s.to_string().ok()?.parse::<u16>().ok(),
        Node::Value(BinXmlValue::UInt16Type(v)) => Some(*v),
        Node::Value(BinXmlValue::UInt32Type(v)) => u16::try_from(*v).ok(),
        Node::Value(BinXmlValue::Int32Type(v)) => u16::try_from(*v).ok(),
        _ => None,
    })
}

#[test]
fn test_mta_debug_event_value_mapping() {
    ensure_env_logger_initialized();

    // ---------------------------------------------------------------
    // Part 1: Parse the MTA file and dump EVT entries
    // ---------------------------------------------------------------
    println!("\n========================================");
    println!("PART 1: MTA EVT section entries");
    println!("========================================");

    let mta_bytes = std::fs::read(mta_test_mta()).expect("failed to read MTA file");
    // We need to re-parse manually to see raw EVT entries, since MtaFile
    // only exposes the final lookup tables.
    // Instead, let's use the public API to probe specific event_values.
    let mta = MtaFile::from_path(mta_test_mta()).expect("failed to load MTA file");

    // Probe event_value range: try 0..30 and print which ones have messages
    println!("\nProbing message_for_event_value(0..30):");
    for ev in 0u32..30 {
        if let Some(msg) = mta.message_for_event_value(ev) {
            let short = if msg.len() > 80 { &msg[..80] } else { msg };
            println!("  event_value={:>5} => msg={}", ev, short);
        }
    }

    // Probe entry_index range: try 0..30 and print which ones have messages
    println!("\nProbing message_for_entry_index(0..30):");
    for idx in 0u32..30 {
        if let Some(msg) = mta.message_for_entry_index(idx) {
            let short = if msg.len() > 80 { &msg[..80] } else { msg };
            println!("  entry_index={:>5} => msg={}", idx, short);
        }
    }

    // Probe record_id range (1..30) -- this uses message_for_record_id which
    // internally calls message_for_event_value, so same as above but with u64.
    println!("\nProbing message_for_record_id(1..30):");
    for rid in 1u64..30 {
        if let Some(msg) = mta.message_for_record_id(rid) {
            let short = if msg.len() > 80 { &msg[..80] } else { msg };
            println!("  record_id={:>5} => msg={}", rid, short);
        }
    }

    // ---------------------------------------------------------------
    // Part 2: Parse the EVTX file and dump record info
    // ---------------------------------------------------------------
    println!("\n========================================");
    println!("PART 2: EVTX records");
    println!("========================================");
    println!("{:<6} {:>12} {:>18} {:>10}  msg_by_record_id  msg_by_event_value",
             "seq", "hdr_rec_id", "xml_EventRecordID", "xml_EventID");

    let mut parser = EvtxParser::from_path(mta_test_evtx()).expect("failed to open EVTX");
    let mut seq = 0usize;

    for chunk in parser.chunks() {
        let mut chunk = match chunk {
            Ok(c) => c,
            Err(_) => continue,
        };
        let settings = Arc::new(Default::default());
        let mut chunk = match chunk.parse(Arc::clone(&settings)) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for record in chunk.iter() {
            let record = match record {
                Ok(r) => r,
                Err(_) => continue,
            };

            let hdr_rec_id = record.event_record_id;
            let xml_erid = extract_xml_event_record_id(&record.tree);
            let xml_eid = extract_xml_event_id(&record.tree);

            let msg_by_rid = mta.message_for_record_id(hdr_rec_id)
                .map(|s| if s.len() > 40 { format!("{}...", &s[..40]) } else { s.to_string() })
                .unwrap_or_else(|| "None".to_string());

            let msg_by_ev = xml_eid
                .and_then(|eid| mta.message_for_event_value(eid as u32))
                .map(|s| if s.len() > 40 { format!("{}...", &s[..40]) } else { s.to_string() })
                .unwrap_or_else(|| "None".to_string());

            println!(
                "{:<6} {:>12} {:>18} {:>10}  {}  |  {}",
                seq,
                hdr_rec_id,
                xml_erid.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string()),
                xml_eid.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string()),
                msg_by_rid,
                msg_by_ev,
            );

            seq += 1;
            if seq >= 20 {
                break;
            }
        }
        if seq >= 20 {
            break;
        }
    }

    // ---------------------------------------------------------------
    // Part 3: Summary / comparison
    // ---------------------------------------------------------------
    println!("\n========================================");
    println!("PART 3: Comparison summary");
    println!("========================================");

    // Re-parse all records to do a systematic check
    let mut parser2 = EvtxParser::from_path(mta_test_evtx()).expect("failed to open EVTX");
    let mut match_hdr = 0u32;
    let mut match_xml_erid = 0u32;
    let mut total = 0u32;

    for chunk in parser2.chunks() {
        let mut chunk = match chunk {
            Ok(c) => c,
            Err(_) => continue,
        };
        let settings = Arc::new(Default::default());
        let mut chunk = match chunk.parse(Arc::clone(&settings)) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for record in chunk.iter() {
            let record = match record {
                Ok(r) => r,
                Err(_) => continue,
            };

            total += 1;
            let hdr_rec_id = record.event_record_id;
            let xml_erid = extract_xml_event_record_id(&record.tree);

            // Check if message_for_record_id (which uses hdr_rec_id as event_value) finds something
            if mta.message_for_record_id(hdr_rec_id).is_some() {
                match_hdr += 1;
            }

            // Check if message_for_event_value using xml EventRecordID finds something
            if let Some(erid) = xml_erid {
                if mta.message_for_event_value(erid as u32).is_some() {
                    match_xml_erid += 1;
                }
            }
        }
    }

    println!("Total EVTX records: {}", total);
    println!("Records where message_for_record_id(hdr_record_id) found a message: {}/{}", match_hdr, total);
    println!("Records where message_for_event_value(xml_EventRecordID) found a message: {}/{}", match_xml_erid, total);
    println!();

    // Now check the *correct* approach: using the xml EventRecordID as the event_value key
    let mut parser3 = EvtxParser::from_path(mta_test_evtx()).expect("failed to open EVTX");
    let mut match_by_xml_erid_value = 0u32;
    let mut match_by_hdr_value = 0u32;
    let mut total3 = 0u32;

    // Load csv to compare messages
    let csv_contents = std::fs::read_to_string(mta_test_csv()).expect("failed to read CSV");
    let mut csv_msgs: Vec<String> = Vec::new();
    for (i, line) in csv_contents.lines().enumerate() {
        if i == 0 { continue; }
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() { continue; }
        let mut parts = line.splitn(6, ',');
        let _level = parts.next();
        let _time = parts.next();
        let _source = parts.next();
        let _event_id = parts.next();
        let _task_category = parts.next();
        let message = parts.next().unwrap_or("").trim().to_string();
        csv_msgs.push(message);
    }
    // CSV is newest-to-oldest; reverse to match EVTX order.
    csv_msgs.reverse();

    for chunk in parser3.chunks() {
        let mut chunk = match chunk {
            Ok(c) => c,
            Err(_) => continue,
        };
        let settings = Arc::new(Default::default());
        let mut chunk = match chunk.parse(Arc::clone(&settings)) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for record in chunk.iter() {
            let record = match record {
                Ok(r) => r,
                Err(_) => continue,
            };

            let hdr_rec_id = record.event_record_id;
            let xml_erid = extract_xml_event_record_id(&record.tree);

            let msg_by_hdr = mta.message_for_record_id(hdr_rec_id);
            let msg_by_xml_erid = xml_erid.and_then(|v| mta.message_for_event_value(v as u32));

            let expected = csv_msgs.get(total3 as usize).map(|s| s.trim());

            if let Some(expected_msg) = expected {
                if !expected_msg.is_empty() {
                    if msg_by_hdr.map(|s| s.trim()) == Some(expected_msg) {
                        match_by_hdr_value += 1;
                    }
                    if msg_by_xml_erid.map(|s| s.trim()) == Some(expected_msg) {
                        match_by_xml_erid_value += 1;
                    }
                }
            }

            total3 += 1;
        }
    }

    println!("Comparing with CSV ground truth ({} records):", total3);
    println!("  Matches using hdr_record_id as event_value:    {}/{}", match_by_hdr_value, total3);
    println!("  Matches using xml_EventRecordID as event_value: {}/{}", match_by_xml_erid_value, total3);
    println!();

    if match_by_xml_erid_value > match_by_hdr_value {
        println!("CONCLUSION: event_value corresponds to the XML <EventRecordID>, NOT the header record_id.");
    } else if match_by_hdr_value > match_by_xml_erid_value {
        println!("CONCLUSION: event_value corresponds to the header record_id (sequential counter).");
    } else if match_by_hdr_value == match_by_xml_erid_value && match_by_hdr_value > 0 {
        println!("CONCLUSION: Both produce the same results -- the header record_id and xml EventRecordID");
        println!("            likely have the same values in this test data. Need different data to distinguish.");
    } else {
        println!("CONCLUSION: Neither approach matched well. The event_value might be something else entirely.");
        println!("            (Perhaps it's the EventID, not the EventRecordID?)");
    }

    // Also check: does event_value == entry_index (i.e., are they the same)?
    // The EVT section stores (entry_index, event_value, msg_index). Let's see
    // if the mapping is simply sequential.
    println!("\n========================================");
    println!("PART 4: Does event_value == header record_id for records?");
    println!("========================================");

    let mut parser4 = EvtxParser::from_path(mta_test_evtx()).expect("failed to open EVTX");
    let mut rec_idx = 0usize;
    for chunk in parser4.chunks() {
        let mut chunk = match chunk {
            Ok(c) => c,
            Err(_) => continue,
        };
        let settings = Arc::new(Default::default());
        let mut chunk = match chunk.parse(Arc::clone(&settings)) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for record in chunk.iter() {
            let record = match record {
                Ok(r) => r,
                Err(_) => continue,
            };

            let hdr = record.event_record_id;
            let xml_erid = extract_xml_event_record_id(&record.tree);

            // Try looking up by entry_index (sequential 0-based index)
            let msg_by_entry = mta.message_for_entry_index(rec_idx as u32);
            let msg_by_hdr = mta.message_for_record_id(hdr);

            let expected = csv_msgs.get(rec_idx).map(|s| s.trim());

            if rec_idx < 20 {
                println!(
                    "rec[{:>3}]: hdr_id={:>5}, xml_erid={:>5}, msg_by_entry_idx={}, msg_by_hdr_rid={}, expected={}",
                    rec_idx,
                    hdr,
                    xml_erid.map(|v| v.to_string()).unwrap_or("?".into()),
                    msg_by_entry.is_some(),
                    msg_by_hdr.is_some(),
                    expected.unwrap_or("?"),
                );
            }

            rec_idx += 1;
        }
    }

    println!("\nTotal records iterated: {}", rec_idx);
}
