mod fixtures;

#[cfg(feature = "wevt_templates")]
mod wevt_templates {
    use super::fixtures::CLI_TEST_LOCK;
    use evtx::binxml::value_variant::BinXmlValue;
    use evtx::wevt_templates::manifest::{
        CrimManifest, EventKey, MapDefinition, WevtManifestError,
    };
    use evtx::wevt_templates::{ResourceIdentifier, extract_wevt_template_resources};
    use evtx::wevt_templates::{
        render_template_definition_to_xml, render_template_definition_to_xml_with_values,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    const MINIMAL_PE: &[u8] = include_bytes!("fixtures/wevt_template_minimal_pe.bin");
    const MINIMAL_RESOURCE_DATA: &[u8] = b"CRIM|K\0\0WEVTTEST";

    fn sized_utf16_z_bytes(s: &str) -> Vec<u8> {
        let u16_count = s.encode_utf16().count() as u32;
        let size = 4 + u16_count * 2 + 2; // size prefix + utf16 + NUL
        let mut out = Vec::with_capacity(size as usize);
        out.extend_from_slice(&size.to_le_bytes());
        for cu in s.encode_utf16() {
            out.extend_from_slice(&cu.to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn wevt_layout_for_single_provider(
        descriptor_count: usize,
        unknown2_count: usize,
    ) -> (u32, u32) {
        let provider_data_off: u32 = 16 + 20; // CRIM header + 1 provider descriptor
        let wevt_size: u32 = 20
            + 8u32.saturating_mul(descriptor_count as u32)
            + 4u32.saturating_mul(unknown2_count as u32);
        (provider_data_off, wevt_size)
    }

    fn element_offsets_after_wevt(
        provider_data_off: u32,
        wevt_size: u32,
        element_sizes: &[usize],
    ) -> Vec<u32> {
        let mut offs = Vec::with_capacity(element_sizes.len());
        let mut cur = provider_data_off + wevt_size;
        for &sz in element_sizes {
            offs.push(cur);
            cur = cur.saturating_add(sz as u32);
        }
        offs
    }

    fn build_crim_single_provider_blob(
        provider_guid: [u8; 16],
        wevt_message_id: u32,
        unknown2: &[u32],
        element_offsets: &[u32],
        elements: &[Vec<u8>],
        tail: &[u8],
    ) -> Vec<u8> {
        assert_eq!(
            element_offsets.len(),
            elements.len(),
            "element_offsets/element vec length mismatch"
        );

        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(elements.len(), unknown2.len());

        if let Some(&first) = element_offsets.first() {
            assert_eq!(
                first,
                provider_data_off + wevt_size,
                "unexpected first element offset"
            );
        }

        let mut blob = Vec::new();

        // CRIM header (size patched after assembly).
        blob.extend_from_slice(b"CRIM");
        blob.extend_from_slice(&0u32.to_le_bytes()); // size placeholder
        blob.extend_from_slice(&3u16.to_le_bytes()); // major
        blob.extend_from_slice(&1u16.to_le_bytes()); // minor
        blob.extend_from_slice(&1u32.to_le_bytes()); // provider_count

        // provider descriptor
        blob.extend_from_slice(&provider_guid);
        blob.extend_from_slice(&provider_data_off.to_le_bytes());

        // WEVT header + descriptors + unknown2
        blob.extend_from_slice(b"WEVT");
        blob.extend_from_slice(&wevt_size.to_le_bytes());
        blob.extend_from_slice(&wevt_message_id.to_le_bytes());
        blob.extend_from_slice(&(elements.len() as u32).to_le_bytes()); // descriptor count
        blob.extend_from_slice(&(unknown2.len() as u32).to_le_bytes()); // unknown2 count
        for &off in element_offsets {
            blob.extend_from_slice(&off.to_le_bytes());
            blob.extend_from_slice(&0u32.to_le_bytes()); // unknown
        }
        for &v in unknown2 {
            blob.extend_from_slice(&v.to_le_bytes());
        }

        // elements (must be appended in the same order as element_offsets were computed).
        for el in elements {
            blob.extend_from_slice(el);
        }

        // trailing bytes (strings, etc.)
        blob.extend_from_slice(tail);

        // Patch CRIM.size.
        let total_size = u32::try_from(blob.len()).unwrap();
        blob[4..8].copy_from_slice(&total_size.to_le_bytes());

        blob
    }

    #[test]
    fn it_extracts_wevt_template_from_minimal_synthetic_pe() {
        let resources =
            extract_wevt_template_resources(MINIMAL_PE).expect("extract should succeed");
        assert_eq!(resources.len(), 1);

        let r = &resources[0];
        assert_eq!(r.resource, ResourceIdentifier::Id(1));
        assert_eq!(r.lang_id, 1033);
        assert_eq!(r.data.as_slice(), MINIMAL_RESOURCE_DATA);
    }

    #[test]
    fn cli_extracts_wevt_template_from_minimal_synthetic_pe() {
        let _guard = CLI_TEST_LOCK.lock().unwrap();
        let d = tempdir().unwrap();
        let out_file = d.path().join("cache.wevtcache");

        let pe_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("wevt_template_minimal_pe.bin");

        let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
        cmd.args([
            "extract-wevt-templates",
            "--input",
            pe_path.to_str().unwrap(),
            "--output",
            out_file.to_str().unwrap(),
            "--overwrite",
        ]);

        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        // Expect a single CRIM blob in the cache file.
        let bytes = fs::read(&out_file).unwrap();
        assert!(bytes.len() > 16 + 1 + 8);
        assert_eq!(&bytes[0..8], b"WEVTCACH");
        assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[12..16].try_into().unwrap()), 1);

        // Entry: kind (1 byte) + len (8 bytes) + payload.
        assert_eq!(bytes[16], 1);
        let len = u64::from_le_bytes(bytes[17..25].try_into().unwrap()) as usize;
        let payload = &bytes[25..25 + len];
        assert_eq!(payload, MINIMAL_RESOURCE_DATA);
    }

    #[test]
    fn it_parses_synthetic_crim_and_links_event_to_template_by_offset() {
        // Minimal CRIM -> WEVT -> EVNT + TTBL with a single TEMP (no BinXML payload).
        //
        // This validates the core join key in libfwevt: `EVNT.template_offset` points to a `TEMP`
        // definition offset (relative to the start of the CRIM blob).
        let guid_bytes: [u8; 16] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00,
        ];

        let provider_data_off: u32 = 16 + 20;
        let wevt_size: u32 = 20 + 8 * 2;
        let evnt_off: u32 = provider_data_off + wevt_size;
        let evnt_size: u32 = 16 + 48; // header + 1 event
        let ttbl_off: u32 = evnt_off + evnt_size;

        let temp_size: u32 = 40;
        let ttbl_size: u32 = 12 + temp_size;
        let temp_off: u32 = ttbl_off + 12;

        // TEMP has no items, so template_items_offset is either 0 or end-of-template.
        let template_items_offset: u32 = temp_off + temp_size;

        // Build EVNT (1 event pointing at TEMP offset)
        let mut evnt = Vec::with_capacity(evnt_size as usize);
        evnt.extend_from_slice(b"EVNT");
        evnt.extend_from_slice(&evnt_size.to_le_bytes());
        evnt.extend_from_slice(&1u32.to_le_bytes()); // count
        evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown
        // Event definition (48 bytes)
        evnt.extend_from_slice(&7u16.to_le_bytes()); // event id
        evnt.push(1u8); // version
        evnt.push(0u8); // channel
        evnt.push(0u8); // level
        evnt.push(0u8); // opcode
        evnt.extend_from_slice(&0u16.to_le_bytes()); // task
        evnt.extend_from_slice(&0u64.to_le_bytes()); // keywords
        evnt.extend_from_slice(&0xffffffffu32.to_le_bytes()); // message id
        evnt.extend_from_slice(&temp_off.to_le_bytes()); // template_offset
        evnt.extend_from_slice(&0u32.to_le_bytes()); // opcode_offset
        evnt.extend_from_slice(&0u32.to_le_bytes()); // level_offset
        evnt.extend_from_slice(&0u32.to_le_bytes()); // task_offset
        evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown_count
        evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown_offset
        evnt.extend_from_slice(&0u32.to_le_bytes()); // flags
        assert_eq!(evnt.len(), evnt_size as usize);

        // Build TTBL (1 TEMP)
        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // template count
        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_name_count
        ttbl.extend_from_slice(&template_items_offset.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type (observed as 1 for EventData)
        ttbl.extend_from_slice(&guid_bytes);
        assert_eq!(ttbl.len(), ttbl_size as usize);

        let total_size = (ttbl_off as usize) + ttbl.len();
        let mut blob = Vec::with_capacity(total_size);

        // CRIM
        blob.extend_from_slice(b"CRIM");
        blob.extend_from_slice(&(total_size as u32).to_le_bytes()); // size
        blob.extend_from_slice(&3u16.to_le_bytes()); // major
        blob.extend_from_slice(&1u16.to_le_bytes()); // minor
        blob.extend_from_slice(&1u32.to_le_bytes()); // provider_count

        // provider descriptor
        blob.extend_from_slice(&[0u8; 16]); // provider GUID (unused here)
        blob.extend_from_slice(&provider_data_off.to_le_bytes());

        // WEVT header + 2 descriptors (EVNT + TTBL)
        blob.extend_from_slice(b"WEVT");
        blob.extend_from_slice(&wevt_size.to_le_bytes());
        blob.extend_from_slice(&0xffffffffu32.to_le_bytes()); // message-table id
        blob.extend_from_slice(&2u32.to_le_bytes()); // descriptor count
        blob.extend_from_slice(&0u32.to_le_bytes()); // unknown2 count
        // descriptor 0: EVNT
        blob.extend_from_slice(&evnt_off.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());
        // descriptor 1: TTBL
        blob.extend_from_slice(&ttbl_off.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes());

        // EVNT + TTBL
        blob.extend_from_slice(&evnt);
        blob.extend_from_slice(&ttbl);

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        assert_eq!(manifest.providers.len(), 1);
        let p = &manifest.providers[0];

        let events = &p
            .wevt
            .elements
            .events
            .as_ref()
            .expect("EVNT present")
            .events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].template_offset, Some(temp_off));

        let tpl = p.template_by_offset(temp_off).expect("template resolved");
        assert_eq!(tpl.offset, temp_off);
        assert_eq!(tpl.size, temp_size);
        assert_eq!(tpl.item_descriptor_count, 0);
        assert_eq!(tpl.template_items_offset, template_items_offset);
    }

