// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Module records against ECMA-262's tables: "Import Forms Mappings to
//! ImportEntry Records" and "Export Forms Mappings to ExportEntry
//! Records" (16.2.1.6), and ParseModule's sorting of exports into local,
//! indirect and star entries.

use parsec_js::module::{ExportEntry, ExportImportName, ImportName, ModuleRecord};
use parsec_js::{Options, SourceType, parse};

fn record(src: &str) -> ModuleRecord {
    let options = Options {
        source_type: SourceType::Module,
        ..Options::default()
    };
    let ast = parse(src, &options).unwrap_or_else(|d| panic!("{src:?}: {d}"));
    ModuleRecord::new(&ast, src)
}

/// An entry as the spec's tables write one: `[[ModuleRequest]]` as the
/// specifier, the names as strings, `null`, or the spec's values.
fn row(r: &ModuleRecord, e: &ExportEntry) -> [String; 4] {
    let opt = |s: &Option<String>| s.clone().unwrap_or_else(|| "null".into());
    let request = e.module_request.map_or_else(
        || "null".into(),
        |i| r.requested_modules[i].specifier.clone(),
    );
    let import = match &e.import_name {
        None => "null".into(),
        Some(ExportImportName::Name(n)) => n.clone(),
        Some(ExportImportName::All) => "all".into(),
        Some(ExportImportName::AllButDefault) => "all-but-default".into(),
    };
    [opt(&e.export_name), request, import, opt(&e.local_name)]
}

