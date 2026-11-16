// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times each selector in a battery over every real page, Parsec's
//! `SelectorList::select` against Lexbor's `lxb_selectors_find`, with both
//! parsers' documents built beforehand.

use std::hint::black_box;
use std::time::Instant;

use crate::lexbor;
use crate::pages::Page;

/// The battery in crates/parsec/examples/select.rs.
const BATTERY: &[&str] = &[
    "*",
    "a",
    "div a",
    "ul > li",
    "li + li",
    "li ~ li",
    "[href]",
    "a[href^='/']",
    "a[href$='.html']",
    "a[href*='?']",
    "[class]",
    "img[alt]",
    "div:first-child",
    "li:last-child",
    "p:nth-child(2n+1)",
    "tr:nth-of-type(odd)",
    "span:only-child",
    ":root > body",
    "div:not([class])",
    ":is(h1, h2, h3)",
    "article:has(img)",
    "meta[name][content]",
    "script:empty",
    "td:nth-last-child(2)",
    "nav ul li a",
    "div > div > div",
    "[data-testid]",
    "input[name]",
    "a:first-of-type",
    "section h2 + p",
    "li:nth-child(-n+3)",
    "body *:last-child",
];

/// The median of `runs` timings of `f`, after one warm-up, in seconds.
fn median(runs: usize, mut f: impl FnMut() -> usize) -> (f64, usize) {
    let found = f();
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            black_box(f());
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    (times[times.len() / 2], found)
}

pub fn run(pages: &[Page]) {
    let parsec_docs: Vec<parsec::Document> = pages
        .iter()
        .map(|p| parsec::parse(&p.html, false))
        .collect();
    let lexbor_docs: Vec<lexbor::Document> = pages
        .iter()
        .map(|p| lexbor::Document::parse(p.html.as_bytes()))
        .collect();
    let engine = lexbor::Selectors::new();

    println!(
        "\nSelectors over the {} real pages, median ms of 30 runs\n{:<28} {:>10} {:>10} {:>10} {:>16}",
        pages.len(),
        "",
        "Parsec",
        "Lexbor",
        "vs Lexbor",
        "matches"
    );
    let (mut parsec_total, mut lexbor_total) = (0.0, 0.0);
    for css in BATTERY {
        let ours = parsec::css::SelectorList::parse(css).unwrap();
        let theirs = engine.parse(css);
        let (p, p_found) = median(30, || {
            parsec_docs
                .iter()
                .map(|d| ours.select(d, d.root()).len())
                .sum()
        });
        let (l, l_found) = median(30, || {
            lexbor_docs.iter().map(|d| engine.count(d, &theirs)).sum()
        });
        parsec_total += p;
        lexbor_total += l;
        // A different count means the two disagree about what matches,
        // which makes the times incomparable: it's shown, not hidden.
        let matches = if p_found == l_found {
            p_found.to_string()
        } else {
            format!("{p_found} vs {l_found}")
        };
        println!(
            "  {:<26} {:>10.2} {:>10.2} {:>9.2}x {:>16}",
            css,
            p * 1e3,
            l * 1e3,
            l / p,
            matches
        );
    }
    println!(
        "  {:<26} {:>10.2} {:>10.2} {:>9.2}x",
        "all selectors",
        parsec_total * 1e3,
        lexbor_total * 1e3,
        lexbor_total / parsec_total
    );
}
