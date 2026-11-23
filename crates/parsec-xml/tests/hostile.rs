// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Documents built to cost more than they're worth: each stops with an
//! error, or parses in time that grows with its size and no faster.

use std::time::{Duration, Instant};

use parsec_xml::{Limits, Options, parse, parse_with};

fn unlimited_depth() -> Options {
    Options {
        limits: Limits::default(),
        ..Options::default()
    }
}

/// How long `f` takes, the best of three.
fn time(f: impl Fn()) -> Duration {
    (0..3)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

/// Ten times the input in at most about ten times the time, with room
/// for noise: a quadratic parse would take a hundred.
fn assert_linear(what: &str, make: impl Fn(usize) -> String, parse: impl Fn(&str)) {
    let (small, big) = (make(20_000), make(200_000));
    let (ts, tb) = (time(|| parse(&small)), time(|| parse(&big)));
    let ratio = tb.as_secs_f64() / ts.as_secs_f64().max(1e-6);
    assert!(
        ratio < 35.0,
        "{what}: 10x the input took {ratio:.1}x the time ({ts:?}, {tb:?})"
    );
}

#[test]
fn a_billion_laughs_stops() {
    let mut dtd = String::from("<!DOCTYPE lolz [<!ENTITY lol0 \"lol\">");
    for i in 1..10 {
        let refs = format!("&lol{};", i - 1).repeat(10);
        dtd.push_str(&format!("<!ENTITY lol{i} \"{refs}\">"));
    }
    dtd.push_str("]>");
    let t = Instant::now();
    let e = parse(&format!("{dtd}<lolz>&lol9;</lolz>")).unwrap_err();
    assert_eq!(e.code(), "entity-expansion-limit");
    // The same in an attribute value.
    let e = parse(&format!("{dtd}<lolz a='&lol9;'/>")).unwrap_err();
    assert_eq!(e.code(), "entity-expansion-limit");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn a_quadratic_blowup_stops() {
    // One big entity, referenced many times: each reference is small, but
    // together they'd be 20 GB.
    let big = "x".repeat(100_000);
    let refs = "&big;".repeat(200_000);
    let src = format!("<!DOCTYPE a [<!ENTITY big \"{big}\">]><a>{refs}</a>");
    let t = Instant::now();
    assert_eq!(parse(&src).unwrap_err().code(), "entity-expansion-limit");
    let src = format!("<!DOCTYPE a [<!ENTITY big \"{big}\">]><a b=\"{refs}\"/>");
    assert_eq!(parse(&src).unwrap_err().code(), "entity-expansion-limit");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    // A big default value on many elements: stored once, but each element
    // hands it out, so it counts against the budget each time.
    let src = format!(
        "<!DOCTYPE a [<!ATTLIST b d CDATA \"{big}\">]><a>{}</a>",
        "<b/>".repeat(100_000)
    );
    let t = Instant::now();
    assert_eq!(parse(&src).unwrap_err().code(), "entity-expansion-limit");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

#[test]
fn ordinary_defaults_on_many_elements_are_fine() {
    // A DTD's usual defaults, a short value on every element, stay well
    // inside the budget however many elements there are.
    let src = format!(
        "<!DOCTYPE a [<!ATTLIST b class CDATA \"item\" lang CDATA \"en\">]><a>{}</a>",
        "<b/>".repeat(200_000)
    );
    let doc = parse(&src).unwrap();
    let b = doc.first_child(doc.root_element()).unwrap();
    assert_eq!(doc.attribute_named(b, "class"), Some("item"));
    // Up to the allowance with no ratio, then over it.
    let tight = Options {
        entity_allowance: 9,
        entity_ratio: 0,
        ..Options::default()
    };
    let two = "<!DOCTYPE a [<!ATTLIST b d CDATA \"1234\">]><a><b/><b/></a>";
    assert!(parse_with(two, &tight).is_ok());
    let three = "<!DOCTYPE a [<!ATTLIST b d CDATA \"1234\">]><a><b/><b/><b/></a>";
    assert_eq!(
        parse_with(three, &tight).unwrap_err().code(),
        "entity-expansion-limit"
    );
}

#[test]
fn expansion_within_the_limit_is_fine() {
    let src = "<!DOCTYPE a [<!ENTITY e \"0123456789\">]><a>&e;&e;&e;</a>";
    let doc = parse(src).unwrap();
    assert_eq!(doc.text(doc.root()), "0123456789".repeat(3));
    let tight = Options {
        entity_allowance: 25,
        entity_ratio: 0,
        ..Options::default()
    };
    assert_eq!(
        parse_with(src, &tight).unwrap_err().code(),
        "entity-expansion-limit"
    );
}

#[test]
fn entities_nest_only_so_deep() {
    let mut dtd = String::from("<!DOCTYPE a [<!ENTITY e0 \"x\">");
    for i in 1..40 {
        dtd.push_str(&format!("<!ENTITY e{i} \"&e{};\">", i - 1));
    }
    dtd.push_str("]>");
    assert!(parse(&format!("{dtd}<a>&e15;</a>")).is_ok());
    assert_eq!(
        parse(&format!("{dtd}<a>&e16;</a>")).unwrap_err().code(),
        "entity-too-deep"
    );
    assert_eq!(
        parse(&format!("{dtd}<a b='&e39;'/>")).unwrap_err().code(),
        "entity-too-deep"
    );
    // However deep the option says, the ceiling holds.
    let deep = Options {
        max_entity_depth: u32::MAX,
        ..Options::default()
    };
    let mut dtd = String::from("<!DOCTYPE a [<!ENTITY e0 \"x\">");
    for i in 1..1000 {
        dtd.push_str(&format!("<!ENTITY e{i} \"&e{};\">", i - 1));
    }
    dtd.push_str("]>");
    let e = parse_with(&format!("{dtd}<a>&e999;</a>"), &deep).unwrap_err();
    assert_eq!(e.code(), "entity-too-deep");
    // Parameter entities too.
    let mut dtd = String::from("<!DOCTYPE a [<!ENTITY % p0 \"\">");
    for i in 1..100 {
        dtd.push_str(&format!("<!ENTITY % p{i} \"&#37;p{};\">", i - 1));
    }
    let e = parse(&format!("{dtd}%p99;]><a/>")).unwrap_err();
    assert_eq!(e.code(), "entity-too-deep");
    let e = parse("<!DOCTYPE a [<!ENTITY % p \"&#37;p;\">%p;]><a/>").unwrap_err();
    assert_eq!(e.code(), "recursive-entity");
}

#[test]
fn a_million_levels_deep() {
    let n = 1_000_000;
    let src = "<a>".repeat(n) + &"</a>".repeat(n);
    let t = Instant::now();
    let e = parse(&src).unwrap_err();
    assert_eq!(e.code(), "too-deep");
    assert!(t.elapsed() < Duration::from_secs(1), "{:?}", t.elapsed());
    // Without a limit it parses, with no recursion to overflow, and
    // walks and writes back out the same way.
    let doc = parse_with(&src, &unlimited_depth()).unwrap();
    assert_eq!(doc.len(), n + 1);
    assert_eq!(doc.descendants(doc.root()).count(), n + 1);
    assert_eq!(
        doc.ancestors(doc.last_child(doc.root()).unwrap()).count(),
        1
    );
    assert_eq!(
        doc.to_xml().len(),
        "<a>".len() * (n - 1) + "</a>".len() * (n - 1) + "<a/>".len()
    );
    // Unclosed, it's an error at the end, not a stack overflow.
    let open = "<a>".repeat(n);
    assert_eq!(
        parse_with(&open, &unlimited_depth()).unwrap_err().code(),
        "unexpected-eof"
    );
    assert_linear(
        "nesting",
        |n| "<a>".repeat(n) + &"</a>".repeat(n),
        |s| {
            parse_with(s, &unlimited_depth()).unwrap();
        },
    );
}

#[test]
fn deep_content_models_and_ignored_sections() {
    let n = 200_000;
    let model = "(".repeat(n) + "a" + &")".repeat(n);
    let src = format!("<!DOCTYPE a [<!ELEMENT a {model}>]><a/>");
    assert!(parse(&src).is_ok());
    let sections = "<![IGNORE[".repeat(n) + &"]]>".repeat(n);
    let src = format!("<!DOCTYPE a [<!ENTITY % s '{sections}'>%s;]><a/>");
    assert!(parse(&src).is_ok());
}

#[test]
fn many_attributes_are_linear() {
    assert_linear(
        "attributes",
        |n| {
            let attrs: String = (0..n).map(|i| format!(" a{i}='{i}'")).collect();
            format!("<e{attrs}/>")
        },
        |s| {
            parse(s).unwrap();
        },
    );
    // A duplicate among many is still found.
    let attrs: String = (0..50_000).map(|i| format!(" a{i}='{i}'")).collect();
    assert_eq!(
        parse(&format!("<e{attrs} a7='x'/>")).unwrap_err().code(),
        "duplicate-attribute"
    );
}

#[test]
fn many_namespaces_are_linear() {
    // Many declarations on one element, each prefix used by an attribute.
    assert_linear(
        "namespaces",
        |n| {
            let decls: String = (0..n).map(|i| format!(" xmlns:p{i}='urn:{i}'")).collect();
            let uses: String = (0..n).map(|i| format!(" p{i}:a='{i}'")).collect();
            format!("<e{decls}{uses}/>")
        },
        |s| {
            parse(s).unwrap();
        },
    );
    // Declarations nested deep, and names resolved under all of them.
    assert_linear(
        "nested namespaces",
        |n| {
            let open: String = (0..n)
                .map(|i| format!("<p{i}:e xmlns:p{i}='urn:{i}'>"))
                .collect();
            let close: String = (0..n).rev().map(|i| format!("</p{i}:e>")).collect();
            open + &close
        },
        |s| {
            parse_with(s, &unlimited_depth()).unwrap();
        },
    );
    let attrs: String = (0..50_000)
        .map(|i| format!(" xmlns:p{i}='urn:same' p{i}:a='1'"))
        .collect();
    assert_eq!(
        parse(&format!("<e{attrs}/>")).unwrap_err().code(),
        "duplicate-attribute"
    );
}

#[test]
fn many_entities_and_long_names_are_linear() {
    assert_linear(
        "entities",
        |n| {
            let decls: String = (0..n).map(|i| format!("<!ENTITY e{i} 'v{i}'>")).collect();
            let refs: String = (0..n).map(|i| format!("&e{i};")).collect();
            format!("<!DOCTYPE a [{decls}]><a>{refs}</a>")
        },
        |s| {
            parse(s).unwrap();
        },
    );
    assert_linear(
        "a long name",
        |n| format!("<{0}>text</{0}>", "n".repeat(n * 10)),
        |s| {
            parse(s).unwrap();
        },
    );
    assert_linear(
        "text and references",
        |n| format!("<a>{}</a>", "x&amp;&#x41;".repeat(n)),
        |s| {
            parse(s).unwrap();
        },
    );
}

#[test]
fn the_step_limit_holds() {
    let limited = Options {
        limits: Limits {
            max_steps: Some(1000),
            ..Options::default().limits
        },
        ..Options::default()
    };
    let src = format!("<a>{}</a>", "<b/>".repeat(2000));
    assert_eq!(parse_with(&src, &limited).unwrap_err().code(), "step-limit");
    let small = Options {
        limits: Limits {
            max_input: Some(10),
            ..Options::default().limits
        },
        ..Options::default()
    };
    assert_eq!(
        parse_with(&src, &small).unwrap_err().code(),
        "input-too-large"
    );
}
