mod fixtures;

#[cfg(feature = "wevt_templates")]
mod wevt_templates {
    use evtx::wevt_templates::{ResourceIdentifier, extract_wevt_template_resources};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::tempdir;

    const MINIMAL_PE: &[u8] = include_bytes!("fixtures/wevt_template_minimal_pe.bin");
    const MINIMAL_RESOURCE_DATA: &[u8] = b"CRIM|K\0\0WEVTTEST";

    #[test]
    fn it_extracts_wevt_template_from_minimal_synthetic_pe() {
        let resources = extract_wevt_template_resources(MINIMAL_PE).expect("extract should succeed");
        assert_eq!(resources.len(), 1);

        let r = &resources[0];
        assert_eq!(r.resource, ResourceIdentifier::Id(1));
        assert_eq!(r.lang_id, 1033);
        assert_eq!(r.data.as_slice(), MINIMAL_RESOURCE_DATA);
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
    }
}
