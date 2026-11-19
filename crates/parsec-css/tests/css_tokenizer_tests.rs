// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Romain Menke's CSS tokenizer tests, pinned in `tests/css-tokenizer-tests`
//! from github.com/romainmenke/css-tokenizer-tests: a stylesheet per case
//! and the tokens it makes, each with its text and position. Every field
//! the case gives is compared: type, text, start and end, and value, type
//! flag, unit and sign.
//!
//! The suite counts positions in UTF-16 code units, as JavaScript does, so
//! byte offsets are converted to compare. It also lists comments as
//! tokens, which the spec's tokenizer doesn't make; they're left out of the
//! expected tokens, and their positions still show in the tokens around
//! them.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::path::{Path, PathBuf};

use parsec_css::{Number, Token, Tokenizer};
use serde_json::{Value, json};

fn data() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/css-tokenizer-tests")
}

fn numeric(n: &Number<'_>, with_type: bool) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    m.insert("value".into(), json!(n.value));
    if with_type {
        m.insert(
            "type".into(),
            json!(if n.integer { "integer" } else { "number" }),
        );
    }
    if let Some(sign) = n.sign() {
        m.insert("signCharacter".into(), json!(sign.to_string()));
    }
    m
}

/// The suite's type name and structured data for a token.
fn describe(t: &Token<'_>) -> (&'static str, Value) {
    let value = |v: &str| json!({ "value": v });
    match t {
        Token::Ident(v) => ("ident-token", value(v)),
        Token::Function(v) => ("function-token", value(v)),
        Token::AtKeyword(v) => ("at-keyword-token", value(v)),
        Token::Hash { value, id } => (
            "hash-token",
            json!({ "value": value, "type": if *id { "id" } else { "unrestricted" } }),
        ),
        Token::String(v) => ("string-token", value(v)),
        Token::BadString => ("bad-string-token", Value::Null),
        Token::Url(v) => ("url-token", value(v)),
        Token::BadUrl => ("bad-url-token", Value::Null),
        Token::Delim(c) => ("delim-token", value(&c.to_string())),
        Token::Number(n) => ("number-token", Value::Object(numeric(n, true))),
        Token::Percentage(n) => ("percentage-token", Value::Object(numeric(n, false))),
        Token::Dimension { number, unit } => {
            let mut m = numeric(number, true);
            m.insert("unit".into(), json!(unit));
            ("dimension-token", Value::Object(m))
        }
        Token::UnicodeRange { start, end } => (
            "unicode-range-token",
            json!({ "startOfRange": start, "endOfRange": end }),
        ),
        Token::Whitespace => ("whitespace-token", Value::Null),
        Token::Cdo => ("CDO-token", Value::Null),
        Token::Cdc => ("CDC-token", Value::Null),
        Token::Colon => ("colon-token", Value::Null),
        Token::Semicolon => ("semicolon-token", Value::Null),
        Token::Comma => ("comma-token", Value::Null),
        Token::LeftBracket => ("[-token", Value::Null),
        Token::RightBracket => ("]-token", Value::Null),
        Token::LeftParen => ("(-token", Value::Null),
        Token::RightParen => (")-token", Value::Null),
        Token::LeftBrace => ("{-token", Value::Null),
        Token::RightBrace => ("}-token", Value::Null),
    }
}

/// Numbers compare as numbers: `1` is `1.0`.
fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len() && x.iter().all(|(k, v)| y.get(k).is_some_and(|w| same(v, w)))
        }
        _ => a == b,
    }
}

/// Runs one case: the first difference, if there is one.
fn check(css: &str, want: &[Value]) -> Result<(), String> {
    // Byte offset to UTF-16 index.
    let mut utf16 = vec![0; css.len() + 1];
    let mut n = 0;
    for (i, c) in css.char_indices() {
        utf16[i] = n;
        n += c.len_utf16();
    }
    utf16[css.len()] = n;

    let want: Vec<&Value> = want.iter().filter(|t| t["type"] != "comment").collect();
    let got: Vec<_> = Tokenizer::new(css).collect();
    for (i, w) in want.iter().enumerate() {
        let Some((token, span)) = got.get(i) else {
            return Err(format!("token {i}: want {w}, got the end"));
        };
        let (kind, structured) = describe(token);
        let found = json!({
            "type": kind,
            "raw": span.text(css),
            "startIndex": utf16[span.start],
            "endIndex": utf16[span.end],
            "structured": structured,
        });
        let given = w.as_object().unwrap();
        if !given.iter().all(|(k, v)| same(v, &found[k])) {
            return Err(format!("token {i}: want {w}\n  got  {found}"));
        }
    }
    if got.len() > want.len() {
        return Err(format!(
            "{} tokens more than the {} wanted",
            got.len() - want.len(),
            want.len()
        ));
    }
    Ok(())
}

#[test]
fn css_tokenizer_tests() {
    let mut cases = Vec::new();
    for group in std::fs::read_dir(data().join("tests")).unwrap() {
        for case in std::fs::read_dir(group.unwrap().path()).unwrap() {
            cases.push(case.unwrap().path());
        }
    }
    cases.sort();
    let (mut passed, mut failed) = (0, 0);
    let mut failures = Vec::new();
    for case in &cases {
        let css = std::fs::read_to_string(case.join("source.css")).unwrap();
        let tokens = std::fs::read_to_string(case.join("tokens.json")).unwrap();
        let want: Vec<Value> = serde_json::from_str(&tokens).unwrap();
        match check(&css, &want) {
            Ok(()) => passed += 1,
            Err(e) => {
                failed += 1;
                let name = case.strip_prefix(data()).unwrap().display();
                failures.push(format!("{name}: {e}"));
            }
        }
    }
    println!("css-tokenizer-tests: {passed} passed, {failed} failed");
    summary::write(&summary::Counts {
        suite: "css-tokenizer-tests",
        passed,
        failed,
        skipped: 0,
        skipped_because: "",
        data: &data(),
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
