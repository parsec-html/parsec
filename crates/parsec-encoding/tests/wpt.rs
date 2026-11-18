// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The web-platform-tests `encoding/` cases that are data rather than
//! browser behaviour, pinned in `tests/wpt` (see its COMMIT and
//! LICENSE.md). The JavaScript files are read for their data, and each
//! test does here what the WPT harness does with it:
//!
//! - `resources/encodings.js`, with `textdecoder-labels.any.js`: every
//!   label, in either case and wrapped in whitespace, names its encoding.
//! - `resources/single-byte-decoder.js`, as `single-byte-decoder.any.js`
//!   runs it: bytes 0 to 254 through every single-byte label.
//! - `iso-2022-jp-decoder.any.js`, `gb18030-decoder.any.js` (with its
//!   `resources/ranges.js`) and `gbk-decoder.any.js`: their byte sequences
//!   and expected strings.
//! - The `*_chars.html` pages for Shift_JIS, EUC-JP, ISO-2022-JP, EUC-KR
//!   and Big5: each whole page goes through `parse_bytes`, which has to
//!   find the encoding in its `<meta>`, and each `<span>` must hold the
//!   character its `data-cp` names. WPT's encoder tests write characters
//!   into URLs and forms, which needs a browser, so the same pages check
//!   the encoder instead: each character encodes back to its
//!   `data-bytes`. That half is ours, not a WPT count, and leaves out
//!   what the spec's encoders never write (EUC-JP's JIS X 0212, Big5's
//!   HKSCS pointers in `big5_chars_extra.html`).
//!
//! What's left out needs a browser: XMLHttpRequest, form submission,
//! `TextDecoder`'s API surface and streams, and the `*-decode-*.html`
//! label variants, which repeat the same pages under other labels (the
//! labels test covers those labels).

use std::path::Path;

use parsec::css::SelectorList;
use parsec_encoding::{Encoding, Options, parse_bytes};
use serde_json::Value;

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

const DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/wpt");

fn read(file: &str) -> String {
    std::fs::read_to_string(Path::new(DIR).join(file)).unwrap()
}

/// The JSON value that starts at the first `open` after `after`.
fn json_in(text: &str, after: &str, open: char) -> Value {
    let text = &text[text.find(after).unwrap()..];
    let text = &text[text.find(open).unwrap()..];
    let mut values = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    values.next().unwrap().unwrap()
}

#[derive(Default)]
struct Tally {
    passed: usize,
    failures: Vec<String>,
}

impl Tally {
    fn check(&mut self, ok: bool, what: impl FnOnce() -> String) {
        if ok {
            self.passed += 1;
        } else {
            self.failures.push(what());
        }
    }

    fn report(self, suite: &str) {
        eprintln!(
            "{suite}: {} passed, {} failed",
            self.passed,
            self.failures.len()
        );
        summary::write(&summary::Counts {
            suite,
            passed: self.passed,
            failed: self.failures.len(),
            skipped: 0,
            skipped_because: "none",
            data: Path::new(DIR),
        });
        let shown: Vec<_> = self.failures.iter().take(20).collect();
        assert!(self.failures.is_empty(), "{shown:#?}");
    }
}

fn label(label: &str) -> Option<&'static str> {
    Encoding::for_label(label.as_bytes()).map(Encoding::name)
}

#[test]
fn labels() {
    let table = json_in(&read("resources/encodings.js"), "encodings_table", '[');
    let mut tally = Tally::default();
    for group in table.as_array().unwrap() {
        for encoding in group["encodings"].as_array().unwrap() {
            let name = encoding["name"].as_str().unwrap();
            for l in encoding["labels"].as_array().unwrap() {
                let l = l.as_str().unwrap();
                let mut variants = vec![l.to_string(), l.to_uppercase()];
                for ws in [" ", "\t", "\n", "\x0C", "\r"] {
                    variants.push(format!("{ws}{l}"));
                    variants.push(format!("{l}{ws}"));
                    variants.push(format!("{ws}{l}{ws}"));
                }
                let wrong: Vec<_> = variants.iter().filter(|v| label(v) != Some(name)).collect();
                tally.check(wrong.is_empty(), || format!("{l} => {name}: {wrong:?}"));
            }
        }
    }
    tally.report("wpt-encoding-labels");
}

#[test]
fn single_byte() {
    let table = json_in(&read("resources/encodings.js"), "encodings_table", '[');
    let source = read("resources/single-byte-decoder.js");
    let indexes = json_in(&source, "singleByteIndexes", '{');
    let bytes: Vec<u8> = (0..255).collect();
    let mut tally = Tally::default();
    let group = table
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["heading"] == "Legacy single-byte encodings")
        .unwrap();
    for encoding in group["encodings"].as_array().unwrap() {
        let name = encoding["name"].as_str().unwrap();
        let index = &indexes[if name == "ISO-8859-8-I" {
            "ISO-8859-8"
        } else {
            name
        }];
        let want: String = bytes
            .iter()
            .map(|&b| match b {
                0..0x80 => char::from(b),
                _ => index[usize::from(b - 0x80)]
                    .as_u64()
                    .map_or('\u{FFFD}', |cp| char::from_u32(cp as u32).unwrap()),
            })
            .collect();
        for l in encoding["labels"].as_array().unwrap() {
            let l = l.as_str().unwrap();
            let e = Encoding::for_label(l.as_bytes());
            let got = e.map(|e| e.decode_without_bom_handling(&bytes).0.into_owned());
            tally.check(
                e.map(Encoding::name) == Some(name) && got.as_deref() == Some(&want),
                || format!("{name}: {l}"),
            );
        }
    }
    tally.report("wpt-encoding-single-byte");
}

