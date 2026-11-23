// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The final boss: a batch of pages as messy as a crawl brings back, fed
//! as bytes to each parser in its own process, timing every page and
//! taking the process's peak resident memory.
//!
//! `cargo run --release --manifest-path bench/Cargo.toml -- boss` runs
//! 1,000 pages three times through each parser. `-- boss --smoke` is the
//! CI check: 300 pages, no page over 2 MB, Parsec only, once, and a
//! non-zero exit if any page crashes it, hangs it or leaves no document.
//! `-- boss --gate` is the release's check (release.yml): all 1,000
//! pages, once, through Parsec and html5ever, with the same non-zero exit,
//! and also if html5ever builds a different number of elements than
//! Parsec on any page.
//! `--pages N`, `--runs N`, `--seed N` and `--tools parsec,lexbor,...`
//! change the rest, and `--now` skips waiting for a quiet machine.
//! `boss-page <n> [pages] [seed]` prints page `n` of the batch, to look
//! into one.
//!
//! The batch comes from a seed: the same seed gives the same bytes, in the
//! same order, every time and for every parser. Page `i` is built from
//! the seed and `i` alone, so a run can restart after any page. Out of
//! every 1,000 pages:
//!
//! - 900 are tiny broken fragments, a few hundred bytes of one to six
//!   kinds of breakage each (the list is in `fragment`), one in twenty
//!   with tags nested 1,000 to 5,000 deep;
//! - 46 are real pages from bench/corpus as they were served (the twelve
//!   under 2 MB, in turn);
//! - 44 are real pages re-encoded, four of each of the eleven quirks in
//!   `encoded`;
//! - 10 are giants of 10 to 30 MB: real pages run together, one huge
//!   table, or one text node full of character references.
//!
//! The fragments are most of the count because they're most of a crawl
//! by number; the giants are 1% of the count and most of the bytes, which
//! is where memory and slow paths show. Without bench/corpus (as in CI),
//! generated shop listings stand in for the real pages.
//!
//! What each parser is given:
//!
//! - Parsec: `parsec_encoding::parse_bytes`, with the transport's
//!   encoding when the page has one, as from an HTTP `Content-Type`.
//! - Lexbor: its BOM sniff, then the transport label, then its own
//!   `<meta>` prescan of the first 1,024 bytes, then windows-1252; then
//!   its encoding module decodes to UTF-8, and `lxb_html_document_parse`
//!   parses. UTF-8, and ASCII in an encoding that keeps ASCII, skip the
//!   decode and go to the parser as they are (so invalid UTF-8 reaches
//!   Lexbor's parser unreplaced). Lexbor 3.0.1 has no detector that
//!   guesses UTF-8 from the bytes, and doesn't parse again for a late
//!   `<meta>`.
//! - html5ever: it takes text, not bytes, so the bytes are decoded first
//!   by encoding_rs with Parsec's sniffing (`parsec_encoding::sniff`),
//!   without the second parse for a late `<meta>`.
//!
//! A page's time is from bytes to a built document. Counting its
//! elements and freeing it aren't timed. A page still parsing after
//! `PAGE_LIMIT` is a timeout: the child process is killed and a new one
//! carries on from the next page, as after a crash. Peak memory is
//! `getrusage(RUSAGE_CHILDREN)`'s `ru_maxrss`, taken in a process whose
//! only children are that parser's, so it covers that parser and the
//! page generator, nothing else; the "no parser" row is the generator
//! alone. Before each parser's run the bench waits for the one-minute
//! load average to drop below 2 (for up to two hours in all).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::{NodeData, RcDom};
use parsec_encoding::encoding_rs::{EUC_KR, GBK, SHIFT_JIS, WINDOWS_1251, WINDOWS_1252};
use parsec_encoding::{Encoding, Options};

use crate::pages::Rng;

pub const SEED: u64 = 20_261_119;
/// How long one page may take before it counts as a timeout.
pub(crate) const PAGE_LIMIT: Duration = Duration::from_secs(10);
/// The exit status a child uses when a page passed PAGE_LIMIT.
const TIMED_OUT: i32 = 124;
/// The load average to wait for before a run, and for how long in all.
const QUIET: f64 = 2.0;
const MAX_WAIT: Duration = Duration::from_secs(2 * 60 * 60);
/// Real pages bigger than this are left to the giants.
const REAL_MAX: usize = 2_000_000;

const TOOLS: [&str; 4] = ["Parsec", "Lexbor", "html5ever", "no parser"];
const CATEGORIES: [&str; 4] = ["fragments", "real pages", "encodings", "giants"];

/// What decides the batch: the same three give the same bytes.
#[derive(Clone, Copy)]
struct Batch {
    pages: usize,
    seed: u64,
    /// The largest a giant may be.
    giant_cap: usize,
}

impl Batch {
    fn args(&self) -> [String; 3] {
        [
            self.pages.to_string(),
            self.seed.to_string(),
            self.giant_cap.to_string(),
        ]
    }

    fn from_args(a: &[String]) -> Batch {
        Batch {
            pages: a[0].parse().unwrap(),
            seed: a[1].parse().unwrap(),
            giant_cap: a[2].parse().unwrap(),
        }
    }

    /// Each page's category, in batch order: the counts above per 1,000,
    /// shuffled with the seed.
    fn plan(&self) -> Vec<usize> {
        let n = self.pages;
        let giants = (n + 50) / 100;
        let encodings = (n * 44 + 500) / 1000;
        let real = (n * 46 + 500) / 1000;
        let mut plan = vec![0; n - giants - encodings - real];
        plan.extend(std::iter::repeat_n(1, real));
        plan.extend(std::iter::repeat_n(2, encodings));
        plan.extend(std::iter::repeat_n(3, giants));
        let mut rng = Rng(mix(self.seed, u64::MAX));
        for i in (1..n).rev() {
            plan.swap(i, rng.below(i + 1));
        }
        plan
    }
}

