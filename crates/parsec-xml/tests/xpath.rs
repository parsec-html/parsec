// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XPath over parsec-xml's tree: names, namespaces, the namespace axis,
//! attributes, text, processing instructions, comments, `id()` and
//! `lang()`.

use parsec_xml::xpath::{Doc, Index, Item, NoExtensions, Value, XPath};
use parsec_xml::{Document, Limits, NodeId, Options};

const DOC: &str = r#"<?xml version="1.0"?>
<!DOCTYPE r:doc [
<!ATTLIST chapter key ID #IMPLIED>
<!ATTLIST note kind CDATA "aside">
]>
<r:doc xmlns:r="urn:r" xmlns="urn:d" xml:lang="en-GB">
  <r:item xml:id="a" r:kind="x" plain="1">hello</r:item>
  <Item xml:lang="fr">bon<![CDATA[jour]]></Item>
  <chapter key="c1">one</chapter>
  <note/>
  <?pi some data?><!--a comment-->
</r:doc>"#;

fn run(doc: &Document, expr: &str) -> Value<'static> {
    run_with(doc, expr, &[("r", "urn:r"), ("d", "urn:d")])
}

fn run_with(doc: &Document, expr: &str, ns: &[(&str, &str)]) -> Value<'static> {
    let index = Index::build(doc, &[0]);
    let ns: Vec<(String, String)> = ns
        .iter()
        .map(|(p, u)| (p.to_string(), u.to_string()))
        .collect();
    let value = XPath::new(expr)
        .unwrap()
        .evaluate(Doc::new(doc, &index), 0, &[], &ns, &NoExtensions)
        .unwrap_or_else(|e| panic!("{expr}: {e}"));
    // Owned, so the index can go with this call.
    match value {
        Value::Nodes(n) => Value::Nodes(n),
        Value::Str(s) => Value::Str(s.into_owned().into()),
        Value::Num(n) => Value::Num(n),
        Value::Bool(b) => Value::Bool(b),
    }
}

fn string(doc: &Document, expr: &str) -> String {
    match run(doc, &format!("string({expr})")) {
        Value::Str(s) => s.into_owned(),
        v => panic!("{expr}: {v:?}"),
    }
}

fn count(doc: &Document, expr: &str) -> f64 {
    match run(doc, &format!("count({expr})")) {
        Value::Num(n) => n,
        v => panic!("{expr}: {v:?}"),
    }
}

fn nodes(doc: &Document, expr: &str) -> Vec<NodeId> {
    match run(doc, expr) {
        Value::Nodes(n) => n
            .into_iter()
            .map(|i| match i {
                Item::Node(n) => NodeId::from_index(n as usize),
                Item::Attr { owner, .. } => panic!("{expr}: an attribute of {owner}"),
                Item::Ns { owner, .. } => panic!("{expr}: a namespace node of {owner}"),
            })
            .collect(),
        v => panic!("{expr}: {v:?}"),
    }
}

#[test]
fn names_and_namespaces() {
    let doc = parsec_xml::parse(DOC).unwrap();
    assert_eq!(nodes(&doc, "/r:doc"), [doc.root_element()]);
    assert_eq!(string(&doc, "name(/*)"), "r:doc");
    assert_eq!(string(&doc, "local-name(/*)"), "doc");
    assert_eq!(string(&doc, "namespace-uri(/*)"), "urn:r");
    // Unprefixed names in a query are in no namespace, so the default
    // namespace's elements need a prefix.
    assert_eq!(count(&doc, "/*/Item"), 0.0);
    assert_eq!(string(&doc, "/*/d:Item"), "bonjour");
    // Names compare exactly.
    assert_eq!(count(&doc, "/*/d:item"), 0.0);
    assert_eq!(count(&doc, "//r:*"), 2.0);
    assert_eq!(count(&doc, "//d:*"), 3.0);
    // A prefix is the query's: another prefix for the same URI matches.
    let other = run_with(&doc, "count(//q:item)", &[("q", "urn:r")]);
    assert!(matches!(other, Value::Num(n) if n == 1.0));
    assert!(
        XPath::new("//nope:x")
            .unwrap()
            .evaluate(
                Doc::new(&doc, &Index::build(&doc, &[0])),
                0,
                &[],
                &[],
                &NoExtensions
            )
            .is_err()
    );
}