/// A JavaScript string literal starting at `s[0]`, and what follows it.
fn js_string(s: &str) -> (String, &str) {
    let quote = s.chars().next().unwrap();
    let mut out = String::new();
    let mut chars = s[1..].char_indices();
    while let Some((i, c)) = chars.next() {
        if c == quote {
            return (out, &s[1 + i + 1..]);
        }
        if c != '\\' {
            out.push(c);
            continue;
        }
        let rest = &s[1 + i + 1..];
        let (cp, len) = match chars.next().unwrap().1 {
            'x' => (u32::from_str_radix(&rest[1..3], 16).unwrap(), 2),
            'u' if rest[1..].starts_with('{') => {
                let end = rest.find('}').unwrap();
                (u32::from_str_radix(&rest[2..end], 16).unwrap(), end)
            }
            'u' => (u32::from_str_radix(&rest[1..5], 16).unwrap(), 4),
            'n' => ('\n' as u32, 0),
            't' => ('\t' as u32, 0),
            'r' => ('\r' as u32, 0),
            other => (other as u32, 0),
        };
        out.push(char::from_u32(cp).unwrap());
        for _ in 0..len {
            chars.next();
        }
    }
    panic!("unterminated string");
}

/// Every `decode([bytes], "expected"` call in a test file whose bytes are
/// all literal numbers.
fn decode_calls(source: &str) -> Vec<(Vec<u8>, String)> {
    let mut out = Vec::new();
    for (at, _) in source.match_indices("decode(") {
        let call = source[at + "decode(".len()..].trim_start();
        let Some(list) = call.strip_prefix('[') else {
            continue;
        };
        let end = list.find(']').unwrap();
        let bytes: Option<Vec<u8>> = list[..end]
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| match t.strip_prefix("0x") {
                Some(hex) => u8::from_str_radix(&hex.replace('_', ""), 16).ok(),
                None => t.parse().ok(),
            })
            .collect();
        let Some(bytes) = bytes else { continue };
        let after = list[end + 1..].trim_start().strip_prefix(',').unwrap();
        out.push((bytes, js_string(after.trim_start()).0));
    }
    out
}

fn decode(label: &str, bytes: &[u8]) -> String {
    let e = Encoding::for_label(label.as_bytes()).unwrap();
    e.decode_without_bom_handling(bytes).0.into_owned()
}

#[test]
fn iso_2022_jp_decoder() {
    let mut tally = Tally::default();
    let source = read("iso-2022-jp-decoder.any.js");
    for (bytes, want) in decode_calls(&source) {
        tally.check(decode("iso-2022-jp", &bytes) == want, || {
            format!("{bytes:x?} => {want:?}")
        });
    }
    // The file's two streaming tests, each with fatal off and on: a
    // decoder starts over after the last chunk.
    for fatal in [false, true] {
        let chunks: [&[(&[u8], bool, &str)]; 2] = [
            &[
                (&[0x1b, 0x28], false, ""),
                (&[0x49, 0x21], true, "\u{FF61}"),
                (&[0x21], true, "!"),
            ],
            &[
                (&[0x1b, 0x28, 0x49], false, ""),
                (&[], true, ""),
                (&[0x21], true, "!"),
            ],
        ];
        for steps in chunks {
            let mut decoder = encoding_rs::ISO_2022_JP.new_decoder_without_bom_handling();
            let ok = steps.iter().all(|&(bytes, last, want)| {
                let mut out = String::with_capacity(16);
                let clean = if fatal {
                    let (result, _) =
                        decoder.decode_to_string_without_replacement(bytes, &mut out, last);
                    result == encoding_rs::DecoderResult::InputEmpty
                } else {
                    let (result, _, _) = decoder.decode_to_string(bytes, &mut out, last);
                    result == encoding_rs::CoderResult::InputEmpty
                };
                if last {
                    decoder = encoding_rs::ISO_2022_JP.new_decoder_without_bom_handling();
                }
                clean && out == want
            });
            tally.check(ok, || format!("streaming, fatal={fatal}: {steps:x?}"));
        }
    }
    tally.report("wpt-encoding-iso-2022-jp");
}

