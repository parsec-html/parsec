// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times JavaScript parsing: Parsec's `parsec-js` against `oxc_parser`
//! and `swc_ecma_parser`, on the bundles in `bench/scripts.txt` and on
//! each real page's inline scripts.
//!
//! Each parser builds its whole tree and keeps it until the clock stops.
//! Parsec and oxc keep the comments too; swc is given no comment store,
//! so it keeps none, which is the least it can be asked to do.

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::pages::Page;

const PARSERS: [&str; 3] = ["Parsec", "oxc", "swc"];

/// One source to parse: a bundle, or all of a page's inline scripts, each
/// parsed on its own.
struct Source {
    name: String,
    pieces: Vec<(String, bool)>,
}

impl Source {
    fn bytes(&self) -> usize {
        self.pieces.iter().map(|(s, _)| s.len()).sum()
    }
}

fn parsec(src: &str, module: bool) -> Box<dyn std::any::Any> {
    let options = parsec_js::Options {
        source_type: if module {
            parsec_js::SourceType::Module
        } else {
            parsec_js::SourceType::Script
        },
        ..parsec_js::Options::default()
    };
    Box::new(parsec_js::parse(src, &options).ok())
}

fn oxc(src: &str, module: bool) -> Box<dyn std::any::Any> {
    let allocator = Box::new(oxc_allocator::Allocator::default());
    let source_type = if module {
        oxc_span::SourceType::mjs()
    } else {
        oxc_span::SourceType::cjs()
    };
    let ok = {
        let ret = oxc_parser::Parser::new(&allocator, src, source_type).parse();
        black_box(&ret.program);
        !ret.fatal_error && ret.diagnostics.is_empty()
    };
    // The program lives in the allocator; keeping that keeps the tree.
    Box::new(ok.then_some(allocator))
}

fn swc(src: &str, module: bool) -> Box<dyn std::any::Any> {
    use swc_common::BytePos;
    use swc_common::input::StringInput;
    let input = StringInput::new(src, BytePos(1), BytePos(1 + src.len() as u32));
    let mut parser =
        swc_ecma_parser::Parser::new(swc_ecma_parser::Syntax::Es(Default::default()), input, None);
    if module {
        Box::new(parser.parse_module().ok())
    } else {
        Box::new(parser.parse_script().ok())
    }
}

fn parse(which: usize, src: &str, module: bool) -> Box<dyn std::any::Any> {
    match which {
        0 => parsec(src, module),
        1 => oxc(src, module),
        _ => swc(src, module),
    }
}

/// Whether parser `which` parses `src`.
fn parses(which: usize, src: &str, module: bool) -> bool {
    let tree = parse(which, src, module);
    match which {
        0 => tree
            .downcast_ref::<Option<parsec_js::Ast>>()
            .is_some_and(Option::is_some),
        1 => tree
            .downcast_ref::<Option<Box<oxc_allocator::Allocator>>>()
            .is_some_and(Option::is_some),
        _ if module => tree
            .downcast_ref::<Option<swc_ecma_ast::Module>>()
            .is_some_and(Option::is_some),
        _ => tree
            .downcast_ref::<Option<swc_ecma_ast::Script>>()
            .is_some_and(Option::is_some),
    }
}

/// Median seconds for parser `which` to parse every piece, and the most
/// the heap grew by while it held them all.
fn measure(which: usize, source: &Source) -> (f64, usize) {
    let all = || -> Vec<_> {
        source
            .pieces
            .iter()
            .map(|(s, m)| parse(which, black_box(s), *m))
            .collect()
    };
    drop(black_box(all()));
    let t = Instant::now();
    drop(black_box(all()));
    let once = t.elapsed().max(Duration::from_micros(1));
    let runs = (0.5 / once.as_secs_f64()).clamp(5.0, 500.0) as usize;
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            let kept = black_box(all());
            let secs = t.elapsed().as_secs_f64();
            drop(kept);
            secs
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], crate::peak(all))
}