#[test]
fn attributes_leave_out_namespace_declarations() {
    let doc = parsec_xml::parse(DOC).unwrap();
    // xmlns:r and xmlns are namespace nodes, not attributes.
    assert_eq!(count(&doc, "/*/@*"), 1.0);
    assert_eq!(string(&doc, "name(/*/@*)"), "xml:lang");
    assert_eq!(count(&doc, "//@xmlns"), 0.0);
    assert_eq!(count(&doc, "//r:item/@*"), 3.0);
    assert_eq!(string(&doc, "//r:item/@r:kind"), "x");
    assert_eq!(string(&doc, "namespace-uri(//r:item/@r:kind)"), "urn:r");
    assert_eq!(string(&doc, "local-name(//r:item/@r:kind)"), "kind");
    // An unprefixed attribute is in no namespace, whatever the default.
    assert_eq!(string(&doc, "//r:item/@plain"), "1");
    assert_eq!(string(&doc, "namespace-uri(//r:item/@plain)"), "");
    // A default the DTD supplied is an attribute too.
    assert_eq!(string(&doc, "//d:note/@kind"), "aside");
    // They're on the namespace axis instead, with xml's.
    assert_eq!(count(&doc, "/*/namespace::*"), 3.0);
}

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";

/// A namespaced document: `r` and a default on the root, `s` on `r:item`,
/// and on `Item` the default taken away and `r` bound again.
const NS_DOC: &str = r#"<r:doc xmlns:r="urn:r" xmlns="urn:d" xml:lang="en">
  <r:item xmlns:s="urn:s" r:k="1" plain="2"/>
  <Item xmlns="" xmlns:r="urn:r2" a="3"/>
</r:doc>"#;

/// A node-set's items as text: `prefix=uri` for a namespace node, `@name`
/// for an attribute and the qualified name for an element.
fn items(doc: &Document, expr: &str) -> Vec<String> {
    let index = Index::build(doc, &[0]);
    let view = Doc::new(doc, &index);
    let value = XPath::new(expr)
        .unwrap()
        .evaluate(view, 0, &[], &[("r".into(), "urn:r".into())], &NoExtensions)
        .unwrap_or_else(|e| panic!("{expr}: {e}"));
    let Value::Nodes(items) = value else {
        panic!("{expr}: {value:?}")
    };
    items
        .into_iter()
        .map(|i| match i {
            Item::Ns { decl, .. } => {
                let (prefix, uri) = view.namespace_at(decl);
                format!("{prefix}={uri}")
            }
            Item::Attr { owner, index } => {
                let a = doc
                    .attributes(NodeId::from_index(owner as usize))
                    .filter(|a| !a.is_namespace_declaration())
                    .nth(index as usize)
                    .unwrap();
                format!("@{}", a.name.qname)
            }
            Item::Node(n) => match doc.kind(NodeId::from_index(n as usize)) {
                parsec_xml::NodeKind::Element(name) => name.qname.to_string(),
                k => format!("{k:?}"),
            },
        })
        .collect()
}

#[test]
fn the_namespace_axis_has_what_is_in_scope() {
    let doc = parsec_xml::parse(NS_DOC).unwrap();
    let xml = format!("xml={XML_NS}");
    // xml first, then the outermost element's bindings, each element's
    // last written first, as libxml2 and lxml give them.
    assert_eq!(items(&doc, "/*/namespace::*"), [&*xml, "=urn:d", "r=urn:r"]);
    assert_eq!(
        items(&doc, "//r:item/namespace::*"),
        [&*xml, "=urn:d", "r=urn:r", "s=urn:s"]
    );
    // xmlns="" takes the default away and leaves no node; xmlns:r rebinds.
    assert_eq!(items(&doc, "//*[@a]/namespace::*"), [&*xml, "r=urn:r2"]);
    assert_eq!(
        items(&doc, "//*[@a]/namespace::node()"),
        [&*xml, "r=urn:r2"]
    );
    assert_eq!(items(&doc, "//r:item/namespace::s"), ["s=urn:s"]);
    assert!(items(&doc, "//*[@a]/namespace::s").is_empty());
    assert_eq!(items(&doc, "//*[namespace::s]"), ["r:item"]);
    assert_eq!(items(&doc, "//*[namespace::r = 'urn:r2']"), ["Item"]);
    assert_eq!(count(&doc, "//namespace::*"), 9.0);
    // A namespace node's name is its prefix and its value its URI.
    assert_eq!(string(&doc, "name(/*/namespace::*[. = 'urn:d'])"), "");
    assert_eq!(string(&doc, "//r:item/namespace::s"), "urn:s");
    assert_eq!(string(&doc, "namespace-uri(//r:item/namespace::s)"), "");
    assert_eq!(items(&doc, "//r:item/namespace::s/.."), ["r:item"]);
    // Only elements have namespace nodes.
    assert!(items(&doc, "/namespace::*").is_empty());
    assert!(items(&doc, "//@plain/namespace::*").is_empty());
}

