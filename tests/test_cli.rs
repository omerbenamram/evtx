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

fn evtx_dump_info(path: &std::path::Path) -> String {
    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["info", "-i"])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn info_shows_the_header_and_each_chunk() {
    insta::assert_snapshot!(evtx_dump_info(&regular_sample()));
}

#[test]
fn info_shows_what_is_damaged() {
    let mut evtx = std::fs::read(samples_dir().join("sample_with_a_bad_chunk_magic.evtx")).unwrap();
    // The next record ID, which the header's CRC32 covers.
    evtx[24] ^= 1;
    let d = tempdir().unwrap();
    let path = d.path().join("damaged.evtx");
    std::fs::write(&path, &evtx).unwrap();

    let info = evtx_dump_info(&path);
    for line in [
        "  CRC32            BAD (stored 0xab5d45a8, computed 0x",
        "0      0x00001000  46728-46857     46728-46857  DIRTY  ok            BAD\n",
        "6      0x00061000  unreadable: Failed to parse chunk header: Invalid EVTX chunk header magic",
        "\n16 chunks: 4 with records, 10 empty, 2 unreadable. 467 records by the chunk headers. 3 chunks fail a CRC32 check.\n",
    ] {
        assert!(info.contains(line), "{line:?} in\n{info}");
    }

    // Neither its header nor its chunk stores CRC32s.
    let info = evtx_dump_info(&samples_dir().join("Application_no_crc32.evtx"));
    for line in [
        "  CRC32            none (NO_CRC32)\n",
        "0      0x00001000  1-17            426-442     DIRTY | NO_CRC32  none          none\n",
    ] {
        assert!(info.contains(line), "{line:?} in\n{info}");
    }
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

/// A reader that stops early, as `head` does, closes the pipe: `evtx_dump` and its subcommands
/// stop there without an error. (The output is megabytes, past what a pipe buffers.)
#[cfg(unix)]
#[test]
fn it_stops_quietly_when_the_reader_closes_the_pipe() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    for args in [
        &["-o", "jsonl", "-t", "1"][..],
        &["dump-template-instances", "-i"][..],
    ] {
        let mut child = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
            .args(args)
            .arg(regular_sample())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert!(line.starts_with('{'), "{line}");
        drop(stdout);
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr), "", "{args:?}");
    }
}

/// Output still buffered at the end is written before `evtx_dump` exits, and a failure to
/// write it is an error, not lost.
#[cfg(target_os = "linux")]
#[test]
fn it_reports_a_failure_to_write_the_last_output() {
    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["-o", "jsonl", "--events=1"])
        .arg(regular_sample())
        .stdout(File::create("/dev/full").unwrap())
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No space left on device"),
        "{output:?}"
    );
}

#[test]
fn it_carves_chunks_from_raw_data() {
    let evtx = std::fs::read(regular_sample()).unwrap();
    // The log's chunks without its file header, between runs of noise.
    let mut image: Vec<u8> = (0..50_001u32).map(|i| (i * 13 + 5) as u8).collect();
    image.extend_from_slice(&evtx[4096..]);
    image.extend_from_slice(&[0x5A; 777]);
    let d = tempdir().unwrap();
    let path = d.path().join("image.bin");
    std::fs::write(&path, &image).unwrap();

    let carved = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["--carve", "-o", "jsonl"])
        .arg(&path)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&carved.stderr);
    assert!(carved.status.success(), "{stderr}");
    assert!(stderr.contains("Carved 26 chunks from "), "{stderr}");

    let intact = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["-o", "jsonl"])
        .arg(regular_sample())
        .output()
        .unwrap();
    assert_eq!(carved.stdout, intact.stdout);
}

/// The JSON lines that `evtx_dump <args>` prints.
fn json_lines(args: &[&str]) -> Vec<serde_json::Value> {
    let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(args)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    let lines = output.stdout.split(|&b| b == b'\n');
    lines
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).unwrap())
        .collect()
}

#[test]
fn template_instance_dump_lists_the_instances_inside_the_outer_one() {
    let sample = regular_sample();
    let sample = sample.to_str().unwrap();
    // Record 3 is event 4902: `System`'s instance, and inside it `EventData`'s.
    let args = ["dump-template-instances", "-i", sample, "--record-id", "3"];
    let lines = json_lines(&args);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["template_instance_index"], 0);
    assert_eq!(lines[0]["substitutions"][3], "4902");
    assert_eq!(lines[1]["template_instance_index"], 1);
    assert_eq!(
        lines[1]["substitutions"],
        serde_json::json!(["0", "0x35cb0"])
    );

    let only = json_lines(&[&args[..], &["--template-instance-index", "1"]].concat());
    assert_eq!(only, lines[1..]);
}

