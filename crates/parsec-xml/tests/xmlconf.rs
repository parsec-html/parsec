// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The W3C XML Conformance Test Suite, from `tests/xmlconf`
//! (`scripts/fetch-xmlconf.sh` downloads it), run as a processor that
//! doesn't validate runs it: every `valid` and `invalid` document parses,
//! every `not-wf` document is an error, and a valid document with
//! canonical output gives exactly that output.
//!
//! The suite's own index is XML, and its parts are external entities,
//! which Parsec doesn't read: the index parses with them left as
//! references, and this driver reads each part itself.
//!
//! Every case runs or is skipped for a reason counted in the summary; any
//! failure fails the run, with each case's ID and what went wrong.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use parsec_xml::{Document, NodeId, NodeKind, Options, parse_bytes};

/// Cases whose expected output is wrong, and how.
const BAD_OUTPUT: &[(&str, &str)] = &[(
    "ibm-valid-P29-ibm29v01.xml",
    "its canonical output puts a processing instruction from the internal subset before the <!DOCTYPE>",
)];

/// `not-wf` cases whose fault is in an external entity, which Parsec never
/// reads, so the main document parses. Listed rather than decided at run
/// time: if the parser stopped catching a fault in a main document, that
/// case would fail here instead of joining these.
const EXTERNAL_FAULT: &[&str] = &[
    "cond01",
    "cond02",
    "decl01",
    "dtd07",
    "encoding07",
    "ibm-not-wf-P30-ibm30n01.xml",
    "ibm-not-wf-P31-ibm31n01.xml",
    "ibm-not-wf-P61-ibm61n01.xml",
    "ibm-not-wf-P62-ibm62n01.xml",
    "ibm-not-wf-P62-ibm62n02.xml",
    "ibm-not-wf-P62-ibm62n03.xml",
    "ibm-not-wf-P62-ibm62n04.xml",
    "ibm-not-wf-P62-ibm62n05.xml",
    "ibm-not-wf-P62-ibm62n06.xml",
    "ibm-not-wf-P62-ibm62n07.xml",
    "ibm-not-wf-P62-ibm62n08.xml",
    "ibm-not-wf-P63-ibm63n01.xml",
    "ibm-not-wf-P63-ibm63n02.xml",
    "ibm-not-wf-P63-ibm63n03.xml",
    "ibm-not-wf-P63-ibm63n04.xml",
    "ibm-not-wf-P63-ibm63n05.xml",
    "ibm-not-wf-P63-ibm63n06.xml",
    "ibm-not-wf-P63-ibm63n07.xml",
    "ibm-not-wf-P64-ibm64n01.xml",
    "ibm-not-wf-P64-ibm64n02.xml",
    "ibm-not-wf-P64-ibm64n03.xml",
    "ibm-not-wf-P65-ibm65n01.xml",
    "ibm-not-wf-P65-ibm65n02.xml",
    "ibm-not-wf-P77-ibm77n01.xml",
    "ibm-not-wf-P77-ibm77n02.xml",
    "ibm-not-wf-P77-ibm77n03.xml",
    "ibm-not-wf-P77-ibm77n04.xml",
    "ibm-not-wf-P78-ibm78n01.xml",
    "ibm-not-wf-P78-ibm78n02.xml",
    "ibm-not-wf-P79-ibm79n01.xml",
    "ibm-not-wf-P79-ibm79n02.xml",
    "ibm-not-wf-p28a-ibm28an01.xml",
    "not-wf-ext-sa-001",
    "not-wf-ext-sa-002",
    "not-wf-ext-sa-003",
    "not-wf-not-sa-001",
    "not-wf-not-sa-003",
    "not-wf-not-sa-004",
    "not-wf-not-sa-006",
    "not-wf-not-sa-007",
    "not-wf-not-sa-008",
    "not-wf-not-sa-009",
    "o-p09fail1",
    "o-p09fail2",
    "o-p30fail1",
    "o-p31fail1",
    "o-p61fail1",
    "o-p62fail1",
    "o-p62fail2",
    "o-p63fail1",
    "o-p63fail2",
    "o-p64fail1",
    "o-p64fail2",
    "rmt-e2e-38",
];

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/xmlconf")
}

struct Case {
    id: String,
    kind: String,
    entities: String,
    file: PathBuf,
    output: Option<PathBuf>,
    namespaces: bool,
    skip: Option<&'static str>,
}

fn read(path: &Path) -> Document {
    let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    match parse_bytes(&bytes, &Options::default()) {
        Ok(p) => p.document,
        Err(e) => panic!("{}: {e}", path.display()),
    }
}

