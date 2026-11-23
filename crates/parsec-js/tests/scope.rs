// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Scope analysis: which binding each reference resolves to, rule by
//! rule of ECMA-262's declaration instantiation, with Annex B's
//! additions, and the redeclarations the spec makes early errors.

use parsec_js::scope::{Access, BindingKind, ScopeKind, Scopes};
use parsec_js::{Ast, Node, Options, SourceType, parse};

fn analyze(src: &str, module: bool) -> (Ast, Scopes) {
    let options = Options {
        source_type: if module {
            SourceType::Module
        } else {
            SourceType::Script
        },
        ..Options::default()
    };
    let ast = parse(src, &options).unwrap_or_else(|d| panic!("{src:?}: {d}"));
    let scopes = Scopes::analyze(&ast, src);
    (ast, scopes)
}

/// Each reference in source order, as `name#n`, where `n` counts the
/// declarations of that name in source order and says which one it
/// resolved to: `name#free` for none, `name#arguments` for the implicit
/// object. A trailing `!` is a reference in the TDZ.
fn show(src: &str, module: bool) -> Vec<String> {
    let (ast, scopes) = analyze(src, module);
    assert!(
        scopes.diagnostics().is_empty(),
        "{src:?}: {:?}",
        scopes.diagnostics()
    );
    let mut refs: Vec<_> = scopes.references().to_vec();
    refs.sort_by_key(|r| ast.span(r.node).start);
    refs.iter()
        .map(|r| {
            let name = r.name(&ast, src);
            let which = match r.binding {
                None => "free".to_string(),
                Some(b) => {
                    let b = scopes.binding(b);
                    match b.node {
                        None => "arguments".to_string(),
                        Some(node) => {
                            let at = ast.span(node).start;
                            let mut starts: Vec<u32> = scopes
                                .bindings()
                                .iter()
                                .filter(|o| o.name == b.name)
                                .filter_map(|o| o.node.map(|n| ast.span(n).start))
                                .collect();
                            starts.sort();
                            starts.dedup();
                            let n = starts.iter().position(|&s| s == at).unwrap() + 1;
                            n.to_string()
                        }
                    }
                }
            };
            format!("{name}#{which}{}", if r.tdz { "!" } else { "" })
        })
        .collect()
}

fn script(src: &str, expected: &[&str]) {
    assert_eq!(show(src, false), expected, "{src:?}");
}

fn module(src: &str, expected: &[&str]) {
    assert_eq!(show(src, true), expected, "{src:?}");
}

/// The codes of the diagnostics for `src`.
fn errors(src: &str, module: bool) -> Vec<&'static str> {
    let (_, scopes) = analyze(src, module);
    scopes.diagnostics().iter().map(|d| d.code).collect()
}

fn bad_script(code: &str, sources: &[&str]) {
    for src in sources {
        assert_eq!(errors(src, false), [code], "{src:?}");
    }
}

fn bad_module(code: &str, sources: &[&str]) {
    for src in sources {
        assert_eq!(errors(src, true), [code], "{src:?}");
    }
}

fn fine(sources: &[&str], module: bool) {
    for src in sources {
        assert_eq!(errors(src, module), Vec::<&str>::new(), "{src:?}");
    }
}

// Resolution.

#[test]
fn references_resolve_to_the_nearest_declaration() {
    script("let x; { let x; x } x", &["x#2", "x#1"]);
    script("var a = 1; function f(a) { return a } a", &["a#2", "a#1"]);
    script("const c = 1; (() => c)()", &["c#1"]);
    script("console.log(x)", &["console#free", "x#free"]);
}

#[test]
fn var_and_function_declarations_hoist_to_their_function() {
    script("x; var x;", &["x#1"]);
    script("f(); function f() {}", &["f#1"]);
    script("function g() { { var v } return v } v", &["v#1", "v#free"]);
    // Declared twice, one binding.
    script("var x; var x; x", &["x#1"]);
    script("var f; function f() {} f", &["f#1"]);
}

#[test]
fn let_const_and_class_are_block_scoped() {
    script(
        "{ let a; const b = 1; class C {} } a; b; C",
        &["a#free", "b#free", "C#free"],
    );
    script(
        "for (let i = 0; i < 1; i++) i; i",
        &["i#1", "i#1", "i#1", "i#free"],
    );
    script("for (const k in o) k; k", &["o#free", "k#1", "k#free"]);
    script(
        "switch (x) { case 1: let y; default: y }",
        &["x#free", "y#1"],
    );
}

