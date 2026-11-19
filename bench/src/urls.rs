// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times URL parsing: Parsec (`parsec-url`) against Lexbor's
//! `lxb_url_parse` and Ada, each parsing the same strings against the same
//! bases, already parsed, and freeing what it built.
//!
//! Two lists: every `href` and `src` on the real pages, against the page's
//! own address, the way a crawler resolves links; and the inputs of WPT's
//! `urltestdata.json`, which are mostly the odd cases.

use std::ffi::c_void;
use std::hint::black_box;
use std::path::Path;
use std::ptr::null_mut;
use std::time::Instant;

use crate::pages::Page;

/// Strings to parse, each against the base at an index into `bases`, or
/// against none.
struct Corpus {
    name: String,
    bases: Vec<String>,
    cases: Vec<(String, Option<usize>)>,
}

/// Every `href` and `src` value on `pages`, against the page's address in
/// pages.txt. A `<base href>` would change that address, but none of the
/// 13 pages has one.
fn page_links(pages: &[Page]) -> Corpus {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(dir.join("pages.txt")).expect("bench/pages.txt");
    let address = |name: &str| {
        list.lines()
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .find(|f| f.first() == Some(&name))
            .map(|f| f[2].to_string())
            .unwrap()
    };
    let attrs = parsec::css::SelectorList::parse("[href], [src]").unwrap();
    let mut corpus = Corpus {
        name: format!("hrefs and srcs on the {} real pages", pages.len()),
        bases: Vec::new(),
        cases: Vec::new(),
    };
    for page in pages {
        let doc = parsec::parse(&page.html, false);
        let base = corpus.bases.len();
        corpus.bases.push(address(&page.name));
        for node in attrs.select(&doc, doc.root()) {
            for (_, name, value) in doc.attrs(node) {
                if name == "href" || name == "src" {
                    corpus.cases.push((value.to_string(), Some(base)));
                }
            }
        }
    }
    corpus
}

/// The inputs of WPT's urltestdata.json, with their bases.
fn wpt_inputs() -> Corpus {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../crates/parsec-url/tests/wpt/urltestdata.json");
    let text = std::fs::read_to_string(path).unwrap();
    // A lone surrogate can't go in a Rust string; those cases are left out.
    let data: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut corpus = Corpus {
        name: "WPT urltestdata.json inputs".into(),
        bases: Vec::new(),
        cases: Vec::new(),
    };
    for case in data
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c.as_object())
    {
        let input = case["input"].as_str().unwrap().to_string();
        let base = case["base"]
            .as_str()
            .map(|b| match corpus.bases.iter().position(|x| x == b) {
                Some(i) => i,
                None => {
                    corpus.bases.push(b.to_string());
                    corpus.bases.len() - 1
                }
            });
        corpus.cases.push((input, base));
    }
    corpus
}

/// One parser, set up with a corpus's bases.
trait Parser {
    /// How many of the cases parse.
    fn count(&mut self, cases: &[(String, Option<usize>)]) -> usize;
    /// The href each case serializes to, or `None` where it fails.
    fn hrefs(&mut self, cases: &[(String, Option<usize>)]) -> Vec<Option<String>>;
}

struct Parsec(Vec<Option<parsec_url::Url>>);

impl Parsec {
    fn parse(&self, input: &str, base: Option<usize>) -> Option<parsec_url::Url> {
        match base {
            Some(i) => parsec_url::Url::parse_with_base(input, self.0[i].as_ref()?).ok(),
            None => parsec_url::Url::parse(input).ok(),
        }
    }
}

impl Parser for Parsec {
    fn count(&mut self, cases: &[(String, Option<usize>)]) -> usize {
        cases
            .iter()
            .filter(|(input, base)| black_box(self.parse(input, *base)).is_some())
            .count()
    }

    fn hrefs(&mut self, cases: &[(String, Option<usize>)]) -> Vec<Option<String>> {
        cases
            .iter()
            .map(|(input, base)| self.parse(input, *base).map(|u| u.href().to_string()))
            .collect()
    }
}

#[repr(C)]
struct LxbUrlParser {
    _private: [u8; 0],
}

#[repr(C)]
struct LxbUrl {
    _private: [u8; 0],
}

type SerializeCallback = unsafe extern "C" fn(*const u8, usize, *mut c_void) -> u32;

