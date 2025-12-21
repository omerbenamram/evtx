mod fixtures;

#[cfg(feature = "wevt_templates")]
mod wevt_templates {
    use evtx::wevt_templates::{ResourceIdentifier, extract_wevt_template_resources};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    fn write_u16(buf: &mut [u8], offset: usize, value: u16) {
        buf[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(buf: &mut [u8], offset: usize, value: u32) {
        buf[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    /// Build a tiny synthetic PE32+ with a single `.rsrc` section containing a `WEVT_TEMPLATE`
    /// resource `id=1` with `lang_id=1033`, whose data bytes are `resource_data`.
    ///
    /// This is specifically designed to be parsed by our minimal PE/resource reader in
    /// `src/wevt_templates.rs` (and avoids shipping Windows binaries in the repo).
    fn build_minimal_pe_with_wevt_template(resource_data: &[u8]) -> Vec<u8> {
        // Layout choices:
        // - PE32+ (0x20b)
        // - 1 section: .rsrc
        // - .rsrc VA: 0x1000, raw ptr: 0x400
        // - resource directory size: 0x200 bytes (enough for names + directory tables)
        // - resource data placed at .rsrc+0x100
        const E_LFANEW: usize = 0x80;
        const COFF_OFFSET: usize = E_LFANEW + 4;
        const OPTIONAL_OFFSET: usize = COFF_OFFSET + 20;
        const OPTIONAL_SIZE: usize = 0xF0;
        const SECTION_HEADERS_OFFSET: usize = OPTIONAL_OFFSET + OPTIONAL_SIZE;

        const RSRC_VA: u32 = 0x1000;
        const RSRC_RAW_PTR: usize = 0x400;
        const RSRC_RAW_SIZE: usize = 0x600;
        const RSRC_DIR_SIZE: u32 = 0x200;

        const RSRC_DIR_OFF: usize = 0x0;
        const WEVT_DIR_OFF: usize = 0x20;
        const ID1_DIR_OFF: usize = 0x40;
        const DATA_ENTRY_OFF: usize = 0x60;
        const NAME_OFF: usize = 0x80;
        const DATA_OFF: usize = 0x100;

        let file_size = RSRC_RAW_PTR + RSRC_RAW_SIZE;
        let mut pe = vec![0u8; file_size];

        // DOS header
        pe[0..2].copy_from_slice(b"MZ");
        write_u32(&mut pe, 0x3c, E_LFANEW as u32);

        // PE signature
        pe[E_LFANEW..E_LFANEW + 4].copy_from_slice(b"PE\0\0");

        // COFF header
        write_u16(&mut pe, COFF_OFFSET + 0, 0x8664); // Machine: AMD64
        write_u16(&mut pe, COFF_OFFSET + 2, 1); // NumberOfSections
        write_u32(&mut pe, COFF_OFFSET + 16, OPTIONAL_SIZE as u32); // SizeOfOptionalHeader

        // Optional header (PE32+)
        write_u16(&mut pe, OPTIONAL_OFFSET + 0, 0x20b);
        // number_of_rva_and_sizes @ +108
        write_u32(&mut pe, OPTIONAL_OFFSET + 108, 16);
        // resource data directory entry (index 2) @ +112 + 2*8
        write_u32(&mut pe, OPTIONAL_OFFSET + 112 + 16, RSRC_VA);
        write_u32(&mut pe, OPTIONAL_OFFSET + 112 + 16 + 4, RSRC_DIR_SIZE);

        // Section header: .rsrc
        let sh = SECTION_HEADERS_OFFSET;
        pe[sh..sh + 8].copy_from_slice(b".rsrc\0\0\0");
        write_u32(&mut pe, sh + 8, 0x400); // virtual size
        write_u32(&mut pe, sh + 12, RSRC_VA); // virtual address
        write_u32(&mut pe, sh + 16, RSRC_RAW_SIZE as u32); // raw size
        write_u32(&mut pe, sh + 20, RSRC_RAW_PTR as u32); // raw ptr

        // Now build `.rsrc` contents.
        let rsrc = &mut pe[RSRC_RAW_PTR..RSRC_RAW_PTR + RSRC_RAW_SIZE];

        // Root directory header: 1 named entry ("WEVT_TEMPLATE"), 0 id entries.
        write_u16(rsrc, RSRC_DIR_OFF + 12, 1);
        write_u16(rsrc, RSRC_DIR_OFF + 14, 0);
        // Root entry
        write_u32(rsrc, RSRC_DIR_OFF + 16, 0x8000_0000 | (NAME_OFF as u32));
        write_u32(rsrc, RSRC_DIR_OFF + 20, 0x8000_0000 | (WEVT_DIR_OFF as u32));

        // "WEVT_TEMPLATE" name at NAME_OFF
        let name = "WEVT_TEMPLATE";
        write_u16(rsrc, NAME_OFF, name.encode_utf16().count() as u16);
        let mut woff = NAME_OFF + 2;
        for c in name.encode_utf16() {
            write_u16(rsrc, woff, c);
            woff += 2;
        }

        // WEVT dir: 0 named, 1 id entry => id=1
        write_u16(rsrc, WEVT_DIR_OFF + 12, 0);
        write_u16(rsrc, WEVT_DIR_OFF + 14, 1);
        write_u32(rsrc, WEVT_DIR_OFF + 16, 1);
        write_u32(rsrc, WEVT_DIR_OFF + 20, 0x8000_0000 | (ID1_DIR_OFF as u32));

        // id=1 dir: 0 named, 1 id entry => lang=1033 => data entry
        write_u16(rsrc, ID1_DIR_OFF + 12, 0);
        write_u16(rsrc, ID1_DIR_OFF + 14, 1);
        write_u32(rsrc, ID1_DIR_OFF + 16, 1033);
        write_u32(rsrc, ID1_DIR_OFF + 20, DATA_ENTRY_OFF as u32);

        // data entry
        write_u32(rsrc, DATA_ENTRY_OFF + 0, RSRC_VA + DATA_OFF as u32);
        write_u32(rsrc, DATA_ENTRY_OFF + 4, resource_data.len() as u32);

        // actual data bytes
        rsrc[DATA_OFF..DATA_OFF + resource_data.len()].copy_from_slice(resource_data);

        pe
    }

    #[test]
    fn it_extracts_wevt_template_from_minimal_synthetic_pe() {
        let data = b"CRIM|K\0\0WEVTTEST";
        let pe = build_minimal_pe_with_wevt_template(data);

        let resources = extract_wevt_template_resources(&pe).expect("extract should succeed");
        assert_eq!(resources.len(), 1);

        let r = &resources[0];
        assert_eq!(r.resource, ResourceIdentifier::Id(1));
        assert_eq!(r.lang_id, 1033);
        assert_eq!(r.data.as_slice(), data);
    }

    #[test]
    fn cli_extracts_wevt_template_from_minimal_synthetic_pe() {
        let d = tempdir().unwrap();
        let pe_path = d.path().join("test_input.bin");
        let out_dir = d.path().join("out");
        fs::create_dir_all(&out_dir).unwrap();

        let data = b"CRIM|K\0\0WEVTTEST";
        let pe = build_minimal_pe_with_wevt_template(data);
        fs::write(&pe_path, pe).unwrap();

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
        assert!(out.status.success(), "stdout: {}\nstderr: {}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));

        // Expect a single JSONL line on stdout.
        let stdout = String::from_utf8(out.stdout).unwrap();
        let line = stdout.lines().next().expect("expected one JSONL line");
        let v: serde_json::Value = serde_json::from_str(line).unwrap();

        assert_eq!(v["resource"], 1);
        assert_eq!(v["lang_id"], 1033);

        let out_path = PathBuf::from(v["output_path"].as_str().unwrap());
        assert!(out_path.starts_with(&out_dir));

        let extracted = fs::read(&out_path).unwrap();
        assert_eq!(extracted.as_slice(), data);
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
        assert!(
            r.data.starts_with(b"CRIM|K\0\0"),
            "expected CRIM header"
        );
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
    }
}


