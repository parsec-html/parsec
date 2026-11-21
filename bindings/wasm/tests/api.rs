// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// The exports as JavaScript calls them, run in Node by
// `wasm-pack test --node`; browser.rs runs the same in a browser.

use parsec_html_wasm::{ByteParser, SelectOptions, parse, parse_bytes, parse_fragment};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen(inline_js = "export function steps(n) { return { maxSteps: n }; }
export function message(e) { return e.message; }")]
extern "C" {
    fn steps(n: f64) -> SelectOptions;
    fn message(e: JsValue) -> String;
}

const PAGE: &str = "<!doctype html><title>T</title>\
    <div id=a class='x y'>one <b>two</b><!--c--></div><p>p1</p>";

#[wasm_bindgen_test]
fn parts_of_a_document() {
    let doc = parse(PAGE, None).unwrap();
    assert_eq!(doc.root().node_type(), 9);
    assert_eq!(
        doc.document_element().unwrap().local_name().unwrap(),
        "html"
    );
    assert_eq!(doc.head().unwrap().node_name(), "HEAD");
    assert_eq!(doc.body().unwrap().children().len(), 2);
    assert_eq!(doc.text(), "Tone twop1");
    assert!(
        doc.html()
            .starts_with("<!DOCTYPE html><html><head><title>T</title>")
    );
    assert_eq!(doc.root().first_child().unwrap().node_type(), 10);
}

#[wasm_bindgen_test]
fn a_step_limit_that_runs_out_throws() {
    // A :has() of two compounds over deep nesting costs depth squared.
    let deep = parse(&"<div>".repeat(3000), None).unwrap();
    let hostile = "div:has(span div)";
    let Err(e) = deep.select(hostile, Some(steps(100_000.0))) else {
        panic!("the step limit didn't stop it");
    };
    assert_eq!(
        message(e.into()),
        "maxSteps: matching took more than 100000 steps"
    );
    assert!(deep.select_first(hostile, Some(steps(100_000.0))).is_err());
    let outer = deep.select_first("div", None).unwrap().unwrap();
    assert!(outer.matches(hostile, Some(steps(100_000.0))).is_err());
    assert!(deep.select("div", Some(steps(-1.0))).is_err());
    assert!(deep.select("div", Some(steps(1.5))).is_err());
}

#[wasm_bindgen_test]
fn a_generous_step_limit_changes_nothing() {
    let doc = parse(PAGE, None).unwrap();
    let html =
        |n: Vec<parsec_html_wasm::Node>| n.iter().map(|n| n.outer_html()).collect::<Vec<_>>();
    for q in ["div b", "div, p", ":has(> b)", "nope"] {
        let limited = doc.select(q, Some(steps(1e6))).unwrap();
        assert_eq!(html(limited), html(doc.select(q, None).unwrap()));
        let first = doc.select_first(q, Some(steps(1e6))).unwrap();
        let plain = doc.select_first(q, None).unwrap();
        assert_eq!(first.map(|n| n.outer_html()), plain.map(|n| n.outer_html()));
    }
    let div = doc.select_first("div", None).unwrap().unwrap();
    assert!(div.matches("body > div", Some(steps(1e6))).unwrap());
    assert!(div.select("div", Some(steps(1e6))).unwrap().is_empty());
}

#[wasm_bindgen_test]
fn selectors() {
    let doc = parse(PAGE, None).unwrap();
    let hits = doc.select("div, p", None).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].text_content(), "p1");
    let div = doc.select_first(".x.y", None).unwrap().unwrap();
    assert_eq!(div.id(), "a");
    assert!(div.matches("body > div", None).unwrap());
    // As querySelectorAll: the node itself isn't one of its matches.
    assert!(div.select("div", None).unwrap().is_empty());
    assert_eq!(
        div.select_first("b", None).unwrap().unwrap().outer_html(),
        "<b>two</b>"
    );
    assert!(doc.select_first("nope", None).unwrap().is_none());
    assert!(doc.select("p[", None).is_err());
}

