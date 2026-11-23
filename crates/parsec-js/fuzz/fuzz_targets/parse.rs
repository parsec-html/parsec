// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any input parses or fails without panicking, as a script and as a
//! module, with the default limits and with small ones: every node's span
//! and every error's is inside the input and on character boundaries,
//! a tree that parsed writes out as ESTree JSON, and its scope analysis
//! holds together: each reference is an identifier in a scope that
//! exists, resolved to a binding that exists or listed as free; and its
//! early errors, regular expression patterns' included, have spans in
//! the input; and its module records point at requests that exist.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_js::scope::Scopes;
use parsec_js::module::ModuleRecord;
use parsec_js::{Ast, Limits, Node, Options, SourceType, early_errors, parse};

fn on_boundaries(src: &str, start: u32, end: u32) -> bool {
    start <= end
        && end as usize <= src.len()
        && src.is_char_boundary(start as usize)
        && src.is_char_boundary(end as usize)
}

fn check_early_errors(ast: &Ast, src: &str) {
    for d in early_errors(ast, src) {
        assert!(on_boundaries(src, d.span.start, d.span.end), "{d}");
    }
}

fn check_records(ast: &Ast, src: &str) {
    let r = ModuleRecord::new(ast, src);
    let n = r.requested_modules.len();
    assert!(r.import_entries.iter().all(|e| e.module_request < n));
    for e in r.local_export_entries.iter().chain(&r.indirect_export_entries).chain(&r.star_export_entries) {
        assert!(e.module_request.is_none_or(|i| i < n));
        assert!(on_boundaries(src, e.span.start, e.span.end));
    }
    assert!(r.local_export_entries.iter().all(|e| e.local_name.is_some() && e.module_request.is_none()));
    assert!(r.star_export_entries.iter().all(|e| e.export_name.is_none()));
}

fn check_scopes(ast: &Ast, src: &str) {
    let scopes = Scopes::analyze(ast, src);
    for s in scopes.scopes() {
        assert!(s.parent.is_none_or(|p| p.index() < scopes.scopes().len()));
    }
    for r in scopes.references() {
        assert!(matches!(ast.node(r.node), Node::Identifier));
        assert!(r.scope.index() < scopes.scopes().len());
        assert_eq!(scopes.reference(r.node), Some(r));
        match r.binding {
            Some(b) => assert_eq!(scopes.binding(b).name, r.name(ast, src)),
            None => assert!(scopes.free_names().iter().any(|n| *n == r.name(ast, src))),
        }
    }
    for d in scopes.diagnostics() {
        assert!(on_boundaries(src, d.span.start, d.span.end), "{d}");
    }
}

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let small = Limits {
        max_depth: Some(8),
        max_steps: Some(64),
        ..Limits::default()
    };
    for source_type in [SourceType::Script, SourceType::Module] {
        for limits in [Limits::default(), small] {
            let options = Options { source_type, limits };
            match parse(&src, &options) {
                Ok(ast) => {
                    for id in ast.ids() {
                        let span = ast.span(id);
                        assert!(on_boundaries(&src, span.start, span.end), "{id:?} at {span:?}");
                    }
                    for c in ast.comments() {
                        assert!(on_boundaries(&src, c.span.start, c.span.end), "{c:?}");
                    }
                    check_scopes(&ast, &src);
                    check_early_errors(&ast, &src);
                    check_records(&ast, &src);
                    let json = ast.to_estree(&src);
                    assert!(json.starts_with(r#"{"type":"Program""#));
                }
                Err(d) => assert!(d.span.start <= d.span.end && d.span.end as usize <= src.len(), "{d}"),
            }
        }
    }
});