/// splitmix64 of the seed and a page number: a different, never-zero
/// start for each page's xorshift.
fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) | 1
}

/// One page as a crawler gets it.
struct Input {
    bytes: Vec<u8>,
    /// The `charset` an HTTP header gave, if any.
    transport: Option<&'static str>,
    category: usize,
    kind: &'static str,
}

/// The real pages in bench/corpus, read when needed so the generator
/// doesn't hold them all.
struct Corpus {
    small: Vec<String>,
    all: Vec<String>,
}

impl Corpus {
    fn load() -> Corpus {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        let list = std::fs::read_to_string(dir.join("pages.txt")).unwrap_or_default();
        let mut corpus = Corpus {
            small: Vec::new(),
            all: Vec::new(),
        };
        for name in list
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .filter_map(|l| l.split_whitespace().next())
        {
            let path = dir.join("corpus").join(format!("{name}.html"));
            if let Ok(meta) = std::fs::metadata(&path) {
                if (meta.len() as usize) < REAL_MAX {
                    corpus.small.push(name.into());
                }
                corpus.all.push(name.into());
            }
        }
        corpus
    }

    fn read(name: &str) -> Vec<u8> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        std::fs::read(dir.join("corpus").join(format!("{name}.html"))).unwrap()
    }

    /// The `k`th small real page, or a generated shop listing of 50 KB to
    /// 1 MB without a corpus.
    fn small(&self, k: usize, rng: &mut Rng) -> Vec<u8> {
        if self.small.is_empty() {
            return crate::pages::shop(50_000 + rng.below(950_000)).into_bytes();
        }
        Corpus::read(&self.small[k % self.small.len()])
    }

    fn any(&self, k: usize, rng: &mut Rng) -> Vec<u8> {
        if self.all.is_empty() {
            return crate::pages::shop(50_000 + rng.below(950_000)).into_bytes();
        }
        Corpus::read(&self.all[k % self.all.len()])
    }
}

/// Page `i` of the batch.
fn input(batch: &Batch, plan: &[usize], corpus: &Corpus, i: usize) -> Input {
    let mut rng = Rng(mix(batch.seed, i as u64));
    let category = plan[i];
    // How many pages of this category come before this one, to take the
    // real pages and the quirks in turn.
    let nth = plan[..i].iter().filter(|&&c| c == category).count();
    let (bytes, transport, kind) = match category {
        0 => {
            let (bytes, kind) = fragment(&mut rng);
            (bytes, None, kind)
        }
        1 => (corpus.small(nth, &mut rng), None, "real page"),
        2 => encoded(corpus.small(nth / 11, &mut rng), nth % 11, &mut rng),
        _ => {
            let size = (10_000_000 + rng.below(20_000_001)).min(batch.giant_cap);
            let (bytes, kind) = giant(nth % 3, size, corpus, &mut rng);
            (bytes, None, kind)
        }
    };
    Input {
        bytes,
        transport,
        category,
        kind,
    }
}

fn pick<'a>(rng: &mut Rng, from: &[&'a str]) -> &'a str {
    from[rng.below(from.len())]
}