#[test]
fn the_temporal_dead_zone_is_flagged() {
    script("x; let x;", &["x#1!"]);
    script("let x = x;", &["x#1!"]);
    script("{ x } let x;", &["x#1!"]);
    script("const c = 1; c", &["c#1"]);
    // A function that runs later sees it initialized.
    script("function f() { return x } let x;", &["x#1"]);
    script("() => x; let x;", &["x#1"]);
    // A class's own name inside it is a second binding, declared by the
    // same identifier.
    script("class C extends C {}", &["C#1!"]);
    script("class C { m() { return C } }", &["C#1"]);
    script("class C { static f = C; [C] }", &["C#1", "C#1!"]);
    script("for (let x of x) ;", &["x#1!"]);
    script("for (let x in {}) x;", &["x#1"]);
    // var, functions and parameters have none.
    script("v; var v;", &["v#1"]);
    script("f; function f() {}", &["f#1"]);
}

#[test]
fn parameters_and_the_body() {
    script("function f(a, b = a) { return b }", &["a#1", "b#1"]);
    // With an expression in the parameters, the body's declarations are
    // apart from them.
    script(
        "var x; function f(a = x) { var x; return x }",
        &["x#1", "x#2"],
    );
    script(
        "var x; function f(a = x) { let x; return x }",
        &["x#1", "x#2"],
    );
    script("var x; function f(a) { var x; return x }", &["x#2"]);
    // A body `var` of a parameter's name is the parameter.
    script("function f(a) { var a; return a }", &["a#1"]);
    script("function f(a, a) { return a }", &["a#1"]);
    script(
        "(a, {b, c: [d]}, ...e) => a + b + d + e",
        &["a#1", "b#1", "d#1", "e#1"],
    );
}

#[test]
fn arguments_is_implicit_in_functions_but_not_arrows() {
    script(
        "function f() { return arguments }",
        &["arguments#arguments"],
    );
    script(
        "function f() { return () => arguments }",
        &["arguments#arguments"],
    );
    script("() => arguments", &["arguments#free"]);
    script(
        "function f(arguments) { return arguments }",
        &["arguments#1"],
    );
    script(
        "function f() { let arguments; return arguments }",
        &["arguments#1"],
    );
    let src = "function f() { arguments; arguments }";
    let (_, scopes) = analyze(src, false);
    let implicit: Vec<_> = scopes
        .bindings()
        .iter()
        .filter(|b| b.kind == BindingKind::Arguments)
        .collect();
    assert_eq!(implicit.len(), 1);
    assert_eq!(scopes.scope(implicit[0].scope).kind, ScopeKind::Function);
}

#[test]
fn a_function_expression_sees_its_own_name() {
    script("var f; (function f() { return f })", &["f#2"]);
    script("(function f() { var f; return f })", &["f#2"]);
    script("(function f(f) { return f })", &["f#2"]);
    script("(function f() {}); f", &["f#free"]);
    script("(class C { m() { C } }); C", &["C#1", "C#free"]);
}

#[test]
fn catch_parameters_are_scoped_to_the_clause() {
    script("try {} catch (e) { e } e", &["e#1", "e#free"]);
    script("try {} catch ({ message }) { message }", &["message#1"]);
    // B.3.4: a var of the parameter's name is allowed, and is hoisted.
    script("try {} catch (e) { var e = 1; } e", &["e#2"]);
    script("try {} catch (e) { for (var e of []) ; }", &[]);
}

#[test]
fn class_fields_and_static_blocks_get_scopes() {
    let src = "class A { x = 1; static { var y; } [k] = 2; m() {} }";
    let (ast, scopes) = analyze(src, false);
    let kinds: Vec<ScopeKind> = scopes.scopes().iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        [
            ScopeKind::Global,
            ScopeKind::Class,
            ScopeKind::ClassField,
            ScopeKind::StaticBlock,
            ScopeKind::ClassField,
            ScopeKind::Function,
        ]
    );
    let y = scopes.bindings().iter().find(|b| b.name == "y").unwrap();
    assert_eq!(scopes.scope(y.scope).kind, ScopeKind::StaticBlock);
    // The computed key is evaluated in the class's scope.
    let k = scopes
        .references()
        .iter()
        .find(|r| r.name(&ast, src) == "k")
        .unwrap();
    assert_eq!(scopes.scope(k.scope).kind, ScopeKind::Class);
    assert!(
        scopes
            .scopes()
            .iter()
            .all(|s| s.strict || s.kind == ScopeKind::Global)
    );
}