#[test]
fn namespace_nodes_sort_after_their_element_and_before_its_attributes() {
    let doc = parsec_xml::parse(NS_DOC).unwrap();
    assert_eq!(
        items(&doc, "/*/@* | /*/namespace::r | /*"),
        ["r:doc", "r=urn:r", "@xml:lang"]
    );
    assert_eq!(
        items(&doc, "//r:item/@* | //r:item/namespace::s | //r:item"),
        ["r:item", "s=urn:s", "@r:k", "@plain"]
    );
    // The declarations are namespace nodes, not attributes.
    assert_eq!(items(&doc, "//*[@a]/@*"), ["@a"]);
}

#[test]
fn without_namespaces_there_are_no_declarations() {
    let options = Options {
        namespaces: false,
        ..Options::default()
    };
    let doc = parsec_xml::parse_with("<a:b xmlns:a='urn:a' xmlns='urn:d'/>", &options).unwrap();
    assert_eq!(items(&doc, "/*/namespace::*"), [format!("xml={XML_NS}")]);
}

#[test]
fn text_comments_and_processing_instructions() {
    let doc = parsec_xml::parse(DOC).unwrap();
    // CDATA is text, and joins the text beside it in string().
    assert_eq!(string(&doc, "//d:Item"), "bonjour");
    assert_eq!(count(&doc, "//d:Item/text()"), 2.0);
    assert_eq!(string(&doc, "//comment()"), "a comment");
    assert_eq!(string(&doc, "//processing-instruction()"), "some data");
    assert_eq!(string(&doc, "name(//processing-instruction('pi'))"), "pi");
    assert_eq!(count(&doc, "//processing-instruction('other')"), 0.0);
    // The doctype and the XML declaration aren't nodes XPath has.
    assert_eq!(count(&doc, "/node()"), 1.0);
}

#[test]
fn id_finds_xml_id_and_dtd_ids() {
    let doc = parsec_xml::parse(DOC).unwrap();
    assert_eq!(string(&doc, "local-name(id('a'))"), "item");
    assert_eq!(string(&doc, "id('c1')"), "one");
    assert_eq!(count(&doc, "id('a c1 missing')"), 2.0);
    // `plain` and `kind` aren't IDs.
    assert_eq!(count(&doc, "id('1')"), 0.0);
    assert_eq!(count(&doc, "id('x')"), 0.0);
}

#[test]
fn lang_reads_xml_lang_inherited() {
    let doc = parsec_xml::parse(DOC).unwrap();
    assert_eq!(count(&doc, "//*[lang('en')]"), 4.0);
    assert_eq!(count(&doc, "//*[lang('en-gb')]"), 4.0);
    assert_eq!(count(&doc, "//*[lang('fr')]"), 1.0);
    assert_eq!(count(&doc, "//*[lang('de')]"), 0.0);
}

#[test]
fn without_namespaces_names_are_as_written() {
    let options = Options {
        namespaces: false,
        ..Options::default()
    };
    let doc = parsec_xml::parse_with("<a:b xmlns:a='urn:a' a:c='1'/>", &options).unwrap();
    // With namespaces off there's nothing to resolve a prefix against, so
    // the name is all local part; only a name test can't spell it.
    assert_eq!(string(&doc, "name(/*)"), "a:b");
    assert_eq!(string(&doc, "namespace-uri(/*)"), "");
    assert_eq!(count(&doc, "/*/@*"), 2.0);
}

