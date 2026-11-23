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

/// Times `parsec_js::scope` on each source's trees, parsed beforehand,
/// and lists every name each source uses that nothing in it declares.
pub fn scopes(pages: &[Page]) {
    let (sources, _) = sources(pages);
    println!("\nJavaScript scope analysis: MB/s of source, median of at least 5 runs");
    println!(
        "{:<34} {:>9}  {:>8}  {:>8}  {:>9}  {:>10}  {:>6}",
        "", "KB", "MB/s", "scopes", "bindings", "references", "free"
    );
    let mut free_lists = Vec::new();
    for s in &sources {
        let trees: Vec<(parsec_js::Ast, &str)> = s
            .pieces
            .iter()
            .map(|(src, module)| {
                let options = parsec_js::Options {
                    source_type: if *module {
                        parsec_js::SourceType::Module
                    } else {
                        parsec_js::SourceType::Script
                    },
                    ..parsec_js::Options::default()
                };
                (parsec_js::parse(src, &options).unwrap(), src.as_str())
            })
            .collect();
        let all = || -> Vec<_> {
            trees
                .iter()
                .map(|(ast, src)| parsec_js::scope::Scopes::analyze(black_box(ast), src))
                .collect()
        };
        let t = Instant::now();
        let analyzed = black_box(all());
        let once = t.elapsed().max(Duration::from_micros(1));
        let runs = (0.5 / once.as_secs_f64()).clamp(5.0, 500.0) as usize;
        let mut times: Vec<f64> = (0..runs)
            .map(|_| {
                let t = Instant::now();
                drop(black_box(all()));
                t.elapsed().as_secs_f64()
            })
            .collect();
        times.sort_by(f64::total_cmp);
        let secs = times[times.len() / 2];
        let count =
            |f: fn(&parsec_js::scope::Scopes) -> usize| analyzed.iter().map(f).sum::<usize>();
        let mut free: Vec<&str> = analyzed
            .iter()
            .flat_map(|a| a.free_names().iter().map(String::as_str))
            .collect();
        free.sort_unstable();
        free.dedup();
        println!(
            "{:<34} {:>9.1}  {:>8.0}  {:>8}  {:>9}  {:>10}  {:>6}",
            s.name,
            s.bytes() as f64 / 1e3,
            s.bytes() as f64 / 1e6 / secs,
            count(|a| a.scopes().len()),
            count(|a| a.bindings().len()),
            count(|a| a.references().len()),
            free.len()
        );
        free_lists.push(format!("{}: {}", s.name, free.join(" ")));
    }
    println!("\nFree names, each source's in order:");
    for line in free_lists {
        println!("  {line}");
    }
}

/// `src` with every regular expression, and every template with a
/// substitution, written over as a string literal of the same byte
/// length, `"xx…x"`, and how many bytes that covered. A lexer on its own
/// can't lex either: oxc's and swc's need their parser to say that a `/`
/// starts a regular expression, or that a `}` goes back into a template.
/// oxc's own lexer bench cleans its corpus the same way. Afterwards every
/// `/` divides and every `}` is a punctuator, which is the goal Parsec's
/// parser would choose for each token, so Parsec's lexer is given
/// [`parsec_js::Goal::Div`] throughout.
fn clean(src: &str, module: bool) -> (String, usize) {
    use parsec_js::ast::LiteralKind;
    let options = parsec_js::Options {
        source_type: parsec_source(module),
        ..parsec_js::Options::default()
    };
    let ast = parsec_js::parse(src, &options).expect("sources() keeps only what parses");
    let mut spans: Vec<(usize, usize)> = ast
        .ids()
        .filter(|&id| match ast.node(id) {
            parsec_js::Node::Literal(LiteralKind::RegExp) => true,
            parsec_js::Node::Template { expressions, .. } => !ast.get(*expressions).is_empty(),
            _ => false,
        })
        .map(|id| {
            let s = ast.span(id);
            (s.start as usize, s.end as usize)
        })
        .collect();
    spans.sort_unstable();
    let mut out = src.as_bytes().to_vec();
    let (mut covered, mut end) = (0, 0);
    for (start, stop) in spans {
        if start < end {
            continue; // inside a template already written over
        }
        assert!(
            matches!(out[start], b'/' | b'`'),
            "a span that isn't a literal"
        );
        out[start..stop].fill(b'x');
        out[start] = b'"';
        out[stop - 1] = b'"';
        covered += stop - start;
        end = stop;
    }
    // Only whole characters were replaced, each byte with ASCII.
    (String::from_utf8(out).unwrap(), covered)
}