    #[test]
    fn it_builds_manifest_index_and_dedupes_event_template_guids() {
        // EVNT.size==0 + TTBL.size==0 branches, and build_index coverage (including the dedupe check).
        let provider_guid = [0u8; 16];
        let wevt_message_id: u32 = 0x12345678;

        let descriptor_count = 2usize; // EVNT + TTBL
        let unknown2: [u32; 0] = [];
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());

        let evnt_off = provider_data_off + wevt_size;
        let evnt_count: u32 = 3;
        let evnt_len = 16 + (48 * evnt_count as usize);
        let ttbl_off = evnt_off + (evnt_len as u32);

        let temp_size: u32 = 40;
        let temp_off = ttbl_off + 12;

        // EVNT (3 events: 1 with no template, 2 duplicates pointing at the same TEMP)
        let mut evnt = Vec::with_capacity(evnt_len);
        evnt.extend_from_slice(b"EVNT");
        evnt.extend_from_slice(&0u32.to_le_bytes()); // size==0
        evnt.extend_from_slice(&evnt_count.to_le_bytes());
        evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown

        let mut push_event = |template_offset: u32| {
            evnt.extend_from_slice(&7u16.to_le_bytes()); // event id
            evnt.push(1u8); // version
            evnt.push(0u8); // channel
            evnt.push(0u8); // level
            evnt.push(0u8); // opcode
            evnt.extend_from_slice(&0u16.to_le_bytes()); // task
            evnt.extend_from_slice(&0u64.to_le_bytes()); // keywords
            evnt.extend_from_slice(&0u32.to_le_bytes()); // message id
            evnt.extend_from_slice(&template_offset.to_le_bytes());
            evnt.extend_from_slice(&0u32.to_le_bytes()); // opcode_offset
            evnt.extend_from_slice(&0u32.to_le_bytes()); // level_offset
            evnt.extend_from_slice(&0u32.to_le_bytes()); // task_offset
            evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown_count
            evnt.extend_from_slice(&0u32.to_le_bytes()); // unknown_offset
            evnt.extend_from_slice(&0u32.to_le_bytes()); // flags
        };

        push_event(0); // None
        push_event(temp_off); // Some
        push_event(temp_off); // Some duplicate
        assert_eq!(evnt.len(), evnt_len);

        // TTBL (size==0) with one TEMP. TEMP has no items and template_items_offset==0 (valid).
        let ttbl_len: usize = 12 + (temp_size as usize);
        let mut ttbl = Vec::with_capacity(ttbl_len);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // size==0
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // template count

        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_name_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // template_items_offset==0
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type
        ttbl.extend_from_slice(&[0x11u8; 16]); // template guid
        assert_eq!(ttbl.len(), ttbl_len);

        let total_size = (ttbl_off as usize) + ttbl.len();

