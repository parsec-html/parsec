// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! tc39/test262, pinned as a submodule in `tests/test262`, through the
//! parser and `early_errors`. Each file's frontmatter says whether it's
//! a module, whether it runs strict, sloppy or both, and whether it
//! should fail to parse.
//!
//! - A file that should parse has to, in each mode it runs in, with no
//!   early error: a pass, or a failure.
//! - A file that should fail to parse has to be rejected, by the parser
//!   or by `early_errors`, in each mode it runs in: a pass, or a failure.
//!   None is skipped for the kind of error it has.
//! - A module (`flags: [module]`) is parsed once, as a module. The
//!   `_FIXTURE.js` files modules import aren't tests: some are meant not
//!   to parse, or use syntax past ES2025, and only the tests importing
//!   them say which. They're skipped.
//! - Files for features past ECMAScript 2025 that change the syntax
//!   (decorators, `using`) are skipped.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::path::{Path, PathBuf};

use parsec_js::{Diagnostic, Options, SourceType, early_errors, parse};

/// Parses `src` and checks its early errors: the parse's error, or the
/// first of `early_errors`.
fn check(src: &str, options: &Options) -> Result<(), Diagnostic> {
    let ast = parse(src, options)?;
    match early_errors(&ast, src).into_iter().next() {
        Some(d) => Err(d),
        None => Ok(()),
    }
}

/// Features whose tests use syntax ECMAScript 2025 doesn't have.
const NOT_IN_ES2025: &[&str] = &[
    "decorators",
    "explicit-resource-management",
    "source-phase-imports",
    "source-phase-imports-module-source",
    "import-defer",
    "export-defer",
];

/// What a test file's frontmatter says, as far as the lexer cares.
#[derive(Debug, Default, PartialEq)]
struct Meta {
    flags: Vec<String>,
    features: Vec<String>,
    /// `negative.phase` is `parse`.
    parse_negative: bool,
}

/// The YAML between `/*---` and `---*/`: `flags` and `features` as
/// `[a, b]` or as `- a` lines, and `negative`'s `phase`.
fn meta(src: &str) -> Meta {
    let mut m = Meta::default();
    let Some(start) = src.find("/*---") else {
        return m;
    };
    let Some(len) = src[start..].find("---*/") else {
        return m;
    };
    let yaml = &src[start + 5..start + len];
    // The block list being read: true for flags, false for features.
    let mut list: Option<bool> = None;
    let mut in_negative = false;
    for line in yaml.lines() {
        let indented = line.starts_with([' ', '\t']);
        let t = line.trim();
        if let Some(item) = t.strip_prefix("- ") {
            match list {
                Some(true) => m.flags.push(item.trim().to_string()),
                Some(false) => m.features.push(item.trim().to_string()),
                None => {}
            }
            continue;
        }
        if !indented {
            list = None;
            in_negative = false;
        }
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match (indented, key) {
            (false, "flags" | "features") => {
                let l = if key == "flags" {
                    &mut m.flags
                } else {
                    &mut m.features
                };
                match value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
                    Some(items) => l.extend(
                        items
                            .split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty()),
                    ),
                    None => list = Some(key == "flags"),
                }
            }
            (false, "negative") => in_negative = true,
            (true, "phase") if in_negative => m.parse_negative = value == "parse",
            _ => {}
        }
    }
    m
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            files(&path, out);
        } else if path.extension().is_some_and(|e| e == "js") {
            out.push(path);
        }
    }
}

#[test]
fn frontmatter() {
    let m = meta(
        "/*---\nflags: [module, raw]\nfeatures:\n  - a\n  - b\nnegative:\n  phase: parse\n  type: SyntaxError\n---*/",
    );
    assert_eq!(m.flags, ["module", "raw"]);
    assert_eq!(m.features, ["a", "b"]);
    assert!(m.parse_negative);
    let m = meta("/*---\nnegative:\n  phase: runtime\nfeatures: [x]\n---*/");
    assert!(!m.parse_negative);
    assert_eq!(m.features, ["x"]);
    assert_eq!(meta("no frontmatter"), Meta::default());
}

/// Parses `src` in the modes its flags ask for: whether each parsed.
fn run(src: &str, m: &Meta, module: bool) -> Vec<Result<(), parsec_js::Diagnostic>> {
    let flag = |f: &str| m.flags.iter().any(|x| x == f);
    if module {
        let options = Options {
            source_type: SourceType::Module,
            ..Options::default()
        };
        return vec![check(src, &options)];
    }
    let options = Options::default();
    let strict = || {
        let prefixed = format!("\"use strict\";\n{src}");
        check(&prefixed, &options)
    };
    let sloppy = || check(src, &options);
    if flag("raw") || flag("noStrict") {
        vec![sloppy()]
    } else if flag("onlyStrict") {
        vec![strict()]
    } else {
        vec![sloppy(), strict()]
    }
}

#[test]
fn test262() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262");
    let test = root.join("test");
    assert!(
        test.is_dir(),
        "tests/test262 is a submodule: run `git submodule update --init crates/parsec-js/tests/test262`"
    );
    let mut paths = Vec::new();
    files(&test, &mut paths);

    let mut passed = 0;
    let (mut skipped_features, mut skipped_fixtures) = (0, 0);
    let mut failed = Vec::new();
    let (mut negatives, mut negatives_caught) = (0, 0);
    for path in &paths {
        let name = path
            .strip_prefix(&root)
            .unwrap()
            .display()
            .to_string()
            .replace('\\', "/");
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            // A few files are deliberately not UTF-8; a source text is code
            // points, so they're outside what a &str parser takes.
            Err(_) => {
                skipped_features += 1;
                continue;
            }
        };
        if name.ends_with("_FIXTURE.js") {
            skipped_fixtures += 1;
            continue;
        }
        let m = meta(&src);
        if m.features
            .iter()
            .any(|f| NOT_IN_ES2025.contains(&f.as_str()))
        {
            skipped_features += 1;
            continue;
        }
        let module = m.flags.iter().any(|f| f == "module");
        let results = run(&src, &m, module);
        if m.parse_negative {
            negatives += 1;
            if results.iter().all(Result::is_err) {
                negatives_caught += 1;
                passed += 1;
            } else {
                failed.push(format!("{name}: parsed, but should be a syntax error"));
            }
        } else if let Some(Err(d)) = results.iter().find(|r| r.is_err()) {
            failed.push(format!("{name}: {d}"));
        } else {
            passed += 1;
        }
    }
    let skipped = skipped_features + skipped_fixtures;
    eprintln!(
        "test262 parsed: {passed} passed, {} failed, {skipped} skipped \
         ({skipped_fixtures} module fixtures, {skipped_features} past ES2025 or not UTF-8); \
         {negatives_caught} of {negatives} parse negatives caught",
        failed.len()
    );
    summary::write(&summary::Counts {
        suite: "test262-parse",
        passed,
        failed: failed.len(),
        skipped,
        skipped_because: "module fixtures, features past ES2025, and files that aren't UTF-8",
        data: &root,
    });
    assert!(
        failed.is_empty(),
        "{} files went wrong:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