#[test]
fn import_forms_map_to_import_entries() {
    let cases: [(&str, &str, &str); 4] = [
        (r#"import v from "mod";"#, "default", "v"),
        (r#"import * as ns from "mod";"#, "namespace-object", "ns"),
        (r#"import {x} from "mod";"#, "x", "x"),
        (r#"import {x as v} from "mod";"#, "x", "v"),
    ];
    for (src, import_name, local_name) in cases {
        let r = record(src);
        assert_eq!(r.import_entries.len(), 1, "{src}");
        let e = &r.import_entries[0];
        assert_eq!(
            r.requested_modules[e.module_request].specifier, "mod",
            "{src}"
        );
        let shown = match &e.import_name {
            ImportName::Name(n) => n.as_str(),
            ImportName::NamespaceObject => "namespace-object",
        };
        assert_eq!(
            (shown, e.local_name.as_str()),
            (import_name, local_name),
            "{src}"
        );
    }
    // `import "mod"` requests the module and imports nothing.
    let r = record(r#"import "mod";"#);
    assert!(r.import_entries.is_empty());
    assert_eq!(r.requested_modules[0].specifier, "mod");
}

#[test]
fn export_forms_map_to_export_entries() {
    // [[ExportName]], [[ModuleRequest]], [[ImportName]], [[LocalName]].
    let cases: [(&str, [&str; 4]); 10] = [
        ("export var v;", ["v", "null", "null", "v"]),
        (
            "export default function f() {}",
            ["default", "null", "null", "f"],
        ),
        (
            "export default function () {}",
            ["default", "null", "null", "*default*"],
        ),
        (
            "export default 42;",
            ["default", "null", "null", "*default*"],
        ),
        ("export {x}; var x;", ["x", "null", "null", "x"]),
        ("export {v as x}; var v;", ["x", "null", "null", "v"]),
        (r#"export {x} from "mod";"#, ["x", "mod", "x", "null"]),
        (r#"export {v as x} from "mod";"#, ["x", "mod", "v", "null"]),
        (
            r#"export * from "mod";"#,
            ["null", "mod", "all-but-default", "null"],
        ),
        (
            r#"export * as ns from "mod";"#,
            ["ns", "mod", "all", "null"],
        ),
    ];
    for (src, expected) in cases {
        let r = record(src);
        let all: Vec<&ExportEntry> = r
            .local_export_entries
            .iter()
            .chain(&r.indirect_export_entries)
            .chain(&r.star_export_entries)
            .collect();
        assert_eq!(all.len(), 1, "{src}");
        assert_eq!(row(&r, all[0]), expected.map(String::from), "{src}");
    }
}

#[test]
fn parse_module_sorts_exports() {
    let r = record(
        r#"import a, { b as c } from "m"; import * as ns from "n";
        export { a, c as d, ns }; export let e; export * from "o"; export { f } from "p"; export * as g from "q";"#,
    );
    let rows = |list: &[ExportEntry]| list.iter().map(|e| row(&r, e)).collect::<Vec<_>>();
    let s = |a: [&str; 4]| a.map(String::from);
    // An imported namespace exported again, and a local binding.
    assert_eq!(
        rows(&r.local_export_entries),
        [
            s(["ns", "null", "null", "ns"]),
            s(["e", "null", "null", "e"])
        ]
    );
    // An imported name exported again is the import's re-export.
    assert_eq!(
        rows(&r.indirect_export_entries),
        [
            s(["a", "m", "default", "null"]),
            s(["d", "m", "b", "null"]),
            s(["f", "p", "f", "null"]),
            s(["g", "q", "all", "null"]),
        ]
    );
    assert_eq!(
        rows(&r.star_export_entries),
        [s(["null", "o", "all-but-default", "null"])]
    );
}

#[test]
fn every_declared_name_is_exported() {
    let r = record(
        "export const a = 1, { b, c: [d, ...e] } = f, [g = 1, , { h }] = i; export function j() {} export class K {}",
    );
    let names: Vec<&str> = r
        .local_export_entries
        .iter()
        .map(|e| e.export_name.as_deref().unwrap())
        .collect();
    assert_eq!(names, ["a", "b", "d", "e", "g", "h", "j", "K"]);
}

#[test]
fn requested_modules_are_listed_once_in_order() {
    let r = record(
        r#"import a from "x"; export * from "y"; import b from "x"; import c from "x" with { type: "json" };
        import d from "x" with { "type": "json" }; export { e } from "z"; import "y";"#,
    );
    let shown: Vec<(&str, usize)> = r
        .requested_modules
        .iter()
        .map(|m| (m.specifier.as_str(), m.attributes.len()))
        .collect();
    assert_eq!(shown, [("x", 0), ("y", 0), ("x", 1), ("z", 0)]);
    let requests: Vec<usize> = r.import_entries.iter().map(|e| e.module_request).collect();
    assert_eq!(requests, [0, 0, 2, 2]);
}

#[test]
fn import_attributes_are_sorted_by_key() {
    let a = record(r#"import x from "m" with { type: "json", b: "1", "a": "2" };"#);
    let b = record(r#"import x from "m" with { a: "2", type: "json", b: "1" };"#);
    let attributes = |r: &ModuleRecord| r.requested_modules[0].attributes.clone();
    let expected =
        [("a", "2"), ("b", "1"), ("type", "json")].map(|(k, v)| (k.to_string(), v.to_string()));
    assert_eq!(attributes(&a), expected);
    assert_eq!(attributes(&a), attributes(&b));
}

#[test]
fn strings_and_escapes_are_decoded() {
    let r = record(r#"import { "a-b" as c, d } from "./x.js"; export { c as "e f" };"#);
    assert_eq!(r.requested_modules[0].specifier, "./x.js");
    assert_eq!(
        r.import_entries[0].import_name,
        ImportName::Name("a-b".into())
    );
    assert_eq!(r.import_entries[1].local_name, "d");
    assert_eq!(
        r.indirect_export_entries[0].export_name.as_deref(),
        Some("e f")
    );
    let r = record(r#"export { "x" } from "m"; export * as "y" from "n";"#);
    assert_eq!(
        row(&r, &r.indirect_export_entries[0]),
        ["x", "m", "x", "null"].map(String::from)
    );
    assert_eq!(
        r.indirect_export_entries[1].export_name.as_deref(),
        Some("y")
    );
}

#[test]
fn spans_point_at_each_entry() {
    let src = r#"import { a as b } from "m"; export default class {} export * from "n";"#;
    let r = record(src);
    let text = |s: parsec_js::Span| s.text(src).to_string();
    assert_eq!(text(r.import_entries[0].span), "a as b");
    assert_eq!(text(r.requested_modules[0].span), r#""m""#);
    assert_eq!(
        text(r.local_export_entries[0].span),
        "export default class {}"
    );
    assert_eq!(text(r.star_export_entries[0].span), r#"export * from "n";"#);
}

#[test]
fn a_script_has_no_records() {
    let src = "import('m'); var a;";
    let ast = parse(src, &Options::default()).unwrap();
    assert_eq!(ModuleRecord::new(&ast, src), ModuleRecord::default());
}

#[test]
fn two_lone_surrogates_are_two_requests() {
    let r = record(r#"import "\uD800"; import "\uD801"; import "\uD800";"#);
    assert_eq!(r.requested_modules.len(), 2);
    assert_eq!(r.import_entries.len(), 0);
}

fn best_of_three(f: impl Fn()) -> std::time::Duration {
    (0..3)
        .map(|_| {
            let t = std::time::Instant::now();
            f();
            t.elapsed()
        })
        .min()
        .unwrap()
}

/// Builds the record for n and 10n of `make`'s input, parse excluded,
/// and fails if 10x the input takes 35x the time.
fn assert_linear(what: &str, make: impl Fn(usize) -> String) {
    let options = Options {
        source_type: SourceType::Module,
        ..Options::default()
    };
    let (small, big) = (make(5_000), make(50_000));
    let (a, b) = (
        parse(&small, &options).unwrap(),
        parse(&big, &options).unwrap(),
    );
    let ts = best_of_three(|| drop(ModuleRecord::new(&a, &small)));
    let tb = best_of_three(|| drop(ModuleRecord::new(&b, &big)));
    let ratio = tb.as_secs_f64() / ts.as_secs_f64().max(1e-6);
    assert!(
        ratio < 35.0,
        "{what}: 10x the input took {ratio:.1}x the time ({ts:?}, {tb:?})"
    );
}

#[test]
fn many_requests_and_exports_stay_linear() {
    let lines = |n: usize, f: &dyn Fn(usize) -> String| (0..n).map(f).collect::<String>();
    assert_linear("distinct imports", |n| {
        lines(n, &|i| format!("import a{i} from 'm{i}';\n"))
    });
    assert_linear("export * from distinct", |n| {
        lines(n, &|i| format!("export * from 'm{i}';\n"))
    });
    assert_linear("imports and local exports", |n| {
        lines(n, &|i| {
            format!("import a{i} from 'm{i}';\nexport {{ a{i} }};\n")
        })
    });
}