#[wasm_bindgen_test]
fn navigation_and_text() {
    let doc = parse(PAGE, None).unwrap();
    let div = doc.select_first("div", None).unwrap().unwrap();
    let kids = div.child_nodes();
    assert_eq!(
        kids.iter().map(|n| n.node_name()).collect::<Vec<_>>(),
        ["#text", "B", "#comment"]
    );
    assert_eq!(div.children().len(), 1);
    assert!(kids[1].previous_sibling().unwrap().is_same_node(&kids[0]));
    assert!(kids[1].parent_node().unwrap().is_same_node(&div));
    assert_eq!(div.next_sibling().unwrap().local_name().unwrap(), "p");
    assert!(div.last_child().unwrap().is_same_node(&kids[2]));
    assert_eq!(div.text_content(), "one two");
    assert_eq!(div.inner_html(), "one <b>two</b><!--c-->");
    let svg = parse("<svg><foreignObject/></svg>", None).unwrap();
    let fo = svg.select_first("foreignObject", None).unwrap().unwrap();
    assert_eq!(fo.node_name(), "foreignObject");
    let tpl = parse("<template><b>t</b></template>", None).unwrap();
    assert!(tpl.select_first("b", None).unwrap().is_none());
    let content = tpl
        .select_first("template", None)
        .unwrap()
        .unwrap()
        .content()
        .unwrap();
    assert_eq!(content.node_type(), 11);
    assert_eq!(content.inner_html(), "<b>t</b>");
}

#[wasm_bindgen_test]
fn attributes_and_edits() {
    let doc = parse(PAGE, None).unwrap();
    let div = doc.select_first("div", None).unwrap().unwrap();
    assert_eq!(div.get_attribute("CLASS").unwrap(), "x y");
    assert!(div.has_attribute("id") && !div.has_attribute("title"));
    div.set_attribute("Title", "t").unwrap();
    assert!(div.remove_attribute("class"));
    assert!(!div.remove_attribute("class"));
    assert_eq!(div.get_attribute_names(), ["id", "title"]);
    assert!(div.first_child().unwrap().set_attribute("a", "b").is_err());
    let p = doc.select_first("p", None).unwrap().unwrap();
    div.insert_before(&p, &div.first_child().unwrap()).unwrap();
    assert_eq!(
        div.outer_html(),
        r#"<div id="a" title="t"><p>p1</p>one <b>two</b><!--c--></div>"#
    );
    // Taken out but kept, as the DOM's remove() keeps it: its contents
    // still read, and it can go back in.
    p.remove();
    assert!(p.parent_node().is_none());
    assert_eq!(p.outer_html(), "<p>p1</p>");
    assert_eq!(p.first_child().unwrap().text_content(), "p1");
    assert!(p.matches("p", None).unwrap());
    doc.body().unwrap().append_child(&p).unwrap();
    assert_eq!(
        doc.body()
            .unwrap()
            .last_child()
            .unwrap()
            .local_name()
            .unwrap(),
        "p"
    );
    assert!(p.append_child(&doc.body().unwrap()).is_err());
    let other = parse("<i>x</i>", None).unwrap();
    assert!(
        div.append_child(&other.select_first("i", None).unwrap().unwrap())
            .is_err()
    );
    let xlink = parse("<svg><a xlink:href=x></a></svg>", None).unwrap();
    let a = xlink.select_first("a", None).unwrap().unwrap();
    assert_eq!(a.get_attribute_names(), ["xlink:href"]);
    assert_eq!(a.get_attribute("xlink:href").unwrap(), "x");
}

#[wasm_bindgen_test]
fn fragments() {
    let rows = parse_fragment("<tr><td>1", Some("table".into()), None).unwrap();
    assert_eq!(rows.root().node_type(), 11);
    assert_eq!(rows.html(), "<tbody><tr><td>1</td></tr></tbody>");
    assert!(rows.body().is_none());
    let div = parse_fragment("<li>a<li>b", None, None).unwrap();
    assert_eq!(div.select("li", None).unwrap().len(), 2);
}

#[wasm_bindgen_test]
fn xpath_scalars_and_edits() {
    let doc = parse("<ul><li class=a>1<li>2</ul>", None).unwrap();
    assert_eq!(doc.xpath("count(//li)").unwrap().as_f64(), Some(2.0));
    assert_eq!(
        doc.xpath("string(//li[2])").unwrap().as_string().as_deref(),
        Some("2")
    );
    assert_eq!(doc.xpath("boolean(//ol)").unwrap().as_bool(), Some(false));
    let ul = doc.select_first("ul", None).unwrap().unwrap();
    assert_eq!(ul.xpath("count(li)").unwrap().as_f64(), Some(2.0));
    // An edit drops the index, so the next query sees it.
    ul.remove();
    assert_eq!(doc.xpath("count(//li)").unwrap().as_f64(), Some(0.0));
    assert_eq!(ul.xpath("count(//li)").unwrap().as_f64(), Some(2.0));
    assert!(doc.xpath("//li[").is_err());
    assert!(doc.xpath("$v").is_err());
}

