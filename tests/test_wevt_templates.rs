mod fixtures;

#[cfg(feature = "wevt_templates")]
mod wevt_templates {
    use evtx::wevt_templates::{
        ResourceIdentifier, extract_temp_templates_from_wevt_blob, extract_wevt_template_resources,
    };
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
    fn it_finds_temp_entries_in_a_synthetic_ttbl_blob() {
        // Minimal TTBL with a single TEMP entry (no BinXML payload).
        //
        // TTBL header: sig + size + count
        // TEMP header: sig + size + id1 + id2 + offset + unk + guid
        let guid_bytes: [u8; 16] = [
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00,
        ];
        let temp_size: u32 = 40;
        let ttbl_size: u32 = 12 + temp_size;

        let mut blob = Vec::with_capacity(ttbl_size as usize);
        blob.extend_from_slice(b"TTBL");
        blob.extend_from_slice(&ttbl_size.to_le_bytes());
        blob.extend_from_slice(&1u32.to_le_bytes()); // count

        blob.extend_from_slice(b"TEMP");
        blob.extend_from_slice(&temp_size.to_le_bytes());
        blob.extend_from_slice(&7u32.to_le_bytes()); // id_1
        blob.extend_from_slice(&7u32.to_le_bytes()); // id_2
        blob.extend_from_slice(&0x1234u32.to_le_bytes()); // offset
        blob.extend_from_slice(&0x9u32.to_le_bytes()); // unk
        blob.extend_from_slice(&guid_bytes);

        let temps = extract_temp_templates_from_wevt_blob(&blob);
        assert_eq!(temps.len(), 1);
        let t = &temps[0];
        assert_eq!(t.ttbl_offset, 0);
        assert_eq!(t.temp_offset, 12);
        assert_eq!(t.temp_size, temp_size);
        assert_eq!(t.header.id_1, 7);
        assert_eq!(t.header.id_2, 7);
        assert_eq!(t.header.offset, 0x1234);
        assert_eq!(t.header.unk, 0x9);
    }

    #[test]
    fn cli_extracts_wevt_template_from_minimal_synthetic_pe() {
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

        let temps = extract_temp_templates_from_wevt_blob(&r.data);
        assert_eq!(
            temps.len(),
            46,
            "expected stable template count for Willi sample"
        );
    }
}