/// A tiny broken fragment: an optional odd doctype, one to six pieces of
/// breakage, sometimes a few thousand levels of nesting, and sometimes an
/// ending that leaves something open to the end of the input. The kind
/// is the first piece's, for reports.
fn fragment(rng: &mut Rng) -> (Vec<u8>, &'static str) {
    let mut out = String::new();
    if rng.below(5) == 0 {
        out += pick(
            rng,
            &[
                r#"<!DOCTYPE html PUBLIC "-//W3C//DTD HTML 4.01 Transitional//EN">"#,
                "<!doctype>",
                r#"<!DOCTYPE html SYSTEM "about:legacy-compat">"#,
                "<!DOCTYPE HTML PUBLIC \"-//W3C//DTD HTML 3.2//EN\"",
                "<!doctypehtml>",
            ],
        );
    }
    let mut kind = "";
    for _ in 0..1 + rng.below(6) {
        let n = 1 + rng.below(4);
        let w = rng.words(n);
        let (piece, name) = match rng.below(14) {
            0 => {
                let tags = ["div", "p", "span", "li", "td", "b", "a", "section", "h1"];
                let s: String = (0..1 + rng.below(6))
                    .map(|_| format!("<{}>{w}", pick(rng, &tags)))
                    .collect();
                (s, "unclosed tags")
            }
            1 => (
                pick(
                    rng,
                    &[
                        "<b><i>W</b>W</i>",
                        "<a href=x><div>W</a>W</div>",
                        "<p><table><p>W</table>",
                        "<ul><li>W<li><ol>W</ul></ol>",
                        "<h1><h2>W</h1></h2>",
                        "<form><form>W</form>W",
                    ],
                )
                .replace('W', &w),
                "misnested tags",
            ),
            2 => {
                let ends = [
                    "</p>", "</table>", "</td>", "</tr>", "</body>", "</html>", "</br>",
                ];
                let s: String = (0..1 + rng.below(6)).map(|_| pick(rng, &ends)).collect();
                (s + &w, "stray end tags")
            }
            3 => (
                pick(
                    rng,
                    &[
                        "<a href=/x?a=1&b=2 class=one class=two>W</a>",
                        "<div id=a id=b id=c data-x=1 data-x=2>W</div>",
                        "<img src=x.png alt=W/>",
                        "<div a=b=c ==x \"q\"name>W</div>",
                        "<p class='mixed\" title=\"W'>W</p>",
                        "<input value=W disabled disabled checked=checked=checked>",
                    ],
                )
                .replace('W', &w),
                "attributes",
            ),
            4 => (
                format!("{w} < {w} <3 << <= a<b <!x <?php echo 1 ?> </ > <-"),
                "< in text",
            ),
            5 => (format!("<p>a\0{w}</p><\0x>\0<b\0c=\0>{w}"), "NUL bytes"),
            6 => (
                format!("AT&T & {w} &amp &ampx &lt3 &gt &copy2026 &#38 &&& &;"),
                "bare &",
            ),
            7 => (
                pick(
                    rng,
                    &[
                        "&#x110000;",
                        "&#xD800;",
                        "&#xDFFF;",
                        "&#0;",
                        "&#128;",
                        "&#x9F;",
                        "&#99999999999999999999;",
                        "&#x;",
                        "&#;",
                        "&#xFFFE;",
                        "&#1114111;",
                        "&#x0000000000041;",
                    ],
                )
                .repeat(1 + rng.below(4))
                    + &w,
                "numeric references",
            ),
            8 => (
                pick(
                    rng,
                    &[
                        "&notin &noti &notit; W",
                        "&CounterClockwiseContourIntegral &CounterClockwiseContourIntegra W",
                        "&supseteq &supsetneqq; &zwnj &xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx; W",
                        "<a href=\"?x=1&notit=2&copy=3&amp=4\">W</a>",
                        "&ThickSpace;&NegativeVeryThinSpace&DoubleLongLeftRightArrow W",
                    ],
                )
                .replace('W', &w),
                "named references",
            ),
            9 => (
                format!("<table><tr>{w}<td>{w}<caption><td>{w}</table>"),
                "tables",
            ),
            10 => (
                pick(
                    rng,
                    &[
                        "<select><option>W<div>W</select>",
                        "<svg><foreignObject><p>W</svg>",
                        "<math><mi><svg><title>W</math>",
                        "<svg><desc><svg><g>W</desc></svg>",
                    ],
                )
                .replace('W', &w),
                "foreign content",
            ),
            11 => (
                format!("<template><td>{w}</template><template shadowrootmode=open><b>{w}"),
                "templates",
            ),
            12 => {
                let fmt = ["a", "b", "i", "u", "s", "em", "strong", "font", "nobr"];
                let s: String = (0..3 + rng.below(10))
                    .map(|_| format!("<{}>", pick(rng, &fmt)))
                    .collect();
                (s + &format!("<p>{w}<p>{w}"), "formatting soup")
            }
            _ => (format!("<p>{w}</p>"), "plain"),
        };
        if kind.is_empty() {
            kind = name;
        }
        out += &piece;
    }
    if rng.below(20) == 0 {
        let open = pick(
            rng,
            &[
                "<div>",
                "<span>",
                "<b>",
                "<a>",
                "<ul><li>",
                "<table><tr><td>",
                "<svg><g>",
                "<i><div>",
            ],
        );
        out += &open.repeat(1000 + rng.below(4001));
        out += "deep";
        kind = "deep nesting";
    }
    if rng.below(5) < 2 {
        out += pick(
            rng,
            &[
                "<!-- never closed",
                "<![CDATA[ never closed",
                "<svg><![CDATA[ never closed",
                "<script>if (a<b) { document.write('<p>'",
                "<textarea>typed <b>text",
                "<title>a title &amp",
                "<style>p { color:",
                "<plaintext><p>rest",
                "<xmp><b>",
                "<div class=\"quote never closed",
                "<a href=x",
                "<!",
                "</",
                "&#x",
                "<script><!--<script>",
            ],
        );
    }
    (out.into_bytes(), kind)
}

/// Text in each legacy encoding's own script, so the decoders have more
/// than ASCII markup to get through.
const JAPANESE: &str = "日本語のテキストと漢字、ひらがな、カタカナ。";
const RUSSIAN: &str = "Съешь же ещё этих мягких французских булок, да выпей чаю.";
const CHINESE: &str = "中文网页的编码测试，简体字与标点符号。";
const KOREAN: &str = "한국어 웹 페이지 인코딩 시험입니다.";

/// A real page with a `<meta charset>` (or other declaration) put where
/// the prescan finds it first: just inside `<head>`, or at the start.
fn declare(html: &str, decl: &str) -> String {
    let head = html.as_bytes()[..html.len().min(900)]
        .windows(5)
        .position(|w| w.eq_ignore_ascii_case(b"<head"))
        .and_then(|at| html[at..].find('>').map(|end| at + end + 1));
    match head {
        Some(at) => format!("{}{decl}{}", &html[..at], &html[at..]),
        None => format!("{decl}{html}"),
    }
}

/// The page with `paragraphs` of `text` before its `</body>`.
fn with_text(html: &str, text: &str, paragraphs: usize) -> String {
    let block: String = (0..paragraphs)
        .map(|_| format!("<p>{text}</p>\n"))
        .collect();
    match html.rfind("</body>") {
        Some(at) => format!("{}{block}{}", &html[..at], &html[at..]),
        None => html.to_string() + &block,
    }
}

/// `html` in `encoding`; a character it lacks becomes a numeric reference.
fn encode(encoding: &'static Encoding, html: &str) -> Vec<u8> {
    encoding.encode(html).0.into_owned()
}

