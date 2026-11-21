// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The spec's early errors, a test per rule, wherever it's checked: the
//! parser's first error, or failing that the first of `early_errors`.
//! Each rule rejects its sources with a stable code and accepts the
//! near misses that aren't errors.

use parsec_js::{Diagnostic, Options, SourceType, early_errors, parse};

fn first_error(src: &str, module: bool) -> Option<Diagnostic> {
    let options = Options {
        source_type: if module {
            SourceType::Module
        } else {
            SourceType::Script
        },
        ..Options::default()
    };
    match parse(src, &options) {
        Err(d) => Some(d),
        Ok(ast) => early_errors(&ast, src).into_iter().next(),
    }
}

fn rejects(code: &str, module: bool, sources: &[&str]) {
    for src in sources {
        match first_error(src, module) {
            None => panic!("{src:?} was accepted, but should fail with {code}"),
            Some(d) => assert_eq!(d.code, code, "{src:?}: {d}"),
        }
    }
}

fn accepts(module: bool, sources: &[&str]) {
    for src in sources {
        if let Some(d) = first_error(src, module) {
            panic!("{src:?} should be fine: {d}");
        }
    }
}

#[test]
fn duplicate_lexical_declarations() {
    rejects(
        "duplicate-declaration",
        false,
        &[
            "let a; let a;",
            "{ const a = 1; class a {} }",
            "function f(a) { let a }",
        ],
    );
    rejects(
        "duplicate-declaration",
        true,
        &[
            "import a from 'm'; let a;",
            "function f() {} function f() {}",
        ],
    );
    accepts(
        false,
        &["let a; { let a; }", "{ function f() {} function f() {} }"],
    );
}

#[test]
fn let_and_var_conflicts() {
    rejects(
        "lexical-var-conflict",
        false,
        &[
            "let a; var a;",
            "{ var a; } let a;",
            "for (let i;;) { var i; }",
        ],
    );
    rejects("lexical-var-conflict", true, &["var f; function f() {}"]);
    accepts(
        false,
        &[
            "var a; var a;",
            "try {} catch (e) { var e; }",
            "{ let a; } var a;",
        ],
    );
}

#[test]
fn duplicate_parameters_where_disallowed() {
    rejects(
        "duplicate-binding",
        false,
        &[
            "'use strict'; function f(a, a) {}",
            "function f(a, a) { 'use strict' }",
            "(a, a) => 1",
            "({ m(a, a) {} })",
            "class C { m(a, a) {} }",
            "function f(a, a = 1) {}",
            "function f(a, [a]) {}",
            "function f(a, ...a) {}",
            "let [a, a] = [];",
        ],
    );
    accepts(false, &["function f(a, a) {}", "(function (a, a) {})"]);
}

#[test]
fn jumps_need_a_target() {
    rejects(
        "jump-outside-loop",
        false,
        &[
            "break;",
            "continue;",
            "while (a) { function f() { break; } }",
            "switch (a) { case 1: continue; }",
        ],
    );
    rejects(
        "continue-not-loop",
        false,
        &["a: { continue a; }", "a: if (b) continue a;"],
    );
    accepts(
        false,
        &[
            "while (a) break;",
            "for (;;) continue;",
            "switch (a) { case 1: break; }",
            "a: { break a; }",
            "a: while (b) continue a;",
        ],
    );
}

#[test]
fn labels_must_be_defined_and_unique() {
    rejects(
        "undefined-label",
        false,
        &[
            "while (a) break b;",
            "a: { } break a;",
            "a: { function f() { break a; } }",
        ],
    );
    rejects("duplicate-label", false, &["a: a: ;", "a: { a: ; }"]);
    accepts(false, &["a: ; a: ;", "a: { b: ; }"]);
}

#[test]
fn assignment_targets_must_be_assignable() {
    rejects(
        "invalid-target",
        false,
        &[
            "1 = a",
            "a + b = c",
            "++(a + b)",
            "a++ = 1",
            "[a + b] = c",
            "({a: 1} = b)",
            "for (a + b of c) ;",
            "(a, b) = c",
        ],
    );
    accepts(
        false,
        &[
            "a = 1",
            "a.b = 1",
            "a[b] = 1",
            "(a) = 1",
            "[a, {b}] = c",
            "({a: b.c} = d)",
            "f() = 1",
        ],
    );
    // B.3.6: in sloppy code, a call is left to fail when it runs.
    rejects(
        "invalid-target",
        false,
        &["'use strict'; f() = 1", "'use strict'; ++a.b()"],
    );
    accepts(false, &["++a.b()", "f() += 1"]);
}

#[test]
fn strict_mode_restrictions() {
    rejects("with-in-strict", false, &["'use strict'; with (a) ;"]);
    rejects("with-in-strict", true, &["with (a) ;"]);
    rejects(
        "legacy-octal-in-strict",
        false,
        &[
            "'use strict'; 07",
            "'use strict'; '\\07'",
            "'\\07'; 'use strict'",
        ],
    );
    rejects(
        "strict-binding",
        false,
        &[
            "'use strict'; var eval;",
            "'use strict'; arguments = 1",
            "'use strict'; function f(eval) {}",
            "function eval() { 'use strict' }",
            "'use strict'; try {} catch (arguments) {}",
        ],
    );
    rejects(
        "reserved-word",
        false,
        &[
            "'use strict'; var let;",
            "'use strict'; var yield;",
            "'use strict'; var implements;",
            "var \\u0063lass;",
        ],
    );
    rejects("delete-identifier", false, &["'use strict'; delete a;"]);
    rejects(
        "use-strict-with-complex-params",
        false,
        &["function f(a = 1) { 'use strict' }"],
    );
    accepts(
        false,
        &[
            "with (a) ;",
            "07",
            "var eval; arguments = 1",
            "var let, yield, implements;",
            "delete a",
        ],
    );
}