unsafe extern "C" {
    fn lxb_url_parser_create() -> *mut LxbUrlParser;
    fn lxb_url_parser_init(parser: *mut LxbUrlParser, mraw: *mut c_void) -> u32;
    fn lxb_url_parser_clean(parser: *mut LxbUrlParser);
    fn lxb_url_parser_destroy(parser: *mut LxbUrlParser, self_destroy: bool) -> *mut LxbUrlParser;
    fn lxb_url_parser_memory_destroy(parser: *mut LxbUrlParser);
    fn lxb_url_parse(
        parser: *mut LxbUrlParser,
        base: *const LxbUrl,
        data: *const u8,
        len: usize,
    ) -> *mut LxbUrl;
    fn lxb_url_destroy(url: *mut LxbUrl) -> *mut LxbUrl;
    fn lxb_url_serialize(
        url: *const LxbUrl,
        cb: SerializeCallback,
        ctx: *mut c_void,
        exclude_fragment: bool,
    ) -> u32;
}

/// Lexbor's URL parser, with the bases parsed into its memory.
struct Lexbor {
    parser: *mut LxbUrlParser,
    bases: Vec<*mut LxbUrl>,
}

impl Lexbor {
    fn new(bases: &[String]) -> Lexbor {
        // SAFETY: the parser is created and initialised once, with its own
        // memory, and destroyed in `drop`.
        let parser = unsafe {
            let p = lxb_url_parser_create();
            assert_eq!(lxb_url_parser_init(p, null_mut()), 0);
            p
        };
        let mut lexbor = Lexbor {
            parser,
            bases: Vec::new(),
        };
        lexbor.bases = bases.iter().map(|b| lexbor.parse(b, None)).collect();
        lexbor
    }

    /// Parses `input`, against the base at `base`; null if it fails or the
    /// base did. The caller destroys a URL it gets.
    fn parse(&self, input: &str, base: Option<usize>) -> *mut LxbUrl {
        let base = match base {
            Some(i) if self.bases[i].is_null() => return null_mut(),
            Some(i) => self.bases[i],
            None => null_mut(),
        };
        // SAFETY: Lexbor reads `input.len()` bytes; `base` is null or a URL
        // this parser made and hasn't destroyed. Cleaning after each parse
        // is what Lexbor asks for before the next.
        unsafe {
            let url = lxb_url_parse(self.parser, base, input.as_ptr(), input.len());
            lxb_url_parser_clean(self.parser);
            url
        }
    }
}

impl Parser for Lexbor {
    fn count(&mut self, cases: &[(String, Option<usize>)]) -> usize {
        cases
            .iter()
            .filter(|(input, base)| {
                let url = black_box(self.parse(input, *base));
                // SAFETY: null or a URL just made, destroyed once.
                unsafe { lxb_url_destroy(url) };
                !url.is_null()
            })
            .count()
    }

    fn hrefs(&mut self, cases: &[(String, Option<usize>)]) -> Vec<Option<String>> {
        unsafe extern "C" fn append(data: *const u8, len: usize, ctx: *mut c_void) -> u32 {
            // SAFETY: Lexbor passes `len` readable bytes, and `ctx` is the
            // Vec below, alive for the call.
            unsafe {
                (*ctx.cast::<Vec<u8>>()).extend_from_slice(std::slice::from_raw_parts(data, len))
            };
            0
        }
        cases
            .iter()
            .map(|(input, base)| {
                let url = self.parse(input, *base);
                if url.is_null() {
                    return None;
                }
                let mut out = Vec::new();
                // SAFETY: `url` is live until destroyed here; `out` outlives
                // the call.
                unsafe {
                    lxb_url_serialize(url, append, (&raw mut out).cast(), false);
                    lxb_url_destroy(url);
                }
                Some(String::from_utf8_lossy(&out).into_owned())
            })
            .collect()
    }
}

impl Drop for Lexbor {
    fn drop(&mut self) {
        // SAFETY: the parser was made in `new`; destroying its memory frees
        // the bases with it.
        unsafe {
            lxb_url_parser_memory_destroy(self.parser);
            lxb_url_parser_destroy(self.parser, true);
        }
    }
}

#[repr(C)]
struct AdaString {
    data: *const u8,
    length: usize,
}

unsafe extern "C" {
    fn ada_parse(input: *const u8, len: usize) -> *mut c_void;
    fn ada_parse_with_base(
        input: *const u8,
        len: usize,
        base: *const u8,
        base_len: usize,
    ) -> *mut c_void;
    fn ada_is_valid(url: *mut c_void) -> bool;
    fn ada_get_href(url: *mut c_void) -> AdaString;
    fn ada_free(url: *mut c_void);
    /// bench/src/ada.cpp.
    fn parsec_bench_ada_parse(input: *const u8, len: usize, base: *const c_void) -> bool;
}