/// The cases in one part of the index, whose `TEST` URIs are relative to
/// `base`.
fn collect(doc: &Document, node: NodeId, base: &Path, out: &mut Vec<Case>) {
    for child in doc.children(node).filter(|&c| doc.is_element(c)) {
        let name = doc.name(child).unwrap().qname;
        let attr = |n: &str| doc.attribute(child, "", n).unwrap_or("");
        let xml_base = doc
            .attribute(child, parsec_xml::XML_NS, "base")
            .unwrap_or("");
        match name {
            "TESTCASES" => collect(doc, child, &base.join(xml_base), out),
            "TEST" => {
                let version = attr("VERSION");
                let rec = attr("RECOMMENDATION");
                let edition = attr("EDITION");
                let skip = if version.contains("1.1") || rec == "XML1.1" || rec == "NS1.1" {
                    Some("XML 1.1 or Namespaces 1.1, which Parsec doesn't implement")
                } else if !edition.is_empty() && !edition.split(' ').any(|e| e == "5") {
                    Some("XML 1.0 before the Fifth Edition")
                } else if attr("TYPE") == "error" {
                    Some("an error the spec lets a processor report or not")
                } else {
                    None
                };
                out.push(Case {
                    id: attr("ID").to_string(),
                    kind: attr("TYPE").to_string(),
                    // Without the attribute, the case uses no external
                    // entities.
                    entities: Some(attr("ENTITIES"))
                        .filter(|e| !e.is_empty())
                        .unwrap_or("none")
                        .to_string(),
                    file: base.join(attr("URI")),
                    output: Some(attr("OUTPUT"))
                        .filter(|o| !o.is_empty())
                        .map(|o| base.join(o)),
                    namespaces: attr("NAMESPACE") != "no",
                    skip,
                });
            }
            _ => {}
        }
    }
}

/// Every case in the suite, in the index's order.
fn cases() -> Vec<Case> {
    let dir = data_dir();
    let index = read(&dir.join("xmlconf.xml"));
    let mut out = Vec::new();
    let suite = index.root_element();
    for testcases in index.children(suite).filter(|&c| index.is_element(c)) {
        for part in index.children(testcases) {
            let NodeKind::EntityRef(name) = index.kind(part) else {
                continue;
            };
            let entity = index.entities().iter().find(|e| e.name == name).unwrap();
            let file = dir.join(entity.system_id.as_deref().unwrap());
            // A part is an external parsed entity, not a document: any
            // number of elements, after an optional text declaration.
            let text = std::fs::read_to_string(&file).unwrap();
            let body = match text.strip_prefix("<?xml") {
                Some(rest) => &rest[rest.find("?>").unwrap() + 2..],
                None => &text,
            };
            let doc = parsec_xml::parse(&format!("<part>{body}</part>"))
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            let base = file.parent().unwrap().to_path_buf();
            collect(&doc, doc.root_element(), &base, &mut out);
        }
    }
    out
}

/// James Clark's canonical form, as the suite's `out` files have it:
/// no XML declaration, comments or CDATA sections, attributes sorted,
/// `<a></a>` for an empty element, and its own escapes. With notations,
/// the second form: a `<!DOCTYPE>` listing them, sorted.
fn canonical(doc: &Document, notations: bool) -> String {
    let mut out = String::new();
    if notations && !doc.notations().is_empty() {
        let mut ns: Vec<_> = doc.notations().iter().collect();
        ns.sort_by(|a, b| a.name.cmp(&b.name));
        out.push_str("<!DOCTYPE ");
        out.push_str(&doc.doctype().unwrap().name);
        out.push_str(" [\n");
        for n in ns {
            out.push_str("<!NOTATION ");
            out.push_str(&n.name);
            match (&n.public_id, &n.system_id) {
                (Some(p), None) => out.push_str(&format!(" PUBLIC '{p}'")),
                (Some(p), Some(s)) => out.push_str(&format!(" PUBLIC '{p}' '{s}'")),
                (None, Some(s)) => out.push_str(&format!(" SYSTEM '{s}'")),
                (None, None) => {}
            }
            out.push_str(">\n");
        }
        out.push_str("]>\n");
    }
    let escape = |s: &str, out: &mut String| {
        for c in s.chars() {
            match c {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\t' => out.push_str("&#9;"),
                '\n' => out.push_str("&#10;"),
                '\r' => out.push_str("&#13;"),
                c => out.push(c),
            }
        }
    };
    let root = doc.root();
    let mut stack = vec![(root, false)];
    while let Some((n, closing)) = stack.pop() {
        match doc.kind(n) {
            NodeKind::Element(name) if closing => {
                out.push_str("</");
                out.push_str(name.qname);
                out.push('>');
            }
            NodeKind::Element(name) => {
                out.push('<');
                out.push_str(name.qname);
                let mut attrs: Vec<_> = doc.attributes(n).collect();
                attrs.sort_by(|a, b| a.name.qname.cmp(b.name.qname));
                for a in attrs {
                    out.push(' ');
                    out.push_str(a.name.qname);
                    out.push_str("=\"");
                    escape(a.value, &mut out);
                    out.push('"');
                }
                out.push('>');
                stack.push((n, true));
                let children: Vec<_> = doc.children(n).collect();
                stack.extend(children.into_iter().rev().map(|c| (c, false)));
            }
            NodeKind::Text(t) | NodeKind::CData(t) => escape(t, &mut out),
            NodeKind::ProcessingInstruction { target, data } => {
                out.push_str("<?");
                out.push_str(target);
                out.push(' ');
                out.push_str(data);
                out.push_str("?>");
            }
            NodeKind::Document => {
                let children: Vec<_> = doc.children(n).collect();
                stack.extend(children.into_iter().rev().map(|c| (c, false)));
            }
            _ => {}
        }
    }
    out
}

