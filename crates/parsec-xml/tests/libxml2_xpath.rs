// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! libxml2's XPath document tests against parsec-xml's tree: each query in
//! test/XPath/tests/<doc>* runs against test/XPath/docs/<doc>, from the
//! root element as libxml2's runtest does, and the result is printed as
//! libxml2's xmlXPathDebugDumpObject prints it, to compare with
//! result/XPath/tests. The files are libxml2's, in the bench/libxml2
//! submodule at the release it's pinned to.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::fmt::Write;
use std::path::{Path, PathBuf};

use parsec_xml::xpath::{Doc, Index, Item, NoExtensions, Value, XPath};
use parsec_xml::{Document, NodeId, NodeKind, Options};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/libxml2")
}

/// Why a query can't pass here, for the ones that can't.
fn skip(expr: &str) -> Option<&'static str> {
    expr.contains("namespace::")
        .then_some("the namespace axis, which parsec-xpath leaves empty")
}

/// libxml2's xmlCtxtDumpString: 40 bytes at most, white space as spaces,
/// bytes over 0x7F in hex, and `...` after anything 40 bytes or longer.
fn dump_string(out: &mut String, s: &str) {
    for &b in s.as_bytes().iter().take(40) {
        match b {
            b' ' | b'\t' | b'\n' | b'\r' => out.push(' '),
            0x80.. => write!(out, "#{b:X}").unwrap(),
            _ => out.push(b as char),
        }
    }
    if s.len() >= 40 {
        out.push_str("...");
    }
}

fn spaces(out: &mut String, depth: usize) {
    out.extend(std::iter::repeat_n(' ', 2 * depth.min(25)));
}

/// An attribute as xmlCtxtDumpAttr prints it: its name, then its value as
/// a text child.
fn dump_attr(out: &mut String, name: &str, value: &str, depth: usize) {
    spaces(out, depth);
    out.push_str("ATTRIBUTE ");
    dump_string(out, name);
    out.push('\n');
    if !value.is_empty() {
        spaces(out, depth + 1);
        out.push_str("TEXT\n");
        spaces(out, depth + 2);
        out.push_str("content=");
        dump_string(out, value);
        out.push('\n');
    }
}

/// A node as xmlXPathDebugDumpNode prints it, at depth 1.
fn dump_node(out: &mut String, doc: &Document, node: NodeId) {
    let content = |out: &mut String, text: &str| {
        spaces(out, 2);
        out.push_str("content=");
        dump_string(out, text);
        out.push('\n');
    };
    spaces(out, 1);
    match doc.kind(node) {
        NodeKind::Document => out.push_str(" /\n"),
        NodeKind::Element(name) => {
            out.push_str("ELEMENT ");
            dump_string(out, name.qname);
            out.push('\n');
            // The declarations on the element (nsDef), then its attributes.
            for a in doc
                .attributes(node)
                .filter(|a| a.is_namespace_declaration())
            {
                spaces(out, 2);
                if a.name.qname == "xmlns" {
                    out.push_str("default namespace href=");
                } else {
                    write!(out, "namespace {} href=", a.name.local).unwrap();
                }
                dump_string(out, a.value);
                out.push('\n');
            }
            for a in doc
                .attributes(node)
                .filter(|a| !a.is_namespace_declaration())
            {
                dump_attr(out, a.name.local, a.value, 2);
            }
        }
        NodeKind::Text(t) => {
            out.push_str("TEXT\n");
            content(out, t);
        }
        NodeKind::CData(t) => {
            out.push_str("CDATA_SECTION\n");
            content(out, t);
        }
        NodeKind::Comment(t) => {
            out.push_str("COMMENT\n");
            content(out, t);
        }
        NodeKind::ProcessingInstruction { target, data } => {
            writeln!(out, "PI {target}").unwrap();
            content(out, data);
        }
        NodeKind::Doctype => out.push_str("DOCUMENT_TYPE\n"),
        NodeKind::EntityRef(name) => writeln!(out, "ENTITY_REF({name})").unwrap(),
    }
}

