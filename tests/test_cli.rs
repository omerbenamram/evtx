mod fixtures;

use fixtures::*;

use assert_cmd::prelude::*;
use evtx::EvtxParser;
use std::fs::File;
use std::io::{Read, Write};
use std::process::Command;
use tempfile::tempdir;

#[test]
fn template_instance_dump_formats_nested_binxml_without_panicking() {
    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["dump-template-instances", "-i"])
        .arg(regular_sample())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!output.stdout.is_empty());
    for line in output
        .stdout
        .split(|&b| b == b'\n')
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = serde_json::from_slice(line).unwrap();
        assert!(value["substitutions"].is_array());
    }
}

#[test]
fn it_warns_about_a_corrupt_file_header_and_reads_the_file() {
    let mut evtx = std::fs::read(regular_sample()).unwrap();
    // The next record ID, which the header's CRC32 covers.
    evtx[24] ^= 1;
    let d = tempdir().unwrap();
    let path = d.path().join("corrupt-header.evtx");
    std::fs::write(&path, &evtx).unwrap();

    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["-o", "jsonl"])
        .arg(&path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr
            .contains("warning: the file header's CRC32 is invalid (stored 0x3f6e33d5, computed "),
        "{stderr}"
    );
    let intact = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["-o", "jsonl"])
        .arg(regular_sample())
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&intact.stderr).contains("CRC32"));
    assert_eq!(output.stdout, intact.stdout);
}

#[test]
fn it_respects_directory_output() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let sample = regular_sample();

    let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd.args(["-f", &f.to_string_lossy(), sample.to_str().unwrap()]);

    assert!(
        cmd.output().unwrap().stdout.is_empty(),
        "Expected output to be printed to file, but was printed to stdout"
    );

    let mut expected = vec![];

    File::open(&f).unwrap().read_to_end(&mut expected).unwrap();
    assert!(
        !expected.is_empty(),
        "Expected output to be printed to file"
    )
}

#[test]
fn test_it_refuses_to_overwrite_directory() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();

    let sample = regular_sample();
    let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd.args(["-f", &d.path().to_string_lossy(), sample.to_str().unwrap()]);

    cmd.assert().failure().code(1);
}

#[test]
fn test_it_overwrites_file_anyways_if_passed_flag() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let mut file = File::create(&f).unwrap();
    file.write_all(b"I'm a file!").unwrap();

    let sample = regular_sample();
    let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd.args([
        "-f",
        &f.to_string_lossy(),
        "--no-confirm-overwrite",
        sample.to_str().unwrap(),
    ]);

    cmd.assert().success();

    let mut expected = vec![];

    File::open(&f).unwrap().read_to_end(&mut expected).unwrap();
    assert!(
        !expected.is_empty(),
        "Expected output to be printed to file"
    )
}

#[test]
fn it_supports_stdin_input_with_dash() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let sample = regular_sample();

    // Pick a single record id to keep CLI output small/deterministic.
    let record_id = {
        let mut parser = EvtxParser::from_path(&sample).unwrap();
        parser
            .records()
            .filter_map(|r| r.ok())
            .next()
            .expect("sample should contain at least one parsable record")
            .event_record_id
            .to_string()
    };

    let mut cmd_file = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd_file.args([
        "-o",
        "jsonl",
        "--events",
        &record_id,
        sample.to_str().unwrap(),
    ]);
    let out_file = cmd_file.output().unwrap();
    assert!(
        out_file.status.success(),
        "expected file-input run to succeed"
    );
    assert!(
        !out_file.stdout.is_empty(),
        "expected file-input run to produce output"
    );

    let stdin_file = File::open(&sample).unwrap();
    let mut cmd_stdin = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd_stdin.args(["-o", "jsonl", "--events", &record_id, "-"]);
    cmd_stdin.stdin(stdin_file);
    let out_stdin = cmd_stdin.output().unwrap();
    assert!(
        out_stdin.status.success(),
        "expected stdin-input run to succeed"
    );
    assert_eq!(
        out_stdin.stdout, out_file.stdout,
        "stdin and file input should produce identical output for the selected record"
    );
}