fn utf16(html: &str, big_endian: bool) -> Vec<u8> {
    let mut out = if big_endian {
        vec![0xFE, 0xFF]
    } else {
        vec![0xFF, 0xFE]
    };
    for unit in html.encode_utf16() {
        out.extend(if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    out
}

/// Replaces every `charset` (any case) so nothing declares an encoding.
fn undeclare(html: &str) -> String {
    let mut out = html.as_bytes().to_vec();
    for at in 0..out.len().saturating_sub(7) {
        if out[at..at + 7].eq_ignore_ascii_case(b"charset") {
            out[at..at + 7].copy_from_slice(b"data-cs");
        }
    }
    String::from_utf8(out).unwrap()
}

/// A real page with quirk `which` of eleven.
fn encoded(
    page: Vec<u8>,
    which: usize,
    rng: &mut Rng,
) -> (Vec<u8>, Option<&'static str>, &'static str) {
    let html = String::from_utf8_lossy(&page).into_owned();
    let n = 50 + rng.below(250);
    let all = format!("{JAPANESE} {RUSSIAN} {CHINESE} {KOREAN}");
    match which {
        0 => (
            encode(
                SHIFT_JIS,
                &with_text(&declare(&html, "<meta charset=shift_jis>"), JAPANESE, n),
            ),
            None,
            "Shift_JIS",
        ),
        1 => (
            encode(
                WINDOWS_1251,
                &with_text(&declare(&html, "<meta charset=windows-1251>"), RUSSIAN, n),
            ),
            None,
            "windows-1251",
        ),
        2 => (
            encode(
                GBK,
                &with_text(&declare(&html, "<meta charset=gbk>"), CHINESE, n),
            ),
            None,
            "GBK",
        ),
        3 => (
            encode(
                EUC_KR,
                &with_text(&declare(&html, "<meta charset=euc-kr>"), KOREAN, n),
            ),
            None,
            "EUC-KR",
        ),
        4 => (
            utf16(&with_text(&html, &all, n), false),
            None,
            "UTF-16LE, BOM",
        ),
        5 => (
            utf16(&with_text(&html, &all, n), true),
            None,
            "UTF-16BE, BOM",
        ),
        6 => (
            encode(
                WINDOWS_1251,
                &with_text(&declare(&html, "<meta charset=utf-8>"), RUSSIAN, n),
            ),
            None,
            "meta says UTF-8, bytes windows-1251",
        ),
        7 => {
            let mut bytes = vec![0xEF, 0xBB, 0xBF];
            bytes
                .extend(with_text(&declare(&html, "<meta charset=windows-1251>"), &all, n).bytes());
            (bytes, None, "UTF-8 BOM, meta says windows-1251")
        }
        8 => (
            encode(
                EUC_KR,
                &with_text(&declare(&html, "<meta charset=iso-8859-1>"), KOREAN, n),
            ),
            Some("euc-kr"),
            "EUC-KR from the transport, meta says ISO-8859-1",
        ),
        9 => (
            with_text(&undeclare(&html), &all, n).into_bytes(),
            None,
            "UTF-8, undeclared",
        ),
        _ => {
            let mut bytes =
                with_text(&declare(&html, "<meta charset=utf-8>"), &all, n).into_bytes();
            let bad: [&[u8]; 6] = [
                b"\xFF",
                b"\xC0\xAF",
                b"\xED\xA0\x80",
                b"\xE2\x82",
                b"\x80",
                b"\xF4\x90\x80\x80",
            ];
            let mut at = rng.below(4096);
            while at < bytes.len() {
                let b = bad[rng.below(bad.len())];
                bytes.splice(at..at, b.iter().copied());
                at += 1 + rng.below(8192);
            }
            (bytes, None, "UTF-8 with invalid bytes")
        }
    }
}

/// A giant page of about `size` bytes, of kind `which` of three.
fn giant(which: usize, size: usize, corpus: &Corpus, rng: &mut Rng) -> (Vec<u8>, &'static str) {
    let mut out = b"<!doctype html><meta charset=utf-8><title>giant</title>".to_vec();
    match which {
        0 => {
            let mut k = rng.below(64);
            while out.len() < size {
                let page = corpus.any(k, rng);
                let room = size - out.len();
                out.extend_from_slice(&page[..page.len().min(room)]);
                k += 1;
            }
            (out, "real pages run together")
        }
        1 => {
            out.extend_from_slice(b"<table class=giant>");
            let mut i = 0;
            while out.len() < size {
                // Every tenth row leaves its cells open for the next row to
                // close.
                let close = if i % 10 == 0 { "" } else { "</td>" };
                let row = format!(
                    "<tr data-row={i}><td>{}{close}<td class=n>{}.{:02}{close}<td><a href=/r/{i}>{}</a>{close}<td>&amp; &lt;{}&gt;</tr>\n",
                    rng.words(2),
                    rng.below(10_000),
                    rng.below(100),
                    rng.words(1),
                    rng.words(1)
                );
                out.extend_from_slice(row.as_bytes());
                i += 1;
            }
            out.extend_from_slice(b"</table>");
            (out, "one huge table")
        }
        _ => {
            out.extend_from_slice(b"<p>");
            let refs = [
                "&amp;",
                "&lt;",
                "&gt;",
                "&quot;",
                "&notin;",
                "&noti",
                "&#x1F600;",
                "&#169;",
                "&CounterClockwiseContourIntegral;",
                "&nbsp",
                "&eacute;",
                "&#0;",
                "& ",
                "&#xD800;",
                "&hellip;",
            ];
            while out.len() < size {
                out.extend_from_slice(pick(rng, &refs).as_bytes());
                if rng.below(4) == 0 {
                    out.extend_from_slice(rng.words(1).as_bytes());
                    out.push(b' ');
                }
            }
            (out, "one text node of references")
        }
    }
}

/// Elements in a Parsec document, walked in tree order from the root.
/// Like the others' counts, it doesn't go into template contents or
/// shadow roots.
fn parsec_elements(doc: &parsec::Document) -> usize {
    let root = doc.root();
    let mut n = 0;
    let mut at = doc.first_child(root);
    while let Some(id) = at {
        if matches!(doc.kind(id), parsec::Kind::Element { .. }) {
            n += 1;
        }
        // A `<template shadowrootmode>` is a shadow root in Parsec, as the
        // spec now says, and an element in Lexbor 3.0.1 and html5ever:
        // count it as the element, so the counts compare.
        if doc.shadow_root(id).is_some() {
            n += 1;
        }
        at = doc.first_child(id).or_else(|| {
            let mut up = id;
            loop {
                if let Some(next) = doc.next_sibling(up) {
                    return Some(next);
                }
                up = doc.parent(up).filter(|&p| p != root)?;
            }
        });
    }
    n
}

fn html5ever_elements(dom: &RcDom) -> usize {
    let mut n = 0;
    let mut stack = vec![dom.document.clone()];
    while let Some(node) = stack.pop() {
        if matches!(node.data, NodeData::Element { .. }) {
            n += 1;
        }
        stack.extend(node.children.borrow().iter().cloned());
    }
    n
}

/// Parses one page with `tool`: the seconds from bytes to document, and
/// its element count, or `None` if the parser refused the page.
fn parse(tool: usize, bytes: &[u8], transport: Option<&str>) -> (f64, Option<usize>) {
    match tool {
        0 => {
            let t = Instant::now();
            let options = Options {
                transport: transport.and_then(|l| Encoding::for_label(l.as_bytes())),
                ..Options::default()
            };
            let parsed = parsec_encoding::parse_bytes(bytes, options);
            let secs = t.elapsed().as_secs_f64();
            (secs, parsed.ok().map(|p| parsec_elements(&p.document)))
        }
        1 => {
            let t = Instant::now();
            let doc = crate::lexbor::Document::parse_bytes(bytes, transport);
            let secs = t.elapsed().as_secs_f64();
            (secs, doc.ok().map(|d| d.elements()))
        }
        2 => {
            let t = Instant::now();
            let transport = transport.and_then(|l| Encoding::for_label(l.as_bytes()));
            let (encoding, _) = parsec_encoding::sniff(bytes, transport, WINDOWS_1252);
            let (text, _) = encoding.decode_with_bom_removal(bytes);
            let mut opts = html5ever::ParseOpts::default();
            opts.tree_builder.scripting_enabled = false;
            let dom = html5ever::parse_document(RcDom::default(), opts).one(&*text);
            let secs = t.elapsed().as_secs_f64();
            drop(text);
            (secs, Some(html5ever_elements(&dom)))
        }
        _ => (0.0, Some(0)),
    }
}

/// When the page in progress started, in nanoseconds since the process
/// did (0 between pages), and which page it is.
static STARTED: AtomicU64 = AtomicU64::new(0);
static PAGE: AtomicUsize = AtomicUsize::new(0);

/// `boss-worker <tool>`: reads pages from stdin, each a line
/// `<page> <length> <transport or ->` and then its bytes, parses each and
/// prints `<page> <seconds> <elements or ->`. The worker holds only the
/// page it's on, as a crawler would. A watchdog ends the process with
/// TIMED_OUT if a page passes PAGE_LIMIT.
fn worker(args: &[String]) {
    let tool: usize = args[0].parse().unwrap();
    let epoch = Instant::now();
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_millis(50));
            let started = STARTED.load(Relaxed);
            if started != 0
                && epoch.elapsed().as_nanos() as u64 - started > PAGE_LIMIT.as_nanos() as u64
            {
                println!("timeout {}", PAGE.load(Relaxed));
                std::process::exit(TIMED_OUT);
            }
        }
    });
    let mut stdin = BufReader::new(std::io::stdin().lock());
    let mut out = std::io::stdout();
    let mut header = String::new();
    // One buffer for every page, as a crawler would keep, so the memory
    // here is the biggest page, not every page freed and not yet reused.
    let mut bytes = Vec::new();
    loop {
        header.clear();
        if stdin.read_line(&mut header).unwrap() == 0 {
            break;
        }
        let mut f = header.split_whitespace();
        let i: usize = f.next().unwrap().parse().unwrap();
        let len: usize = f.next().unwrap().parse().unwrap();
        let transport = f.next().filter(|&t| t != "-");
        bytes.clear();
        bytes.resize(len, 0);
        stdin.read_exact(&mut bytes).unwrap();
        PAGE.store(i, Relaxed);
        STARTED.store(epoch.elapsed().as_nanos() as u64 | 1, Relaxed);
        let (secs, elements) = parse(tool, &bytes, transport);
        STARTED.store(0, Relaxed);
        let elements = elements.map_or("-".to_string(), |n| n.to_string());
        writeln!(out, "{i} {secs} {elements}").unwrap();
        out.flush().unwrap();
    }
}

