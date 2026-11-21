// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The ESTree output against acorn's, on the sources in `tests/estree`:
//! each `.json` there is acorn 8.19.0's tree for the `.js` script or
//! `.mjs` module beside it,
//! written by `scripts/gen-estree-fixtures.sh`. They're compared as JSON
//! values, so key order doesn't matter, but every node, field and offset
//! does.

use std::path::Path;

use parsec_js::{EstreeOptions, Options, SourceType, parse};
use serde_json::Value;

/// Where `a` and `b` first differ, as a path of keys and indexes.
fn first_difference(a: &Value, b: &Value, path: &str) -> Option<String> {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for key in x.keys().chain(y.keys()) {
                let (u, v) = (
                    x.get(key).unwrap_or(&Value::Null),
                    y.get(key).unwrap_or(&Value::Null),
                );
                if x.contains_key(key) != y.contains_key(key) {
                    return Some(format!("{path}.{key}: only in one of them"));
                }
                if let Some(d) = first_difference(u, v, &format!("{path}.{key}")) {
                    return Some(d);
                }
            }
            None
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{path}: {} items, not {}", y.len(), x.len()));
            }
            x.iter()
                .zip(y)
                .enumerate()
                .find_map(|(i, (u, v))| first_difference(u, v, &format!("{path}[{i}]")))
        }
        _ if a == b => None,
        _ => Some(format!("{path}: {b}, not {a}")),
    }
}

/// JSON with a lone surrogate's `\\udxxx` escape spelled as six
/// characters instead: serde_json refuses lone surrogates, which
/// `JSON.stringify` writes for a string like `"\\uD800"`.
fn parse_json(text: &str) -> Value {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        out.push(c);
        if c != '\\' {
            continue;
        }
        let Some(next) = chars.next() else { break };
        if next == 'u' {
            let hex: String = chars.clone().take(4).collect();
            if u16::from_str_radix(&hex, 16).is_ok_and(|u| (0xD800..0xE000).contains(&u)) {
                out.push('\\');
            }
        }
        out.push(next);
    }
    serde_json::from_str(&out).unwrap()
}

#[test]
fn matches_acorn() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/estree");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "js" || e == "mjs"))
        .collect();
    names.sort();
    assert!(names.len() >= 10, "the fixtures are missing");
    let mut failures = Vec::new();
    for js in &names {
        let name = js.file_name().unwrap().to_string_lossy().into_owned();
        let src = std::fs::read_to_string(js).unwrap();
        let expected = parse_json(&std::fs::read_to_string(js.with_extension("json")).unwrap());
        let source_type = if name.ends_with(".mjs") {
            SourceType::Module
        } else {
            SourceType::Script
        };
        let options = Options {
            source_type,
            ..Options::default()
        };
        let ast = match parse(&src, &options) {
            Ok(ast) => ast,
            Err(d) => {
                failures.push(format!("{name}: {d}"));
                continue;
            }
        };
        let options = EstreeOptions {
            loc: name == "locations.js",
        };
        let ours = parse_json(&ast.to_estree_with(&src, options));
        if let Some(d) = first_difference(&expected, &ours, "") {
            failures.push(format!("{name}: {d}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
