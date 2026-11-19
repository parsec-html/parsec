// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! css-parsing-tests, the implementation-independent CSS Syntax suite
//! Simon Sapin started for Servo and tinycss2, pinned in
//! `tests/css-parsing-tests` from its maintained home,
//! github.com/CourtBouillon/css-parsing-tests (where SimonSapin's repository
//! now points). Each pair in a file is one case. `tests/urange` adds the
//! `<urange>` cases from rust-cssparser's copy of the suite, which the
//! original doesn't have.
//!
//! The suite was written against an older draft than the one followed
//! here, so the output is compared with four allowances, each for a
//! difference of representation rather than of parsing:
//!
//! - Two delims written side by side, like `~` `=`, count as the one
//!   `~=` token the 2013 draft had; the current draft has no such token.
//! - The `eof-in-string` and `eof-in-url` errors tinycss2 puts into its
//!   output are left out of the expected values. They're parse errors the
//!   spec doesn't place in the tree.
//! - Whitespace at the two ends of a declaration's value isn't compared:
//!   the spec trims it, and the suite's parsers keep it.
//! - The `color_*` files test CSS Color, not CSS Syntax, so they're
//!   counted as skipped until typed values exist.
//!
//! Cases in `KNOWN_FAILURES` are where the current draft and the suite
//! disagree; they're counted as failed, and the run fails if any other
//! case fails or a listed one starts to pass.
//!
//! Every input the suite has is also serialized and parsed again, and must
//! come back the same, runs of whitespace aside.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::path::{Path, PathBuf};

use parsec_css::*;
use serde_json::{Value, json};

/// File and input of each case the current draft parses differently from
/// the suite, with why.
const KNOWN_FAILURES: &[(&str, &str)] = &[
    // The suite expects a <unicode-range-token> wherever `u+` starts one.
    // The current draft makes that token only in a unicode-range
    // descriptor's value, and parse_unicode_ranges passes the same ranges
    // (tests/urange); everywhere else this is an ident and numbers.
    (
        "component_value_list.json",
        "u+1 U+10 U+100 U+1000 U+10000 U+100000 U+1000000",
    ),
    (
        "component_value_list.json",
        "u+? u+1? U+10? U+100? U+1000? U+10000? U+100000?",
    ),
    (
        "component_value_list.json",
        "u+?? U+1?? U+10?? U+100?? U+1000?? U+10000??",
    ),
    (
        "component_value_list.json",
        "u+??? U+1??? U+10??? U+100??? U+1000???",
    ),
    (
        "component_value_list.json",
        "u+???? U+1???? U+10???? U+100????",
    ),
    ("component_value_list.json", "u+????? U+1????? U+10?????"),
    ("component_value_list.json", "u+?????? U+1??????"),
    (
        "component_value_list.json",
        "u+1-2 U+100000-2 U+1000000-2 U+10-200000",
    ),
    (
        "component_value_list.json",
        "ù+12 Ü+12 u +12 U+ 12 U+12 - 20 U+1?2 U+1?-50",
    ),
    // The suite has every code point from U+0080 up as an ident code point.
    // Since 2024 the draft takes the set HTML allows in custom element
    // names, which leaves out U+0080 to U+00B6, so `\u{80}\u{81}` at the
    // end is two delims here.
    (
        "component_value_list.json",
        "\\- red0 -red --red -\\-red\\ blue 0red -0red \0red _Red .red rêd r\\êd \x7f\u{80}\u{81}",
    ),
    // In the 2021 draft "parse a declaration" took the rest of the input as
    // the value, `;` and all. Since nesting it stops at `;`, so what
    // follows is extra input.
    ("one_declaration.json", "foo:;"),
    ("one_declaration.json", "foo:;bar:;"),
];

fn dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(name)
}

fn load(path: &Path) -> Vec<Value> {
    let text = std::fs::read_to_string(path).unwrap();
    serde_json::from_str::<Value>(&text)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .as_array()
        .unwrap()
        .clone()
}

fn number(kind: &str, n: &Number<'_>) -> Vec<Value> {
    let flag = if n.integer { "integer" } else { "number" };
    vec![json!(kind), json!(n.repr), json!(n.value), json!(flag)]
}

