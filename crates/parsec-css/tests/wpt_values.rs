// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! web-platform-tests' CSS parsing tests, the static part:
//! `test_valid_value(property, value, serialized)`,
//! `test_invalid_value(property, value)` and
//! `test_shorthand_value(shorthand, value, longhands)` from every
//! `css/*/parsing/*.html`, extracted by `scripts/wpt-values-cases.js`
//! into `tests/wpt-values/cases.json`, pinned at the commit in its
//! `COMMIT` file.
//!
//! A valid case passes if the value parses for the property and writes
//! back as the test expects, and the written form parses back to itself,
//! as the test also checks. Color tests that WPT compares with a
//! tolerance (`fuzzy_test_valid_color`) are compared to within the
//! test's epsilon, 0.01 unless it gives one. An
//! invalid case passes if the value doesn't parse. A shorthand case
//! passes if each longhand it names gets the value the test expects.
//!
//! `tests/wpt-values/known-failures.txt` lists every case that fails
//! today, one JSON array per line; the run fails if any other case fails
//! or a listed one passes. `UPDATE_KNOWN_FAILURES=1` rewrites the list.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use parsec_css::parse_component_values;
use parsec_css::values::{expand, parse_value};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt-values")
}

/// `fontFamily` as `font-family`, for the few tests that use the
/// JavaScript name.
fn css_name(p: &str) -> String {
    let mut out = String::new();
    for c in p.chars() {
        if c.is_ascii_uppercase() {
            out.push('-');
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

fn serialize(property: &str, value: &str) -> Option<String> {
    let values = parse_component_values(value);
    parse_value(property, &values).ok().map(|v| v.to_string())
}

/// Numbers within `epsilon`, everything else the same.
fn fuzzy_same(got: &str, want: &str, epsilon: f64) -> bool {
    fn parts(s: &str) -> (Vec<f64>, String) {
        let mut numbers = Vec::new();
        let mut text = String::new();
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            let after_word = text.ends_with(|p: char| p.is_ascii_alphanumeric() || p == '-');
            let starts = !after_word
                && (c.is_ascii_digit()
                    || (matches!(c, '-' | '.')
                        && rest[1..].starts_with(|d: char| d.is_ascii_digit())));
            if starts {
                // Digits, a point, a sign, and `e` only as an exponent:
                // `1em` is the number 1 and a unit.
                let b = rest.as_bytes();
                let mut len = 1;
                while len < b.len() {
                    let d = b[len];
                    let exponent = d == b'e'
                        && b.get(len + 1).is_some_and(|n| {
                            n.is_ascii_digit()
                                || (matches!(n, b'-' | b'+')
                                    && b.get(len + 2).is_some_and(u8::is_ascii_digit))
                        });
                    if exponent {
                        len += 2;
                    } else if d.is_ascii_digit() || d == b'.' {
                        len += 1;
                    } else {
                        break;
                    }
                }
                numbers.push(rest[..len].parse().unwrap_or(f64::NAN));
                text.push('#');
                rest = &rest[len..];
            } else {
                text.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        (numbers, text)
    }
    let ((g, gt), (w, wt)) = (parts(got), parts(want));
    gt == wt && g.len() == w.len() && g.iter().zip(&w).all(|(a, b)| (a - b).abs() <= epsilon)
}

/// Whether a case passes, and what it got, for the failure list.
fn run(case: &Value) -> (bool, String) {
    let property = css_name(case["property"].as_str().unwrap());
    let value = case["value"].as_str().unwrap();
    match case["kind"].as_str().unwrap() {
        "valid" => {
            let Some(got) = serialize(&property, value) else {
                return (false, "invalid".into());
            };
            let fuzzy = case["fuzzy"].as_bool().unwrap_or(false);
            let epsilon = case["epsilon"].as_f64().unwrap_or(0.01);
            let matches = |want: &str| {
                if fuzzy {
                    fuzzy_same(&got, want, epsilon)
                } else {
                    got == want
                }
            };
            let ok = match &case["expect"] {
                Value::String(s) => matches(s),
                Value::Array(a) => a.iter().filter_map(Value::as_str).any(matches),
                _ => false,
            };
            let round_trips = serialize(&property, &got).as_deref() == Some(&*got);
            (ok && round_trips, got)
        }
        "invalid" => match serialize(&property, value) {
            None => (true, String::new()),
            Some(got) => (false, got),
        },
        "shorthand" => {
            let values = parse_component_values(value);
            let Ok(longhands) = expand(&property, &values) else {
                return (false, "invalid".into());
            };
            let got: BTreeMap<String, String> = longhands
                .iter()
                .map(|(n, v)| (n.to_string(), v.to_string()))
                .collect();
            let want = case["longhands"].as_object().unwrap();
            // A test may name a longhand by a legacy alias, `grid-row-gap`.
            let canonical = |name: &str| {
                let n = css_name(name);
                parsec_css::values::property(&n)
                    .and_then(|p| p.alias_of)
                    .map_or(n, String::from)
            };
            let ok = want
                .iter()
                .all(|(name, v)| got.get(&canonical(name)).map(String::as_str) == v.as_str());
            (ok, serde_json::to_string(&got).unwrap())
        }
        other => panic!("unknown case kind {other}"),
    }
}

fn key(case: &Value) -> String {
    let mut k = vec![
        case["kind"].clone(),
        case["property"].clone(),
        case["value"].clone(),
    ];
    match case["kind"].as_str() {
        Some("valid") => k.push(case["expect"].clone()),
        Some("shorthand") => k.push(case["longhands"].clone()),
        _ => {}
    }
    serde_json::to_string(&k).unwrap()
}

#[test]
fn wpt_values() {
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let known_path = dir().join("known-failures.txt");
    let known: BTreeSet<String> = std::fs::read_to_string(&known_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    let mut counts: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    let mut failing = BTreeSet::new();
    let mut unexpected = Vec::new();
    let mut skipped = 0;
    for file in data["files"].as_array().unwrap() {
        if let Some(s) = file["skipped"].as_object() {
            skipped += s.values().filter_map(Value::as_u64).sum::<u64>() as usize;
        }
        let Some(cases) = file["cases"].as_array() else {
            continue;
        };
        for case in cases {
            let (ok, got) = run(case);
            let k = key(case);
            let count = counts.entry(case["kind"].as_str().unwrap()).or_default();
            if ok {
                count.0 += 1;
                if known.contains(&k) {
                    unexpected.push(format!("listed as failing, but passes: {k}"));
                }
            } else {
                count.1 += 1;
                if !known.contains(&k) {
                    unexpected.push(format!("{k}\n  got: {got}"));
                }
                failing.insert(k);
            }
        }
    }
    let passed: usize = counts.values().map(|c| c.0).sum();
    let failed: usize = counts.values().map(|c| c.1).sum();
    println!("wpt css parsing: {passed} passed, {failed} failed, {skipped} skipped");
    for (kind, (p, f)) in &counts {
        println!("  {kind}: {p} passed, {f} failed");
    }
    summary::write(&summary::Counts {
        suite: "wpt-css-parsing",
        passed,
        failed,
        skipped,
        skipped_because: "they need computed values, layout or a DOM",
        data: &dir(),
    });
    if let Some(path) = std::env::var_os("WPT_DUMP") {
        std::fs::write(path, unexpected.join("\n")).unwrap();
    }
    if std::env::var_os("UPDATE_KNOWN_FAILURES").is_some() {
        let mut text = String::from(
            "# Every WPT CSS parsing case that fails today, written by\n\
             # UPDATE_KNOWN_FAILURES=1 cargo test -p parsec-html-css --test wpt_values\n",
        );
        for k in &failing {
            text.push_str(k);
            text.push('\n');
        }
        std::fs::write(&known_path, text).unwrap();
        return;
    }
    assert!(
        unexpected.is_empty(),
        "{} unexpected:\n{}",
        unexpected.len(),
        unexpected
            .iter()
            .take(50)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