#[test]
fn applying_a_cached_template_takes_the_values_of_the_records_own_instance() {
    let d = tempdir().unwrap();
    let cache = d.as_ref().join("templates.wevtcache");
    let cache = cache.to_str().unwrap();
    let dll = samples_dir().join("dlls").join("adtschema.dll");
    Command::new(assert_cmd::cargo_bin!("evtx_dump"))
        .args(["extract-wevt-templates", "--output", cache, "--input"])
        .arg(dll)
        .assert()
        .success();

    // Event 4902 of Microsoft-Windows-Security-Auditing, with the values of record 3. It used
    // to get the values of the instance that fills in `System`, and print 0 for both.
    let sample = regular_sample();
    let apply = |extra: &[&str]| {
        let output = Command::new(assert_cmd::cargo_bin!("evtx_dump"))
            .args(["apply-wevt-cache", "--cache", cache])
            .args(["--provider-guid", "54849625-5478-4994-a5ba-3e3b0328c30d"])
            .args(["--event-id", "4902", "--version", "0", "--record-id", "3"])
            .arg("--evtx")
            .arg(&sample)
            .args(extra)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        String::from_utf8(output.stdout).unwrap()
    };
    let xml = apply(&[]);
    assert!(xml.contains(r#"<Data Name="PuaCount">0</Data>"#), "{xml}");
    assert!(
        xml.contains(r#"<Data Name="PuaPolicyId">0x35cb0</Data>"#),
        "{xml}"
    );
    assert_eq!(apply(&["--template-instance-index", "1"]), xml);
    assert_ne!(apply(&["--template-instance-index", "0"]), xml);

#[test]
fn carving_fails_when_there_is_nothing_to_carve() {
    let d = tempdir().unwrap();
    let no_chunks = d.as_ref().join("zeros.bin");
    std::fs::write(&no_chunks, vec![0u8; 100_000]).unwrap();

    // On Linux a directory opens like a file, is as long as a file can be, and fails every
    // read: this used to never end. Elsewhere it fails in its own way.
    for (input, message) in [
        (d.as_ref(), ""),
        (no_chunks.as_path(), "No EVTX chunks were found in"),
    ] {
        let assert = assert_cmd::Command::new(assert_cmd::cargo_bin!("evtx_dump"))
            .arg("--carve")
            .arg(input)
            .timeout(std::time::Duration::from_secs(60))
            .assert()
            .failure();
        let stderr = String::from_utf8_lossy(&assert.get_output().stderr);
        assert!(stderr.contains(message), "{stderr}");
    }
}

/// Runs `evtx_dump -f <output> --no-confirm-overwrite <args>`, which must fail, and returns
/// what it printed to stderr.
fn failing_dump_to(output: &std::path::Path, args: &[&str]) -> String {
    let mut cmd = Command::new(assert_cmd::cargo_bin!("evtx_dump"));
    cmd.args(["-f", output.to_str().unwrap(), "--no-confirm-overwrite"]);
    cmd.args(args);
    let assert = cmd.assert().failure();
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

#[test]
fn a_run_that_cannot_start_leaves_the_output_file_as_it_was() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();
    let output = d.as_ref().join("kept.xml");
    let missing = d.as_ref().join("missing");
    let missing = missing.to_str().unwrap();
    let not_a_log = d.as_ref().join("not-a-log.evtx");
    std::fs::write(&not_a_log, b"not an EVTX file").unwrap();
    let sample = regular_sample();
    let sample = sample.to_str().unwrap();

    for args in [
        // No input, an input that isn't a log, and side files that can't be loaded.
        &[missing][..],
        &[not_a_log.to_str().unwrap()],
        &["--wevt-cache", missing, sample],
        &["--mta", missing, sample],
    ] {
        std::fs::write(&output, b"precious").unwrap();
        failing_dump_to(&output, args);
        assert_eq!(std::fs::read(&output).unwrap(), b"precious", "{args:?}");
    }
}

#[test]
fn a_file_the_run_reads_is_refused_as_the_output_file() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();
    let log = d.as_ref().join("log.evtx");
    std::fs::copy(regular_sample(), &log).unwrap();
    let original = std::fs::read(&log).unwrap();
    let log_arg = log.to_str().unwrap();

    // The input under its own name, another spelling of it, and a hard link to it.
    let other_spelling = d.as_ref().join(".").join("log.evtx");
    let link = d.as_ref().join("link.evtx");
    std::fs::hard_link(&log, &link).unwrap();
    for output in [&log, &other_spelling, &link] {
        let stderr = failing_dump_to(output, &[log_arg]);
        assert!(stderr.contains("which this run reads"), "{stderr}");
        assert!(std::fs::read(&log).unwrap() == original);
    }

    // The side files are refused before they are read, so they needn't be valid.
    let side = d.as_ref().join("side-file");
    for flag in ["--wevt-cache", "--mta"] {
        std::fs::write(&side, b"precious").unwrap();
        let stderr = failing_dump_to(&side, &[flag, side.to_str().unwrap(), log_arg]);
        assert!(stderr.contains("which this run reads"), "{stderr}");
        assert_eq!(std::fs::read(&side).unwrap(), b"precious");
    }
}

#[test]
fn a_bad_output_path_fails_before_the_input_is_read() {
    let _guard = CLI_TEST_LOCK.lock().unwrap();
    let d = tempdir().unwrap();
    // The input doesn't exist, and the complaint is about the output.
    let missing = d.as_ref().join("missing.evtx");
    let stderr = failing_dump_to(d.as_ref(), &[missing.to_str().unwrap()]);
    assert!(stderr.contains("There is a directory at"), "{stderr}");
}