#[wasm_bindgen_test]
fn no_spans_unless_asked() {
    let doc = parse("<p a=1>x</p>", None).unwrap();
    let p = doc.select_first("p", None).unwrap().unwrap();
    assert!(p.span().is_none() && p.end_span().is_none());
    assert!(p.attribute_span("a").is_none());
    assert!(doc.errors().is_empty());
}

#[wasm_bindgen_test]
fn bytes_are_sniffed() {
    // 日本 in Shift_JIS, named by a <meta>.
    let doc = parse_bytes(b"<meta charset=sjis><p>\x93\xfa\x96\x7b", None).unwrap();
    assert_eq!(doc.text(), "日本");
    assert_eq!(doc.character_set(), "Shift_JIS");
    assert_eq!(doc.encoding_confidence().as_deref(), Some("certain"));
    let plain = parse_bytes(b"<p>caf\xe9", None).unwrap();
    assert_eq!(plain.text(), "café");
    assert_eq!(plain.character_set(), "windows-1252");
    assert_eq!(plain.encoding_confidence().as_deref(), Some("tentative"));
    let string = parse("<p>x", None).unwrap();
    assert_eq!(string.character_set(), "UTF-8");
    assert_eq!(string.encoding_confidence(), None);
}

#[wasm_bindgen_test]
fn bytes_in_pieces() {
    let page = b"<!doctype html><meta charset=sjis><p>\x93\xfa\x96\x7b</p>";
    let mut parser = ByteParser::new(None).unwrap();
    // Cut inside the meta and inside the first kanji.
    for piece in [&page[..20], &page[20..39], &page[39..]] {
        parser.feed(piece).unwrap();
    }
    let doc = parser.finish().unwrap();
    assert_eq!(doc.html(), parse_bytes(page, None).unwrap().html());
    assert_eq!(doc.character_set(), "Shift_JIS");
    assert!(parser.feed(b"x").is_err());
    assert!(parser.finish().is_err());
}

/// The html5lib-tests `encoding/` cases, whole and in pieces of 1 to 7
/// bytes. The scripted one needs `document.write`, so it isn't here.
#[wasm_bindgen_test]
fn html5lib_encoding_cases() {
    const DIR: &str = "../../crates/parsec/tests/html5lib-tests/encoding/";
    let files: [&[u8]; 3] = [
        include_bytes!("../../../crates/parsec/tests/html5lib-tests/encoding/tests1.dat"),
        include_bytes!("../../../crates/parsec/tests/html5lib-tests/encoding/tests2.dat"),
        include_bytes!("../../../crates/parsec/tests/html5lib-tests/encoding/test-yahoo-jp.dat"),
    ];
    let mut cases = Vec::new();
    for file in files {
        let mut rest = file;
        while let Some(at) = find(rest, b"#data\n") {
            rest = &rest[at + 6..];
            let end = find(rest, b"\n#encoding\n").unwrap();
            let label = rest[end + 11..].split(|&b| b == b'\n').next().unwrap();
            cases.push((&rest[..end], std::str::from_utf8(label).unwrap()));
            rest = &rest[end..];
        }
    }
    assert_eq!(cases.len(), 82, "cases in {DIR}");
    for (i, (page, want)) in cases.into_iter().enumerate() {
        let whole = parse_bytes(page, None).unwrap().character_set();
        let mut parser = ByteParser::new(None).unwrap();
        let (mut at, mut size) = (0, 1 + i % 7);
        while at < page.len() {
            parser.feed(&page[at..(at + size).min(page.len())]).unwrap();
            at += size;
            size = 1 + (size + i) % 7;
        }
        let streamed = parser.finish().unwrap().character_set();
        assert!(
            whole.eq_ignore_ascii_case(want),
            "case {i}: {whole}, want {want}"
        );
        assert!(
            streamed.eq_ignore_ascii_case(want),
            "case {i}: {streamed} streamed, want {want}"
        );
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}
