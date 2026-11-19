// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! web-platform-tests' cascade and at-rule tests, the static part:
//! css/css-cascade (layers, `@scope`, `@import`),
//! css/css-conditional/container-queries, css/css-properties-values-api
//! (`@property`), css/css-counter-styles/counter-style-at-rule and
//! css/css-pseudo,
//! extracted by `scripts/wpt-cascade-cases.js` into `tests/wpt-cascade`
//! with each test file beside its cases.
//!
//! A computed-value case parses the file with Parsec, puts in the
//! `<template>` contents the test put in and the declarations it set on
//! its `<style>`s' rules, applies those and the `<style>`s the test added,
//! and reads the element the test read (the nth match of the selector it
//! found it with, under the element it looked in), or its pseudo-element:
//! it must compute to what the test expects, or to what another read the
//! test compares it with gives. `@import`s are
//! fetched from the files beside the test, or from their `data:` URL. A
//! container's size is its computed `width` and `height`, where they're
//! in pixels, which is what the tests give them.
//!
//! A rule case reads a sheet as CSSOM would: how many rules it keeps (an
//! invalid at-rule or style rule is dropped), and an attribute of one of
//! them (`conditionText`, `syntax`, whether its text has a descriptor in
//! it). A case for an element the page doesn't have (the test looped over
//! every match) isn't counted.
//!
//! `tests/wpt-cascade/known-failures.txt` lists what fails today; the run
//! fails if anything else fails or a listed case passes.
//! `UPDATE_KNOWN_FAILURES=1` rewrites it.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use parsec::css::SelectorList;
use parsec::{Document, Kind, NodeId};
use parsec_css::cascade::{Origin, Pseudo, Styled, Styler};
use parsec_css::media::Environment;
use parsec_css::rules::{
    ContainerQuery, CounterStyle, Import, Keyframes, PropertyRule, ScopeBounds,
};
use parsec_css::{Block, Rule};
use serde_json::Value;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/wpt-cascade")
}

/// The element a case names, under the element it was looked for in.
fn find(doc: &Document, el: &Value) -> Option<NodeId> {
    let scope = match el.get("within") {
        Some(w) if !w.is_null() => find(doc, w)?,
        _ => doc.root(),
    };
    let sel = SelectorList::parse(el["selector"].as_str()?).ok()?;
    let index = el["index"].as_u64()? as usize;
    sel.select(doc, scope).get(index).copied()
}

/// A copy of `from`'s children put under `into`: a `<template>`'s
/// contents cloned into the page, as `append(content.cloneNode(true))`.
fn clone_into(doc: &mut Document, from: NodeId, into: NodeId) {
    let mut stack: Vec<(NodeId, NodeId)> = doc.children(from).map(|c| (c, into)).collect();
    stack.reverse();
    while let Some((n, parent)) = stack.pop() {
        let made = match doc.kind(n) {
            Kind::Element { ns, name } => {
                let name = name.to_string();
                let attrs: Vec<(parsec::AttrNs, String, String)> = doc
                    .attrs(n)
                    .map(|(a, k, v)| (a, k.to_string(), v.to_string()))
                    .collect();
                let e = doc.create_element(ns, &name);
                for (a, k, v) in &attrs {
                    doc.add_attr(e, *a, k, v);
                }
                e
            }
            Kind::Text(t) => {
                let t = t.to_string();
                doc.create_text(&t)
            }
            _ => continue,
        };
        let _ = doc.append(parent, made);
        if matches!(doc.kind(made), Kind::Element { name, .. } if name != "template") {
            let kids: Vec<NodeId> = doc.children(n).collect();
            for k in kids.into_iter().rev() {
                stack.push((k, made));
            }
        }
    }
}

/// The page a case reads: the file, with the declarations it set on its
/// `<style>`s' rules, and the templates it put in.
fn page(source: &str, inserts: Option<&Value>, restyle: Option<&Value>) -> Document {
    let mut doc = parsec::parse(source, true);
    if let Some(list) = restyle.and_then(Value::as_array) {
        let styles = SelectorList::parse("style")
            .unwrap()
            .select(&doc, doc.root());
        for r in list {
            let (Some(i), Some(text)) = (r["index"].as_u64(), r["text"].as_str()) else {
                continue;
            };
            let Some(&style) = styles.get(i as usize) else {
                continue;
            };
            let kids: Vec<NodeId> = doc.children(style).collect();
            for k in kids {
                doc.remove(k);
            }
            let t = doc.create_text(text);
            let _ = doc.append(style, t);
        }
    }
    let Some(list) = inserts.and_then(Value::as_array) else {
        return doc;
    };
    let templates = SelectorList::parse("template")
        .unwrap()
        .select(&doc, doc.root());
    for i in list {
        let (Some(into), Some(t)) = (find(&doc, &i["into"]), i["template"].as_u64()) else {
            continue;
        };
        let Some(content) = templates
            .get(t as usize)
            .and_then(|t| doc.template_contents(*t))
        else {
            continue;
        };
        clone_into(&mut doc, content, into);
    }
    doc
}

