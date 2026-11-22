// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Diagnostics as people read them: code frames for a chosen set of
//! test262's parse negatives, compared with `tests/snapshots/
//! diagnostics.txt`, and the parser's recovery after an error.
//!
//! A change to a message, a label or the renderer changes the snapshot.
//! Read the new one, and if it's what you meant, write it with
//! `PARSEC_BLESS=1 cargo test -p parsec-js --test diagnostics`.

use std::path::Path;

use parsec_js::{
    Diagnostic, Limits, LineIndex, Options, RenderOptions, SourceType, early_errors, parse,
    parse_recovering,
};

/// test262 negatives, one or two for each kind of error, and some that
/// have more than one.
const CASES: &[&str] = &[
    // Tokens out of place, and brackets left open.
    "test/language/statements/block/12.1-4.js",
    "test/language/rest-parameters/position-invalid.js",
    "test/language/asi/S7.9_A10_T4.js",
    "test/language/asi/S7.9_A9_T6.js",
    "test/language/statements/if/if-cls-no-else.js",
    "test/language/global-code/return.js",
    "test/language/statements/break/S12.8_A8_T2.js",
    "test/language/module-code/early-undef-break.js",
    "test/language/asi/S7.9_A4.js",
    // Names declared, bound, labelled or exported twice.
    "test/language/statements/function/13.1-5gs.js",
    "test/language/import/dup-bound-names.js",
    "test/language/statements/let/redeclaration-error-from-within-strict-mode-function.js",
    "test/language/statements/class/elements/syntax/early-errors/grammar-class-body-ctor-duplicate.js",
    "test/language/statements/switch/S12.11_A2_T1.js",
    "test/staging/sm/module/duplicate-exported-names-in-single-export-declaration.js",
    "test/language/module-code/early-dup-lables.js",
    "test/language/statements/labeled/continue.js",
    "test/language/statements/class/private-non-static-getter-static-setter-early-error.js",
    "test/language/expressions/object/__proto__-duplicate.js",
    // Strict code.
    "test/language/literals/string/legacy-octal-escape-sequence-prologue-strict.js",
    "test/language/expressions/object/method-definition/early-errors-object-method-NSPL-with-USD.js",
    "test/language/expressions/delete/identifier-strict.js",
    "test/language/statements/variable/eval-strict-list-final-init.js",
    "test/language/statements/with/12.10.1-11gs.js",
    // Declarations and their patterns.
    "test/language/statements/const/syntax/block-scope-syntax-const-declarations-without-initialiser.js",
    "test/language/statements/let/syntax/identifier-let-disallowed-as-boundname.js",
    "test/language/block-scope/syntax/for-in/disallow-initialization-assignment.js",
    "test/language/statements/for-of/head-lhs-async-invalid.js",
    "test/language/types/boolean/S8.3_A2.1.js",
    "test/language/expressions/object/cover-initialized-name.js",
    // Expressions.
    "test/language/expressions/coalesce/cannot-chain-head-with-logical-and.js",
    "test/language/expressions/exponentiation/exp-operator-syntax-error-bitnot-unary-expression-base.js",
    "test/language/expressions/optional-chaining/early-errors-tail-position-null-op-template-string.js",
    "test/language/expressions/template-literal/invalid-unicode-escape-sequence-2.js",
    "test/language/global-code/new.target.js",
    // Functions, classes and modules.
    "test/language/expressions/async-arrow-function/early-errors-arrow-body-contains-super-call.js",
    "test/language/expressions/async-arrow-function/early-errors-arrow-await-in-formals.js",
    "test/language/expressions/object/method-definition/generator-param-init-yield.js",
    "test/language/statements/class/static-init-invalid-arguments.js",
    "test/language/module-code/private-identifiers-not-empty.js",
    "test/language/module-code/parse-err-decl-pos-export-block-stmt.js",
    "test/language/module-code/early-export-unresolvable.js",
    "test/staging/sm/fields/await-identifier-module-2.js",
    // The lexer's errors.
    "test/language/identifiers/vertical-tilde-start.js",
    "test/language/identifiers/start-zwj-escaped.js",
    "test/language/literals/numeric/numeric-followed-by-ident.js",
    "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dd-nsl-dds-dunder-err.js",
    "test/language/comments/S7.4_A2_T2.js",
    "test/language/line-terminators/invalid-regexp-cr.js",
    "test/language/literals/string/S7.8.4_A3.1_T1.js",
    // Regular expressions, from `early_errors`.
    "test/language/literals/regexp/named-groups/invalid-duplicate-groupspecifier.js",
    "test/language/literals/regexp/u-invalid-quantifier-out-of-order.js",
    "test/language/literals/regexp/invalid-optional-lookbehind.js",
];