fn token(t: &Token<'_>) -> Value {
    match t {
        Token::Ident(v) => json!(["ident", v]),
        Token::AtKeyword(v) => json!(["at-keyword", v]),
        Token::Hash { value, id } => {
            json!(["hash", value, if *id { "id" } else { "unrestricted" }])
        }
        Token::String(v) => json!(["string", v]),
        Token::BadString => json!(["error", "bad-string"]),
        Token::Url(v) => json!(["url", v]),
        Token::BadUrl => json!(["error", "bad-url"]),
        Token::Delim(c) => json!(c.to_string()),
        Token::Number(n) => Value::Array(number("number", n)),
        Token::Percentage(n) => Value::Array(number("percentage", n)),
        Token::Dimension { number: n, unit } => {
            let mut v = number("dimension", n);
            v.push(json!(unit));
            Value::Array(v)
        }
        Token::UnicodeRange { start, end } => json!(["unicode-range", start, end]),
        Token::Whitespace => json!(" "),
        Token::Cdo => json!("<!--"),
        Token::Cdc => json!("-->"),
        Token::Colon => json!(":"),
        Token::Semicolon => json!(";"),
        Token::Comma => json!(","),
        Token::RightBracket => json!(["error", "]"]),
        Token::RightParen => json!(["error", ")"]),
        Token::RightBrace => json!(["error", "}"]),
        Token::Function(_) | Token::LeftBracket | Token::LeftParen | Token::LeftBrace => {
            unreachable!("never a component value")
        }
    }
}

const MATCHES: &[&str] = &["~=", "|=", "^=", "$=", "*=", "||"];