#[test]
fn annex_b_hoists_sloppy_functions_in_blocks() {
    // Inside the block, the block's; outside, the var Annex B adds.
    script("{ function f() {} f } f", &["f#1", "f#1"]);
    let src = "{ function f() {} } f";
    let (ast, scopes) = analyze(src, false);
    let outer = scopes
        .references()
        .iter()
        .find(|r| ast.span(r.node).start == 20)
        .unwrap();
    let b = scopes.binding(outer.binding.unwrap());
    assert_eq!(scopes.scope(b.scope).kind, ScopeKind::Global);
    assert_eq!(b.kind, BindingKind::Function);
    // Not where a var of the name would be an error, nor for a
    // parameter's name, nor in strict code, nor for a generator.
    script("let f; { function f() {} } f", &["f#1"]);
    script("function g(f) { { function f() {} } return f }", &["f#1"]);
    script("'use strict'; { function f() {} } f", &["f#free"]);
    script("{ function* f() {} } f", &["f#free"]);
    script("{ async function f() {} } f", &["f#free"]);
    // The nearest var scope, past blocks and a simple catch parameter.
    script(
        "function g() { { { function f() {} } } return f } f",
        &["f#1", "f#free"],
    );
    script("try {} catch (f) { { function f() {} } } f", &["f#2"]);
    // `if (a) function f() {}` is as if in a block.
    script("if (a) function f() {} f", &["a#free", "f#1"]);
    script("let f; if (a) function f() {}", &["a#free"]);
    let (_, scopes) = analyze("{ function f() {} }", false);
    assert!(scopes.diagnostics().is_empty());
}

#[test]
fn modules_bind_imports_and_refer_to_exports() {
    module(
        "import a, { b as c, d } from 'm'; import * as ns from 'n'; a; c; d; ns; b",
        &["a#1", "c#1", "d#1", "ns#1", "b#free"],
    );
    module(
        "let x; export { x, x as y }; export default x",
        &["x#1", "x#1", "x#1"],
    );
    module("export { x } from 'm'; x", &["x#free"]);
    module("export function f() {} f", &["f#1"]);
    let (_, scopes) = analyze("import a from 'm'", true);
    assert_eq!(scopes.scope(scopes.root()).kind, ScopeKind::Module);
    assert_eq!(scopes.bindings()[0].kind, BindingKind::Import);
}

#[test]
fn writes_are_told_from_reads() {
    let src = "x = 1; y += 1; z++; [a, {b}] = c; for (d of e) ; f ||= g; h.i = j";
    let (ast, scopes) = analyze(src, false);
    let access = |name: &str| {
        scopes
            .references()
            .iter()
            .find(|r| r.name(&ast, src) == name)
            .unwrap()
            .access
    };
    assert_eq!(access("x"), Access::Write);
    assert_eq!(access("y"), Access::ReadWrite);
    assert_eq!(access("z"), Access::ReadWrite);
    assert_eq!(access("a"), Access::Write);
    assert_eq!(access("b"), Access::Write);
    assert_eq!(access("c"), Access::Read);
    assert_eq!(access("d"), Access::Write);
    assert_eq!(access("f"), Access::ReadWrite);
    assert_eq!(access("h"), Access::Read);
    assert_eq!(access("j"), Access::Read);
}

#[test]
fn names_that_are_not_references_are_left_alone() {
    script(
        "a.b; ({c: d, [e]: f, g() {}}); h: for (;;) { break h; } function n() { new.target } class K { #p; i = 1; j() { this.#p } }",
        &["a#free", "d#free", "e#free", "f#free"],
    );
    // A shorthand property is a reference.
    script("({ s })", &["s#free"]);
    module("import.meta; export * as q from 'm'", &[]);
}

#[test]
fn escaped_names_are_the_same_name() {
    script("let \\u0061; a", &["a#1"]);
}

#[test]
fn eval_and_with_make_scopes_dynamic() {
    let (_, scopes) = analyze("function f() { { eval('x') } } function g() {}", false);
    let dynamic: Vec<(ScopeKind, bool)> = scopes
        .scopes()
        .iter()
        .map(|s| (s.kind, s.dynamic))
        .collect();
    assert_eq!(
        dynamic,
        [
            (ScopeKind::Global, true),
            (ScopeKind::Function, true),
            (ScopeKind::Block, true),
            (ScopeKind::Function, false),
        ]
    );
    // Only a direct call: not a member, not optional, not in a sequence.
    for src in ["o.eval(x)", "eval?.(x)", "(0, eval)(x)", "eval"] {
        let (_, scopes) = analyze(src, false);
        assert!(!scopes.scope(scopes.root()).dynamic, "{src}");
    }
    let (_, scopes) = analyze("(eval)(x)", false);
    assert!(scopes.scope(scopes.root()).dynamic);
    let (ast, scopes) = analyze("with (o) { x }", false);
    let with = scopes
        .scopes()
        .iter()
        .find(|s| s.kind == ScopeKind::With)
        .unwrap();
    assert!(with.dynamic);
    assert!(matches!(ast.node(with.node), Node::With { .. }));
    assert!(!scopes.scope(scopes.root()).dynamic);
}

