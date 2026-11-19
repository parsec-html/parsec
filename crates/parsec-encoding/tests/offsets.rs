// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `Offsets` on real pages: every element's start tag and every
//! attribute in the html5lib-tests `encoding/` inputs (as they are, behind
//! a UTF-8 byte order mark and widened to UTF-16 both ways) and the
//! web-platform-tests legacy multi-byte pages, with spans mapped back to
//! the bytes, decode to what the spans cover in the decoded page.

use std::path::{Path, PathBuf};

use parsec_encoding::{Offsets, Options, parse_bytes};

const HTML5LIB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../parsec/tests/html5lib-tests/encoding"
);
const WPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wpt");

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn inputs() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for file in ["tests1.dat", "tests2.dat", "test-yahoo-jp.dat"] {
        let bytes = std::fs::read(Path::new(HTML5LIB).join(file)).unwrap();
        let mut rest = &bytes[..];
        while let Some(start) = find(rest, b"#data\n") {
            rest = &rest[start + b"#data\n".len()..];
            let end = find(rest, b"\n#encoding\n").unwrap();
            let data = &rest[..end];
            out.push(data.to_vec());
            out.push([&b"\xEF\xBB\xBF"[..], data].concat());
            let le: Vec<u8> = data.iter().flat_map(|&b| [b, 0]).collect();
            out.push([&b"\xFF\xFE"[..], &le].concat());
            let be: Vec<u8> = data.iter().flat_map(|&b| [0, b]).collect();
            out.push([&b"\xFE\xFF"[..], &be].concat());
            rest = &rest[end..];
        }
    }
    let mut dirs = vec![PathBuf::from(WPT)];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "html") {
                out.push(std::fs::read(path).unwrap());
            }
        }
    }
    out
}

#[test]
fn spans_map_back_to_the_bytes() {
    let options = Options {
        html: parsec::Options {
            spans: true,
            ..parsec::Options::default()
        },
        ..Options::default()
    };
    let (mut pages, mut checked) = (0, 0);
    for bytes in inputs() {
        let parsed = parse_bytes(&bytes, options).unwrap();
        let (doc, encoding) = (&parsed.document, parsed.encoding);
        let (text, _) = encoding.decode_with_bom_removal(&bytes);
        let offsets = Offsets::new(&bytes, encoding);
        let mut spans = Vec::new();
        for n in 0..doc.len() as u32 {
            spans.extend(doc.span(n).filter(|_| doc.lower_name(n).is_some()));
            spans.extend((0..doc.attrs(n).count()).filter_map(|i| doc.attr_span(n, i)));
        }
        for span in spans {
            let (start, end) = (offsets.to_source(span.start), offsets.to_source(span.end));
            let (from_bytes, _) = encoding.decode_without_bom_handling(&bytes[start..end]);
            assert_eq!(
                from_bytes,
                &text[span.clone()],
                "{} page {pages}: {span:?} is {start}..{end}",
                encoding.name()
            );
            assert_eq!(offsets.to_decoded(start), span.start);
            checked += 1;
        }
        pages += 1;
    }
    eprintln!("offsets: {checked} spans on {pages} pages map back to their bytes");
    assert!(checked > 1000);
}
