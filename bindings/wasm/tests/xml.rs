// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

// parseXml and parseXmlBytes as JavaScript calls them, in Node with
// `wasm-pack test --node`; browser.rs runs the same in a browser.

#[cfg(feature = "xml")]
mod xml_api {
    use parsec_html_wasm::{XmlParseOptions, parse_xml, parse_xml_bytes};
    use wasm_bindgen::prelude::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    #[wasm_bindgen(
        inline_js = "export function xmlOptions(json) { return JSON.parse(json); }
export function field(o, k) { return String(o[k]); }
export function isSyntaxError(e) { return e instanceof SyntaxError; }"
    )]
    extern "C" {
        #[wasm_bindgen(js_name = xmlOptions)]
        fn xml_options(json: &str) -> XmlParseOptions;
        fn field(o: &JsValue, k: &str) -> String;
        #[wasm_bindgen(js_name = isSyntaxError)]
        fn is_syntax_error(e: &JsValue) -> bool;
    }

    const ATOM: &str = "http://www.w3.org/2005/Atom";
    const XLINK: &str = "http://www.w3.org/1999/xlink";
    const FEED: &str = "<feed xmlns=\"http://www.w3.org/2005/Atom\" \
        xmlns:x=\"http://www.w3.org/1999/xlink\">\
        <entry x:href=\"/one\" n=\"1\">Ships &amp; ports</entry>\
        <?pi data?><!--c--><x:link><![CDATA[<raw>]]></x:link></feed>";

    /// The error a parse threw: its code, line and column.
    fn thrown(e: JsValue) -> (String, String, String) {
        assert!(is_syntax_error(&e));
        (field(&e, "code"), field(&e, "line"), field(&e, "column"))
    }

    #[wasm_bindgen_test]
    fn a_namespaced_document_walks() {
        let doc = parse_xml(FEED, None).unwrap();
        assert_eq!(doc.root().node_type(), 9);
        let feed = doc.document_element();
        assert!(feed.parent_node().unwrap().is_same_node(&doc.root()));
        assert_eq!(feed.node_name(), "feed");
        assert_eq!(feed.namespace_uri().as_deref(), Some(ATOM));
        assert_eq!(feed.prefix(), None);
        let kids = feed.child_nodes();
        assert_eq!(kids.len(), 4);
        assert_eq!(feed.children().len(), 2);
        let (entry, pi, comment, link) = (&kids[0], &kids[1], &kids[2], &kids[3]);
        assert!(entry.next_sibling().unwrap().is_same_node(pi));
        assert!(link.previous_sibling().unwrap().is_same_node(comment));
        assert!(feed.last_child().unwrap().is_same_node(link));
        assert_eq!((pi.node_type(), pi.node_name()), (7, "pi".to_string()));
        assert_eq!(pi.node_value().as_deref(), Some("data"));
        assert_eq!(comment.node_name(), "#comment");
        assert_eq!(link.node_name(), "x:link");
        assert_eq!(link.local_name().as_deref(), Some("link"));
        assert_eq!(link.prefix().as_deref(), Some("x"));
        assert_eq!(link.namespace_uri().as_deref(), Some(XLINK));
        let cdata = link.first_child().unwrap();
        assert_eq!(
            (cdata.node_type(), cdata.node_name()),
            (4, "#cdata-section".to_string())
        );
        assert_eq!(cdata.node_value().as_deref(), Some("<raw>"));
        assert_eq!(entry.text_content(), "Ships & ports");
        assert_eq!(doc.character_set(), "UTF-8");
    }

    #[wasm_bindgen_test]
    fn attributes_with_prefixes() {
        let doc = parse_xml(FEED, None).unwrap();
        let feed = doc.document_element();
        let entry = feed.first_child().unwrap();
        let attrs = entry.attributes();
        assert_eq!(attrs.len(), 2);
        assert_eq!(attrs[0].name, "x:href");
        assert_eq!(attrs[0].local_name, "href");
        assert_eq!(attrs[0].prefix.as_deref(), Some("x"));
        assert_eq!(attrs[0].namespace_uri.as_deref(), Some(XLINK));
        assert_eq!(attrs[0].value, "/one");
        assert!(attrs[0].specified);
        assert_eq!(
            &FEED[attrs[0].span.start as usize..attrs[0].span.end as usize],
            "x:href=\"/one\""
        );
        assert_eq!(
            (attrs[1].prefix.clone(), attrs[1].namespace_uri.clone()),
            (None, None)
        );
        assert_eq!(
            feed.attributes()[1].namespace_uri.as_deref(),
            Some("http://www.w3.org/2000/xmlns/")
        );
        assert_eq!(entry.get_attribute("x:href").as_deref(), Some("/one"));
        assert_eq!(entry.get_attribute("href"), None);
        assert_eq!(
            entry
                .get_attribute_ns(Some(XLINK.into()), "href")
                .as_deref(),
            Some("/one")
        );
        assert_eq!(entry.get_attribute_ns(None, "n").as_deref(), Some("1"));
        assert!(entry.has_attribute("n") && !entry.has_attribute("x"));
        assert_eq!(entry.get_attribute_names(), ["x:href", "n"]);
    }

    #[wasm_bindgen_test]
    fn serializing_gives_back_what_was_parsed() {
        let doc = parse_xml(FEED, None).unwrap();
        assert_eq!(doc.xml(), FEED);
        assert_eq!(parse_xml(&doc.xml(), None).unwrap().xml(), FEED);
        assert_eq!(
            doc.document_element().last_child().unwrap().xml(),
            format!("<x:link xmlns=\"{ATOM}\" xmlns:x=\"{XLINK}\"><![CDATA[<raw>]]></x:link>")
        );
    }

    #[wasm_bindgen_test]
    fn a_well_formedness_error_has_its_code_and_place() {
        let e = parse_xml("<a>\n  <b></a>", None).err().unwrap();
        assert_eq!(
            thrown(e.clone()),
            ("mismatched-end-tag".into(), "2".into(), "6".into())
        );
        assert_eq!(field(&e, "message"), "`</a>` closes `<b>`");
    }

    #[wasm_bindgen_test]
    fn a_billion_laughs_is_an_error_not_a_hang() {
        let mut lol = String::from("<!DOCTYPE l [<!ENTITY a \"lollollollollollollollollollol\">");
        for (p, n) in "abcdefgh".chars().zip("bcdefghi".chars()) {
            lol += &format!("<!ENTITY {n} \"{}\">", format!("&{p};").repeat(10));
        }
        lol += "]><l>&i;</l>";
        let e = parse_xml(&lol, None).err().unwrap();
        assert_eq!(thrown(e).0, "entity-expansion-limit");
        let e = parse_xml("<a><b><c/></b></a>", Some(xml_options(r#"{"maxDepth":2}"#)))
            .err()
            .unwrap();
        assert_eq!(thrown(e).0, "too-deep");
        let small = "<!DOCTYPE a [<!ENTITY e \"12345\">]><a>&e;&e;</a>";
        let strict = xml_options(r#"{"entityAllowance":0,"entityRatio":0}"#);
        assert_eq!(
            thrown(parse_xml(small, Some(strict)).err().unwrap()).0,
            "entity-expansion-limit"
        );
        let off = parse_xml("<p:a/>", Some(xml_options(r#"{"namespaces":false}"#))).unwrap();
        assert_eq!(off.document_element().local_name().as_deref(), Some("p:a"));
        let bad = parse_xml("<a/>", Some(xml_options(r#"{"maxDepth":-1}"#)))
            .err()
            .unwrap();
        assert!(field(&bad, "message").contains("maxDepth"));
    }

    #[wasm_bindgen_test]
    fn bytes_in_the_encoding_they_declare() {
        let src: &[u8] =
            b"<?xml version='1.0' encoding='ISO-8859-1'?>\n<!DOCTYPE p SYSTEM 'p.dtd'><p>caf\xe9</p>";
        let doc = parse_xml_bytes(src, None).unwrap();
        assert_eq!(doc.character_set(), "windows-1252");
        let p = doc.document_element();
        assert_eq!(p.text_content(), "café");
        let span = p.span();
        assert_eq!(
            &src[span.start as usize..span.end as usize],
            b"<p>caf\xe9</p>"
        );
        assert_eq!((span.line, span.column), (2, 28));
        assert_eq!(p.previous_sibling().unwrap().node_type(), 10);
        let warnings = doc.warnings();
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].code, "external-dtd-not-loaded");
        let e = parse_xml_bytes(b"<p>\xff</p>", None).err().unwrap();
        assert_eq!(thrown(e).0, "invalid-byte-sequence");
    }

    #[wasm_bindgen_test]
    fn spans_in_a_string_count_utf16() {
        let doc = parse_xml("<a>😀<b x='1'/></a>", None).unwrap();
        let b = doc.document_element().last_child().unwrap();
        // The emoji is two UTF-16 units.
        assert_eq!((b.span().start, b.span().end), (5, 15));
        assert_eq!(b.span().column, 5);
    }
}