        let element_offsets = vec![evnt_off, ttbl_off];
        let elements = vec![evnt, ttbl];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &[],
        );
        assert_eq!(blob.len(), total_size);

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        let idx = manifest.build_index();

        let provider = &manifest.providers[0];
        let ttbl = provider
            .wevt
            .elements
            .templates
            .as_ref()
            .expect("TTBL present");
        let tpl = &ttbl.templates[0];

        let tpl_guid_str = tpl.guid.to_string();
        assert!(
            idx.templates_by_guid.contains_key(&tpl_guid_str),
            "expected templates_by_guid to contain template guid"
        );

        assert_eq!(
            idx.event_to_template_guids.len(),
            1,
            "expected only one unique EventKey entry"
        );

        let key = EventKey {
            provider_guid: provider.guid.to_string(),
            event_id: 7,
            version: 1,
            channel: 0,
            level: 0,
            opcode: 0,
            task: 0,
            keywords: 0,
        };
        let mapped = idx
            .event_to_template_guids
            .get(&key)
            .expect("expected EventKey mapping");
        assert_eq!(mapped.len(), 1, "expected deduped guid list");
        assert_eq!(mapped[0].to_string(), tpl.guid.to_string());
    }

    #[test]
    fn it_labels_substitutions_using_template_item_names() {
        fn name_hash_utf16(s: &str) -> u16 {
            let mut hash: u32 = 0;
            for cu in s.encode_utf16() {
                hash = hash.wrapping_mul(65599).wrapping_add(u32::from(cu));
            }
            (hash & 0xffff) as u16
        }

        fn push_inline_name(buf: &mut Vec<u8>, name: &str) {
            let hash = name_hash_utf16(name);
            buf.extend_from_slice(&hash.to_le_bytes());
            buf.extend_from_slice(&(name.encode_utf16().count() as u16).to_le_bytes());
            for cu in name.encode_utf16() {
                buf.extend_from_slice(&cu.to_le_bytes());
            }
            buf.extend_from_slice(&0u16.to_le_bytes());
        }

        // Build a minimal CRIM with one provider and one TTBL/TEMP that contains a BinXML fragment
        // with a substitution token. The TEMP item descriptor provides a name for substitution 0.
        let provider_data_off: u32 = 16 + 20;
        let wevt_size: u32 = 28; // WEVT header (20) + 1 descriptor (8)
        let ttbl_off: u32 = provider_data_off + wevt_size;
        let temp_off: u32 = ttbl_off + 12;

        // BinXML fragment: <EventData><Data>{sub:0}</Data></EventData>
        let mut binxml = Vec::new();
        binxml.extend_from_slice(&[0x0f, 0x01, 0x01, 0x00]); // StartOfStream + fragment header

        // <EventData>
        binxml.push(0x01); // OpenStartElement
        binxml.extend_from_slice(&0xFFFFu16.to_le_bytes()); // dependency id
        binxml.extend_from_slice(&0u32.to_le_bytes()); // data size (not enforced)
        push_inline_name(&mut binxml, "EventData");
        binxml.push(0x02); // CloseStartElement

        // <Data>
        binxml.push(0x01); // OpenStartElement
        binxml.extend_from_slice(&0xFFFFu16.to_le_bytes()); // dependency id
        binxml.extend_from_slice(&0u32.to_le_bytes()); // data size
        push_inline_name(&mut binxml, "Data");
        binxml.push(0x02); // CloseStartElement

        // {sub:0} as a normal substitution, type=StringType (0x01)
        binxml.push(0x0d);
        binxml.extend_from_slice(&0u16.to_le_bytes());
        binxml.push(0x01);

        // </Data></EventData>
        binxml.push(0x04);
        binxml.push(0x04);
        binxml.push(0x00); // EndOfStream

        let item_name = "Foo";
        let item_name_u16_count = item_name.encode_utf16().count() as u32;
        let item_name_struct_size: u32 = 4 + item_name_u16_count * 2 + 2; // size + utf16 + NUL

        let descriptor_count: u32 = 1;
        let name_count: u32 = 1;
        let template_items_offset: u32 = temp_off + 40 + (binxml.len() as u32);
        let name_offset: u32 = template_items_offset + 20; // right after 1 descriptor

        let temp_size: u32 =
            40 + (binxml.len() as u32) + 20 * descriptor_count + item_name_struct_size;
        let ttbl_size: u32 = 12 + temp_size;

        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // template count

        // TEMP header
        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&descriptor_count.to_le_bytes());
        ttbl.extend_from_slice(&name_count.to_le_bytes());
        ttbl.extend_from_slice(&template_items_offset.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type
        ttbl.extend_from_slice(&[0x11u8; 16]); // template guid

        // BinXML fragment
        ttbl.extend_from_slice(&binxml);

        // Template item descriptor (20 bytes)
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // unknown1
        ttbl.push(0x01); // inType (UnicodeString)
        ttbl.push(0x01); // outType (xs:string)
        ttbl.extend_from_slice(&0u16.to_le_bytes()); // unknown3
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // unknown4
        ttbl.extend_from_slice(&1u16.to_le_bytes()); // count
        ttbl.extend_from_slice(&0u16.to_le_bytes()); // length
        ttbl.extend_from_slice(&name_offset.to_le_bytes());

        // Template item name (size-prefixed utf16 + NUL)
        ttbl.extend_from_slice(&item_name_struct_size.to_le_bytes());
        for cu in item_name.encode_utf16() {
            ttbl.extend_from_slice(&cu.to_le_bytes());
        }
        ttbl.extend_from_slice(&0u16.to_le_bytes());

        assert_eq!(ttbl.len(), ttbl_size as usize);

        let total_size = (ttbl_off as usize) + ttbl.len();
        let mut blob = Vec::with_capacity(total_size);

        // CRIM
        blob.extend_from_slice(b"CRIM");
        blob.extend_from_slice(&(total_size as u32).to_le_bytes()); // size
        blob.extend_from_slice(&3u16.to_le_bytes()); // major
        blob.extend_from_slice(&1u16.to_le_bytes()); // minor
        blob.extend_from_slice(&1u32.to_le_bytes()); // provider_count

        // provider descriptor
        blob.extend_from_slice(&[0x22u8; 16]); // provider GUID (unused here)
        blob.extend_from_slice(&provider_data_off.to_le_bytes());

        // WEVT header + 1 descriptor (TTBL)
        blob.extend_from_slice(b"WEVT");
        blob.extend_from_slice(&wevt_size.to_le_bytes());
        blob.extend_from_slice(&0xffffffffu32.to_le_bytes());
        blob.extend_from_slice(&1u32.to_le_bytes()); // descriptor count
        blob.extend_from_slice(&0u32.to_le_bytes()); // unknown2 count
        blob.extend_from_slice(&ttbl_off.to_le_bytes()); // TTBL offset
        blob.extend_from_slice(&0u32.to_le_bytes()); // unknown

        // TTBL
        blob.extend_from_slice(&ttbl);

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        let provider = &manifest.providers[0];
        let ttbl = provider
            .wevt
            .elements
            .templates
            .as_ref()
            .expect("TTBL present");
        let tpl = &ttbl.templates[0];

        assert_eq!(tpl.items.len(), 1);
        assert_eq!(tpl.items[0].name.as_deref(), Some(item_name));

        let xml = render_template_definition_to_xml(tpl, encoding::all::WINDOWS_1252)
            .expect("render should succeed");
        assert!(
            xml.contains("{sub:0:Foo}"),
            "expected substitution placeholder to include item name, got: {xml}"
        );

        let subs = vec![BinXmlValue::AnsiStringType("BAR")];
        let bump = bumpalo::Bump::new();
        let applied = render_template_definition_to_xml_with_values(
            tpl,
            &subs,
            encoding::all::WINDOWS_1252,
            &bump,
        )
        .expect("render with substitutions should succeed");
        assert!(
            applied.contains("BAR") && !applied.contains("{sub:"),
            "expected placeholders to be replaced, got: {applied}"
        );
    }

    fn build_defs_manifest_with_sizes(size_zero: bool) -> CrimManifest<'static> {
        // Build a single-provider CRIM with CHAN/KEYW/LEVL/OPCO/TASK elements and out-of-band names.
        let provider_guid = [0x44u8; 16];
        let wevt_message_id: u32 = 0x0bad_f00d;
        let unknown2 = [0xdead_beefu32, 0xcafe_babeu32];

        let chan_len: usize = 12 + 16; // header + 1 channel
        let keyw_len: usize = 12 + 16; // header + 1 keyword
        let levl_len: usize = 12 + 12; // header + 1 level
        let opco_len: usize = 12 + 12; // header + 1 opcode
        let task_len: usize = 12 + 28; // header + 1 task

        let element_sizes = [chan_len, keyw_len, levl_len, opco_len, task_len];
        let descriptor_count = element_sizes.len();
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let element_offsets =
            element_offsets_after_wevt(provider_data_off, wevt_size, &element_sizes);

        let tail_off = provider_data_off + wevt_size + (element_sizes.iter().sum::<usize>() as u32);
        let chan_name = sized_utf16_z_bytes("ChanA");
        let keyw_name = sized_utf16_z_bytes("KeywA");
        let levl_name = sized_utf16_z_bytes("LevlA");
        let opco_name = sized_utf16_z_bytes("OpcoA");
        let task_name = sized_utf16_z_bytes("TaskA");

        let chan_name_off = tail_off;
        let keyw_name_off = chan_name_off + (chan_name.len() as u32);
        let levl_name_off = keyw_name_off + (keyw_name.len() as u32);
        let opco_name_off = levl_name_off + (levl_name.len() as u32);
        let task_name_off = opco_name_off + (opco_name.len() as u32);

        let mut tail = Vec::new();
        tail.extend_from_slice(&chan_name);
        tail.extend_from_slice(&keyw_name);
        tail.extend_from_slice(&levl_name);
        tail.extend_from_slice(&opco_name);
        tail.extend_from_slice(&task_name);

        let size_or = |v: usize| if size_zero { 0u32 } else { v as u32 };

        // CHAN
        let mut chan = Vec::with_capacity(chan_len);
        chan.extend_from_slice(b"CHAN");
        chan.extend_from_slice(&size_or(chan_len).to_le_bytes());
        chan.extend_from_slice(&1u32.to_le_bytes()); // count
        chan.extend_from_slice(&42u32.to_le_bytes()); // identifier
        chan.extend_from_slice(&chan_name_off.to_le_bytes());
        chan.extend_from_slice(&0x1111u32.to_le_bytes()); // unknown
        chan.extend_from_slice(&0x2222u32.to_le_bytes()); // message_identifier (Some)
        assert_eq!(chan.len(), chan_len);

        // KEYW
        let mut keyw = Vec::with_capacity(keyw_len);
        keyw.extend_from_slice(b"KEYW");
        keyw.extend_from_slice(&size_or(keyw_len).to_le_bytes());
        keyw.extend_from_slice(&1u32.to_le_bytes()); // count
        keyw.extend_from_slice(&0x1122334455667788u64.to_le_bytes()); // identifier
        keyw.extend_from_slice(&0xffffffffu32.to_le_bytes()); // message_identifier (None)
        keyw.extend_from_slice(&keyw_name_off.to_le_bytes()); // data_offset
        assert_eq!(keyw.len(), keyw_len);

        // LEVL
        let mut levl = Vec::with_capacity(levl_len);
        levl.extend_from_slice(b"LEVL");
        levl.extend_from_slice(&size_or(levl_len).to_le_bytes());
        levl.extend_from_slice(&1u32.to_le_bytes()); // count
        levl.extend_from_slice(&5u32.to_le_bytes()); // identifier
        levl.extend_from_slice(&0x3333u32.to_le_bytes()); // message_identifier (Some)
        levl.extend_from_slice(&levl_name_off.to_le_bytes()); // data_offset
        assert_eq!(levl.len(), levl_len);

        // OPCO
        let mut opco = Vec::with_capacity(opco_len);
        opco.extend_from_slice(b"OPCO");
        opco.extend_from_slice(&size_or(opco_len).to_le_bytes());
        opco.extend_from_slice(&1u32.to_le_bytes()); // count
        opco.extend_from_slice(&9u32.to_le_bytes()); // identifier
        opco.extend_from_slice(&0xffffffffu32.to_le_bytes()); // message_identifier (None)
        opco.extend_from_slice(&opco_name_off.to_le_bytes()); // data_offset
        assert_eq!(opco.len(), opco_len);

        // TASK
        let mut task = Vec::with_capacity(task_len);
        task.extend_from_slice(b"TASK");
        task.extend_from_slice(&size_or(task_len).to_le_bytes());
        task.extend_from_slice(&1u32.to_le_bytes()); // count
        task.extend_from_slice(&7u32.to_le_bytes()); // identifier
        task.extend_from_slice(&0x4444u32.to_le_bytes()); // message_identifier (Some)
        task.extend_from_slice(&[0x33u8; 16]); // mui_identifier
        task.extend_from_slice(&task_name_off.to_le_bytes()); // data_offset
        assert_eq!(task.len(), task_len);

        let elements = vec![chan, keyw, levl, opco, task];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &tail,
        );

        // Leak to extend lifetime for test convenience.
        let blob: &'static [u8] = Box::leak(blob.into_boxed_slice());
        CrimManifest::parse(blob).expect("manifest parse should succeed")
    }

    #[test]
    fn it_parses_common_definition_elements_with_explicit_sizes() {
        let manifest = build_defs_manifest_with_sizes(false);
        let provider = &manifest.providers[0];

        assert_eq!(provider.wevt.message_identifier, Some(0x0bad_f00d));
        assert_eq!(provider.wevt.unknown2, vec![0xdead_beef, 0xcafe_babe]);

        let chan = provider
            .wevt
            .elements
            .channels
            .as_ref()
            .expect("CHAN present");
        assert_eq!(chan.channels.len(), 1);
        assert_eq!(chan.channels[0].name.as_deref(), Some("ChanA"));
        assert_eq!(chan.channels[0].message_identifier, Some(0x2222));

        let keyw = provider
            .wevt
            .elements
            .keywords
            .as_ref()
            .expect("KEYW present");
        assert_eq!(keyw.keywords.len(), 1);
        assert_eq!(keyw.keywords[0].name.as_deref(), Some("KeywA"));
        assert_eq!(keyw.keywords[0].message_identifier, None);

        let levl = provider
            .wevt
            .elements
            .levels
            .as_ref()
            .expect("LEVL present");
        assert_eq!(levl.levels.len(), 1);
        assert_eq!(levl.levels[0].name.as_deref(), Some("LevlA"));
        assert_eq!(levl.levels[0].message_identifier, Some(0x3333));

        let opco = provider
            .wevt
            .elements
            .opcodes
            .as_ref()
            .expect("OPCO present");
        assert_eq!(opco.opcodes.len(), 1);
        assert_eq!(opco.opcodes[0].name.as_deref(), Some("OpcoA"));
        assert_eq!(opco.opcodes[0].message_identifier, None);

        let task = provider.wevt.elements.tasks.as_ref().expect("TASK present");
        assert_eq!(task.tasks.len(), 1);
        assert_eq!(task.tasks[0].name.as_deref(), Some("TaskA"));
        assert_eq!(task.tasks[0].message_identifier, Some(0x4444));
    }

    #[test]
    fn it_parses_common_definition_elements_with_size_zero_compat() {
        let manifest = build_defs_manifest_with_sizes(true);
        let provider = &manifest.providers[0];

        assert!(provider.wevt.elements.channels.is_some());
        assert!(provider.wevt.elements.keywords.is_some());
        assert!(provider.wevt.elements.levels.is_some());
        assert!(provider.wevt.elements.opcodes.is_some());
        assert!(provider.wevt.elements.tasks.is_some());
    }

    #[test]
    fn it_parses_maps_with_implied_first_offset_and_size_zero() {
        let provider_guid = [0x55u8; 16];
        let wevt_message_id: u32 = 0xffffffff; // None
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize; // MAPS only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let maps_off = provider_data_off + wevt_size;

        let map_count: u32 = 3;
        let vmap_entry_count: u32 = 2;
        let vmap_size: u32 = 16 + 8 * vmap_entry_count + 2; // header + entries + trailing

        let map1_off = (maps_off + 16 + 8) + vmap_size; // implied_first + vmap_size
        let map2_off = map1_off + 4;

        let maps_len: usize = 16 + 8 + (vmap_size as usize) + 4 + 4;
        let map_string_off = maps_off + (maps_len as u32);

        let mut maps = Vec::with_capacity(maps_len);
        maps.extend_from_slice(b"MAPS");
        maps.extend_from_slice(&0u32.to_le_bytes()); // size==0
        maps.extend_from_slice(&map_count.to_le_bytes());
        maps.extend_from_slice(&0u32.to_le_bytes()); // first_map_offset==0 => implied

        // Remaining offsets array (count-1).
        maps.extend_from_slice(&map1_off.to_le_bytes());
        maps.extend_from_slice(&map2_off.to_le_bytes());

        // VMAP at implied_first = maps_off + 16 + (count-1)*4 = maps_off + 24
        maps.extend_from_slice(b"VMAP");
        maps.extend_from_slice(&vmap_size.to_le_bytes());
        maps.extend_from_slice(&map_string_off.to_le_bytes());
        maps.extend_from_slice(&vmap_entry_count.to_le_bytes());
        // entries
        maps.extend_from_slice(&1u32.to_le_bytes());
        maps.extend_from_slice(&0xffffffffu32.to_le_bytes()); // None
        maps.extend_from_slice(&2u32.to_le_bytes());
        maps.extend_from_slice(&1234u32.to_le_bytes()); // Some
        // trailing
        maps.extend_from_slice(&[0xaa, 0xbb]);

        // BMAP
        maps.extend_from_slice(b"BMAP");
        // unknown map type
        maps.extend_from_slice(b"ZZZZ");

        assert_eq!(maps.len(), maps_len);

        let tail = sized_utf16_z_bytes("MapStr");
        let element_offsets = vec![maps_off];
        let elements = vec![maps];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &tail,
        );

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        let provider = &manifest.providers[0];
        assert_eq!(provider.wevt.message_identifier, None);

        let maps = provider.wevt.elements.maps.as_ref().expect("MAPS present");
        assert_eq!(maps.maps.len(), 3);

        match &maps.maps[0] {
            MapDefinition::ValueMap(v) => {
                assert_eq!(v.size, vmap_size);
                assert_eq!(v.map_string.as_deref(), Some("MapStr"));
                assert_eq!(v.entries.len(), 2);
                assert_eq!(v.entries[0].message_identifier, None);
                assert_eq!(v.entries[1].message_identifier, Some(1234));
                assert_eq!(v.trailing, &[0xaa, 0xbb]);
            }
            _ => panic!("expected VMAP first"),
        }

        match &maps.maps[1] {
            MapDefinition::Bitmap(b) => {
                assert_eq!(b.data, b"BMAP");
            }
            _ => panic!("expected BMAP second"),
        }

        match &maps.maps[2] {
            MapDefinition::Unknown {
                signature, data, ..
            } => {
                assert_eq!(signature, b"ZZZZ");
                assert_eq!(data.len(), 4, "unknown map types are capped");
            }
            _ => panic!("expected unknown map third"),
        }
    }

    #[test]
    fn it_parses_maps_with_out_of_order_offsets() {
        // Regression test: some providers (e.g. `wevtsvc.dll`) have MAPS offset arrays that are not
        // sorted. We should compute boundaries in file order, not array order.
        let provider_guid = [0x55u8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize; // MAPS only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let maps_off = provider_data_off + wevt_size;

        let map_count: u32 = 3;
        let vmap_entry_count: u32 = 1;
        let vmap_size: u32 = 16 + 8 * vmap_entry_count;

        let implied_first = maps_off + 16 + 8;
        let map0_off = implied_first;
        let map1_off = map0_off + vmap_size;
        let map2_off = map1_off + 4;

        let maps_len: usize = 16 + 8 + (vmap_size as usize) + 4 + 4;
        let map_string_off = maps_off + (maps_len as u32);

        let mut maps = Vec::with_capacity(maps_len);
        maps.extend_from_slice(b"MAPS");
        maps.extend_from_slice(&(maps_len as u32).to_le_bytes());
        maps.extend_from_slice(&map_count.to_le_bytes());
        maps.extend_from_slice(&map0_off.to_le_bytes()); // first map offset is valid...

        // ...but the remaining offsets array is intentionally out-of-order.
        maps.extend_from_slice(&map2_off.to_le_bytes());
        maps.extend_from_slice(&map1_off.to_le_bytes());

        maps.extend_from_slice(b"VMAP");
        maps.extend_from_slice(&vmap_size.to_le_bytes());
        maps.extend_from_slice(&map_string_off.to_le_bytes());
        maps.extend_from_slice(&vmap_entry_count.to_le_bytes());
        maps.extend_from_slice(&0u32.to_le_bytes());
        maps.extend_from_slice(&0xffffffffu32.to_le_bytes());

        maps.extend_from_slice(b"BMAP");
        maps.extend_from_slice(b"ZZZZ");
        assert_eq!(maps.len(), maps_len);

        let tail = sized_utf16_z_bytes("X");
        let element_offsets = vec![maps_off];
        let elements = vec![maps];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &tail,
        );

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        let provider = &manifest.providers[0];
        let maps = provider.wevt.elements.maps.as_ref().expect("MAPS present");
        assert_eq!(maps.maps.len(), 3);
        assert!(matches!(maps.maps[0], MapDefinition::ValueMap(_)));
        assert!(matches!(maps.maps[1], MapDefinition::Bitmap(_)));
        assert!(matches!(maps.maps[2], MapDefinition::Unknown { .. }));
    }

    #[test]
    fn it_captures_unknown_provider_elements() {
        let provider_guid = [0x66u8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize;
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let unk_off = provider_data_off + wevt_size;

        let mut unk = Vec::new();
        unk.extend_from_slice(b"ZZZZ");
        unk.extend_from_slice(&12u32.to_le_bytes());
        unk.extend_from_slice(&0x01020304u32.to_le_bytes());
        assert_eq!(unk.len(), 12);

        let element_offsets = vec![unk_off];
        let elements = vec![unk];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &[],
        );

        let manifest = CrimManifest::parse(&blob).expect("manifest parse should succeed");
        let provider = &manifest.providers[0];
        assert_eq!(provider.wevt.elements.unknown.len(), 1);
        let u = &provider.wevt.elements.unknown[0];
        assert_eq!(u.signature, *b"ZZZZ");
        assert_eq!(u.offset, unk_off);
        assert_eq!(u.size, 12);
        assert_eq!(u.data.len(), 12);
    }

    #[test]
    fn it_reports_wevt_manifest_error_variants() {
        // InvalidSignature
        let err = CrimManifest::parse(b"NOPE1234").unwrap_err();
        match err {
            WevtManifestError::InvalidSignature {
                offset,
                expected,
                found,
            } => {
                assert_eq!(offset, 0);
                assert_eq!(expected, *b"CRIM");
                assert_eq!(found, *b"NOPE");
            }
            other => panic!("unexpected error: {other:?}"),
        }

        // Truncated
        let err = CrimManifest::parse(b"CRI").unwrap_err();
        assert!(matches!(err, WevtManifestError::Truncated { .. }));

        // SizeOutOfBounds: CRIM.size bigger than buffer.
        let mut blob = Vec::new();
        blob.extend_from_slice(b"CRIM");
        blob.extend_from_slice(&100u32.to_le_bytes()); // size
        blob.extend_from_slice(&3u16.to_le_bytes());
        blob.extend_from_slice(&1u16.to_le_bytes());
        blob.extend_from_slice(&0u32.to_le_bytes()); // provider_count
        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::SizeOutOfBounds {
                what: "CRIM.size",
                ..
            }
        ));

        // CountOutOfBounds: TEMP has item_descriptor_count==0 but item_name_count!=0.
        let provider_guid = [0x77u8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];
        let descriptor_count = 1usize;
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let ttbl_off = provider_data_off + wevt_size;

        let temp_size: u32 = 40;
        let ttbl_size: u32 = 12 + temp_size;
        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes());
        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // item_name_count (invalid)
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // template_items_offset
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type
        ttbl.extend_from_slice(&[0u8; 16]);
        assert_eq!(ttbl.len(), ttbl_size as usize);

        let element_offsets = vec![ttbl_off];
        let elements = vec![ttbl];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &[],
        );
        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::CountOutOfBounds { what, .. }
                if what.starts_with("TEMP.item_name_count")
        ));

        // OffsetOutOfBounds: TEMP.template_items_offset < template offset when item_descriptor_count>0.
        let temp_size: u32 = 40;
        let ttbl_size: u32 = 12 + temp_size;
        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes());
        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_name_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // template_items_offset (invalid here)
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type
        ttbl.extend_from_slice(&[0u8; 16]);
        assert_eq!(ttbl.len(), ttbl_size as usize);

        let element_offsets = vec![ttbl_off];
        let elements = vec![ttbl];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &[],
        );
        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::OffsetOutOfBounds {
                what: "TEMP.template_items_offset",
                ..
            }
        ));

        // InvalidUtf16String: CHAN name string has odd byte count.
        let descriptor_count = 1usize; // CHAN only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let chan_off = provider_data_off + wevt_size;
        let chan_len: usize = 12 + 16;
        let name_off = chan_off + (chan_len as u32);

        let mut chan = Vec::with_capacity(chan_len);
        chan.extend_from_slice(b"CHAN");
        chan.extend_from_slice(&(chan_len as u32).to_le_bytes());
        chan.extend_from_slice(&1u32.to_le_bytes());
        chan.extend_from_slice(&1u32.to_le_bytes());
        chan.extend_from_slice(&name_off.to_le_bytes());
        chan.extend_from_slice(&0u32.to_le_bytes());
        chan.extend_from_slice(&0xffffffffu32.to_le_bytes());
        assert_eq!(chan.len(), chan_len);

        let mut bad = Vec::new();
        bad.extend_from_slice(&5u32.to_le_bytes()); // size (4 + 1 byte)
        bad.push(0u8);

        let element_offsets = vec![chan_off];
        let elements = vec![chan];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &bad,
        );
        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::InvalidUtf16String {
                what: "CHAN name",
                ..
            }
        ));
    }

    #[test]
    fn it_errors_when_template_item_name_offset_overlaps_descriptor_table() {
        // Cover parse_template_items' boundary enforcement between descriptor table and name table.
        let provider_guid = [0x88u8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize; // TTBL only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let ttbl_off = provider_data_off + wevt_size;
        let temp_off = ttbl_off + 12;

        let temp_size: u32 = 60; // 40 header + 20 descriptor
        let ttbl_size: u32 = 12 + temp_size;
        let template_items_offset: u32 = temp_off + 40; // descriptor table begins immediately after TEMP header
        let name_offset: u32 = template_items_offset; // overlaps descriptor table => should error

        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // template count

        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // item_name_count
        ttbl.extend_from_slice(&template_items_offset.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type
        ttbl.extend_from_slice(&[0u8; 16]);

        // One template item descriptor (20 bytes). Name offset points inside this table.
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // unknown1
        ttbl.push(0x01); // inType
        ttbl.push(0x01); // outType
        ttbl.extend_from_slice(&0u16.to_le_bytes()); // unknown3
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // unknown4
        ttbl.extend_from_slice(&1u16.to_le_bytes()); // count
        ttbl.extend_from_slice(&0u16.to_le_bytes()); // length
        ttbl.extend_from_slice(&name_offset.to_le_bytes());

        assert_eq!(ttbl.len(), ttbl_size as usize);

        let element_offsets = vec![ttbl_off];
        let elements = vec![ttbl];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &[],
        );

        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::OffsetOutOfBounds {
                what: "template item name_offset overlaps descriptor table",
                ..
            }
        ));
    }

    #[test]
    fn it_errors_on_invalid_utf16_surrogate_in_names() {
        // Drive decode_utf16_z() through the String::from_utf16 error path (invalid surrogate).
        let provider_guid = [0x99u8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize; // CHAN only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let chan_off = provider_data_off + wevt_size;
        let chan_len: usize = 12 + 16;
        let name_off = chan_off + (chan_len as u32);

        let mut chan = Vec::with_capacity(chan_len);
        chan.extend_from_slice(b"CHAN");
        chan.extend_from_slice(&(chan_len as u32).to_le_bytes());
        chan.extend_from_slice(&1u32.to_le_bytes());
        chan.extend_from_slice(&1u32.to_le_bytes());
        chan.extend_from_slice(&name_off.to_le_bytes());
        chan.extend_from_slice(&0u32.to_le_bytes());
        chan.extend_from_slice(&0xffffffffu32.to_le_bytes());
        assert_eq!(chan.len(), chan_len);

        // size=8, payload = [0xD800, 0x0000] => invalid (unpaired surrogate).
        let mut bad = Vec::new();
        bad.extend_from_slice(&8u32.to_le_bytes());
        bad.extend_from_slice(&0xD800u16.to_le_bytes());
        bad.extend_from_slice(&0u16.to_le_bytes());

        let element_offsets = vec![chan_off];
        let elements = vec![chan];
        let blob = build_crim_single_provider_blob(
            provider_guid,
            wevt_message_id,
            &unknown2,
            &element_offsets,
            &elements,
            &bad,
        );
        let err = CrimManifest::parse(&blob).unwrap_err();
        assert!(matches!(
            err,
            WevtManifestError::InvalidUtf16String {
                what: "CHAN name",
                ..
            }
        ));
    }

    #[test]
    fn it_reports_vmap_error_paths() {
        // Cover parse_vmap() Truncated and SizeOutOfBounds branches via MAPS.
        let provider_guid = [0xaau8; 16];
        let wevt_message_id: u32 = 0xffffffff;
        let unknown2: [u32; 0] = [];

        let descriptor_count = 1usize; // MAPS only
        let (provider_data_off, wevt_size) =
            wevt_layout_for_single_provider(descriptor_count, unknown2.len());
        let maps_off = provider_data_off + wevt_size;

        // Case 1: VMAP slice < 16 => Truncated { what: "VMAP header" }.
        {
            let map_count: u32 = 2;
            let implied_first = maps_off + 16 + 4;
            let map0_off = implied_first;
            let map1_off = map0_off + 8; // only 8 bytes available for VMAP
            let maps_len: usize = 16 + 4 + 8 + 4;

            let mut maps = Vec::with_capacity(maps_len);
            maps.extend_from_slice(b"MAPS");
            maps.extend_from_slice(&(maps_len as u32).to_le_bytes());
            maps.extend_from_slice(&map_count.to_le_bytes());
            maps.extend_from_slice(&0u32.to_le_bytes()); // implied first
            maps.extend_from_slice(&map1_off.to_le_bytes());
            maps.extend_from_slice(b"VMAP");
            maps.extend_from_slice(&0u32.to_le_bytes()); // dummy
            maps.extend_from_slice(b"BMAP");
            assert_eq!(maps.len(), maps_len);

            let element_offsets = vec![maps_off];
            let elements = vec![maps];
            let blob = build_crim_single_provider_blob(
                provider_guid,
                wevt_message_id,
                &unknown2,
                &element_offsets,
                &elements,
                &[],
            );
            let err = CrimManifest::parse(&blob).unwrap_err();
            assert!(matches!(
                err,
                WevtManifestError::Truncated { what: "VMAP header", offset, .. }
                    if offset == map0_off
            ));
        }

        // Case 2: VMAP.size larger than available slice => SizeOutOfBounds { what: "VMAP.size" }.
        {
            let map_count: u32 = 2;
            let implied_first = maps_off + 16 + 4;
            let map0_off = implied_first;
            let map1_off = map0_off + 16; // exactly 16 bytes available
            let maps_len: usize = 16 + 4 + 16 + 4;

            let mut maps = Vec::with_capacity(maps_len);
            maps.extend_from_slice(b"MAPS");
            maps.extend_from_slice(&(maps_len as u32).to_le_bytes());
            maps.extend_from_slice(&map_count.to_le_bytes());
            maps.extend_from_slice(&0u32.to_le_bytes());
            maps.extend_from_slice(&map1_off.to_le_bytes());

            maps.extend_from_slice(b"VMAP");
            maps.extend_from_slice(&32u32.to_le_bytes()); // size > slice len
            maps.extend_from_slice(&0u32.to_le_bytes()); // map_string_offset
            maps.extend_from_slice(&0u32.to_le_bytes()); // entry_count
            maps.extend_from_slice(b"BMAP");
            assert_eq!(maps.len(), maps_len);

            let element_offsets = vec![maps_off];
            let elements = vec![maps];
            let blob = build_crim_single_provider_blob(
                provider_guid,
                wevt_message_id,
                &unknown2,
                &element_offsets,
                &elements,
                &[],
            );
            let err = CrimManifest::parse(&blob).unwrap_err();
            assert!(matches!(
                err,
                WevtManifestError::SizeOutOfBounds { what: "VMAP.size", offset, .. }
                    if offset == map0_off
            ));
        }
    }
}

