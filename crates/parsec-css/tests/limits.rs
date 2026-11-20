// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Stylesheets built to be deep or to blow up, parsed and cascaded on a
//! small stack in bounded time.

use std::time::{Duration, Instant};

use parsec_css::cascade::{Origin, Styler};

/// Runs `f` on a thread with `stack` bytes of stack, failing if it takes
/// longer than a quadratic path would let it finish in.
fn bounded(name: &str, stack: usize, f: impl FnOnce() + Send + 'static) {
    let start = Instant::now();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap_or_else(|_| panic!("{name} panicked"));
    let took = start.elapsed();
    assert!(took < Duration::from_secs(20), "{name} took {took:?}");
}

fn style(html: &str, css: &str) -> Option<String> {
    let doc = parsec::parse(html, false);
    let mut styler = Styler::new(Default::default());
    styler.add(css, Origin::Author);
    let styled = styler.with_document_styles(&doc).style(&doc);
    let p = parsec::css::SelectorList::parse("p").unwrap();
    let p = p.select_first(&doc, doc.root())?;
    styled.get(p, "color")
}

const STACK: usize = 1 << 20;

#[test]
fn deep_values_and_rules_stay_on_the_stack() {
    let cases: Vec<(&str, String, String)> = vec![
        (
            "var() fallbacks",
            String::new(),
            format!("p {{ color: red; --x: {} }}", "var(--a, ".repeat(50_000)),
        ),
        (
            "a style attribute's blocks",
            format!("<p style='--x:{}'>", "{".repeat(50_000)),
            String::new(),
        ),
        ("blocks as rules", String::new(), "{b".repeat(100_000)),
        (
            "nested @media",
            String::new(),
            format!(
                "{} p {{ color: red }}",
                "@media (min-width:1px){".repeat(20_000)
            ),
        ),
        (
            "nested style rules",
            String::new(),
            format!("{} color: red", "p{".repeat(20_000)),
        ),
    ];
    for (name, html, css) in cases {
        bounded(name, STACK, move || {
            let sheet = parsec_css::parse_stylesheet_deep(&css);
            let _ = format!("{sheet:?}");
            assert!(sheet.clone() == sheet);
            style(&format!("{html}<p>x"), &css);
        });
    }
}

#[test]
fn shallow_enough_still_applies() {
    // Well inside the caps, nesting works as before.
    let css = format!(
        "{} p {{ color: red }} {}",
        "@media all { ".repeat(100),
        "}".repeat(100)
    );
    assert_eq!(style("<p>x", &css).as_deref(), Some("rgb(255, 0, 0)"));
    let css = format!("{}p {{ color: red }}{}", "div { ".repeat(8), "}".repeat(8));
    let html = format!("{}<p>x", "<div>".repeat(8));
    assert_eq!(style(&html, &css).as_deref(), Some("rgb(255, 0, 0)"));
}

#[test]
fn var_substitution_is_capped_in_bytes() {
    // Each property twice the one before, from a 20 KB string: 2^18 copies.
    let mut css = format!("p {{ --v0: \"{}\";", "\\61".repeat(5_000));
    for i in 1..=18 {
        css += &format!(" --v{i}: var(--v{}) var(--v{});", i - 1, i - 1);
    }
    css += " color: red; color: var(--v18) }";
    bounded("doubling vars", STACK, move || {
        // Too big to substitute: invalid at computed-value time, so unset.
        assert_eq!(style("<p>x", &css).as_deref(), Some("rgb(0, 0, 0)"));
    });
    // A value under the cap substitutes as before.
    let css = "p { --a: green; --b: var(--a); color: var(--b) }";
    assert_eq!(style("<p>x", css).as_deref(), Some("rgb(0, 128, 0)"));
}