/// `data:text/css,…` decoded, or a file beside the test.
fn fetch(base: &Path, url: &str) -> Option<String> {
    if let Some(data) = url.strip_prefix("data:") {
        let (_, body) = data.split_once(',')?;
        let bytes = body.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%'
                && let Some(h) = body.get(i + 1..i + 3)
                && let Ok(b) = u8::from_str_radix(h, 16)
            {
                out.push(b);
                i += 3;
                continue;
            }
            out.push(bytes[i]);
            i += 1;
        }
        return String::from_utf8(out).ok();
    }
    std::fs::read_to_string(base.join(url)).ok()
}

fn styler(env: &Environment, doc: &Document, sheets: &[&str], base: PathBuf) -> Styler {
    let mut s = Styler::new(env.clone())
        .with_document_styles(doc)
        .fetch(move |url| fetch(&base, url))
        .container_sizes(|_, c| {
            let px = |p: &str| c.get(p)?.strip_suffix("px")?.parse::<f64>().ok();
            Some((px("width")?, px("height").unwrap_or(0.0)))
        });
    for css in sheets {
        s.add(css, Origin::Author);
    }
    s
}

/// `getComputedStyle(el, pseudo)[property]`, for the read `case` names.
/// A pseudo-element the cascade doesn't style reads as the empty string.
fn read(styled: &Styled, el: NodeId, case: &Value) -> String {
    let property = case["property"].as_str().unwrap();
    match case["pseudo"].as_str() {
        None => styled.get(el, property),
        Some(p) => Pseudo::parse(p).and_then(|p| styled.get_pseudo(el, &p, property)),
    }
    .unwrap_or_default()
}

