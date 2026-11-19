// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Bytes fed in pieces give what the whole gives: every html5lib-tests
//! `encoding/` input and the web-platform-tests legacy multi-byte pages,
//! cut in two and into many pieces, through `ByteParser` against
//! `parse_bytes` on the same bytes: the serialized tree, the encoding and
//! the confidence.

use std::path::Path;

use parsec_encoding::{ByteParser, Options, Parsed, parse_bytes};

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

const HTML5LIB: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../parsec/tests/html5lib-tests/encoding"
);
const WPT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wpt");

fn outcome(p: Parsed) -> (String, &'static str, parsec_encoding::Confidence) {
    (
        p.document.outer_html(p.document.root()),
        p.encoding.name(),
        p.confidence,
    )
}

fn pieces<'a>(options: Options, pieces: impl IntoIterator<Item = &'a [u8]>) -> Parsed {
    let mut parser = ByteParser::new(options);
    for piece in pieces {
        parser.feed(piece);
    }
    parser.finish().unwrap()
}

/// Every `#data` in the html5lib encoding files, the scripted one too:
/// its bytes are as good a test of the streaming as any.
fn html5lib_inputs() -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for file in [
        "tests1.dat",
        "tests2.dat",
        "test-yahoo-jp.dat",
        "scripted/tests1.dat",
    ] {
        let bytes = std::fs::read(Path::new(HTML5LIB).join(file)).unwrap();
        let mut rest = &bytes[..];
        while let Some(start) = find(rest, b"#data\n") {
            rest = &rest[start + b"#data\n".len()..];
            let end = find(rest, b"\n#encoding\n").unwrap();
            out.push(rest[..end].to_vec());
            rest = &rest[end..];
        }
    }
    // And each behind byte order marks, which a cut can split: UTF-8's
    // on the bytes as they are, and UTF-16's on them widened.
    let plain = out.clone();
    for data in plain {
        out.push([&b"\xEF\xBB\xBF"[..], &data].concat());
        out.push(
            [
                &b"\xFF\xFE"[..],
                &data.iter().flat_map(|&b| [b, 0]).collect::<Vec<_>>(),
            ]
            .concat(),
        );
        out.push(
            [
                &b"\xFE\xFF"[..],
                &data.iter().flat_map(|&b| [0, b]).collect::<Vec<_>>(),
            ]
            .concat(),
        );
    }
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A fixed-seed xorshift, so a failure can be run again.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

/// `bytes` in random pieces of 1 to `max` bytes.
fn random_cut<'a>(bytes: &'a [u8], max: usize, rng: &mut Rng) -> Vec<&'a [u8]> {
    let mut out = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let n = (1 + rng.below(max)).min(rest.len());
        out.push(&rest[..n]);
        rest = &rest[n..];
    }
    out
}