/// `struct rusage`, the same shape on 64-bit macOS and Linux: two
/// `timeval`s of 16 bytes, then `ru_maxrss` and 13 more longs.
#[repr(C)]
struct Rusage {
    times: [i64; 4],
    maxrss: i64,
    rest: [i64; 13],
}

const RUSAGE_CHILDREN: i32 = -1;

unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut Rusage) -> i32;
    fn getloadavg(loads: *mut f64, n: i32) -> i32;
}

/// The largest resident set of any child this process has waited for, in
/// bytes: macOS gives `ru_maxrss` in bytes, Linux in kilobytes.
fn children_peak_rss() -> u64 {
    let mut usage = Rusage {
        times: [0; 4],
        maxrss: 0,
        rest: [0; 13],
    };
    // SAFETY: `usage` has the layout getrusage writes.
    assert_eq!(unsafe { getrusage(RUSAGE_CHILDREN, &mut usage) }, 0);
    let rss = usage.maxrss as u64;
    if cfg!(target_os = "macos") {
        rss
    } else {
        rss * 1024
    }
}

pub(crate) fn load_average() -> f64 {
    let mut loads = [0.0f64; 3];
    // SAFETY: room for the three averages asked for.
    let got = unsafe { getloadavg(loads.as_mut_ptr(), 3) };
    if got < 1 { f64::NAN } else { loads[0] }
}

