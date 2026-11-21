// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! tc39/test262, pinned as a submodule in `tests/test262`, through the
//! parser. Each file's frontmatter says whether it's a module, whether it
//! runs strict, sloppy or both, and whether it should fail to parse.
//!
//! - A file that should parse has to, in each mode it runs in: a pass, or
//!   a failure.
//! - A file that should fail to parse passes if the parser rejects it in
//!   each mode. One it accepts is a failure, unless its error is an early
//!   error from the list in `early_error`: those need scope analysis or
//!   the regular expression grammar, which come later, and are skipped
//!   and counted.
//! - Modules are skipped for now, and so are files for features past
//!   ECMAScript 2025 that change the syntax (decorators, `using`).

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::path::{Path, PathBuf};

use parsec_js::{Options, parse};

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
fn run(src: &str, m: &Meta) -> Vec<Result<(), parsec_js::Diagnostic>> {
    let flag = |f: &str| m.flags.iter().any(|x| x == f);
    let options = Options::default();
    let strict = || {
        let prefixed = format!("\"use strict\";\n{src}");
        parse(&prefixed, &options).map(|_| ())
    };
    let sloppy = || parse(src, &options).map(|_| ());
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
    let (mut skipped_modules, mut skipped_features, mut skipped_early) = (0, 0, 0);
    let mut failed = Vec::new();
    let mut early = Vec::new();
    let (mut negatives, mut negatives_caught) = (0, 0);
    for path in &paths {
        // With `/` on every OS, so `early_error` matches on Windows too.
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
        let m = meta(&src);
        if m.features
            .iter()
            .any(|f| NOT_IN_ES2025.contains(&f.as_str()))
        {
            skipped_features += 1;
            continue;
        }
        // Module fixtures are imported by module tests, so they're modules.
        if m.flags.iter().any(|f| f == "module") || name.ends_with("_FIXTURE.js") {
            skipped_modules += 1;
            continue;
        }
        let results = run(&src, &m);
        if m.parse_negative {
            negatives += 1;
            if results.iter().all(Result::is_err) {
                negatives_caught += 1;
                passed += 1;
            } else if let Some(reason) = early_error(&name) {
                skipped_early += 1;
                early.push(format!("{name}: {reason}"));
            } else {
                failed.push(format!("{name}: parsed, but should be a syntax error"));
            }
        } else if let Some(Err(d)) = results.iter().find(|r| r.is_err()) {
            failed.push(format!("{name}: {d}"));
        } else {
            passed += 1;
        }
    }
    let skipped = skipped_modules + skipped_features + skipped_early;
    eprintln!(
        "test262 parsed: {passed} passed, {} failed, {skipped} skipped \
         ({skipped_early} early errors for M11, {skipped_modules} modules, \
         {skipped_features} past ES2025 or not UTF-8); \
         {negatives_caught} of {negatives} parse negatives caught",
        failed.len()
    );
    if std::env::var_os("TEST262_LIST_EARLY").is_some() {
        eprintln!("{}", early.join("\n"));
    }
    summary::write(&summary::Counts {
        suite: "test262-parse",
        passed,
        failed: failed.len(),
        skipped,
        skipped_because: "early errors that need scope analysis or the regular expression grammar (M11), modules (#154), features past ES2025, and files that aren't UTF-8",
        data: &root,
    });
    assert!(
        failed.is_empty(),
        "{} files went wrong:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// Why a parse negative the parser accepts is an early error it doesn't
/// check yet, or `None` if it should have been caught.
fn early_error(name: &str) -> Option<&'static str> {
    const REGEXP: &[&str] = &["language/literals/regexp/", "built-ins/RegExp/"];
    // Each of these declares a name twice in one scope, or a lexical name
    // where a var or parameter of the same name is.
    const SCOPE: &[&str] = &[
        "redeclaration",
        "-redecl",
        "formals-body-duplicate",
        "duplicate-binding",
        "-lex-dup",
        "-lex-var",
        "bound-names-in-stmt",
        "early-catch-function",
        "early-catch-lex",
    ];
    if REGEXP.iter().any(|p| name.contains(p)) {
        return Some("a regular expression's pattern, which isn't checked yet");
    }
    if SCOPE.iter().any(|p| name.contains(p)) {
        return Some("a name declared twice, which needs scope analysis");
    }
    None
}