/// How many non-trivia tokens lexer `which` reads in `src`: the loop
/// that's timed.
fn lex(which: usize, src: &str, module: bool) -> usize {
    match which {
        0 => {
            let mut lexer = parsec_js::Lexer::new(src, parsec_source(module));
            let mut n = 0;
            loop {
                match lexer.next_token(parsec_js::Goal::Div).kind {
                    parsec_js::Kind::Eof => return n,
                    k if k.is_trivia() => {}
                    _ => n += 1,
                }
            }
        }
        1 => {
            let allocator = oxc_allocator::Allocator::default();
            let mut lexer = oxc_lexer(&allocator, src, module);
            // first_token reads a hashbang and drops it.
            let mut kind = lexer.first_token().kind();
            let mut n = 0;
            while kind != oxc_parser::lexer::Kind::Eof {
                n += 1;
                kind = lexer.next_token_for_benchmarks().kind();
            }
            n
        }
        _ => swc_lexer(src).count(),
    }
}

fn parsec_source(module: bool) -> parsec_js::SourceType {
    if module {
        parsec_js::SourceType::Module
    } else {
        parsec_js::SourceType::Script
    }
}

fn oxc_lexer<'a>(
    allocator: &'a oxc_allocator::Allocator,
    src: &'a str,
    module: bool,
) -> oxc_parser::lexer::Lexer<'a, oxc_parser::config::NoTokensLexerConfig> {
    let source_type = if module {
        oxc_span::SourceType::mjs()
    } else {
        oxc_span::SourceType::cjs()
    };
    oxc_parser::lexer::Lexer::new_for_benchmarks(
        allocator,
        src,
        source_type,
        oxc_parser::config::NoTokensLexerConfig,
    )
}

// shortcut: swc's lexer isn't told a source is a module, which changes
// only whether `<!--` is a comment; the token check would catch one.
fn swc_lexer(src: &str) -> swc_ecma_parser::Lexer<'_> {
    use swc_common::BytePos;
    use swc_common::input::StringInput;
    swc_ecma_parser::Lexer::new(
        swc_ecma_parser::Syntax::Es(Default::default()),
        swc_ecma_ast::EsVersion::latest(),
        StringInput::new(src, BytePos(1), BytePos(1 + src.len() as u32)),
        None,
    )
}

/// The byte spans of the non-trivia tokens lexer `which` reads in `src`,
/// as Parsec's lexer splits them, or an error it reported. Two splits are
/// joined back, each where its parser would join them: oxc reads every
/// `>` on its own, so TypeScript's `a<b<c>>` closes twice, and its parser
/// rejoins `>=`, `>>`, `>>=`, `>>>` and `>>>=`; swc reads `?.` as `?`
/// then `.`. Neither is joined across a space, so nothing else is.
fn spans(which: usize, src: &str, module: bool) -> Result<Vec<(u32, u32)>, String> {
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut push = |start: u32, end: u32, joins: &[&str]| match out.last_mut() {
        Some(last) if last.1 == start && joins.contains(&&src[last.0 as usize..end as usize]) => {
            last.1 = end;
        }
        _ => out.push((start, end)),
    };
    match which {
        0 => {
            let mut lexer = parsec_js::Lexer::new(src, parsec_source(module));
            loop {
                let t = lexer.next_token(parsec_js::Goal::Div);
                match t.kind {
                    parsec_js::Kind::Eof => break,
                    k if k.is_trivia() => {}
                    _ => push(t.span.start, t.span.end, &[]),
                }
            }
            if let Some(d) = lexer.diagnostics().first() {
                return Err(format!("{d:?}"));
            }
        }
        1 => {
            let allocator = oxc_allocator::Allocator::default();
            let mut lexer = oxc_lexer(&allocator, src, module);
            let mut t = lexer.first_token();
            while t.kind() != oxc_parser::lexer::Kind::Eof {
                push(t.start(), t.end(), &[">=", ">>", ">>=", ">>>", ">>>="]);
                t = lexer.next_token_for_benchmarks();
            }
            if let Some(e) = lexer.errors().first() {
                return Err(format!("{e:?}"));
            }
        }
        _ => {
            for t in swc_lexer(src) {
                // swc's token type is public only behind its `unstable`
                // feature; the name is enough to spot an error.
                let name = format!("{:?}", t.token);
                if name.starts_with("Error") {
                    return Err(name);
                }
                if name != "Shebang" {
                    push(t.span.lo.0 - 1, t.span.hi.0 - 1, &["?."]);
                }
            }
        }
    }
    Ok(out)
}

