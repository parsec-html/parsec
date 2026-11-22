// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The `parsec-reduce` binary, run as a user would run it.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

fn exe() -> Command {
    Command::new(env!("CARGO_BIN_EXE_parsec-reduce"))
}

fn dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("parsec-reduce-test-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn child(kind: &str, input: &str) -> Output {
    let mut c = exe()
        .args(["--child", kind])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    c.wait_with_output().unwrap()
}

#[test]
fn the_child_parses_and_compares() {
    assert!(child("parse", "<p>fine").status.success());
    assert!(child("differs", "<table><td>same in both").status.success());
}

#[test]
fn a_page_that_doesnt_fail_is_refused() {
    let d = dir("refused");
    let page = d.join("page.html");
    std::fs::write(&page, "<!DOCTYPE html><p>fine</p>").unwrap();
    for check in ["panic", "hang:5000", "differs-from-html5ever"] {
        let out = exe().arg(&page).args(["--check", check]).output().unwrap();
        assert_eq!(out.status.code(), Some(1), "{check}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains("doesn't fail that way"), "{check}: {err}");
    }
    std::fs::remove_dir_all(d).unwrap();
}

#[test]
fn bad_arguments() {
    for args in [&[][..], &["page.html"], &["page.html", "--check", "crash"]] {
        assert_eq!(
            exe().args(args).output().unwrap().status.code(),
            Some(2),
            "{args:?}"
        );
    }
}

/// A seeded bug as a command: it "fails" while the page has a `<table>`
/// open inside an `<li>`.
#[cfg(unix)]
#[test]
fn a_command_check_reduces_a_page() {
    let d = dir("cmd");
    let page = d.join("page.html");
    let mut html = String::from("<!DOCTYPE html><html><body>");
    for i in 0..2000 {
        html.push_str(&format!(
            "<div class=row><a href=/p/{i}>Item {i}</a> <b>{i}.00</b></div>\n"
        ));
        if i == 1234 {
            html.push_str("<ul><li class=x><table><tr><td>t</table></ul>");
        }
    }
    std::fs::write(&page, &html).unwrap();
    let out = d.join("small.html");
    let status = exe()
        .arg(&page)
        .args([
            "--check",
            r#"cmd:! tr -d '\n' < "$1" | grep -q '<li[^<]*<table'"#,
        ])
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "<li<table");
    std::fs::remove_dir_all(d).unwrap();
}