#[cfg(feature = "wevt_templates")]
mod wevt_templates_research {
    use evtx::wevt_templates::manifest::CrimManifest;
    use evtx::wevt_templates::{
        ResourceIdentifier, extract_temp_templates_from_wevt_blob, extract_wevt_template_resources,
        render_template_definition_to_xml,
    };
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::tempdir;

    #[test]
    fn it_finds_temp_entries_in_a_synthetic_ttbl_blob() {
        // Minimal CRIM + WEVT + TTBL with a single TEMP entry (no BinXML payload).
        //
        // This is structured according to libfwevt's "Windows Event manifest binary format":
        // CRIM header -> provider descriptor -> WEVT header -> provider element descriptor -> TTBL -> TEMP.
        let guid_bytes: [u8; 16] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00,
        ];
        let temp_size: u32 = 40;
        let ttbl_size: u32 = 12 + temp_size;

        // Layout sizes:
        // CRIM header: 16
        // provider descriptor: 20
        // WEVT header: 20 + 8 * 1 descriptor = 28
        let provider_data_off: u32 = 16 + 20;
        let ttbl_off: u32 = provider_data_off + 28;

        let mut ttbl = Vec::with_capacity(ttbl_size as usize);
        ttbl.extend_from_slice(b"TTBL");
        ttbl.extend_from_slice(&ttbl_size.to_le_bytes());
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // template count

