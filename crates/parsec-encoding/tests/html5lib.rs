// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The html5lib-tests `encoding/` files: each page's bytes and the
//! encoding a browser picks for it with no transport-layer hint and a
//! windows-1252 fallback. A case passes when `parse_bytes` decodes with
//! that encoding. Seven cases put their `<meta>` past the first 1,024
//! bytes, beyond the prescan, so only the parser's own change of encoding
//! finds them; the sniffing alone is counted too, for the record.

use std::path::Path;

use parsec_encoding::{Encoding, Options, parse_bytes, sniff};

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

const DIR: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../parsec/tests/html5lib-tests/encoding"
);

/// `scripted/` holds pages whose `<meta>` comes from `document.write`, so
/// they need a script engine. (`chardet/` isn't a test file: it's a sample
/// for guessing an encoding from text, which the spec leaves optional and
/// Parsec doesn't do.)
const SCRIPTED: &str = "scripted/tests1.dat";

/// (file, input, expected encoding label) for every case in `file`.
fn cases(file: &str) -> Vec<(String, Vec<u8>, String)> {
    let bytes = std::fs::read(Path::new(DIR).join(file)).unwrap();
    let mut out = Vec::new();
    let mut rest = &bytes[..];
    while let Some(start) = find(rest, b"#data\n") {
        rest = &rest[start + b"#data\n".len()..];
        let end = find(rest, b"\n#encoding\n").unwrap();
        let data = rest[..end].to_vec();
        rest = &rest[end + b"\n#encoding\n".len()..];
        let line = rest.split(|&c| c == b'\n').next().unwrap();
        out.push((
            file.to_string(),
            data,
            String::from_utf8(line.to_vec()).unwrap(),
        ));
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[test]
fn html5lib_encoding() {
    let all: Vec<_> = ["tests1.dat", "tests2.dat", "test-yahoo-jp.dat"]
        .iter()
        .flat_map(|f| cases(f))
        .collect();
    let skipped = cases(SCRIPTED).len();
    let mut failures = Vec::new();
    let mut by_prescan = 0;
    for (i, (file, data, want)) in all.iter().enumerate() {
        let want = Encoding::for_label(want.as_bytes()).unwrap();
        by_prescan += usize::from(sniff(data, None, encoding_rs::WINDOWS_1252).0 == want);
        let parsed = parse_bytes(data, Options::default()).unwrap().encoding;
        if parsed != want {
            failures.push(format!(
                "{file} case {i}: want {}, got {}",
                want.name(),
                parsed.name()
            ));
        }
    }
    let passed = all.len() - failures.len();
    eprintln!(
        "html5lib encoding: {passed} passed, {} failed, {skipped} skipped (scripted); \
         {by_prescan} of {} decided by the sniffing alone",
        failures.len(),
        all.len()
    );
    summary::write(&summary::Counts {
        suite: "html5lib-encoding",
        passed,
        failed: failures.len(),
        skipped,
        skipped_because: "the scripted case writes its <meta> with document.write",
        data: Path::new(DIR),
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
