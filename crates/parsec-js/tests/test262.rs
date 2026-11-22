// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! tc39/test262, pinned as a submodule in `tests/test262`, through the
//! lexer. Each file's frontmatter says whether it's a module, strict, and
//! whether it should fail to parse.
//!
//! - A file that should parse has to tokenize with no errors: a pass, or
//!   a failure.
//! - A file that should fail to parse passes if the lexer reports an
//!   error. One it doesn't is skipped: its error is in the syntax, which
//!   the parser (M10) will check, and the lexer can't tell those apart
//!   from lexical ones.
//! - Files for features past ECMAScript 2025 that change what tokens
//!   there are (decorators' `@`) are skipped.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

mod driver;

use std::path::{Path, PathBuf};

use parsec_js::{Severity, SourceType};

/// Features whose tests use tokens ECMAScript 2025 doesn't have.
const NOT_IN_ES2025: &[&str] = &["decorators"];

/// Codes the lexer reports as warnings in a script, which strict code
/// makes errors.
const STRICT_ERRORS: &[&str] = &[
    "legacy-octal-literal",
    "leading-zero-literal",
    "legacy-octal-escape",
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

    let (mut passed, mut skipped_syntax, mut skipped_features) = (0, 0, 0);
    let mut failed = Vec::new();
    let (mut negatives, mut negatives_caught) = (0, 0);
    for path in &paths {
        let name = path.strip_prefix(&root).unwrap().display().to_string();
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            // A few files are deliberately not UTF-8; a source text is code
            // points, so they're outside what a &str lexer takes.
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
        let module = m.flags.iter().any(|f| f == "module") || name.ends_with("_FIXTURE.js");
        let strict = module || m.flags.iter().any(|f| f == "onlyStrict");
        let source = if module {
            SourceType::Module
        } else {
            SourceType::Script
        };
        let (_, diagnostics) = driver::tokenize(&src, source);
        let error = diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error || strict && STRICT_ERRORS.contains(&d.code));
        if m.parse_negative {
            negatives += 1;
            if error.is_some() {
                negatives_caught += 1;
                passed += 1;
            } else {
                skipped_syntax += 1;
            }
        } else if let Some(d) = error {
            let at = d.span.start as usize;
            let line = src[..at.min(src.len())].matches('\n').count() + 1;
            failed.push(format!("{name}:{line}: {d}"));
        } else {
            passed += 1;
        }
    }
    let skipped = skipped_syntax + skipped_features;
    eprintln!(
        "test262 tokenized: {passed} passed, {} failed, {skipped} skipped \
         ({skipped_syntax} negatives with syntax errors for the parser, {skipped_features} past ES2025 or not UTF-8); \
         {negatives_caught} of {negatives} parse negatives caught by the lexer",
        failed.len()
    );
    summary::write(&summary::Counts {
        suite: "test262-tokens",
        passed,
        failed: failed.len(),
        skipped,
        skipped_because: "syntax errors the parser will check, features past ES2025, and files that aren't UTF-8",
        data: &root,
    });
    assert!(
        failed.is_empty(),
        "{} files didn't tokenize:\n{}",
        failed.len(),
        failed.join("\n")
    );
}
