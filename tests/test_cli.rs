mod fixtures;

use fixtures::*;

use assert_cmd::prelude::*;
use std::fs::File;
use std::io::{Read, Write};
use std::process::Command;
use tempfile::tempdir;

use assert_cmd::cargo::cargo_bin;
#[cfg(not(target_os = "windows"))]
use rexpect::spawn;

#[test]
fn it_respects_directory_output() {
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let sample = regular_sample();

    let mut cmd = Command::cargo_bin("evtx_dump").expect("failed to find binary");
    cmd.args(&["-f", &f.to_string_lossy(), sample.to_str().unwrap()]);

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

// It should behave the same on windows, but interactive testing relies on unix pty internals.
#[test]
#[cfg(not(target_os = "windows"))]
fn test_it_confirms_before_overwriting_a_file() {
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let mut file = File::create(&f).unwrap();
    file.write_all(b"I'm a file!").unwrap();

    let sample = regular_sample();

    let cmd_string = format!(
        "{bin} -f {output_file} {sample}",
        bin = cargo_bin("evtx_dump").display(),
        output_file = f.to_string_lossy(),
        sample = sample.to_str().unwrap()
    );
    let mut p = spawn(&cmd_string, Some(3000)).unwrap();
    p.exp_regex(r#"Are you sure you want to override.*"#)
        .unwrap();
    p.send_line("y").unwrap();
    p.exp_eof().unwrap();

    let mut expected = vec![];

    File::open(&f).unwrap().read_to_end(&mut expected).unwrap();
    assert!(
        !expected.len() > 100,
        "Expected output to be printed to file"
    )
}

#[test]
#[cfg(not(target_os = "windows"))]
fn test_it_confirms_before_overwriting_a_file_and_quits() {
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let mut file = File::create(&f).unwrap();
    file.write_all(b"I'm a file!").unwrap();

    let sample = regular_sample();

    let cmd_string = format!(
        "{bin} -f {output_file} {sample}",
        bin = cargo_bin("evtx_dump").display(),
        output_file = f.to_string_lossy(),
        sample = sample.to_str().unwrap()
    );
    let mut p = spawn(&cmd_string, Some(3000)).unwrap();
    p.exp_regex(r#"Are you sure you want to override.*"#)
        .unwrap();
    p.send_line("n").unwrap();
    p.exp_eof().unwrap();

    let mut expected = vec![];

    File::open(&f).unwrap().read_to_end(&mut expected).unwrap();
    assert!(
        !expected.len() > 100,
        "Expected output to be printed to file"
    )
}

#[test]
fn test_it_refuses_to_overwrite_directory() {
    let d = tempdir().unwrap();

    let sample = regular_sample();
    let mut cmd = Command::cargo_bin("evtx_dump").expect("failed to find binary");
    cmd.args(&["-f", &d.path().to_string_lossy(), sample.to_str().unwrap()]);

    cmd.assert().failure().code(1);
}

#[test]
fn test_it_overwrites_file_anyways_if_passed_flag() {
    let d = tempdir().unwrap();
    let f = d.as_ref().join("test.out");

    let mut file = File::create(&f).unwrap();
    file.write_all(b"I'm a file!").unwrap();

    let sample = regular_sample();
    let mut cmd = Command::cargo_bin("evtx_dump").expect("failed to find binary");
    cmd.args(&[
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
