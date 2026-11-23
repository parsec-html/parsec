// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `Ast::reparse` against a full parse, on random edits to real sources:
//! test262's harness and a sample of its tests, and the benchmark's
//! bundles when `bench/fetch-pages.sh` has fetched them. After each
//! edit the tree has to be the one `parse` gives for the new source, as
//! ESTree JSON with its comments and warnings, or the error the same
//! error, with the tree left as it was.
//!
//! The edits come from a seeded generator, so a failure names the seed
//! and the edit, and comes back with them. `PARSEC_REPARSE_SEED` picks
//! another seed.

use std::path::{Path, PathBuf};

use parsec_js::{Edit, Options, SourceType, Span, parse};

/// xorshift64*: enough to spread edits around, and the same every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// What tends to break a reparse: brackets, quotes, comment and template
/// delimiters, a `/` that may start a regular expression, line breaks
/// that change where semicolons go, and code.
const SNIPPETS: &[&str] = &[
    " ",
    "\n",
    ";",
    "{",
    "}",
    "(",
    ")",
    "[",
    "]",
    "/",
    "/*",
    "*/",
    "//",
    "'",
    "\"",
    "`",
    "${",
    "x",
    "1",
    ".",
    ",",
    "=",
    "=>",
    "+",
    "++",
    "!",
    "?.",
    "#a",
    "a.b",
    "f()",
    "let ",
    "await ",
    "yield ",
    "return ",
    "break;",
    "\"use strict\";",
    "function(){}",
    "class{}",
    "/re/g",
    "<!--",
    "-->",
    "0o7",
    "07",
    "\\u0061",
];

/// An edit somewhere in `src`: most change a name, a digit or the space
/// between tokens, the kind of edit typing makes and that keeps the
/// source parsing; the rest put in or take out punctuation.
fn random_edit<'a>(rng: &mut Rng, src: &str) -> (Span, &'a str) {
    let at = |rng: &mut Rng| {
        let mut i = rng.below(src.len() + 1);
        while !src.is_char_boundary(i) {
            i -= 1;
        }
        i as u32
    };
    const LETTERS: [&str; 6] = ["a", "e", "n", "s", "t", "z"];
    const DIGITS: [&str; 4] = ["0", "3", "7", "9"];
    match rng.below(10) {
        0..=3 => {
            // A letter or digit replaced with another.
            for _ in 0..20 {
                let i = at(rng) as usize;
                match src.as_bytes().get(i) {
                    Some(c) if c.is_ascii_alphabetic() => {
                        return (Span::new(i as u32, i as u32 + 1), LETTERS[rng.below(6)]);
                    }
                    Some(c) if c.is_ascii_digit() => {
                        return (Span::new(i as u32, i as u32 + 1), DIGITS[rng.below(4)]);
                    }
                    _ => {}
                }
            }
            let i = at(rng);
            (Span::at(i), " ")
        }
        4 => (Span::at(at(rng)), " "),
        5 => (Span::at(at(rng)), "\n"),
        6 | 8 => {
            let start = at(rng) as usize;
            let most = if rng.below(2) == 0 { 1 } else { 12 };
            let mut end = (start + 1 + rng.below(most)).min(src.len());
            while !src.is_char_boundary(end) {
                end += 1;
            }
            (Span::new(start as u32, end as u32), "")
        }
        7 => (Span::at(at(rng)), SNIPPETS[rng.below(SNIPPETS.len())]),
        _ => {
            let start = at(rng) as usize;
            let mut end = (start + rng.below(6)).min(src.len());
            while !src.is_char_boundary(end) {
                end += 1;
            }
            (
                Span::new(start as u32, end as u32),
                SNIPPETS[rng.below(SNIPPETS.len())],
            )
        }
    }
}

#[derive(Default, Debug)]
struct Stats {
    edits: usize,
    /// Parsed again in part.
    pieces: usize,
    /// Parsed whole, and fine.
    whole: usize,
    /// A syntax error, which reparse and parse agree on.
    errors: usize,
    /// Bytes parsed again, against the sources' bytes.
    piece_bytes: usize,
    source_bytes: usize,
}