/// Whether test262's frontmatter flags `src` with `flag`, in a
/// `flags: [a, b]` list or a `- a` one under `flags:`.
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

/// Every error in `src`: the parser's, all of them, or if it parses,
/// the early errors.
fn errors(src: &str, options: &Options) -> Vec<Diagnostic> {
    match parse_recovering(src, options) {
        Err(errors) => errors,
        Ok(ast) => early_errors(&ast, src),
    }
}

#[test]
fn test262_code_frames() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262");
    assert!(
        root.join("test").is_dir(),
        "tests/test262 is a submodule: run `git submodule update --init crates/parsec-js/tests/test262`"
    );
    let mut out = String::new();
    for name in CASES {
        let src = std::fs::read_to_string(root.join(name)).unwrap();
        let options = Options {
            source_type: if flagged(&src, "module") {
                SourceType::Module
            } else {
                SourceType::Script
            },
            ..Options::default()
        };
        // test262 runs these with "use strict" on a line before.
        let (src, note) = if flagged(&src, "onlyStrict") {
            (
                format!("\"use strict\";\n{src}"),
                " (\"use strict\"; added as line 1)",
            )
        } else {
            (src, "")
        };
        let found = errors(&src, &options);
        assert!(!found.is_empty(), "{name} has no error");
        out.push_str(&format!("=== {name}{note}\n"));
        let lines = LineIndex::ecmascript(&src);
        let o = RenderOptions {
            path: Some(name),
            ..RenderOptions::default()
        };
        for d in &found {
            out.push_str(&d.render(&src, &lines, &o));
            out.push('\n');
        }
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots/diagnostics.txt");
    if std::env::var_os("PARSEC_BLESS").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &out).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if want != out {
        let line = want
            .lines()
            .zip(out.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(want.lines().count().min(out.lines().count()));
        panic!(
            "the code frames changed, first at line {}:\n  was: {:?}\n  now: {:?}\n\
             if that's meant, run with PARSEC_BLESS=1",
            line + 1,
            want.lines().nth(line).unwrap_or(""),
            out.lines().nth(line).unwrap_or("")
        );
    }
}

fn codes(src: &str) -> Vec<(&'static str, u32)> {
    parse_recovering(src, &Options::default())
        .err()
        .unwrap_or_default()
        .iter()
        .map(|d| (d.code, d.span.start))
        .collect()
}

#[test]
fn recovery_reports_each_statement() {
    // Each bad statement is one error, and the good ones between parse.
    let src = "let a = ;\nlet b = 1;\nf(;\nwhile (x) { y z }\nlet c = 2\nlet d = )";
    assert_eq!(
        codes(src),
        [
            ("unexpected-token", 8),
            ("unexpected-token", 23),
            ("unexpected-token", 39),
            ("unexpected-token", 61)
        ]
    );
    // The first is always the one `parse` stops at.
    let first = parse(src, &Options::default()).unwrap_err();
    assert_eq!((first.code, first.span.start), ("unexpected-token", 8));
}

#[test]
fn recovery_resyncs_at_a_statement_keyword_on_a_new_line() {
    // No `;` or `}` after the error: the `if` starting a line ends the
    // skip, and so does a `return` in a function.
    assert_eq!(
        codes("let x = 1 2 3\nif (x) {}\nlet = = ;"),
        [("unexpected-token", 10), ("unexpected-token", 30)]
    );
    assert_eq!(
        codes("function f() {\n  a b c\n  return 1\n}\nf(]"),
        [("unexpected-token", 19), ("unexpected-token", 38)]
    );
}

#[test]
fn recovery_skips_braces_templates_and_regexps_whole() {
    // A `;` or `}` inside a block, a template's substitution or a regular
    // expression skipped over doesn't end the statement early.
    assert_eq!(codes("a b { c; d; }\nx;"), [("unexpected-token", 2)]);
    assert_eq!(codes("a b `${ { c; } }`;\nx;"), [("unexpected-token", 2)]);
    assert_eq!(codes("a b = /;}/;\nok;"), [("unexpected-token", 2)]);
    // A `{` open before the error counts too, so the error inside an
    // object doesn't stop at its `}`; an open `(` doesn't hide the rest.
    assert_eq!(codes("f({ a: 1 b: 2 });\nok;"), [("unexpected-token", 9)]);
    assert_eq!(
        codes("f(a b;\nok;\nc d;"),
        [("unexpected-token", 4), ("unexpected-token", 13)]
    );
}