type Outcome = (String, &'static str, parsec_encoding::Confidence);

/// Cuts compared with the whole, and what differed.
#[derive(Default)]
struct Tally {
    cuts: usize,
    failures: Vec<String>,
}

impl Tally {
    fn check(&mut self, whole: &Outcome, got: Parsed, what: impl FnOnce() -> String) {
        self.cuts += 1;
        let got = outcome(got);
        if got != *whole && self.failures.len() < 20 {
            self.failures.push(format!(
                "{}: got {} {:?}, want {} {:?}",
                what(),
                got.1,
                got.2,
                whole.1,
                whole.2
            ));
        } else if got != *whole {
            self.failures.push(String::new());
        }
    }

    fn report(self, suite: &str, data: &Path, skipped_because: &str, skipped: usize) {
        let failed = self.failures.len();
        eprintln!("{suite}: {} cuts, {failed} differ", self.cuts);
        summary::write(&summary::Counts {
            suite,
            passed: self.cuts - failed,
            failed,
            skipped,
            skipped_because,
            data,
        });
        let shown: Vec<_> = self
            .failures
            .into_iter()
            .filter(|f| !f.is_empty())
            .collect();
        assert!(failed == 0, "{failed} differ:\n{}", shown.join("\n"));
    }
}

#[test]
fn html5lib_inputs_cut_anywhere() {
    let inputs = html5lib_inputs();
    let mut tally = Tally::default();
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    // Sniffing alone, then with a transport-layer encoding, which decides
    // before the prescan can. A byte order mark beats both, so the inputs
    // behind one (all but the first quarter) run only the first way.
    let transport = Options {
        transport: Some(encoding_rs::EUC_JP),
        ..Options::default()
    };
    let runs = [
        (Options::default(), inputs.len()),
        (transport, inputs.len() / 4),
    ];
    for (options, take) in runs {
        for (i, data) in inputs.iter().enumerate().take(take) {
            let whole = outcome(parse_bytes(data, options).unwrap());
            for at in 0..=data.len() {
                let (a, b) = data.split_at(at);
                tally.check(&whole, pieces(options, [a, b]), || {
                    format!("case {i} cut at byte {at}")
                });
            }
            for max in [1, 3, 16, 300] {
                for _ in 0..25 {
                    let cut = random_cut(data, max, &mut rng);
                    let n = cut.len();
                    tally.check(&whole, pieces(options, cut), || {
                        format!("case {i} in {n} random pieces of up to {max}")
                    });
                }
            }
        }
    }
    tally.report("streaming-html5lib-encoding", Path::new(HTML5LIB), "", 0);
}

/// The legacy multi-byte pages, 120 KB to 850 KB each. Cutting each
/// whole page at every byte would be 3.3 million parses of up to 850 KB,
/// hours in a debug build, so it's sampled: each page's first 8 KB (its
/// `<meta>`, then thousands of characters of two and three bytes) as a
/// page of its own, cut at every byte, then each whole page cut at 40
/// evenly spaced points and fed in 4 KB and in random pieces of up to
/// 1 KB, 4 KB and 64 KB.
#[test]
fn wpt_multi_byte_pages_cut_in_pieces() {
    let pages = [
        "legacy-mb-japanese/shift_jis/sjis_chars.html",
        "legacy-mb-japanese/euc-jp/eucjp_chars.html",
        "legacy-mb-japanese/iso-2022-jp/iso2022jp_chars.html",
        "legacy-mb-korean/euc-kr/euckr_chars.html",
        "legacy-mb-tchinese/big5/big5_chars.html",
        "legacy-mb-tchinese/big5/big5_chars_extra.html",
    ];
    let mut tally = Tally::default();
    let mut rng = Rng(0x2545_F491_4F6C_DD1D);
    let options = Options::default();
    for file in pages {
        let page = std::fs::read(Path::new(WPT).join(file)).unwrap();
        let head = &page[..8192];
        let whole = outcome(parse_bytes(head, options).unwrap());
        for at in 0..=head.len() {
            let (a, b) = head.split_at(at);
            tally.check(&whole, pieces(options, [a, b]), || {
                format!("{file}'s first 8 KB cut at {at}")
            });
        }
        let whole = outcome(parse_bytes(&page, options).unwrap());
        for k in 1..=40 {
            let at = page.len() * k / 41;
            let (a, b) = page.split_at(at);
            tally.check(&whole, pieces(options, [a, b]), || {
                format!("{file} cut at {at}")
            });
        }
        tally.check(&whole, pieces(options, page.chunks(4096)), || {
            format!("{file} in 4 KB pieces")
        });
        for max in [1024, 4096, 65536] {
            let cut = random_cut(&page, max, &mut rng);
            let n = cut.len();
            tally.check(&whole, pieces(options, cut), || {
                format!("{file} in {n} random pieces of up to {max}")
            });
        }
    }
    tally.report("streaming-wpt-legacy-multi-byte", Path::new(WPT), "", 0);
}
