#[cfg(feature = "wevt_templates")]
mod wevt_cache_fallback {
    use std::sync::Arc;

    use evtx::EvtxParser;
    use evtx::ParserSettings;
    use evtx::wevt_templates::WevtCache;

    const EVTX_FILE_HEADER_SIZE: usize = 4096;
    const EVTX_CHUNK_SIZE: usize = 65536;

    fn compute_wevt_inline_name_hash_utf16(name: &str) -> u16 {
        const MULT: u32 = 65599;
        let mut hash: u32 = 0;
        for cu in name.encode_utf16() {
            hash = hash.wrapping_mul(MULT).wrapping_add(u32::from(cu));
        }
        (hash & 0xffff) as u16
    }

    fn wevt_inline_name_bytes(name: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let hash = compute_wevt_inline_name_hash_utf16(name);
        out.extend_from_slice(&hash.to_le_bytes());
        out.extend_from_slice(&(name.encode_utf16().count() as u16).to_le_bytes());
        for cu in name.encode_utf16() {
            out.extend_from_slice(&cu.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes()); // NUL
        out
    }

    fn build_minimal_wevt_template_binxml() -> Vec<u8> {
        // Template definition (WEVT inline names, dep_id present):
        // <?xml?> is added by the renderer; this is just the BinXML fragment.
        //
        // <Event><Data>%{0}</Data></Event>
        let mut buf = Vec::new();

        // Fragment header: token 0x0f + version 1.1 + flags 0
        buf.extend_from_slice(&[0x0f, 0x01, 0x01, 0x00]);

        // OpenStartElement Event (0x01), with dep_id (u16) + data_size (u32) + inline name
        buf.push(0x01);
        buf.extend_from_slice(&0xFFFFu16.to_le_bytes()); // dep id
        buf.extend_from_slice(&0u32.to_le_bytes()); // data_size (unused by builder)
        buf.extend_from_slice(&wevt_inline_name_bytes("Event"));
        buf.push(0x02); // CloseStartElement

        // OpenStartElement Data
        buf.push(0x01);
        buf.extend_from_slice(&0xFFFFu16.to_le_bytes()); // dep id
        buf.extend_from_slice(&0u32.to_le_bytes()); // data_size
        buf.extend_from_slice(&wevt_inline_name_bytes("Data"));
        buf.push(0x02); // CloseStartElement

        // Substitution %{0} (NormalSubstitution token 0x0d, index u16, type u8=StringType(0x01))
        buf.push(0x0d);
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.push(0x01);

        buf.push(0x04); // CloseElement (Data)
        buf.push(0x04); // CloseElement (Event)
        buf.push(0x00); // EndOfStream
        buf
    }

    fn find_first_template_instance_guid_and_offset(
        evtx_bytes: &[u8],
    ) -> (usize, u64, u32, winstructs::guid::Guid) {
        let settings = ParserSettings::default().num_threads(1);
        let mut parser = EvtxParser::from_buffer(evtx_bytes.to_vec())
            .expect("parse EVTX buffer")
            .with_configuration(settings.clone());

        for (chunk_index, chunk_res) in parser.chunks().enumerate() {
            let mut chunk_data = chunk_res.expect("chunk");
            let mut chunk = chunk_data
                .parse(Arc::new(settings.clone()))
                .expect("parse chunk");

            for record_res in chunk.iter() {
                let record = record_res.expect("record parse should succeed");
                let instances = record.template_instances().expect("template instances");
                let Some(tpl) = instances.first() else {
                    continue;
                };

                let template_def_offset = tpl.template_def_offset;
                let guid = if let Some(g) = tpl.template_guid.as_ref() {
                    g.clone()
                } else {
                    // Template header: u32 next + [u8;16] guid + u32 size
                    let off = template_def_offset as usize + 4;
                    let guid_bytes = record
                        .chunk
                        .data
                        .get(off..off + 16)
                        .expect("guid bytes in chunk");
                    winstructs::guid::Guid::from_buffer(guid_bytes).expect("guid parse")
                };

                return (
                    chunk_index,
                    record.event_record_id,
                    template_def_offset,
                    guid,
                );
            }
        }

        panic!("no TemplateInstance found in sample EVTX");
    }

    fn corrupt_chunk_template_binxml(
        evtx_bytes: &mut [u8],
        chunk_index: usize,
        template_def_offset: u32,
    ) {
        // Corrupt the first token of the template's BinXML fragment (after the 24-byte template header).
        let template_data_start = EVTX_FILE_HEADER_SIZE
            + chunk_index * EVTX_CHUNK_SIZE
            + template_def_offset as usize
            + 24;
        let b = evtx_bytes
            .get_mut(template_data_start)
            .expect("template data start in bounds");
        *b = 0xFF;
    }

    #[test]
    fn wevt_cache_fallback_recovers_corrupt_chunk_template_for_xml_and_json() {
        let sample = include_bytes!("../samples/security.evtx");
        let (chunk_index, record_id, template_def_offset, guid) =
            find_first_template_instance_guid_and_offset(sample);

        // Build a minimal in-memory WEVT cache that can serve this GUID.
        let mut temp_bytes = vec![0u8; 40];
        temp_bytes.extend_from_slice(&build_minimal_wevt_template_binxml());
        let cache = Arc::new(WevtCache::new());
        cache.insert_temp_bytes(&guid.to_string(), Arc::new(temp_bytes));

        // Corrupt the EVTX buffer so chunk-template parsing fails.
        let mut corrupted = sample.to_vec();
        corrupt_chunk_template_binxml(&mut corrupted, chunk_index, template_def_offset);

        // Without cache: the target record should fail to parse.
        {
            let settings = ParserSettings::default().num_threads(1);
            let mut parser = EvtxParser::from_buffer(corrupted.clone())
                .expect("parse corrupted buffer")
                .with_configuration(settings.clone());

            let mut saw_target_error = false;
            for chunk_res in parser.chunks() {
                let mut chunk_data = chunk_res.expect("chunk");
                let mut chunk = chunk_data
                    .parse(Arc::new(settings.clone()))
                    .expect("parse chunk");
                for record_res in chunk.iter() {
                    match record_res {
                        Ok(record) => {
                            if record.event_record_id == record_id {
                                panic!("expected record {record_id} to fail without cache");
                            }
                        }
                        Err(e) => {
                            if let evtx::err::EvtxError::FailedToParseRecord {
                                record_id: rid, ..
                            } = &e
                                && *rid == record_id
                            {
                                saw_target_error = true;
                                break;
                            }
                        }
                    }
                }
                if saw_target_error {
                    break;
                }
            }
            assert!(
                saw_target_error,
                "expected to observe FailedToParseRecord for record_id={record_id} without cache"
            );
        }

        // With cache: the target record should parse and render.
        {
            let settings = ParserSettings::default()
                .num_threads(1)
                .wevt_cache(Some(cache));
            let mut parser = EvtxParser::from_buffer(corrupted)
                .expect("parse corrupted buffer")
                .with_configuration(settings.clone());

            let mut saw_target_ok = false;
            for chunk_res in parser.chunks() {
                let mut chunk_data = chunk_res.expect("chunk");
                let mut chunk = chunk_data
                    .parse(Arc::new(settings.clone()))
                    .expect("parse chunk");
                for record_res in chunk.iter() {
                    let record = match record_res {
                        Ok(r) => r,
                        Err(e) => panic!("unexpected parse error with cache: {e:?}"),
                    };
                    if record.event_record_id != record_id {
                        continue;
                    }

                    let xml = record.clone().into_xml().expect("render xml").data;
                    assert!(xml.contains("<Event>") && xml.contains("</Event>"));
                    assert!(xml.contains("<Data>") && xml.contains("</Data>"));

                    // Also validate JSON output is well-formed and contains Event root.
                    let json = record.into_json().expect("render json").data;
                    let v: serde_json::Value = serde_json::from_str(&json).expect("valid json");
                    assert!(v.get("Event").is_some(), "expected Event key in JSON");

                    saw_target_ok = true;
                    break;
                }
                if saw_target_ok {
                    break;
                }
            }

            assert!(
                saw_target_ok,
                "expected to render record_id={record_id} successfully with cache"
            );
        }
    }
}