#[test]
fn lookups_by_node() {
    let src = "let a = 1; function f(b) { return a + b }";
    let (ast, scopes) = analyze(src, false);
    let Node::Program { body, .. } = *ast.node(ast.root()) else {
        unreachable!()
    };
    let f = ast.get(body)[1];
    let fs = scopes.scope_of(f).unwrap();
    assert_eq!(scopes.scope(fs).kind, ScopeKind::Function);
    assert_eq!(scopes.scope(fs).parent, Some(scopes.root()));
    assert_eq!(scopes.scope_of(ast.root()), Some(scopes.root()));
    let b = scopes.lookup(fs, "b").unwrap();
    let decl = scopes.binding(b).node.unwrap();
    assert_eq!(scopes.declaration(decl), Some(b));
    assert!(scopes.reference(decl).is_none());
    for r in scopes.references() {
        assert_eq!(scopes.reference(r.node), Some(r));
        assert!(scopes.declaration(r.node).is_none());
    }
    assert_eq!(scopes.lookup(fs, "a"), scopes.lookup(scopes.root(), "a"));
    assert_eq!(scopes.lookup(fs, "nope"), None);
}

#[test]
fn free_names_are_listed_once_in_order() {
    let (_, scopes) = analyze("a; b; a; let c; c; d.e; f()", false);
    assert_eq!(scopes.free_names(), ["a", "b", "d", "f"]);
}

#[test]
fn deep_trees_are_analyzed_without_recursion() {
    let src = format!("x{}", ".y".repeat(200_000));
    let ast = parse(&src, &Options::default()).unwrap();
    let scopes = std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || Scopes::analyze(&ast, &src).free_names().to_vec())
        .unwrap()
        .join()
        .unwrap();
    assert_eq!(scopes, ["x"]);
}

// Early errors: redeclarations.

#[test]
fn lexical_names_declared_twice() {
    bad_script(
        "duplicate-declaration",
        &[
            "let a; let a;",
            "let a; const a = 1;",
            "class A {} let A;",
            "{ let a; class a {} }",
            "switch (x) { case 1: let a; case 2: let a; }",
            "{ function f() {} let f; }",
            "{ async function f() {} function f() {} }",
            "{ function* f() {} function* f() {} }",
            "'use strict'; { function f() {} function f() {} }",
            "function f(a) { let a; }",
            "function f(a = 1) { let a; }",
            "(a) => { const a = 1; }",
            "try {} catch (e) { let e; }",
            "try {} catch (f) { function f() {} }",
            "class C { static { let a; let a; } }",
        ],
    );
    bad_module(
        "duplicate-declaration",
        &[
            "import a from 'm'; import a from 'n';",
            "import { a, b as a } from 'm';",
            "import a from 'm'; let a;",
            "function f() {} function f() {}",
            "function f() {} async function f() {}",
            "let f; export default function f() {}",
        ],
    );
}

#[test]
fn lexical_and_var_names_that_meet() {
    bad_script(
        "lexical-var-conflict",
        &[
            "let a; var a;",
            "var a; let a;",
            "const a = 1; { var a; }",
            "{ { var a; } let a; }",
            "{ let a; { var a; } }",
            "function f() {} let f;",
            "let f; function f() {}",
            "{ function f() {} var f; }",
            "for (let i;;) { var i; }",
            "for (const i of []) { var i; }",
            "try {} catch ([e]) { var e; }",
            "class C { static { let a; var a; } }",
            "function g() { let a; { var a; } }",
        ],
    );
    bad_module(
        "lexical-var-conflict",
        &["var f; function f() {}", "import a from 'm'; var a;"],
    );
}

#[test]
fn redeclarations_the_spec_allows() {
    fine(
        &[
            "var a; var a;",
            "function f() {} function f() {}",
            "var f; function f() {}",
            "{ function f() {} function f() {} }",
            "switch (x) { case 1: function f() {} case 2: function f() {} }",
            "function f(a) { var a; function a() {} }",
            "function f(a, a) {}",
            "try {} catch (e) { var e; }",
            "try {} catch (e) { for (var e of []) ; }",
            "let a; { let a; }",
            "{ let a; } var a;",
            "(function a() { let a; })",
            "function f() { let arguments; }",
            "{ var a; } { var a; }",
            "label: function f() {} var f;",
            "if (a) function f() {} let f;",
        ],
        false,
    );
    fine(
        &[
            "var a; var a;",
            "export let a; let b;",
            "import { a as b, b as a } from 'm';",
        ],
        true,
    );
}

#[test]
fn a_diagnostic_points_at_the_second_and_labels_the_first() {
    let src = "let a; let a;";
    let (_, scopes) = analyze(src, false);
    let d = &scopes.diagnostics()[0];
    assert_eq!(d.span.text(src), "a");
    assert_eq!(d.span.start, 11);
    assert_eq!(d.labels[0].span.start, 4);
    assert!(d.message.contains("`a`"));
}