        ttbl.extend_from_slice(b"TEMP");
        ttbl.extend_from_slice(&temp_size.to_le_bytes());
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_descriptor_count
        ttbl.extend_from_slice(&0u32.to_le_bytes()); // item_name_count
        ttbl.extend_from_slice(&(ttbl_off + 12 + temp_size).to_le_bytes()); // template_items_offset (end-of-template)
        ttbl.extend_from_slice(&1u32.to_le_bytes()); // event_type (observed as 1 for EventData)
        ttbl.extend_from_slice(&guid_bytes); // template GUID

        let total_size = (ttbl_off as usize) + ttbl.len();
        let mut blob = Vec::with_capacity(total_size);

        // CRIM
        blob.extend_from_slice(b"CRIM");
        blob.extend_from_slice(&(total_size as u32).to_le_bytes()); // size
        blob.extend_from_slice(&3u16.to_le_bytes()); // major
        blob.extend_from_slice(&1u16.to_le_bytes()); // minor
        blob.extend_from_slice(&1u32.to_le_bytes()); // provider_count

        // provider descriptor
        blob.extend_from_slice(&[0u8; 16]); // provider GUID (unused in this test)
        blob.extend_from_slice(&provider_data_off.to_le_bytes());

        // WEVT
        blob.extend_from_slice(b"WEVT");
        blob.extend_from_slice(&(28u32).to_le_bytes()); // size
        blob.extend_from_slice(&0xffffffffu32.to_le_bytes()); // message-table id
        blob.extend_from_slice(&1u32.to_le_bytes()); // provider element desc count
        blob.extend_from_slice(&0u32.to_le_bytes()); // unknown value count
        // provider element descriptor
        blob.extend_from_slice(&ttbl_off.to_le_bytes()); // provider element offset (relative to CRIM)
        blob.extend_from_slice(&0u32.to_le_bytes()); // unknown