fn values(list: &[ComponentValue<'_>]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut last_delim_end = None;
    for v in list {
        let json = match v {
            ComponentValue::Token(t, span) => {
                // `~` and `=` side by side are the suite's `~=`.
                if let (Token::Delim(c), Some(end), Some(Value::String(prev))) =
                    (t, last_delim_end, out.last())
                    && end == span.start
                {
                    let joined = format!("{prev}{c}");
                    if MATCHES.contains(&joined.as_str()) {
                        *out.last_mut().unwrap() = json!(joined);
                        last_delim_end = None;
                        continue;
                    }
                }
                last_delim_end = matches!(t, Token::Delim(_)).then_some(span.end);
                token(t)
            }
            ComponentValue::Function(f) => {
                last_delim_end = None;
                let mut v = vec![json!("function"), json!(f.name)];
                v.extend(values(&f.arguments));
                Value::Array(v)
            }
            ComponentValue::Block(b) => {
                last_delim_end = None;
                let kind = match b.bracket {
                    Bracket::Paren => "()",
                    Bracket::Square => "[]",
                    Bracket::Curly => "{}",
                };
                let mut v = vec![json!(kind)];
                v.extend(values(&b.contents));
                Value::Array(v)
            }
        };
        out.push(json);
    }
    out
}

fn rule(r: &Rule<'_>) -> Value {
    match r {
        Rule::At(r) => json!([
            "at-rule",
            r.name,
            values(&r.prelude),
            r.block.as_ref().map(|b| values(&b.contents))
        ]),
        Rule::Qualified(r) => {
            json!([
                "qualified rule",
                values(&r.prelude),
                values(&r.block.contents)
            ])
        }
    }
}

fn declaration(d: &Declaration<'_>) -> Value {
    json!(["declaration", d.name, values(&d.value), d.important])
}

fn error(e: &ParseError) -> Value {
    let kind = match e.kind {
        ErrorKind::Empty => "empty",
        ErrorKind::ExtraInput => "extra-input",
        ErrorKind::InvalidDeclaration | ErrorKind::InvalidRule => "invalid",
    };
    json!(["error", kind])
}

/// Results and errors in the order they appear in the input.
fn in_order(mut found: Vec<(usize, Value)>, errors: &[ParseError]) -> Value {
    found.extend(errors.iter().map(|e| (e.span.start, error(e))));
    found.sort_by_key(|(at, _)| *at);
    Value::Array(found.into_iter().map(|(_, v)| v).collect())
}

fn sheet(s: &Stylesheet<'_>) -> Value {
    let rules = s.rules.iter().map(|r| (r.span().start, rule(r))).collect();
    in_order(rules, &s.errors)
}

fn block(b: &BlockContents<'_>) -> Value {
    let items = b
        .items
        .iter()
        .map(|i| match i {
            Item::Declaration(d) => (d.span.start, declaration(d)),
            Item::Rule(r) => (r.span().start, rule(r)),
        })
        .collect();
    in_order(items, &b.errors)
}

/// The bytes a `css_bytes` string stands for: one per code point.
fn bytes(s: &str) -> Vec<u8> {
    s.chars().map(|c| u8::try_from(c as u32).unwrap()).collect()
}

/// Runs one case, or `None` for a file that isn't run.
fn run(file: &str, input: &Value) -> Option<Value> {
    let css = input.as_str().unwrap_or_default();
    Some(match file {
        "component_value_list.json" => Value::Array(values(&parse_component_values(css))),
        "one_component_value.json" => match parse_component_value(css) {
            Ok(v) => values(std::slice::from_ref(&v)).remove(0),
            Err(e) => error(&e),
        },
        "declaration_list.json" => block(&parse_declaration_list(css)),
        "blocks_contents.json" => block(&parse_block_contents(css)),
        "one_declaration.json" => match parse_declaration(css) {
            Ok(d) => declaration(&d),
            Err(e) => error(&e),
        },
        "one_rule.json" => match parse_rule(css) {
            Ok(r) => rule(&r),
            Err(e) => error(&e),
        },
        "rule_list.json" => sheet(&parse_rule_list(css)),
        "stylesheet.json" => sheet(&parse_stylesheet(css)),
        "stylesheet_bytes.json" => {
            let label = |k: &str| input.get(k).and_then(Value::as_str);
            let css = bytes(label("css_bytes").unwrap());
            let environment =
                label("environment_encoding").and_then(|l| Encoding::for_label(l.as_bytes()));
            let (text, encoding) = decode_stylesheet(&css, label("protocol_encoding"), environment);
            json!([
                sheet(&parse_stylesheet(&text)),
                encoding.name().to_ascii_lowercase()
            ])
        }
        "An+B.json" => match parse_anb(css) {
            Some((a, b)) => json!([a, b]),
            None => Value::Null,
        },
        "urange.json" => Value::Array(
            parse_unicode_ranges(css)
                .into_iter()
                .map(|r| r.map_or(Value::Null, |r| json!([r.start, r.end])))
                .collect(),
        ),
        _ => return None,
    })
}

/// The expected value with the allowances in the module comment applied.
fn expected(v: &Value) -> Value {
    match v {
        Value::Array(items) => {
            let items: Vec<Value> = items
                .iter()
                .filter(|i| {
                    !matches!(i.as_array().map(Vec::as_slice),
                        Some([e, kind]) if e == "error" && (kind == "eof-in-string" || kind == "eof-in-url"))
                })
                .map(expected)
                .collect();
            if let [kind, name, Value::Array(value), important] = items.as_slice()
                && kind == "declaration"
                && name.is_string()
                && important.is_boolean()
            {
                let mut value = value.clone();
                while value.first().is_some_and(|v| v == " ") {
                    value.remove(0);
                }
                while value.last().is_some_and(|v| v == " ") {
                    value.pop();
                }
                return json!([kind, name, value, important]);
            }
            Value::Array(items)
        }
        _ => v.clone(),
    }
}

/// JSON equality with numbers compared as numbers: `42` is `42.0`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Array(x), Value::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
        }
        _ => a == b,
    }
}

/// Collapses runs of whitespace, which serialization may do, and splits
/// the suite's `~=` back into the two delims it is, which serialization
/// may write apart.
fn collapse(v: Value) -> Value {
    match v {
        Value::Array(items) => {
            let mut out: Vec<Value> = Vec::new();
            for item in items.into_iter().map(collapse) {
                if item == " " && out.last().is_some_and(|l| l == " ") {
                    continue;
                }
                if let Some(m) = item.as_str().filter(|m| MATCHES.contains(m)) {
                    out.extend(m.chars().map(|c| json!(c.to_string())));
                    continue;
                }
                out.push(item);
            }
            Value::Array(out)
        }
        v => v,
    }
}