/// A rule as CSSOM keeps it.
enum Om {
    /// With its selector's text.
    Style(String),
    Container(Vec<ContainerQuery>),
    CounterStyle(CounterStyle),
    Property(PropertyRule),
    Keyframes(Keyframes),
    Scope(ScopeBounds),
    Import,
    Other(&'static str),
}

/// A rule as CSSOM has it, or `None` for one it drops.
fn cssom(rule: &Rule<'_, Block<'_>>) -> Option<Om> {
    match rule {
        Rule::Qualified(q) => SelectorList::parse(parsec_css::to_css(&q.prelude).trim())
            .ok()
            .map(|_| Om::Style(parsec_css::to_css(&q.prelude).trim().to_string())),
        Rule::At(a) => {
            let name = a.name.to_ascii_lowercase();
            Some(match name.as_str() {
                "container" => {
                    a.block.as_ref()?;
                    Om::Container(ContainerQuery::parse_list(&a.prelude)?)
                }
                "counter-style" => Om::CounterStyle(CounterStyle::parse(a)?),
                "property" => Om::Property(PropertyRule::parse(a)?),
                "keyframes" => Om::Keyframes(Keyframes::parse(a)?),
                "scope" => {
                    a.block.as_ref()?;
                    let b = ScopeBounds::parse(&a.prelude)?;
                    for s in b.start.iter().chain(&b.end) {
                        SelectorList::parse(s).ok()?;
                    }
                    Om::Scope(b)
                }
                "import" => {
                    Import::parse(a)?;
                    Om::Import
                }
                "media" => Om::Other("CSSMediaRule"),
                "supports" => Om::Other("CSSSupportsRule"),
                "layer" if a.block.is_some() => Om::Other("CSSLayerBlockRule"),
                "layer" => Om::Other("CSSLayerStatementRule"),
                "font-face" => Om::Other("CSSFontFaceRule"),
                "page" => Om::Other("CSSPageRule"),
                _ => return None,
            })
        }
    }
}

/// A CSSOM attribute of a rule, as JSON.
fn attribute(om: &Om, attr: &str) -> Value {
    let s = |x: &str| Value::String(x.into());
    match (om, attr) {
        (Om::Style(_), "constructor.name") => s("CSSStyleRule"),
        (Om::Style(sel), "selectorText") => s(sel),
        (Om::Container(_), "constructor.name") => s("CSSContainerRule"),
        (Om::CounterStyle(_), "constructor.name") => s("CSSCounterStyleRule"),
        (Om::Property(_), "constructor.name") => s("CSSPropertyRule"),
        (Om::Keyframes(_), "constructor.name") => s("CSSKeyframesRule"),
        (Om::Scope(_), "constructor.name") => s("CSSScopeRule"),
        (Om::Import, "constructor.name") => s("CSSImportRule"),
        (Om::Other(n), "constructor.name") => s(n),
        (Om::Container(q), "conditionText") => s(&q
            .iter()
            .map(|q| q.to_string())
            .collect::<Vec<_>>()
            .join(", ")),
        (Om::Container(q), "containerName") => s(q[0].name.as_deref().unwrap_or("")),
        (Om::Container(q), "containerQuery") => s(&q[0]
            .condition
            .as_ref()
            .map(|c| c.to_string())
            .unwrap_or_default()),
        (Om::Property(p), "name") => s(&p.name),
        (Om::Property(p), "syntax") => s(&p.syntax),
        (Om::Property(p), "inherits") => Value::Bool(p.inherits),
        (Om::Property(p), "initialValue") => p.initial_value.as_deref().map_or(Value::Null, s),
        (Om::CounterStyle(c), "name") => s(&c.name),
        (Om::CounterStyle(c), d) => {
            let d: String = d
                .chars()
                .flat_map(|ch| match ch.is_ascii_uppercase() {
                    true => vec!['-', ch.to_ascii_lowercase()],
                    false => vec![ch],
                })
                .collect();
            s(c.get(&d).unwrap_or(""))
        }
        (Om::Keyframes(k), "name") => s(&k.name),
        (Om::Scope(b), "start") => b.start.as_deref().map_or(Value::Null, s),
        (Om::Scope(b), "end") => b.end.as_deref().map_or(Value::Null, s),
        _ => Value::String(format!("<no {attr}>")),
    }
}

fn css_text(om: &Om) -> String {
    match om {
        Om::CounterStyle(c) => c.to_string(),
        Om::Property(p) => p.to_string(),
        Om::Keyframes(k) => k.to_string(),
        _ => String::new(),
    }
}

/// What a rule case reads, and whether that's what the test expects.
fn rule_case(case: &Value) -> (bool, String) {
    let sheet = parsec_css::parse_stylesheet_deep(case["sheet"].as_str().unwrap());
    let kept: Vec<Om> = sheet.rules.iter().filter_map(cssom).collect();
    let index = case["index"].as_u64().unwrap_or(0) as usize;
    match case["kind"].as_str().unwrap() {
        "rules" => {
            let got = Value::from(kept.len());
            (got == case["expect"], got.to_string())
        }
        "rule" => {
            let got = kept.get(index).map_or(Value::Null, |om| {
                attribute(om, case["attr"].as_str().unwrap())
            });
            (got == case["expect"], got.to_string())
        }
        "contains" => {
            let text = case["text"].as_str().unwrap();
            // A test that finds no rule takes that as the descriptor's
            // being invalid.
            let got = kept.get(index).map(|om| css_text(om).contains(text));
            let ok = match got {
                Some(g) => Value::Bool(g) == case["expect"],
                None => case["expect"] == Value::Bool(false),
            };
            (ok, format!("{got:?}"))
        }
        other => panic!("unknown case kind {other}"),
    }
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
    let mut by_area: HashMap<String, (usize, usize)> = HashMap::new();
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
        let source = std::fs::read_to_string(dir().join(name)).unwrap();
        let base = dir().join(name).parent().unwrap().to_path_buf();
        type Page = (Document, HashMap<String, Styled>);
        let mut pages: HashMap<String, Page> = HashMap::new();
        for case in cases {
            let mut key = case.clone();
            key.as_object_mut().unwrap().remove("file");
            let key = serde_json::to_string(&[Value::String(name.into()), key]).unwrap();
            if !seen.insert(key.clone()) {
                continue;
            }
            let (ok, got) = match case.get("kind").and_then(Value::as_str) {
                Some(_) => rule_case(case),
                None => {
                    let sheets: Vec<&str> = case["sheets"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(Value::as_str)
                        .collect();
                    let (inserts, restyle) = (case.get("inserts"), case.get("restyle"));
                    let (doc, styled) = pages
                        .entry(format!("{inserts:?}{restyle:?}"))
                        .or_insert_with(|| (page(&source, inserts, restyle), HashMap::new()));
                    let Some(el) = find(doc, &case["element"]) else {
                        seen.remove(&key);
                        continue;
                    };
                    let s = styled
                        .entry(sheets.join("\u{0}"))
                        .or_insert_with(|| styler(&env, doc, &sheets, base.clone()).style(doc));
                    let got = read(s, el, case);
                    // Or what the test reads somewhere else.
                    let want = match &case["like"] {
                        Value::Null => case["expect"].as_str().map(String::from),
                        like => find(doc, &like["element"]).map(|o| read(s, o, like)),
                    };
                    (Some(&got) == want.as_ref(), got)
                }
            };
            let area = name.split('/').nth(1).unwrap_or("").to_string();
            let count = by_area.entry(area).or_default();
            if ok {
                passed += 1;
                count.0 += 1;
                if known.contains(&key) {
                    unexpected.push(format!("listed as failing, but passes: {key}"));
                }
            } else {
                failed += 1;
                count.1 += 1;
                if !known.contains(&key) {
                    unexpected.push(format!("{key}\n  got: {got}"));
                }
                failing.insert(key);
            }
        }
    }
    println!("wpt cascade: {passed} passed, {failed} failed, {skipped} skipped");
    let mut areas: Vec<_> = by_area.into_iter().collect();
    areas.sort();
    for (area, (p, f)) in areas {
        println!("  {area}: {p} passed, {f} failed");
    }
    summary::write(&summary::Counts {
        suite: "wpt-cascade",
        passed,
        failed,
        skipped,
        skipped_because: "they change the page from script beyond adding styles and templates",
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