        // TTBL
        blob.extend_from_slice(&ttbl);

        let temps = extract_temp_templates_from_wevt_blob(&blob).expect("parse should succeed");
        assert_eq!(temps.len(), 1);
        let t = &temps[0];
        assert_eq!(t.ttbl_offset, ttbl_off);
        assert_eq!(t.temp_offset, ttbl_off + 12);
        assert_eq!(t.temp_size, temp_size);
        assert_eq!(t.header.item_descriptor_count, 0);
        assert_eq!(t.header.item_name_count, 0);
        assert_eq!(t.header.template_items_offset, ttbl_off + 12 + temp_size);
        assert_eq!(t.header.event_type, 1);
    }

    #[test]
    #[ignore]
    fn it_extracts_wevt_template_from_willi_services_exe_sample() {
        // This test is intentionally ignored by default:
        // - it downloads a Windows binary (large, proprietary)
        // - it requires network access / curl
        //
        // Run with:
        //   cargo test --features wevt_templates -- --ignored

        const URL: &str = "https://user-images.githubusercontent.com/156560/84550172-1e987a00-acc7-11ea-8f8e-7e1310b13ec4.gif";

        // Prefer local developer copy if present.
        let local_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("samples_local")
            .join("services.exe.gif");

        let bytes = if local_path.exists() {
            fs::read(&local_path).unwrap()
        } else {
            let d = tempdir().unwrap();
            let p = d.path().join("services.exe.gif");

            let status = Command::new("curl")
                .args(["-L", "-s", "-o"])
                .arg(&p)
                .arg(URL)
                .status()
                .expect("failed to spawn curl");
            assert!(status.success(), "curl failed");

            fs::read(p).unwrap()
        };

        let resources = extract_wevt_template_resources(&bytes).expect("extract should succeed");
        assert!(
            !resources.is_empty(),
            "expected at least one WEVT_TEMPLATE resource"
        );

        // This particular sample has: WEVT_TEMPLATE / 1 / 1033
        let found = resources.iter().find(|r| r.lang_id == 1033);
        assert!(found.is_some(), "expected lang_id=1033 resource");
        let r = found.unwrap();
        assert_eq!(r.resource, ResourceIdentifier::Id(1));
        assert!(r.data.starts_with(b"CRIM|K\0\0"), "expected CRIM header");
        assert!(
            r.data.windows(4).any(|w| w == b"WEVT"),
            "expected embedded WEVT marker"
        );
        assert!(
            r.data.windows(4).any(|w| w == b"TTBL"),
            "expected embedded TTBL marker"
        );
        assert!(
            r.data.windows(4).any(|w| w == b"TEMP"),
            "expected embedded TEMP marker"
        );

        let temps = extract_temp_templates_from_wevt_blob(&r.data).expect("parse should succeed");
        assert_eq!(
            temps.len(),
            46,
            "expected stable template count for Willi sample"
        );

        // Parse the full manifest and ensure every TEMP BinXML fragment parses cleanly with strict
        // NameHash validation (MS-EVEN6) and the current token support.
        let manifest = CrimManifest::parse(&r.data).expect("manifest parse should succeed");
        let _arena = bumpalo::Bump::new();
        let mut parsed_templates = 0usize;
        for provider in &manifest.providers {
            if let Some(ttbl) = provider.wevt.elements.templates.as_ref() {
                for tpl in &ttbl.templates {
                    let _ = render_template_definition_to_xml(tpl, encoding::all::WINDOWS_1252)
                        .expect("BinXML parse/render should succeed");
                    parsed_templates += 1;
                }
            }
        }
        assert!(
            parsed_templates > 0,
            "expected at least one parsed template"
        );
    }
}