/// The bundles, then each real page's inline scripts that all three
/// parsers take.
fn sources(pages: &[Page]) -> (Vec<Source>, usize) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(dir.join("scripts.txt")).expect("bench/scripts.txt");
    let mut sources: Vec<Source> = list
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
        .map(|name| {
            let path = dir.join("corpus").join(name);
            let src = std::fs::read_to_string(&path).unwrap_or_else(|_| {
                panic!("{} is missing: run bench/fetch-pages.sh", path.display())
            });
            Source {
                name: name.to_string(),
                pieces: vec![(src, name.ends_with(".mjs"))],
            }
        })
        .collect();
    let classic = parsec::css::SelectorList::parse(
        r#"script:not([src]):not([type]), script:not([src])[type="text/javascript"], script:not([src])[type="application/javascript"]"#,
    )
    .unwrap();
    let modules = parsec::css::SelectorList::parse(r#"script:not([src])[type="module"]"#).unwrap();
    let mut dropped = 0;
    for p in pages {
        let doc = parsec::parse(&p.html, false);
        let mut pieces: Vec<(String, bool)> = Vec::new();
        for (list, module) in [(&classic, false), (&modules, true)] {
            for n in list.select(&doc, doc.root()) {
                let src = doc.text(n);
                if src.trim().is_empty() {
                    continue;
                }
                if (0..3).all(|w| parses(w, &src, module)) {
                    pieces.push((src, module));
                } else {
                    dropped += 1;
                }
            }
        }
        if !pieces.is_empty() {
            sources.push(Source {
                name: format!("{} <script>", p.name),
                pieces,
            });
        }
    }
    (sources, dropped)
}

fn row(name: &str, bytes: usize, results: &[(f64, usize); 3]) {
    let mbs = |secs: f64| bytes as f64 / 1e6 / secs;
    let cells: Vec<String> = results
        .iter()
        .map(|&(secs, peak)| format!("{:>8.0} {:>8.1}", mbs(secs), peak as f64 / 1e6))
        .collect();
    println!(
        "{name:<34} {:>9.1}  {}  {:>5.2}x {:>5.2}x",
        bytes as f64 / 1e3,
        cells.join("  "),
        results[1].0 / results[0].0,
        results[2].0 / results[0].0,
    );
}

/// Writes every source to `dir`, a file a piece, `.mjs` for a module, so
/// `crates/parsec-js/scripts/compare-acorn.sh` can check them all.
pub fn dump(pages: &[Page], dir: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    for s in sources(pages).0 {
        let stem: String = s
            .name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let stem = stem
            .trim_end_matches(".js")
            .trim_end_matches(".mjs")
            .to_string();
        for (i, (src, module)) in s.pieces.iter().enumerate() {
            let ext = if *module { "mjs" } else { "js" };
            std::fs::write(dir.join(format!("{stem}-{i}.{ext}")), src).unwrap();
        }
    }
}

pub fn run(pages: &[Page]) {
    let (sources, dropped) = sources(pages);
    println!(
        "\nJavaScript: oxc_parser {}, swc_ecma_parser {}. MB/s and peak MB, median of at least 5 runs",
        env!("OXC_VERSION"),
        env!("SWC_VERSION")
    );
    println!(
        "{:<34} {:>9}  {:>17}  {:>17}  {:>17}  {:>6} {:>6}",
        "", "KB", PARSERS[0], PARSERS[1], PARSERS[2], "vs oxc", "vs swc"
    );
    let mut totals = [(0.0, 0); 3];
    let mut total_bytes = 0;
    for s in &sources {
        let bytes = s.bytes();
        let results = [measure(0, s), measure(1, s), measure(2, s)];
        for (t, r) in totals.iter_mut().zip(results) {
            t.0 += r.0;
            t.1 = t.1.max(r.1);
        }
        total_bytes += bytes;
        row(&s.name, bytes, &results);
    }
    row("all", total_bytes, &totals);
    println!(
        "  (\"vs\" is Parsec's speed over the other's; {dropped} inline scripts left out because a parser rejects them)"
    );
}
