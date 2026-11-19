// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! web-platform-tests' cascade tests (css/css-cascade), the static part,
//! extracted by `scripts/wpt-cascade-cases.js` into `tests/wpt-cascade`
//! with each test file beside its cases: the file is parsed with Parsec,
//! its own `<style>` elements and the ones the test adds applied, and the
//! element the test reads (the nth match of the selector it found it
//! with) must compute to what the test expects. A case for an element the
//! page doesn't have (the test looped over every match) isn't counted.
//!
//! `tests/wpt-cascade/known-failures.txt` lists what fails today; the run
//! fails if anything else fails or a listed case passes.
//! `UPDATE_KNOWN_FAILURES=1` rewrites it.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use parsec_css::cascade::{Origin, Styled, Styler};
use parsec_css::media::Environment;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt-cascade")
}

#[test]
fn wpt_cascade() {
    let data: Value =
        serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
    let known_path = dir().join("known-failures.txt");
    let known: BTreeSet<String> = std::fs::read_to_string(&known_path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect();
    let (mut passed, mut failed, mut skipped) = (0, 0, 0);
    let (mut failing, mut unexpected, mut seen) = (BTreeSet::new(), Vec::new(), BTreeSet::new());
    let env = Environment {
        width: 800.0,
        height: 600.0,
        ..Environment::default()
    };
    for file in data["files"].as_array().unwrap() {
        if let Some(s) = file["skipped"].as_object() {
            skipped += s.values().filter_map(Value::as_u64).sum::<u64>() as usize;
        }
        let Some(cases) = file["cases"].as_array() else {
            continue;
        };
        let name = file["file"].as_str().unwrap();
        let doc = parsec::parse(&std::fs::read_to_string(dir().join(name)).unwrap(), true);
        let mut styled: HashMap<String, Styled> = HashMap::new();
        for case in cases {
            let sheets: Vec<&str> = case["sheets"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(Value::as_str)
                .collect();
            let selector = case["element"]["selector"].as_str().unwrap();
            let index = case["element"]["index"].as_u64().unwrap() as usize;
            let Ok(sel) = parsec::css::SelectorList::parse(selector) else {
                skipped += 1;
                continue;
            };
            let Some(el) = sel.select(&doc, doc.root()).get(index).copied() else {
                continue;
            };
            let key = serde_json::to_string(&[
                Value::String(name.into()),
                case["sheets"].clone(),
                case["element"].clone(),
                case["property"].clone(),
                case["expect"].clone(),
            ])
            .unwrap();
            if !seen.insert(key.clone()) {
                continue;
            }
            let s = styled.entry(sheets.join("\u{0}")).or_insert_with(|| {
                let mut styler = Styler::new(env.clone()).with_document_styles(&doc);
                for css in &sheets {
                    styler.add(css, Origin::Author);
                }
                styler.style(&doc)
            });
            let got = s
                .get(el, case["property"].as_str().unwrap())
                .unwrap_or_default();
            if Some(got.as_str()) == case["expect"].as_str() {
                passed += 1;
                if known.contains(&key) {
                    unexpected.push(format!("listed as failing, but passes: {key}"));
                }
            } else {
                failed += 1;
                if !known.contains(&key) {
                    unexpected.push(format!("{key}\n  got: {got}"));
                }
                failing.insert(key);
            }
        }
    }
    println!("wpt cascade: {passed} passed, {failed} failed, {skipped} skipped");
    summary::write(&summary::Counts {
        suite: "wpt-cascade",
        passed,
        failed,
        skipped,
        skipped_because: "they change the page from script beyond adding styles",
        data: &dir(),
    });
    if let Some(path) = std::env::var_os("WPT_DUMP") {
        std::fs::write(path, unexpected.join("\n")).unwrap();
    }
    if std::env::var_os("UPDATE_KNOWN_FAILURES").is_some() {
        let mut text = String::from(
            "# Every WPT cascade case that fails today, written by\n\
             # UPDATE_KNOWN_FAILURES=1 cargo test -p parsec-html-css --test wpt_cascade\n",
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
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