#[test]
fn gb18030_and_gbk_decoders() {
    let mut tally = Tally::default();
    let source = read("legacy-mb-schinese/gb18030/gb18030-decoder.any.js");
    let mut cases = decode_calls(&source);
    // The file's loop over the four-byte ranges.
    let ranges = read("legacy-mb-schinese/gb18030/resources/ranges.js");
    for line in ranges.lines().map(str::trim).filter(|l| l.starts_with('[')) {
        let (pointer, rest) = line[1..].split_once(',').unwrap();
        let pointer: u32 = pointer.parse().unwrap();
        let want = js_string(rest.trim_start()).0;
        let bytes = vec![
            (pointer / 12600 + 0x81) as u8,
            (pointer % 12600 / 1260 + 0x30) as u8,
            (pointer % 1260 / 10 + 0x81) as u8,
            (pointer % 10 + 0x30) as u8,
        ];
        cases.push((bytes, want));
    }
    for (bytes, want) in cases {
        let ok = ["gb18030", "gbk"]
            .iter()
            .all(|label| decode(label, &bytes) == want);
        tally.check(ok, || format!("{bytes:x?} => {want:?}"));
    }

    let source = read("legacy-mb-schinese/gbk/gbk-decoder.any.js");
    let array = |name: &str| -> Vec<u32> {
        let at = source.find(name).unwrap();
        let list = &source[at..];
        let list = &list[list.find('[').unwrap() + 1..list.find(']').unwrap()];
        list.split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| match t.strip_prefix("0x") {
                Some(hex) => u32::from_str_radix(hex, 16).unwrap(),
                None => t.parse().unwrap(),
            })
            .collect()
    };
    for (pointer, cp) in array("gbkPointers").into_iter().zip(array("codePoints")) {
        let trail = pointer % 190;
        let offset = if trail < 0x3F { 0x40 } else { 0x41 };
        let bytes = [(pointer / 190 + 0x81) as u8, (trail + offset) as u8];
        let got = decode("GBK", &bytes).chars().next().map(u32::from);
        tally.check(got == Some(cp), || format!("gbk pointer {pointer}"));
    }
    tally.report("wpt-encoding-gb18030");
}

#[test]
fn chars_pages() {
    let pages: [(&str, &str, bool); 6] = [
        (
            "legacy-mb-japanese/shift_jis/sjis_chars.html",
            "Shift_JIS",
            true,
        ),
        ("legacy-mb-japanese/euc-jp/eucjp_chars.html", "EUC-JP", true),
        (
            "legacy-mb-japanese/iso-2022-jp/iso2022jp_chars.html",
            "ISO-2022-JP",
            true,
        ),
        ("legacy-mb-korean/euc-kr/euckr_chars.html", "EUC-KR", true),
        ("legacy-mb-tchinese/big5/big5_chars.html", "Big5", true),
        (
            "legacy-mb-tchinese/big5/big5_chars_extra.html",
            "Big5",
            false,
        ),
    ];
    let spans = SelectorList::parse("span[data-cp]").unwrap();
    let mut decoding = Tally::default();
    let mut encoding = Tally::default();
    for (file, name, encodes) in pages {
        let bytes = std::fs::read(Path::new(DIR).join(file)).unwrap();
        let parsed = parse_bytes(&bytes, Options::default()).unwrap();
        assert_eq!(parsed.encoding.name(), name, "{file}'s <meta>");
        let doc = &parsed.document;
        for span in spans.select(doc, doc.root()) {
            let attr = |want: &str| {
                doc.attrs(span)
                    .find(|&(_, n, _)| n == want)
                    .map(|(_, _, v)| v.to_string())
                    .unwrap()
            };
            let cp = u32::from_str_radix(&attr("data-cp"), 16).unwrap();
            let ch = char::from_u32(cp).unwrap();
            // WPT compares with its own decoder's reading of the bytes, which
            // is the character itself except where the spec's encoders map
            // a character onto bytes that decode as another: U+00A5 and
            // U+203E onto ASCII's 0x5C and 0x7E, and U+2212 onto U+FF0D's.
            let want = match (name, ch) {
                ("Shift_JIS" | "EUC-JP", '\u{A5}') => '\\',
                ("Shift_JIS" | "EUC-JP", '\u{203E}') => '~',
                ("Shift_JIS" | "EUC-JP" | "ISO-2022-JP", '\u{2212}') => '\u{FF0D}',
                _ => ch,
            }
            .to_string();
            let text = doc.text(span);
            decoding.check(text == want, || format!("{file}: U+{cp:04X} gave {text:?}"));

            let hex = attr("data-bytes");
            if !encodes || (name == "EUC-JP" && hex.starts_with("8F")) {
                continue;
            }
            let want: Vec<u8> = hex
                .split(' ')
                .map(|b| u8::from_str_radix(b, 16).unwrap())
                .collect();
            let ch = ch.to_string();
            let (got, _, unmappable) = parsed.encoding.encode(&ch);
            encoding.check(!unmappable && *got == want[..], || {
                format!("{file}: U+{cp:04X} encoded as {got:02X?}, not {hex}")
            });
        }
    }
    decoding.report("wpt-encoding-legacy-multi-byte");
    encoding.report("encoding-legacy-multi-byte-encoder");
}