#[test]
fn xmlconf() {
    if !data_dir().join("COMMIT").exists() {
        assert!(
            std::env::var_os("CI").is_none(),
            "tests/xmlconf is missing: run crates/parsec-xml/scripts/fetch-xmlconf.sh"
        );
        eprintln!(
            "tests/xmlconf is missing, so xmlconf didn't run: run crates/parsec-xml/scripts/fetch-xmlconf.sh"
        );
        return;
    }
    let cases = cases();
    // A stale entry would hide nothing, but a typo would hide a real skip.
    for id in EXTERNAL_FAULT {
        let case = cases.iter().find(|c| c.id == *id);
        assert!(
            case.is_some_and(|c| c.kind == "not-wf" && c.entities != "none"),
            "EXTERNAL_FAULT lists {id}, which isn't a not-wf case with external entities"
        );
    }
    let (mut passed, mut compared, mut failures) = (0, 0, Vec::new());
    let mut skipped: BTreeMap<&str, usize> = BTreeMap::new();
    for case in &cases {
        if let Some(why) = case.skip {
            *skipped.entry(why).or_default() += 1;
            continue;
        }
        let bytes = std::fs::read(&case.file).unwrap();
        let options = Options {
            namespaces: case.namespaces,
            ..Options::default()
        };
        let result = std::panic::catch_unwind(|| parse_bytes(&bytes, &options).map(|p| p.document));
        let result = match result {
            Ok(r) => r,
            Err(_) => {
                failures.push(format!("{}: panicked", case.id));
                continue;
            }
        };
        match (case.kind.as_str(), result) {
            ("valid" | "invalid", Ok(doc)) => {
                // Written back out, it parses to the same thing.
                let xml = doc.to_xml();
                match parsec_xml::parse_with(&xml, &options) {
                    Ok(again)
                        if again.to_xml() == xml
                            && canonical(&again, true) == canonical(&doc, true) => {}
                    Ok(_) => {
                        failures.push(format!(
                            "{}: written out, it parses differently:\n{xml}",
                            case.id
                        ));
                        continue;
                    }
                    Err(e) => {
                        failures.push(format!(
                            "{}: written out, it doesn't parse: {e}\n{xml}",
                            case.id
                        ));
                        continue;
                    }
                }
                let compare = case.kind == "valid"
                    && case.entities == "none"
                    && !BAD_OUTPUT.iter().any(|(id, _)| *id == case.id);
                match &case.output {
                    Some(out) if compare => {
                        let want = std::fs::read_to_string(out).unwrap();
                        let got = canonical(&doc, want.starts_with("<!DOCTYPE"));
                        if got == want {
                            passed += 1;
                            compared += 1;
                        } else {
                            failures.push(format!(
                                "{}: canonical form\n   got: {got:?}\n  want: {want:?}",
                                case.id
                            ));
                        }
                    }
                    _ => passed += 1,
                }
            }
            ("valid" | "invalid", Err(e)) => {
                failures.push(format!("{} ({}): {e}", case.id, case.kind));
            }
            ("not-wf", Err(_)) => passed += 1,
            ("not-wf", Ok(_)) if EXTERNAL_FAULT.contains(&case.id.as_str()) => {
                *skipped
                    .entry("not well-formed in an external entity, which Parsec never reads")
                    .or_default() += 1;
            }
            ("not-wf", Ok(_)) => failures.push(format!("{}: not-wf, but parsed", case.id)),
            (kind, _) => failures.push(format!("{}: unknown type {kind}", case.id)),
        }
    }
    let skipped_total: usize = skipped.values().sum();
    let because = skipped
        .iter()
        .map(|(why, n)| format!("{n} {why}"))
        .collect::<Vec<_>>()
        .join("; ");
    summary::write(&summary::Counts {
        suite: "xmlconf",
        passed,
        failed: failures.len(),
        skipped: skipped_total,
        skipped_because: &because,
        data: &data_dir(),
    });
    eprintln!(
        "xmlconf: {passed} passed ({compared} matching their canonical output), {} failed, {skipped_total} skipped ({because})",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "{} of {} cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