#[test]
fn nesting_selectors_are_capped() {
    // Every level doubles the selector it writes in.
    let css = format!(
        "p {{ color: red; {} color: blue }}",
        "& &, & & {".repeat(400)
    );
    bounded("doubling &", STACK, move || {
        assert_eq!(style("<p>x", &css).as_deref(), Some("rgb(255, 0, 0)"));
    });
    // Nested under one parent, each level used to wrap the one above in
    // `:is()`, and matching that doubled with every level.
    let css = format!(
        "{}p {{ color: blue }}{}",
        "div { ".repeat(40),
        "}".repeat(40)
    );
    let html = format!("{}<p>x", "<div>".repeat(40));
    bounded("nested under real ancestors", STACK, move || {
        assert_eq!(style(&html, &css).as_deref(), Some("rgb(0, 0, 255)"));
    });
    let css = "div { & > p { color: blue } }";
    assert_eq!(style("<div><p>x", css).as_deref(), Some("rgb(0, 0, 255)"));
    assert_eq!(style("<div><b><p>x", css).as_deref(), Some("rgb(0, 0, 0)"));
    let css = "div { & > & > p { color: blue } }";
    assert_eq!(
        style("<div><div><p>x", css).as_deref(),
        Some("rgb(0, 0, 255)")
    );
    assert_eq!(style("<div><p>x", css).as_deref(), Some("rgb(0, 0, 0)"));
    let css = format!("p {{ {} color: blue }}", "& {".repeat(2_000));
    bounded("deep &", STACK, move || {
        style("<p>x", &css);
    });
}

#[test]
fn layer_names_are_capped() {
    let css = format!(
        "@layer {} {{ p {{ color: red }} }} p {{ color: green }}",
        vec!["a"; 5_000].join(".")
    );
    bounded("long layer name", STACK, move || {
        assert_eq!(style("<p>x", &css).as_deref(), Some("rgb(0, 128, 0)"));
    });
    let css = format!("{} p {{ color: red }}", "@layer x {".repeat(5_000));
    bounded("nested layers", STACK, move || {
        style("<p>x", &css);
    });
    let css = format!("{} p {{ color: red }}", "@layer x {".repeat(10));
    assert_eq!(style("<p>x", &css).as_deref(), Some("rgb(255, 0, 0)"));
    let css = "@layer a.b.c { p { color: red } } @layer a { p { color: green } }";
    assert_eq!(style("<p>x", css).as_deref(), Some("rgb(0, 128, 0)"));
}

#[test]
fn rules_without_semicolons_parse_in_linear_time() {
    let css = "a{}".repeat(40_000);
    bounded("a{} block contents", STACK, move || {
        assert_eq!(parsec_css::parse_block_contents(&css).items.len(), 40_000);
        let wrapped = format!("x{{{css}}}");
        let sheet = parsec_css::parse_stylesheet_deep(&wrapped);
        let parsec_css::Rule::Qualified(x) = &sheet.rules[0] else {
            panic!()
        };
        assert_eq!(x.block.items.len(), 40_000);
        let sheet = parsec_css::parse_stylesheet(&wrapped);
        let parsec_css::Rule::Qualified(x) = sheet.rules.into_iter().next().unwrap() else {
            panic!()
        };
        assert_eq!(x.block.contents(&wrapped).items.len(), 40_000);
    });
    let css = "a:b{}".repeat(40_000);
    bounded("a:b{} block contents", STACK, move || {
        assert_eq!(parsec_css::parse_block_contents(&css).items.len(), 40_000);
        let wrapped = format!("x{{{css}}}");
        let sheet = parsec_css::parse_stylesheet_deep(&wrapped);
        let parsec_css::Rule::Qualified(x) = &sheet.rules[0] else {
            panic!()
        };
        assert_eq!(x.block.items.len(), 40_000);
    });
    // What a declaration can be is unchanged.
    let block =
        parsec_css::parse_block_contents("a: {b} !important; c: {d} e; f: g {h}; --i: {j} k");
    let names: Vec<_> = block
        .items
        .iter()
        .map(|i| match i {
            parsec_css::Item::Declaration(d) => format!("decl {}", d.name),
            parsec_css::Item::Rule(_) => "rule".to_string(),
        })
        .collect();
    assert_eq!(names, ["decl a", "rule", "rule", "decl --i"]);
}
