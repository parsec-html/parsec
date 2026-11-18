// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// The exports as JavaScript calls them, run in Node by
// `wasm-pack test --node`; browser.rs runs the same in a browser.

use parsec_html_wasm::{parse, parse_fragment};
use wasm_bindgen_test::wasm_bindgen_test;

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
fn selectors() {
    let doc = parse(PAGE, None).unwrap();
    let hits = doc.select("div, p").unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[1].text_content(), "p1");
    let div = doc.select_first(".x.y").unwrap().unwrap();
    assert_eq!(div.id(), "a");
    assert!(div.matches("body > div").unwrap());
    // As querySelectorAll: the node itself isn't one of its matches.
    assert!(div.select("div").unwrap().is_empty());
    assert_eq!(
        div.select_first("b").unwrap().unwrap().outer_html(),
        "<b>two</b>"
    );
    assert!(doc.select_first("nope").unwrap().is_none());
    assert!(doc.select("p[").is_err());
}

#[wasm_bindgen_test]
fn navigation_and_text() {
    let doc = parse(PAGE, None).unwrap();
    let div = doc.select_first("div").unwrap().unwrap();
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
    let fo = svg.select_first("foreignObject").unwrap().unwrap();
    assert_eq!(fo.node_name(), "foreignObject");
    let tpl = parse("<template><b>t</b></template>", None).unwrap();
    assert!(tpl.select_first("b").unwrap().is_none());
    let content = tpl
        .select_first("template")
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
    let div = doc.select_first("div").unwrap().unwrap();
    assert_eq!(div.get_attribute("CLASS").unwrap(), "x y");
    assert!(div.has_attribute("id") && !div.has_attribute("title"));
    div.set_attribute("Title", "t").unwrap();
    assert!(div.remove_attribute("class"));
    assert!(!div.remove_attribute("class"));
    assert_eq!(div.get_attribute_names(), ["id", "title"]);
    assert!(div.first_child().unwrap().set_attribute("a", "b").is_err());
    let p = doc.select_first("p").unwrap().unwrap();
    div.insert_before(&p, &div.first_child().unwrap()).unwrap();
    assert_eq!(
        div.outer_html(),
        r#"<div id="a" title="t"><p>p1</p>one <b>two</b><!--c--></div>"#
    );
    p.remove();
    assert!(p.parent_node().is_none());
    assert_eq!(p.outer_html(), "<p>p1</p>");
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
        div.append_child(&other.select_first("i").unwrap().unwrap())
            .is_err()
    );
    let xlink = parse("<svg><a xlink:href=x></a></svg>", None).unwrap();
    let a = xlink.select_first("a").unwrap().unwrap();
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
    assert_eq!(div.select("li").unwrap().len(), 2);
}