/// `boss-tool <tool> <batch>`: builds the batch and feeds it to workers
/// for one tool, starting a new worker after a crash or timeout, and
/// prints their lines, `crash <page> <status>` for a crash, and last
/// `rss <bytes>`. Its only children are those workers, so the peak is
/// theirs; building the pages happens here, outside them.
fn tool_run(args: &[String]) {
    let batch = Batch::from_args(&args[1..]);
    let plan = &batch.plan();
    let corpus = &Corpus::load();
    feed(&args[0], batch.pages, &|i| {
        let page = input(&batch, plan, corpus, i);
        (page.bytes, page.transport.map(str::to_string))
    });
}

/// Feeds pages `0..pages`, each as `page` gives its bytes and transport
/// label, to `boss-worker <tool>` processes, starting a new worker after
/// a crash or timeout, and prints their lines, `crash <page> <status>` for
/// a crash, and last `rss <bytes>`.
pub(crate) fn feed(
    tool: &str,
    pages: usize,
    page: &(dyn Fn(usize) -> (Vec<u8>, Option<String>) + Sync),
) {
    let mut next = 0;
    let mut out = std::io::stdout();
    while next < pages {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .arg("boss-worker")
            .arg(tool)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("couldn't start a worker");
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let from = next;
        std::thread::scope(|s| {
            s.spawn(move || {
                for i in from..pages {
                    let (bytes, transport) = page(i);
                    let header = format!(
                        "{i} {} {}\n",
                        bytes.len(),
                        transport.as_deref().unwrap_or("-")
                    );
                    // A worker that died stops taking pages; the loop below
                    // sees it end.
                    if stdin.write_all(header.as_bytes()).is_err()
                        || stdin.write_all(&bytes).is_err()
                    {
                        return;
                    }
                }
            });
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                let first = line.split(' ').next().unwrap_or("");
                next = match line.strip_prefix("timeout ") {
                    Some(page) => page.parse::<usize>().unwrap() + 1,
                    None => first.parse::<usize>().map_or(next, |i| i + 1),
                };
                writeln!(out, "{line}").unwrap();
            }
        });
        let status = child.wait().unwrap();
        if status.code() != Some(0) && status.code() != Some(TIMED_OUT) {
            writeln!(out, "crash {next} {status}").unwrap();
            next += 1;
        }
    }
    writeln!(out, "rss {}", children_peak_rss()).unwrap();
}

/// How one page went for one tool in one run.
#[derive(Clone, Copy)]
pub(crate) enum Outcome {
    Parsed { secs: f64, elements: usize },
    Refused { secs: f64 },
    TimedOut,
    Crashed,
}

impl Outcome {
    pub(crate) fn secs(&self) -> f64 {
        match *self {
            Outcome::Parsed { secs, .. } | Outcome::Refused { secs } => secs,
            // A timeout counts as the limit; a crash, as nothing.
            Outcome::TimedOut => PAGE_LIMIT.as_secs_f64(),
            Outcome::Crashed => 0.0,
        }
    }
}

struct Run {
    pages: Vec<Outcome>,
    rss: u64,
    load: f64,
}

/// Waits until the one-minute load is under `quiet` or `deadline`
/// passes, and returns the load it starts at.
pub(crate) fn wait_for_quiet(quiet: f64, deadline: Instant) -> f64 {
    loop {
        let load = load_average();
        if load < quiet || Instant::now() >= deadline {
            return load;
        }
        std::thread::sleep(Duration::from_secs(20));
    }
}

/// Runs this program with `args`, a process that `feed`s `pages` pages,
/// and returns how each went, the peak RSS and the crashes and timeouts.
pub(crate) fn measure(args: &[String], pages: usize) -> (Vec<Outcome>, u64, Vec<String>) {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(args)
        .stdout(Stdio::piped())
        .spawn()
        .expect("couldn't start a run");
    let mut pages = vec![Outcome::Crashed; pages];
    let mut rss = 0;
    let mut events = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).lines() {
        let line = line.unwrap();
        let mut f = line.split(' ');
        match f.next() {
            Some("rss") => rss = f.next().unwrap().parse().unwrap(),
            Some("timeout") => {
                pages[f.next().unwrap().parse::<usize>().unwrap()] = Outcome::TimedOut;
                events.push(line.clone());
            }
            Some("crash") => events.push(line.clone()),
            Some(i) => {
                let i: usize = i.parse().unwrap();
                let secs: f64 = f.next().unwrap().parse().unwrap();
                pages[i] = match f.next().unwrap().parse() {
                    Ok(elements) => Outcome::Parsed { secs, elements },
                    Err(_) => Outcome::Refused { secs },
                };
            }
            None => {}
        }
    }
    assert!(child.wait().unwrap().success(), "a run's process failed");
    (pages, rss, events)
}

