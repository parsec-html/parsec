// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! web-platform-tests' media query tests, the static part, extracted by
//! `scripts/wpt-media-cases.js` into `tests/wpt-media/cases.json`: whether
//! a query applies (with the viewport size the test set), whether it
//! parses, and how `matchMedia().media` writes it. Most come from the
//! Mozilla suite, `test_media_queries.html`.
//!
//! `tests/wpt-media/known-failures.txt` lists what fails today, one JSON
//! array per line; the run fails if anything else fails or a listed case
//! passes. `UPDATE_KNOWN_FAILURES=1` rewrites it.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use parsec_css::media::{Environment, MediaQuery, MediaQueryList};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt-media")
}

fn run(case: &Value, screen: &Value) -> (bool, String) {
    let query = case["query"].as_str().unwrap();
    let list = MediaQueryList::parse_str(query);
    match case["kind"].as_str().unwrap() {
        "serialize" => {
            let got = list.to_string();
            (Some(got.as_str()) == case["expect"].as_str(), got)
        }
        "parseable" => {
            // As `@media screen, <query>` would hold it: the query, unless
            // it's `not all` for not parsing.
            let parsed =
                !list.queries.is_empty() && list.queries.iter().all(|q| *q != MediaQuery::Invalid);
            (
                parsed == case["expect"].as_bool().unwrap(),
                format!("parses: {parsed}"),
            )
        }
        "applies" => {
            let size = |k: &str, default: f64| case["env"][k].as_f64().unwrap_or(default);
            let env = Environment {
                width: size("width", 800.0),
                height: size("height", 600.0),
                device_width: screen["width"].as_f64().unwrap_or(1280.0),
                device_height: screen["height"].as_f64().unwrap_or(720.0),
                ..Environment::default()
            };
            let got = list.matches(&env);
            (
                got == case["expect"].as_bool().unwrap(),
                format!("applies: {got}"),
            )
        }
        other => panic!("unknown case kind {other}"),
    }
}

#[test]
fn wpt_media() {
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let known_path = dir().join("known-failures.txt");
    let known: BTreeSet<String> = std::fs::read_to_string(&known_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let (mut failing, mut unexpected, mut skipped) = (BTreeSet::new(), Vec::new(), 0);
    for file in data["files"].as_array().unwrap() {
        if let Some(s) = file["skipped"].as_object() {
            skipped += s.values().filter_map(Value::as_u64).sum::<u64>() as usize;
        }
        let Some(cases) = file["cases"].as_array() else {
            continue;
        };
        for case in cases {
            let (ok, got) = run(case, &file["screen"]);
            let key = serde_json::to_string(&[
                &case["kind"],
                &case["query"],
                &case["env"],
                &case["expect"],
            ])
            .unwrap();
            let count = counts
                .entry(case["kind"].as_str().unwrap().to_string())
                .or_default();
            if ok {
                count.0 += 1;
                if known.contains(&key) {
                    unexpected.push(format!("listed as failing, but passes: {key}"));
                }
            } else {
                count.1 += 1;
                if !known.contains(&key) {
                    unexpected.push(format!("{key}\n  got: {got}"));
                }
                failing.insert(key);
            }
        }
    }
    let passed: usize = counts.values().map(|c| c.0).sum();
    let failed: usize = counts.values().map(|c| c.1).sum();
    println!("wpt media queries: {passed} passed, {failed} failed, {skipped} skipped");
    for (kind, (p, f)) in &counts {
        println!("  {kind}: {p} passed, {f} failed");
    }
    summary::write(&summary::Counts {
        suite: "wpt-media-queries",
        passed,
        failed,
        skipped,
        skipped_because: "they probe the browser more than once in a test",
        data: &dir(),
    });
    if let Some(path) = std::env::var_os("WPT_DUMP") {
        std::fs::write(path, unexpected.join("\n")).unwrap();
    }
    if std::env::var_os("UPDATE_KNOWN_FAILURES").is_some() {
        let mut text = String::from(
            "# Every WPT media query case that fails today, written by\n\
             # UPDATE_KNOWN_FAILURES=1 cargo test -p parsec-html-css --test wpt_media\n",
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
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