#[test]
fn super_new_target_await_and_yield_have_their_places() {
    rejects(
        "super-outside-method",
        false,
        &[
            "super.a",
            "function f() { super.a }",
            "class C { constructor() { super() } }",
            "({ m() { super() } })",
        ],
    );
    accepts(
        false,
        &[
            "({ m() { super.a } })",
            "class C extends D { constructor() { super() } }",
            "class C { static m() { super.a } }",
        ],
    );
    rejects(
        "new-target-outside-function",
        false,
        &["new.target", "() => new.target"],
    );
    accepts(
        false,
        &[
            "function f() { new.target }",
            "function f() { () => new.target }",
        ],
    );
    rejects(
        "await-in-params",
        false,
        &["async function f(a = await b) {}", "async (a = await) => 1"],
    );
    rejects("yield-in-params", false, &["function* g(a = yield) {}"]);
    rejects("reserved-word", true, &["var await;"]);
    rejects(
        "reserved-word",
        false,
        &[
            "function* g() { var yield; }",
            "async function f() { var await; }",
        ],
    );
    rejects("return-outside-function", false, &["return;"]);
    accepts(
        false,
        &[
            "var await, yield;",
            "async function f() { await a }",
            "function* g() { yield a }",
        ],
    );
    accepts(true, &["await a;"]);
}

#[test]
fn exports_are_unique_and_declared() {
    rejects(
        "duplicate-export",
        true,
        &[
            "let a; export { a, a };",
            "export default 1; export default 2;",
            "export let a; export { a };",
            "export * as a from 'm'; export const a = 1;",
        ],
    );
    rejects(
        "export-undeclared",
        true,
        &[
            "export { a };",
            "export { a as b };",
            "{ let a; } export { a };",
            "function f() { var a; } export { a };",
        ],
    );
    accepts(
        true,
        &[
            "let a; export { a, a as b };",
            "export { a } from 'm';",
            "import a from 'm'; export { a };",
            "export { a }; var a;",
            "export { f }; function f() {}",
            "export { C }; class C {}",
            "export * from 'm'; export * from 'n';",
        ],
    );
}

#[test]
fn proto_once_in_an_object_literal() {
    rejects(
        "duplicate-proto",
        false,
        &[
            "({ __proto__: a, __proto__: b })",
            "({ '__proto__': a, __proto__: b })",
        ],
    );
    accepts(
        false,
        &[
            "({ __proto__: a, __proto__: b } = c)",
            "({ __proto__: a, ['__proto__']: b })",
            "({ __proto__: a, __proto__ })",
            "({ __proto__: a, __proto__() {} })",
        ],
    );
}

#[test]
fn private_names_and_arguments_in_classes() {
    rejects(
        "undeclared-private-name",
        false,
        &["class C { m() { this.#a } }"],
    );
    rejects("duplicate-private-name", false, &["class C { #a; #a; }"]);
    rejects(
        "arguments-in-class-init",
        false,
        &[
            "class C { a = arguments; }",
            "class C { static { arguments } }",
        ],
    );
    accepts(
        false,
        &[
            "class C { #a; m() { this.#a } }",
            "class C { get #a() {} set #a(v) {} }",
            "class C { a = () => function () { arguments } }",
        ],
    );
}

#[test]
fn regular_expression_patterns() {
    rejects("regexp-range-order", false, &["/[z-a]/"]);
    rejects("regexp-duplicate-group-name", false, &["/(?<a>.)(?<a>.)/"]);
    rejects("regexp-property", false, &["/\\p{Lettr}/u"]);
    rejects("regexp-class-syntax", false, &["/[a-z&&b]/v"]);
    accepts(
        false,
        &[
            "/[a-z]/",
            "/(?<a>.)|(?<a>.)/",
            "/\\p{Letter}/u",
            "/[\\p{L}--[a-z]]/v",
            "/\\p{Lettr}/",
        ],
    );
}

#[test]
fn early_errors_come_in_source_order_with_spans_in_the_source() {
    let src = "/[9-0]/; let a; let a; export { b };";
    let options = Options {
        source_type: SourceType::Module,
        ..Options::default()
    };
    let ast = parse(src, &options).unwrap();
    let errors = early_errors(&ast, src);
    let found: Vec<(&str, &str)> = errors.iter().map(|d| (d.code, d.span.text(src))).collect();
    assert_eq!(
        found,
        [
            ("regexp-range-order", "9-0"),
            ("duplicate-declaration", "a"),
            ("export-undeclared", "b"),
        ]
    );
    let ast = parse("let a = /b/u;", &Options::default()).unwrap();
    assert!(early_errors(&ast, "let a = /b/u;").is_empty());
}