pub(crate) fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// `boss [--smoke | --gate] [--now] [--pages N] [--runs N] [--seed N] [--tools a,b]`.
pub fn run(args: &[String]) {
    match args.first().map(String::as_str) {
        Some("boss-worker") => return worker(&args[1..]),
        Some("boss-tool") => return tool_run(&args[1..]),
        Some("boss-page") => {
            // `boss-page <page> [pages] [seed]`: one page's bytes, to look
            // into a crash or a difference.
            let arg = |i: usize, default: u64| args.get(i).map_or(default, |s| s.parse().unwrap());
            let batch = Batch {
                pages: arg(2, 1000) as usize,
                seed: arg(3, SEED),
                giant_cap: usize::MAX,
            };
            let page = input(
                &batch,
                &batch.plan(),
                &Corpus::load(),
                args[1].parse().unwrap(),
            );
            std::io::stdout().write_all(&page.bytes).unwrap();
            return;
        }
        _ => {}
    }
    let smoke = args.iter().any(|a| a == "--smoke");
    let gate = args.iter().any(|a| a == "--gate");
    // Don't wait for a quiet machine (for trying things out).
    let now = args.iter().any(|a| a == "--now");
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    let batch = Batch {
        pages: value("--pages").map_or(if smoke { 300 } else { 1000 }, |v| v.parse().unwrap()),
        seed: value("--seed").map_or(SEED, |v| v.parse().unwrap()),
        giant_cap: if smoke { 2_000_000 } else { usize::MAX },
    };
    let runs: usize =
        value("--runs").map_or(if smoke || gate { 1 } else { 3 }, |v| v.parse().unwrap());
    let tools: Vec<usize> = match value("--tools") {
        Some(list) => list
            .split(',')
            .map(|t| {
                TOOLS
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(t) || (t == "none" && *n == "no parser"))
                    .unwrap_or_else(|| panic!("no tool {t}"))
            })
            .collect(),
        None if smoke => vec![0],
        None if gate => vec![0, 2],
        None => vec![0, 1, 2, 3],
    };

    // The batch itself, described once.
    let plan = batch.plan();
    let corpus = Corpus::load();
    let mut bytes = [0usize; 4];
    let mut counts = [0usize; 4];
    let mut kinds = Vec::new();
    let mut sizes = Vec::new();
    let mut fingerprint: u64 = 0xCBF2_9CE4_8422_2325;
    for i in 0..batch.pages {
        let page = input(&batch, &plan, &corpus, i);
        bytes[page.category] += page.bytes.len();
        counts[page.category] += 1;
        kinds.push(page.kind);
        sizes.push(page.bytes.len());
        // FNV-1a over every byte in order, to show two runs had the same
        // batch.
        for &b in &page.bytes {
            fingerprint = (fingerprint ^ u64::from(b)).wrapping_mul(0x100_0000_01B3);
        }
        if i == batch.pages / 2 {
            let again = input(&batch, &plan, &corpus, i);
            assert!(
                again.bytes == page.bytes,
                "page {i} came out different twice"
            );
        }
    }
    println!(
        "The final boss: {} pages, seed {}, {} run{} per parser{}",
        batch.pages,
        batch.seed,
        runs,
        if runs == 1 { "" } else { "s" },
        if corpus.small.is_empty() {
            "; no bench/corpus, so generated shop listings stand in for real pages"
        } else {
            ""
        }
    );
    for c in 0..4 {
        println!(
            "  {:<12} {:>5} pages {:>9.1} MB",
            CATEGORIES[c],
            counts[c],
            bytes[c] as f64 / 1e6
        );
    }
    println!("  fingerprint of every byte, in order: {fingerprint:016x}");

    let deadline = Instant::now() + MAX_WAIT;
    let mut results: Vec<Vec<Run>> = tools.iter().map(|_| Vec::new()).collect();
    let mut events: Vec<String> = Vec::new();
    for r in 0..runs {
        // A different parser goes first each run.
        for k in 0..tools.len() {
            let slot = (k + r) % tools.len();
            let tool = tools[slot];
            let load = if smoke || gate || now {
                load_average()
            } else {
                wait_for_quiet(QUIET, deadline)
            };
            let args = [
                vec!["boss-tool".to_string(), tool.to_string()],
                batch.args().to_vec(),
            ];
            let (pages, rss, ev) = measure(&args.concat(), batch.pages);
            println!(
                "  run {} {:<10} load {load:.2} at start, {:.1} s",
                r + 1,
                TOOLS[tool],
                pages.iter().map(Outcome::secs).sum::<f64>()
            );
            for e in ev {
                let i: usize = e.split(' ').nth(1).unwrap().parse().unwrap_or(0);
                events.push(format!(
                    "{} run {}: {e} ({}, {})",
                    TOOLS[tool],
                    r + 1,
                    CATEGORIES[plan[i.min(plan.len() - 1)]],
                    kinds[i.min(kinds.len() - 1)]
                ));
            }
            results[slot].push(Run { pages, rss, load });
        }
    }

    let per_1000 = 1000.0 / batch.pages as f64;
    println!(
        "\n{:<12} {:>15} {:>13} {:>10} {:>8} {:>9} {:>8} {:>12}",
        "median",
        "s/1,000 pages",
        "slowest page",
        "peak RSS",
        "crashes",
        "timeouts",
        "refused",
        "no document"
    );
    let mut failed = false;
    for (slot, &tool) in tools.iter().enumerate() {
        let runs = &results[slot];
        let total = median(
            runs.iter()
                .map(|r| r.pages.iter().map(Outcome::secs).sum())
                .collect(),
        );
        let slowest = median(
            runs.iter()
                .map(|r| r.pages.iter().map(Outcome::secs).fold(0.0, f64::max))
                .collect(),
        );
        let rss = median(runs.iter().map(|r| r.rss as f64).collect());
        let count = |f: &dyn Fn(&Outcome) -> bool| {
            runs.iter()
                .map(|r| r.pages.iter().filter(|p| f(p)).count())
                .max()
                .unwrap_or(0)
        };
        let crashes = count(&|p| matches!(p, Outcome::Crashed));
        let timeouts = count(&|p| matches!(p, Outcome::TimedOut));
        let refused = count(&|p| matches!(p, Outcome::Refused { .. }));
        // Every document has at least html, head and body.
        let empty = count(&|p| matches!(p, Outcome::Parsed { elements, .. } if *elements < 3));
        if tool == 3 {
            println!(
                "{:<12} {:>15} {:>13} {:>7.0} MB {:>8} {:>9} {:>8} {:>12}",
                TOOLS[tool],
                "-",
                "-",
                rss / 1e6,
                "-",
                "-",
                "-",
                "-"
            );
            continue;
        }
        failed |= crashes + timeouts + refused + empty > 0;
        println!(
            "{:<12} {:>15.2} {:>11.3} s {:>7.0} MB {:>8} {:>9} {:>8} {:>12}",
            TOOLS[tool],
            total * per_1000,
            slowest,
            rss / 1e6,
            crashes,
            timeouts,
            refused,
            empty
        );
    }

    println!("\nPer category, median seconds for the pages in it (slowest page in brackets)");
    print!("{:<12} {:>6} {:>9}", "", "pages", "MB");
    for &tool in tools.iter().filter(|&&t| t != 3) {
        print!(" {:>22}", TOOLS[tool]);
    }
    println!();
    for c in 0..4 {
        print!(
            "{:<12} {:>6} {:>9.1}",
            CATEGORIES[c],
            counts[c],
            bytes[c] as f64 / 1e6
        );
        for (slot, _) in tools.iter().enumerate().filter(|&(_, &t)| t != 3) {
            let runs = &results[slot];
            let in_c = |r: &Run| {
                r.pages
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| plan[i] == c)
                    .map(|(_, p)| p.secs())
                    .collect::<Vec<f64>>()
            };
            let total = median(runs.iter().map(|r| in_c(r).iter().sum()).collect());
            let slowest = median(
                runs.iter()
                    .map(|r| in_c(r).into_iter().fold(0.0, f64::max))
                    .collect(),
            );
            print!(" {:>10.3} s ({:>6.3} s)", total, slowest);
        }
        println!();
    }

    // The giants by kind, and each parser's slowest page, from the medians
    // of each page's times over the runs.
    let page_median =
        |slot: usize, i: usize| median(results[slot].iter().map(|r| r.pages[i].secs()).collect());
    let giant_kinds: Vec<&str> = (0..batch.pages)
        .filter(|&i| plan[i] == 3)
        .map(|i| kinds[i])
        .fold(Vec::new(), |mut v, k| {
            if !v.contains(&k) {
                v.push(k);
            }
            v
        });
    for kind in giant_kinds {
        let of_kind: Vec<usize> = (0..batch.pages)
            .filter(|&i| plan[i] == 3 && kinds[i] == kind)
            .collect();
        print!("  {:<29} {:>3}", kind, of_kind.len());
        for (slot, _) in tools.iter().enumerate().filter(|&(_, &t)| t != 3) {
            let total: f64 = of_kind.iter().map(|&i| page_median(slot, i)).sum();
            print!(" {:>22}", format!("{total:.3} s"));
        }
        println!();
    }
    for (slot, &tool) in tools.iter().enumerate().filter(|&(_, &t)| t != 3) {
        let (i, secs) = (0..batch.pages)
            .map(|i| (i, page_median(slot, i)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap();
        println!(
            "{}'s slowest page: {i} ({}, {}, {:.1} MB), {secs:.3} s",
            TOOLS[tool],
            CATEGORIES[plan[i]],
            kinds[i],
            sizes[i] as f64 / 1e6
        );
    }

    // Element counts against Parsec's, from the first run.
    if let Some(parsec) = tools.iter().position(|&t| t == 0) {
        for (slot, &tool) in tools.iter().enumerate().filter(|&(_, &t)| t == 1 || t == 2) {
            let mut differ = [0usize; 4];
            let mut examples = Vec::new();
            for (i, (a, b)) in results[parsec][0]
                .pages
                .iter()
                .zip(&results[slot][0].pages)
                .enumerate()
            {
                if let (Outcome::Parsed { elements: a, .. }, Outcome::Parsed { elements: b, .. }) =
                    (a, b)
                    && a != b
                {
                    if differ.iter().sum::<usize>() < 5 {
                        examples.push(format!(
                            "  page {i} ({}, {}): Parsec {a}, {} {b}",
                            CATEGORIES[plan[i]], kinds[i], TOOLS[tool]
                        ));
                    }
                    differ[plan[i]] += 1;
                }
            }
            failed |= gate && differ.iter().sum::<usize>() > 0;
            println!(
                "Pages where {}'s element count differs from Parsec's: {} (fragments {}, real {}, encodings {}, giants {})",
                TOOLS[tool],
                differ.iter().sum::<usize>(),
                differ[0],
                differ[1],
                differ[2],
                differ[3]
            );
            for e in examples {
                println!("{e}");
            }
        }
    }
    let loads: Vec<String> = results
        .iter()
        .flatten()
        .map(|r| format!("{:.2}", r.load))
        .collect();
    println!(
        "Load average at the start of each run, by parser: {}",
        loads.join(", ")
    );
    if events.is_empty() {
        println!("No crashes or timeouts.");
    }
    for e in &events {
        println!("{e}");
    }
    if (smoke || gate) && failed {
        eprintln!(
            "A parser crashed, hung, refused a page or left one without a document{}",
            if gate {
                ", or the element counts differ"
            } else {
                ""
            }
        );
        std::process::exit(1);
    }
}