/// Ada, with the bases parsed by its C API.
struct Ada {
    bases: Vec<*mut c_void>,
    base_text: Vec<String>,
}

impl Ada {
    fn new(bases: &[String]) -> Ada {
        Ada {
            // SAFETY: Ada reads `len` bytes; each result is freed in `drop`.
            bases: bases
                .iter()
                .map(|b| unsafe { ada_parse(b.as_ptr(), b.len()) })
                .collect(),
            base_text: bases.to_vec(),
        }
    }
}

impl Parser for Ada {
    fn count(&mut self, cases: &[(String, Option<usize>)]) -> usize {
        cases
            .iter()
            .filter(|(input, base)| {
                let base = base.map_or(std::ptr::null(), |i| self.bases[i].cast_const());
                // SAFETY: Ada reads `len` bytes; `base` is null or a live
                // result from `ada_parse`.
                black_box(unsafe { parsec_bench_ada_parse(input.as_ptr(), input.len(), base) })
            })
            .count()
    }

    fn hrefs(&mut self, cases: &[(String, Option<usize>)]) -> Vec<Option<String>> {
        cases
            .iter()
            .map(|(input, base)| {
                // SAFETY: Ada reads the lengths given; the result is freed
                // here, after its href is copied out.
                unsafe {
                    let url = match base {
                        Some(i) => {
                            let b = &self.base_text[*i];
                            ada_parse_with_base(input.as_ptr(), input.len(), b.as_ptr(), b.len())
                        }
                        None => ada_parse(input.as_ptr(), input.len()),
                    };
                    let href = ada_is_valid(url).then(|| {
                        let s = ada_get_href(url);
                        String::from_utf8_lossy(std::slice::from_raw_parts(s.data, s.length))
                            .into_owned()
                    });
                    ada_free(url);
                    href
                }
            })
            .collect()
    }
}

impl Drop for Ada {
    fn drop(&mut self) {
        for &b in &self.bases {
            // SAFETY: from `ada_parse`, freed once.
            unsafe { ada_free(b) };
        }
    }
}

const RUNS: usize = 30;

/// The median seconds of `RUNS` passes over the corpus, after one warm-up,
/// and how many cases parsed.
fn time(parser: &mut dyn Parser, corpus: &Corpus) -> (f64, usize) {
    let parsed = parser.count(&corpus.cases);
    let mut times: Vec<f64> = (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            black_box(parser.count(black_box(&corpus.cases)));
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[RUNS / 2], parsed)
}

pub fn run(pages: &[Page]) {
    println!(
        "\nURLs, median of {RUNS} passes: ns per URL (URLs that parsed)\n{:<34} {:>8} {:>16} {:>16} {:>16} {:>10} {:>10}",
        "", "URLs", "Parsec", "Lexbor", "Ada", "vs Lexbor", "vs Ada"
    );
    for corpus in [page_links(pages), wpt_inputs()] {
        let mut parsers: [Box<dyn Parser>; 3] = [
            Box::new(Parsec(
                corpus
                    .bases
                    .iter()
                    .map(|b| parsec_url::Url::parse(b).ok())
                    .collect(),
            )),
            Box::new(Lexbor::new(&corpus.bases)),
            Box::new(Ada::new(&corpus.bases)),
        ];
        let results: Vec<(f64, usize)> = parsers
            .iter_mut()
            .map(|p| time(&mut **p, &corpus))
            .collect();
        let n = corpus.cases.len() as f64;
        let cell = |(secs, parsed): (f64, usize)| format!("{:>7.0} ({parsed:>6})", secs / n * 1e9);
        println!(
            "  {:<32} {:>8} {:>16} {:>16} {:>16} {:>9.2}x {:>9.2}x",
            corpus.name,
            corpus.cases.len(),
            cell(results[0]),
            cell(results[1]),
            cell(results[2]),
            results[1].0 / results[0].0,
            results[2].0 / results[0].0,
        );
        // Where the others' hrefs differ from Parsec's, the times compare
        // slightly different work; the counts are shown, not hidden.
        let ours = parsers[0].hrefs(&corpus.cases);
        for (name, i) in [("Lexbor", 1), ("Ada", 2)] {
            let theirs = parsers[i].hrefs(&corpus.cases);
            let differ = ours.iter().zip(&theirs).filter(|(a, b)| a != b).count();
            println!("    {name} disagrees with Parsec on {differ}");
        }
    }
}
