mod fixtures;

#[cfg(feature = "wevt_templates")]
mod wevt_templates {
    use evtx::wevt_templates::manifest::CrimManifest;
    use evtx::wevt_templates::{ResourceIdentifier, extract_wevt_template_resources};
    use evtx::wevt_templates::render_template_definition_to_xml;
    use super::fixtures::CLI_TEST_LOCK;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    const MINIMAL_PE: &[u8] = include_bytes!("fixtures/wevt_template_minimal_pe.bin");
    const MINIMAL_RESOURCE_DATA: &[u8] = b"CRIM|K\0\0WEVTTEST";

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
        let out_dir = d.path().join("out");
        std::fs::create_dir_all(&out_dir).unwrap();

        let pe_path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("wevt_template_minimal_pe.bin");

        let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
        cmd.args([
            "extract-wevt-templates",
            "--input",
            pe_path.to_str().unwrap(),
            "--output-dir",
            out_dir.to_str().unwrap(),
            "--overwrite",
        ]);

        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "stdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        // Expect a single JSONL line on stdout.
        let stdout = String::from_utf8(out.stdout).unwrap();
        let line = stdout.lines().next().expect("expected one JSONL line");
        let v: serde_json::Value = serde_json::from_str(line).unwrap();

        assert_eq!(v["resource"], 1);
        assert_eq!(v["lang_id"], 1033);

        let out_path = PathBuf::from(v["output_path"].as_str().unwrap());
        assert!(out_path.starts_with(&out_dir));

        let extracted = fs::read(&out_path).unwrap();
        assert_eq!(extracted.as_slice(), MINIMAL_RESOURCE_DATA);
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
        let wevt_size: u32 = 20 + 8 * 1;
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

        let temp_size: u32 = 40 + (binxml.len() as u32) + 20 * descriptor_count + item_name_struct_size;
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
        let ttbl = provider.wevt.elements.templates.as_ref().expect("TTBL present");
        let tpl = &ttbl.templates[0];

        assert_eq!(tpl.items.len(), 1);
        assert_eq!(tpl.items[0].name.as_deref(), Some(item_name));

        let xml = render_template_definition_to_xml(tpl, encoding::all::WINDOWS_1252)
            .expect("render should succeed");
        assert!(
            xml.contains("{sub:0:Foo}"),
            "expected substitution placeholder to include item name, got: {xml}"
        );
    }
}

#[cfg(feature = "wevt_templates")]
mod wevt_templates_research {
    use evtx::wevt_templates::{
        ResourceIdentifier, extract_temp_templates_from_wevt_blob, extract_wevt_template_resources,
    };
    use evtx::wevt_templates::manifest::CrimManifest;
    use evtx::wevt_templates::parse_wevt_binxml_fragment;
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
        let mut parsed_templates = 0usize;
        for provider in &manifest.providers {
            if let Some(ttbl) = provider.wevt.elements.templates.as_ref() {
                for tpl in &ttbl.templates {
                    let _ = parse_wevt_binxml_fragment(tpl.binxml, encoding::all::WINDOWS_1252)
                        .expect("BinXML parse should succeed");
                    parsed_templates += 1;
                }
            }
        }
        assert!(parsed_templates > 0, "expected at least one parsed template");
    }
}
