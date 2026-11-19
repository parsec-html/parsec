// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Unicode's NormalizationTest.txt, pinned in `tests/unicode`, against
//! `normalize::nfc` and `nfd`. Each line has five columns, c1 to c5, and
//! the file states what NFC and NFD must give for each; every code point
//! that no line of part 1 lists must come back unchanged.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::HashSet;
use std::path::Path;

use parsec_url::normalize::{nfc, nfd};

fn column(field: &str) -> String {
    field
        .split(' ')
        .map(|cp| char::from_u32(u32::from_str_radix(cp, 16).unwrap()).unwrap())
        .collect()
}

#[test]
fn normalization_test() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/unicode");
    let text = std::fs::read_to_string(dir.join("NormalizationTest.txt")).unwrap();
    let (mut passed, mut failed) = (0, Vec::new());
    let mut part1 = HashSet::new();
    let mut part = "";
    for line in text.lines() {
        let line = line.split('#').next().unwrap().trim();
        if line.is_empty() {
            continue;
        }
        if let Some(p) = line.strip_prefix('@') {
            part = p.split_whitespace().next().unwrap_or("");
            continue;
        }
        let c: Vec<String> = line.split(';').take(5).map(column).collect();
        if part == "Part1" {
            part1.insert(c[0].chars().next().unwrap());
        }
        let checks = [
            (nfc(&c[0]) == c[1], "NFC(c1) == c2"),
            (nfc(&c[1]) == c[1], "NFC(c2) == c2"),
            (nfc(&c[2]) == c[1], "NFC(c3) == c2"),
            (nfc(&c[3]) == c[3], "NFC(c4) == c4"),
            (nfc(&c[4]) == c[3], "NFC(c5) == c4"),
            (nfd(&c[0]) == c[2], "NFD(c1) == c3"),
            (nfd(&c[1]) == c[2], "NFD(c2) == c3"),
            (nfd(&c[2]) == c[2], "NFD(c3) == c3"),
            (nfd(&c[3]) == c[4], "NFD(c4) == c5"),
            (nfd(&c[4]) == c[4], "NFD(c5) == c5"),
        ];
        match checks.iter().find(|(ok, _)| !ok) {
            None => passed += 1,
            Some((_, what)) => failed.push(format!("{line}: {what}")),
        }
    }
    // Part 1's invariant: anything it doesn't list normalizes to itself.
    let mut unlisted_ok = true;
    for c in (0..=0x10FFFF).filter_map(char::from_u32) {
        if !part1.contains(&c) {
            let s = c.to_string();
            if nfc(&s) != s || nfd(&s) != s {
                failed.push(format!("{:04X} isn't listed but changes", c as u32));
                unlisted_ok = false;
            }
        }
    }
    if unlisted_ok {
        passed += 1;
    }
    eprintln!(
        "NormalizationTest.txt: {passed} passed, {} failed, 0 skipped",
        failed.len()
    );
    summary::write(&summary::Counts {
        suite: "unicode-normalization",
        passed,
        failed: failed.len(),
        skipped: 0,
        skipped_because: "none",
        data: &dir,
    });
    assert!(
        failed.is_empty(),
        "{} failures:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