#[test]
fn recovery_inside_functions_and_blocks() {
    // An error deep in a function is kept in the function: the code after
    // it, in it and after it, is still parsed.
    let src = "function f() {\n  if (a) { b c }\n  return 1;\n}\nlet y = ;";
    assert_eq!(
        codes(src),
        [("unexpected-token", 28), ("unexpected-token", 54)]
    );
    // A stray `}` at the top is one error, and the parse goes on.
    assert_eq!(
        codes("a;\n}\nb c;"),
        [("unexpected-token", 3), ("unexpected-token", 7)]
    );
    // A statement whose first token is the error moves on by a token.
    assert_eq!(
        codes("return 1;\nreturn 2;"),
        [
            ("return-outside-function", 0),
            ("return-outside-function", 10)
        ]
    );
}

#[test]
fn recovery_stops_at_limits() {
    let deep = "[".repeat(100) + "\na b;";
    let options = Options {
        limits: Limits {
            max_depth: Some(50),
            ..Limits::default()
        },
        ..Options::default()
    };
    let errors = parse_recovering(&deep, &options).unwrap_err();
    assert_eq!(errors.last().unwrap().code, "too-deep");
    assert_eq!(errors.len(), 1);
    let options = Options {
        limits: Limits {
            max_steps: Some(20),
            ..Limits::default()
        },
        ..Options::default()
    };
    let errors = parse_recovering(&"a b;\n".repeat(100), &options).unwrap_err();
    assert_eq!(errors.last().unwrap().code, "step-budget");
    assert!(errors.len() < 20);
}

#[test]
fn recovery_changes_nothing_for_good_programs() {
    // Every test262 file that should parse gives the same tree, comments
    // and warnings with recovery on; a sample of them, every seventh.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262/test/language");
    let mut paths = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|e| e == "js") {
                paths.push(p);
            }
        }
    }
    paths.sort();
    let mut compared = 0;
    for p in paths.iter().step_by(7) {
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
        let Ok(plain) = parse(&src, &options) else {
            continue;
        };
        let recovering = parse_recovering(&src, &options).unwrap();
        assert_eq!(
            plain.to_estree(&src),
            recovering.to_estree(&src),
            "{}",
            p.display()
        );
        assert_eq!(plain.comments(), recovering.comments());
        assert_eq!(plain.warnings(), recovering.warnings());
        compared += 1;
    }
    assert!(compared > 2000, "only {compared} compared");
}

#[test]
fn labels_point_back() {
    let label = |src: &str| {
        let d = parse(src, &Options::default()).unwrap_err();
        let l = &d.labels[0];
        (d.code, l.span.text(src).to_string(), l.message.to_string())
    };
    assert_eq!(
        label("if (a { }"),
        (
            "unexpected-token",
            "(".into(),
            "this `(` is still open".into()
        )
    );
    assert_eq!(
        label("f(a, b"),
        (
            "unexpected-end",
            "(".into(),
            "this `(` is still open".into()
        )
    );
    assert_eq!(
        label("x = [1, 2 3]"),
        (
            "unexpected-token",
            "[".into(),
            "this `[` is still open".into()
        )
    );
    assert_eq!(
        label("function f() { a;"),
        (
            "unexpected-end",
            "{".into(),
            "this `{` is still open".into()
        )
    );
    assert_eq!(
        label("class A { m() {}"),
        (
            "unexpected-end",
            "{".into(),
            "this `{` is still open".into()
        )
    );
    assert_eq!(
        label("a: { a: ; }"),
        (
            "duplicate-label",
            "a".into(),
            "the label is defined here".into()
        )
    );
    assert_eq!(
        label("\"use strict\"; function f(a, a) {}"),
        ("duplicate-binding", "a".into(), "bound here first".into())
    );
    assert_eq!(
        label("switch (x) { default: case 1: default: }"),
        (
            "duplicate-default",
            "default".into(),
            "the first `default`".into()
        )
    );
    assert_eq!(
        label("function f(a = 1) { \"use strict\" }"),
        (
            "use-strict-with-complex-params",
            "\"use strict\"".into(),
            "it says so here".into()
        )
    );
    assert_eq!(
        label("throw\nerr;"),
        (
            "newline-after-throw",
            "throw".into(),
            "the line ends after this `throw`".into()
        )
    );
    let module = Options {
        source_type: SourceType::Module,
        ..Options::default()
    };
    let d = parse("export let a; export { a };", &module).unwrap_err();
    assert_eq!(
        (d.code, d.labels[0].span),
        ("duplicate-export", parsec_js::Span::new(11, 12))
    );
}
