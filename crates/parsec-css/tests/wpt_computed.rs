// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! web-platform-tests' computed-value tests, the static part, extracted
//! by `scripts/wpt-computed-cases.js` into `tests/wpt-computed`, with each
//! test file beside its cases: the file is parsed with Parsec, its
//! `<style>` elements and the HTML user agent stylesheet applied, and
//! `#target` styled the way the test styles it from script, in an
//! 800×600 viewport as WPT's is.
//!
//! - `computed`: `target.style[property] = specified`, then
//!   `getComputedStyle(target)[property]` must be what the test expects
//!   (to within its epsilon for the colour tests that give one), and
//!   setting that back must give it again.
//! - `initial`: `target.style[property] = 'initial'` computes to the
//!   test's initial value.
//! - `inherited`: with `other` set on `#container`, `#target` gets it if
//!   and only if the property inherits, and always with `inherit`.
//!
//! `tests/wpt-computed/known-failures.txt` lists what fails today; the run
//! fails if anything else fails or a listed case passes.
//! `UPDATE_KNOWN_FAILURES=1` rewrites it.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use parsec::{Document, NodeId};
use parsec_css::cascade::Styler;
use parsec_css::media::Environment;
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt-computed")
}

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

fn by_id(doc: &Document, id: &str) -> Option<NodeId> {
    let sel = parsec::css::SelectorList::parse(&format!("[id=\"{id}\"]")).ok()?;
    sel.select_first(doc, doc.root())
}

fn style_attr(doc: &Document, el: NodeId) -> String {
    doc.attrs(el)
        .find(|(_, n, _)| *n == "style")
        .map(|(_, _, v)| v.to_string())
        .unwrap_or_default()
}

/// Numbers within `epsilon`, everything else the same.
fn fuzzy_same(got: &str, want: &str, epsilon: f64) -> bool {
    fn parts(s: &str) -> (Vec<f64>, String) {
        let (mut numbers, mut text, mut rest) = (Vec::new(), String::new(), s);
        while let Some(c) = rest.chars().next() {
            let after_word = text.ends_with(|p: char| p.is_ascii_alphanumeric() || p == '-');
            if !after_word
                && (c.is_ascii_digit()
                    || (matches!(c, '-' | '.')
                        && rest[1..].starts_with(|d: char| d.is_ascii_digit())))
            {
                let len = rest
                    .find(|d: char| !(d.is_ascii_digit() || d == '.' || d == '-' || d == 'e'))
                    .unwrap_or(rest.len());
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

struct Page {
    doc: Document,
    target: NodeId,
    container: Option<NodeId>,
    target_style: String,
    container_style: String,
}

impl Page {
    fn styled(
        &mut self,
        target: &str,
        container: &str,
    ) -> (
        Option<parsec_css::cascade::Computed>,
        Option<parsec_css::cascade::Computed>,
    ) {
        let t = format!("{};{target}", self.target_style);
        self.doc.set_attr(self.target, "style", &t);
        if let Some(c) = self.container {
            let s = format!("{};{container}", self.container_style);
            self.doc.set_attr(c, "style", &s);
        }
        let env = Environment {
            width: 800.0,
            height: 600.0,
            ..Environment::default()
        };
        let styler = Styler::new(env).with_document_styles(&self.doc);
        (
            styler.style_element(&self.doc, self.target),
            self.container
                .and_then(|c| styler.style_element(&self.doc, c)),
        )
    }

    fn target(&mut self, property: &str, target: &str, container: &str) -> String {
        let (t, _) = self.styled(target, container);
        t.and_then(|t| t.get(property)).unwrap_or_default()
    }
}

fn matches(got: &str, want: &Value, fuzzy: Option<f64>) -> bool {
    let one = |w: &str| match fuzzy {
        Some(e) => fuzzy_same(got, w, e),
        None => got == w,
    };
    match want {
        Value::String(s) => one(s),
        Value::Array(a) => a.iter().filter_map(Value::as_str).any(one),
        _ => false,
    }
}

fn run(page: &mut Page, case: &Value) -> (bool, String) {
    let p = css_name(case["property"].as_str().unwrap());
    match case["kind"].as_str().unwrap() {
        "computed" => {
            let specified = case["specified"].as_str().unwrap();
            let got = page.target(&p, &format!("{p}: {specified}"), "");
            let fuzzy = case["fuzzy"]
                .as_bool()
                .unwrap_or(false)
                .then(|| case["epsilon"].as_f64().unwrap_or(0.01));
            let ok = matches(&got, &case["expect"], fuzzy);
            let again = if ok && got != specified {
                page.target(&p, &format!("{p}: {got}"), "")
            } else {
                got.clone()
            };
            (ok && again == got, got)
        }
        "initial" => {
            let got = page.target(&p, &format!("{p}: initial"), "");
            (matches(&got, &case["expect"], None), got)
        }
        "inherited" => {
            let other = case["other"].as_str().unwrap();
            let container = format!("{p}: {other}");
            let unset = page.target(&p, "", &container);
            let inherit = page.target(&p, &format!("{p}: inherit"), &container);
            let inherits = case["inherited"].as_bool().unwrap();
            let ok = (unset == other) == inherits && inherit == other;
            (ok, format!("unset: {unset}, inherit: {inherit}"))
        }
        k => panic!("unknown case kind {k}"),
    }
}

#[test]
fn wpt_computed() {
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
        let name = file["file"].as_str().unwrap();
        let html = std::fs::read_to_string(dir().join(name)).unwrap();
        let doc = parsec::parse(&html, true);
        let Some(target) = by_id(&doc, "target") else {
            skipped += cases.len();
            continue;
        };
        let container = by_id(&doc, "container");
        let mut page = Page {
            target_style: style_attr(&doc, target),
            container_style: container.map(|c| style_attr(&doc, c)).unwrap_or_default(),
            doc,
            target,
            container,
        };
        for case in cases {
            let (ok, got) = run(&mut page, case);
            let mut key_parts = vec![
                case["kind"].clone(),
                Value::String(name.into()),
                case["property"].clone(),
            ];
            for k in ["specified", "expect", "other"] {
                if !case[k].is_null() {
                    key_parts.push(case[k].clone());
                }
            }
            let key = serde_json::to_string(&key_parts).unwrap();
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
    println!("wpt computed values: {passed} passed, {failed} failed, {skipped} skipped");
    for (kind, (p, f)) in &counts {
        println!("  {kind}: {p} passed, {f} failed");
    }
    summary::write(&summary::Counts {
        suite: "wpt-computed-values",
        passed,
        failed,
        skipped,
        skipped_because: "they change the page from script, or need layout",
        data: &dir(),
    });
    if let Some(path) = std::env::var_os("WPT_DUMP") {
        std::fs::write(path, unexpected.join("\n")).unwrap();
    }
    if std::env::var_os("UPDATE_KNOWN_FAILURES").is_some() {
        let mut text = String::from(
            "# Every WPT computed-value case that fails today, written by\n\
             # UPDATE_KNOWN_FAILURES=1 cargo test -p parsec-html-css --test wpt_computed\n",
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