/// What xmlXPathDebugDumpObject prints, for comparison with libxml2's.
fn dump(doc: &Document, value: &Value<'_>) -> String {
    let mut out = String::new();
    match value {
        Value::Nodes(items) => {
            out.push_str("Object is a Node Set :\n");
            writeln!(out, "Set contains {} nodes:", items.len()).unwrap();
            for (i, item) in items.iter().enumerate() {
                write!(out, "{}", i + 1).unwrap();
                match *item {
                    Item::Node(n) => dump_node(&mut out, doc, NodeId::from_index(n as usize)),
                    Item::Attr { owner, index } => {
                        let a = doc
                            .attributes(NodeId::from_index(owner as usize))
                            .filter(|a| !a.is_namespace_declaration())
                            .nth(index as usize)
                            .expect("an attribute the query found");
                        dump_attr(&mut out, a.name.local, a.value, 1);
                    }
                }
            }
        }
        Value::Bool(b) => writeln!(out, "Object is a Boolean : {b}").unwrap(),
        Value::Str(s) => {
            out.push_str("Object is a string : ");
            dump_string(&mut out, s);
            out.push('\n');
        }
        // `%g` for whole numbers small enough to print in full; the
        // documents' queries give no others.
        Value::Num(n) if n.fract() == 0.0 && n.abs() < 1e6 => {
            writeln!(out, "Object is a number : {}", *n as i64).unwrap();
        }
        Value::Num(n) => writeln!(out, "Object is a number : {n}").unwrap(),
    }
    out
}

/// The expected file's blocks: each query and what libxml2 printed.
fn expected(path: &Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .replace("\r\n", "\n");
    text.split("\n========================\n")
        .skip(1)
        .map(|block| {
            let (first, rest) = block.split_once('\n').unwrap_or((block, ""));
            let expr = first
                .strip_prefix("Expression: ")
                .unwrap_or_else(|| panic!("{}: no expression in {block:?}", path.display()));
            // Blank lines separate blocks; the output itself has none.
            (
                expr.to_string(),
                rest.trim_end_matches('\n').to_string() + "\n",
            )
        })
        .collect()
}

#[test]
fn libxml2_xpath_documents() {
    let data = data_dir();
    let docs_dir = data.join("test/XPath/docs");
    let Ok(entries) = std::fs::read_dir(&docs_dir) else {
        // CONFORMANCE_DIR's gate fails a suite that wrote no counts, so CI
        // can't pass without the data.
        eprintln!(
            "libxml2 XPath documents: skipped, {} isn't there (git submodule update --init bench/libxml2)",
            docs_dir.display()
        );
        return;
    };
    let mut docs: Vec<String> = entries
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    docs.sort();
    let tests: Vec<String> = {
        let mut t: Vec<String> = std::fs::read_dir(data.join("test/XPath/tests"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        t.sort();
        t
    };
    let (mut passed, mut skipped, mut failed) = (0, 0, Vec::new());
    // libxml2 reads them with DTD defaults and entities expanded, as
    // parsec-xml always does.
    let options = Options::default();
    for name in &docs {
        let bytes = std::fs::read(docs_dir.join(name)).unwrap();
        let doc = parsec_xml::parse_bytes(&bytes, &options)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .document;
        let index = Index::build(&doc, &[0]);
        let root = doc.root_element().index() as u32;
        for test in tests.iter().filter(|t| t.starts_with(name.as_str())) {
            let cases = expected(&data.join("result/XPath/tests").join(test));
            for (expr, want) in cases {
                if skip(&expr).is_some() {
                    skipped += 1;
                    continue;
                }
                let got = match XPath::new(&expr)
                    .and_then(|x| x.evaluate(Doc::new(&doc, &index), root, &[], &[], &NoExtensions))
                {
                    Ok(v) => dump(&doc, &v),
                    Err(_) => "Object is empty (NULL)\n".to_string(),
                };
                if got == want {
                    passed += 1;
                } else {
                    failed.push(format!(
                        "{test}: {expr}\n--- libxml2\n{want}--- parsec\n{got}"
                    ));
                }
            }
        }
    }
    summary::write(&summary::Counts {
        suite: "libxml2-xpath-documents",
        passed,
        failed: failed.len(),
        skipped,
        skipped_because: skip("namespace::").unwrap(),
        data: &data,
    });
    eprintln!(
        "libxml2 XPath documents: {passed} passed, {} failed, {skipped} skipped ({})",
        failed.len(),
        skip("namespace::").unwrap()
    );
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}