/// Whether `css` comes back the same after serializing, as values, as a
/// stylesheet and as a block's contents.
fn round_trips(css: &str) -> Result<(), String> {
    let check = |what: &str, before: Value, again: String, after: Value| {
        if same(&collapse(before.clone()), &collapse(after.clone())) {
            Ok(())
        } else {
            Err(format!(
                "{what} of {css:?} wrote {again:?}\n  before: {before}\n  after:  {after}"
            ))
        }
    };
    let v = parse_component_values(css);
    let again = to_css(&v);
    check(
        "values",
        Value::Array(values(&v)),
        again.clone(),
        Value::Array(values(&parse_component_values(&again))),
    )?;
    // Errors are dropped constructs, which serializing leaves out.
    let rules = |s: &Stylesheet<'_>| Value::Array(s.rules.iter().map(rule).collect());
    let s = parse_stylesheet(css);
    let again = s.to_string();
    let after = rules(&parse_stylesheet(&again));
    check("stylesheet", rules(&s), again.clone(), after)?;
    let b = parse_block_contents(css);
    let again = b.to_string();
    let reparsed = parse_block_contents(&again);
    let items = |b: &BlockContents<'_>| {
        Value::Array(
            b.items
                .iter()
                .map(|i| match i {
                    Item::Declaration(d) => declaration(d),
                    Item::Rule(r) => rule(r),
                })
                .collect(),
        )
    };
    let after = items(&reparsed);
    check("block", items(&b), again.clone(), after)
}

#[test]
fn css_parsing_tests() {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir("css-parsing-tests"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    let mut unexpected = Vec::new();
    let mut inputs = Vec::new();
    for path in &files {
        let file = path.file_name().unwrap().to_str().unwrap();
        let cases = load(path);
        for pair in cases.chunks(2) {
            let (input, want) = (&pair[0], &pair[1]);
            if let Some(css) = input.as_str() {
                inputs.push(css.to_string());
            }
            let Some(got) = run(file, input) else {
                skipped += 1;
                continue;
            };
            let known = KNOWN_FAILURES
                .iter()
                .any(|(f, i)| *f == file && input.as_str() == Some(i));
            let ok = same(&got, &expected(want));
            match (ok, known) {
                (true, false) => passed += 1,
                (false, true) => failed += 1,
                (false, false) => {
                    failed += 1;
                    unexpected.push(format!("{file} {input}\n  want: {want}\n  got:  {got}"));
                }
                (true, true) => {
                    passed += 1;
                    unexpected.push(format!("{file} {input}: listed as failing, but passes"));
                }
            }
        }
    }
    println!("css-parsing-tests: {passed} passed, {failed} failed, {skipped} skipped (CSS Color)");
    summary::write(&summary::Counts {
        suite: "css-parsing-tests",
        passed,
        failed,
        skipped,
        skipped_because: "the color_* files test CSS Color, not CSS Syntax",
        data: &dir("css-parsing-tests"),
    });

    let mut round_trip_failures = Vec::new();
    for css in &inputs {
        if let Err(e) = round_trips(css) {
            round_trip_failures.push(e);
        }
    }
    assert!(
        unexpected.is_empty(),
        "{} unexpected:\n{}",
        unexpected.len(),
        unexpected.join("\n")
    );
    assert!(
        round_trip_failures.is_empty(),
        "{} didn't round-trip:\n{}",
        round_trip_failures.len(),
        round_trip_failures.join("\n")
    );
}

#[test]
fn urange() {
    let cases = load(&dir("urange").join("urange.json"));
    let (mut passed, mut failed) = (0, 0);
    let mut bad = Vec::new();
    for pair in cases.chunks(2) {
        let got = run("urange.json", &pair[0]).unwrap();
        if same(&got, &pair[1]) {
            passed += 1;
        } else {
            failed += 1;
            bad.push(format!("{}\n  want: {}\n  got:  {got}", pair[0], pair[1]));
        }
    }
    println!("urange: {passed} passed, {failed} failed");
    summary::write(&summary::Counts {
        suite: "css-parsing-tests-urange",
        passed,
        failed,
        skipped: 0,
        skipped_because: "",
        data: &dir("urange"),
    });
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