/// Makes `n` edits one after another to `src`, each checked against a
/// full parse.
fn check(name: &str, src: &str, options: &Options, n: usize, rng: &mut Rng, stats: &mut Stats) {
    let Ok(mut ast) = parse(src, options) else {
        return;
    };
    let mut src = src.to_string();
    for _ in 0..n {
        let (range, text) = random_edit(rng, &src);
        let edit = Edit { range, text };
        let new = format!(
            "{}{}{}",
            &src[..range.start as usize],
            text,
            &src[range.end as usize..]
        );
        let before = (src.len() < 1 << 16).then(|| ast.to_estree(&src));
        let full = parse(&new, options);
        let seed = rng.0;
        let context = || {
            let lo = range.start.saturating_sub(40) as usize;
            let hi = (range.end as usize + 40).min(src.len());
            format!(
                "{name}, rng state {seed:#x}: {edit:?} in {:?}",
                src.get(lo..hi).unwrap_or("")
            )
        };
        stats.edits += 1;
        match (ast.reparse(&src, &edit, options), full) {
            (Ok(done), Ok(full)) => {
                assert_eq!(done.source, new);
                assert!(
                    ast.to_estree(&new) == full.to_estree(&new),
                    "the trees differ: {}",
                    context()
                );
                assert_eq!(ast.comments(), full.comments(), "{}", context());
                assert_eq!(ast.warnings(), full.warnings(), "{}", context());
                match done.reparsed {
                    Some(s) => {
                        stats.pieces += 1;
                        stats.piece_bytes += s.len() as usize;
                        stats.source_bytes += new.len();
                    }
                    None => stats.whole += 1,
                }
                src = new;
            }
            (Err(a), Err(b)) => {
                assert_eq!(a, b, "{}", context());
                if let Some(before) = before {
                    assert!(
                        ast.to_estree(&src) == before,
                        "an error changed the tree: {}",
                        context()
                    );
                }
                stats.errors += 1;
            }
            (r, f) => panic!(
                "reparse gave {:?}, parse {:?}: {}",
                r.map(|d| d.reparsed),
                f.map(|_| ()).map_err(|d| d.to_string()),
                context()
            ),
        }
    }
}

/// Whether test262's frontmatter flags `src` with `flag`.
fn flagged(src: &str, flag: &str) -> bool {
    let mut in_flags = false;
    for line in src.lines() {
        let t = line.trim();
        if let Some(inline) = t.strip_prefix("flags:") {
            in_flags = true;
            if inline
                .split(|c: char| !c.is_alphanumeric())
                .any(|w| w == flag)
            {
                return true;
            }
        } else if in_flags && t.strip_prefix("- ") == Some(flag) {
            return true;
        } else if !line.starts_with([' ', '\t']) {
            in_flags = false;
        }
    }
    false
}

fn js_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries {
        let p = e.unwrap().path();
        if p.is_dir() {
            js_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "js") {
            out.push(p);
        }
    }
}

fn seed() -> u64 {
    std::env::var("PARSEC_REPARSE_SEED")
        .ok()
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0x5EED_0F1D)
}

#[test]
fn reparse_matches_a_full_parse_on_test262() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262");
    assert!(
        root.join("test").is_dir(),
        "tests/test262 is a submodule: run `git submodule update --init crates/parsec-js/tests/test262`"
    );
    let mut paths = Vec::new();
    js_files(&root.join("harness"), &mut paths);
    let mut tests = Vec::new();
    js_files(&root.join("test/language"), &mut tests);
    tests.sort();
    paths.sort();
    paths.extend(tests.into_iter().step_by(40));
    let mut rng = Rng(seed());
    let mut stats = Stats::default();
    for p in &paths {
        let Ok(src) = std::fs::read_to_string(p) else {
            continue;
        };
        let options = Options {
            source_type: if flagged(&src, "module") {
                SourceType::Module
            } else {
                SourceType::Script
            },
            ..Options::default()
        };
        let src = if flagged(&src, "onlyStrict") {
            format!("\"use strict\";\n{src}")
        } else {
            src
        };
        let name = p.strip_prefix(&root).unwrap().display().to_string();
        check(&name, &src, &options, 12, &mut rng, &mut stats);
    }
    eprintln!("reparse on test262: {stats:?}");
    assert!(stats.edits > 5_000, "{stats:?}");
    // Most edits that parse were parsed again in part.
    assert!(stats.pieces * 2 > stats.pieces + stats.whole, "{stats:?}");
}

#[test]
fn reparse_matches_a_full_parse_on_bundles() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/corpus");
    let release = !cfg!(debug_assertions);
    let mut rng = Rng(seed() ^ 0xB0B);
    let mut stats = Stats::default();
    for (name, n) in [
        ("react-dom-19.2.0.js", 300),
        ("three-0.181.0.mjs", 300),
        ("typescript-5.9.3.js", 60),
    ] {
        let Ok(src) = std::fs::read_to_string(dir.join(name)) else {
            continue;
        };
        let options = Options {
            source_type: if name.ends_with(".mjs") {
                SourceType::Module
            } else {
                SourceType::Script
            },
            ..Options::default()
        };
        // A debug build parses ten times slower; a few edits there.
        let n = if release { n } else { n / 20 };
        check(name, &src, &options, n, &mut rng, &mut stats);
    }
    eprintln!("reparse on the bench bundles: {stats:?}");
}

#[test]
fn an_ast_from_reparse_reparses_again() {
    // A thousand edits in a row to one tree, each on the last's result.
    let src = "function a(x) {\n  return x + 1;\n}\nclass B { m() { return a(2); } }\nfor (const k of [1, 2]) { a(k); }\n";
    let options = Options::default();
    let mut rng = Rng(seed() ^ 0xA57);
    let mut stats = Stats::default();
    check("inline", src, &options, 1000, &mut rng, &mut stats);
    assert!(stats.pieces > 100, "{stats:?}");
}