/// How long `f` takes, the best of three.
fn time(f: impl Fn()) -> std::time::Duration {
    (0..3)
        .map(|_| {
            let t = std::time::Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

#[test]
fn the_namespace_axis_stays_linear() {
    // A chain of n elements, each binding `p` again and with a leaf that
    // binds `q`, and one element with n declarations and n attributes.
    // Ten times the size should take about ten times as long.
    fn chain(n: usize) -> Document {
        let deep = Options {
            limits: Limits {
                max_depth: None,
                ..Limits::default()
            },
            ..Options::default()
        };
        let open: String = (0..n)
            .map(|i| format!("<e xmlns:p='urn:{i}'><l xmlns:q='urn:q'/>"))
            .collect();
        parsec_xml::parse_with(&(open + &"</e>".repeat(n)), &deep).unwrap()
    }
    fn wide(n: usize) -> Document {
        let attrs: String = (0..n)
            .map(|i| format!(" xmlns:p{i}='urn:{i}' a{i}='{i}'"))
            .collect();
        parsec_xml::parse(&format!("<w{attrs}/>")).unwrap()
    }
    type Build = fn(usize) -> Document;
    let cases: [(Build, &str, usize, usize); 6] = [
        (chain, "count(//e/namespace::*)", 2, 0),
        (chain, "count(//l/namespace::*)", 3, 0),
        (chain, "count(//*[namespace::p = 'urn:7'])", 0, 2),
        (wide, "count(/w/namespace::*)", 1, 1),
        (wide, "count(/w/namespace::* | /w/@*)", 2, 1),
        (wide, "count(/w/namespace::p7) + count(/w/@a7)", 0, 2),
    ];
    for (make, expr, a, b) in cases {
        let (small, big) = (make(2_000), make(20_000));
        let run = |doc: &Document, n: usize| {
            let index = Index::build(doc, &[0]);
            let got = XPath::new(expr).unwrap().evaluate(
                Doc::new(doc, &index),
                0,
                &[],
                &[],
                &NoExtensions,
            );
            let want = (a * n + b) as f64;
            assert!(
                matches!(got, Ok(Value::Num(v)) if v == want),
                "{expr}: {got:?}"
            );
        };
        let ts = time(|| run(&small, 2_000));
        let tb = time(|| run(&big, 20_000));
        let ratio = tb.as_secs_f64() / ts.as_secs_f64().max(1e-3);
        eprintln!("{expr}: 2,000 {ts:?}, 20,000 {tb:?}, {ratio:.1}x");
        assert!(
            ratio < 20.0,
            "{expr}: 10x the size took {ratio:.1}x ({ts:?}, {tb:?})"
        );
    }
}

#[test]
fn many_declarations_and_prefixed_attributes_stay_linear() {
    // One element with n attributes. With declarations, every hundredth
    // is an `xmlns:pN`, and the one after it uses that prefix, so the
    // attributes XPath sees aren't the element's own run of them. Each
    // query reads every attribute, and `lang()` its owner's `xml:lang`.
    for declarations in [false, true] {
        let make = |n: usize| {
            let attrs: String = (0..n)
                .map(|i| match i % 100 {
                    50 if declarations => format!(" xmlns:p{i}='urn:{i}'"),
                    51 if declarations => format!(" p{}:k{i}='{i}'", i - 1),
                    _ => format!(" k{i}='{i}'"),
                })
                .collect();
            parsec_xml::parse(&format!("<a xml:lang='en'{attrs}/>")).unwrap()
        };
        let (small, big) = (make(20_000), make(200_000));
        let queries = |doc: &Document, n: usize| {
            let index = Index::build(doc, &[0]);
            // Less any declarations, and with xml:lang.
            let want = n - if declarations { n / 100 } else { 0 } + 1;
            for (expr, expect) in [
                ("count(/a/@*)".to_string(), want as f64),
                ("count(/a/@*[local-name()='k0'])".to_string(), 1.0),
                ("count(/a/@*[lang('en')])".to_string(), want as f64),
                (format!("count(/a/@k{})", n - 1), 1.0),
            ] {
                let got = XPath::new(&expr).unwrap().evaluate(
                    Doc::new(doc, &index),
                    0,
                    &[],
                    &[],
                    &NoExtensions,
                );
                assert!(
                    matches!(got, Ok(Value::Num(v)) if v == expect),
                    "{expr}: {got:?}"
                );
            }
        };
        let ts = time(|| queries(&small, 20_000));
        let tb = time(|| queries(&big, 200_000));
        let ratio = tb.as_secs_f64() / ts.as_secs_f64().max(1e-6);
        eprintln!(
            "one wide element, declarations {declarations}: 20,000 attributes {ts:?}, 200,000 {tb:?}, {ratio:.1}x"
        );
        assert!(
            ratio < 20.0,
            "declarations {declarations}: 10x the attributes took {ratio:.1}x the time ({ts:?}, {tb:?})"
        );
    }
}