/// Where lexer `which` first reads `src` differently from Parsec's, if it
/// does, as a line for the output.
fn differs(which: usize, src: &str, module: bool) -> Option<String> {
    let ours = spans(0, src, module);
    let theirs = spans(which, src, module);
    let (ours, theirs) = match (ours, theirs) {
        (Ok(a), Ok(b)) => (a, b),
        (a, b) => {
            return Some(format!(
                "errors: Parsec {:?}, {} {:?}",
                a.err(),
                PARSERS[which],
                b.err()
            ));
        }
    };
    let text = |&(s, e): &(u32, u32)| src[s as usize..e as usize].to_string();
    let k = (0..ours.len().max(theirs.len())).find(|&k| ours.get(k) != theirs.get(k))?;
    Some(format!(
        "{} tokens against Parsec's {}; the first that differs, at byte {}: {:?} against {:?}",
        theirs.len(),
        ours.len(),
        ours.get(k).or(theirs.get(k)).map_or(0, |t| t.0),
        theirs.get(k).map(text),
        ours.get(k).map(text),
    ))
}

/// Times the three lexers on each source, cleaned by [`clean`], after
/// checking that they read the same tokens from every piece of it.
pub fn lex_run(pages: &[Page]) {
    let (sources, dropped) = sources(pages);
    println!(
        "\nJavaScript lexing: oxc_parser {}, swc_ecma_parser {}. MB/s and millions of tokens a second, median of at least 5 runs",
        env!("OXC_VERSION"),
        env!("SWC_VERSION")
    );
    println!(
        "{:<34} {:>9} {:>7} {:>8}  {:>13}  {:>13}  {:>13}  {:>6} {:>6}",
        "", "KB", "cleaned", "tokens", PARSERS[0], PARSERS[1], PARSERS[2], "vs oxc", "vs swc"
    );
    let mut totals = [0.0; 3];
    let (mut total_bytes, mut total_covered, mut total_tokens) = (0, 0, 0);
    let mut mismatches = Vec::new();
    for s in &sources {
        let mut covered = 0;
        let pieces: Vec<(String, bool)> = s
            .pieces
            .iter()
            .map(|(src, module)| {
                let (cleaned, c) = clean(src, *module);
                covered += c;
                (cleaned, *module)
            })
            .collect();
        let mut tokens = 0;
        for (i, (src, module)) in pieces.iter().enumerate() {
            for (w, name) in PARSERS.iter().enumerate().skip(1) {
                if let Some(d) = differs(w, src, *module) {
                    mismatches.push(format!("{} piece {i}, {name}: {d}", s.name));
                }
            }
            tokens += lex(0, src, *module);
        }
        let secs: Vec<f64> = (0..3)
            .map(|w| {
                median(|| {
                    for (src, module) in &pieces {
                        black_box(lex(w, black_box(src), *module));
                    }
                })
            })
            .collect();
        let bytes = s.bytes();
        for (t, s) in totals.iter_mut().zip(&secs) {
            *t += s;
        }
        total_bytes += bytes;
        total_covered += covered;
        total_tokens += tokens;
        lex_row(&s.name, bytes, covered, tokens, &secs);
    }
    lex_row("all", total_bytes, total_covered, total_tokens, &totals);
    println!(
        "  (\"cleaned\" is the share written over as strings; \"vs\" is Parsec's speed over the other's; {dropped} inline scripts left out because a parser rejects them)"
    );
    if mismatches.is_empty() {
        println!(
            "  All three lexers read the same tokens, at the same spans, with no errors, in every piece."
        );
    } else {
        println!("  Where a lexer reads a piece differently from Parsec:");
        for m in &mismatches {
            println!("    {m}");
        }
    }
}

fn lex_row(name: &str, bytes: usize, covered: usize, tokens: usize, secs: &[f64]) {
    let cells: Vec<String> = secs
        .iter()
        .map(|&s| {
            format!(
                "{:>6.0} {:>6.1}",
                bytes as f64 / 1e6 / s,
                tokens as f64 / 1e6 / s
            )
        })
        .collect();
    println!(
        "{name:<34} {:>9.1} {:>6.1}% {:>8}  {}  {:>5.2}x {:>5.2}x",
        bytes as f64 / 1e3,
        covered as f64 * 100.0 / bytes as f64,
        tokens,
        cells.join("  "),
        secs[1] / secs[0],
        secs[2] / secs[0],
    );
}

/// Median seconds `f` takes, over at least five runs after a warm-up,
/// more for quick ones, up to half a second's worth, as `measure` does.
fn median(mut f: impl FnMut()) -> f64 {
    f();
    let t = Instant::now();
    f();
    let once = t.elapsed().max(Duration::from_micros(1));
    let runs = (0.5 / once.as_secs_f64()).clamp(5.0, 500.0) as usize;
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}
