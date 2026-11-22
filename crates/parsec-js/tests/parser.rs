// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The parser, production by production of ECMA-262 clauses 13 to 15:
//! what each accepts, and what it rejects with which code. The trees
//! themselves are checked against acorn's in `tests/estree.rs`.

use parsec_js::{CommentKind, Node, Options, Severity, parse, parse_script};

/// Each source parses as a script.
fn ok(sources: &[&str]) {
    for src in sources {
        if let Err(d) = parse_script(src) {
            panic!("{src:?} didn't parse: {d}");
        }
    }
}

/// Each source fails to parse as a script with `code`.
fn bad(code: &str, sources: &[&str]) {
    for src in sources {
        match parse_script(src) {
            Ok(_) => panic!("{src:?} parsed, but should fail with {code}"),
            Err(d) => assert_eq!(d.code, code, "{src:?}: {d}"),
        }
    }
}

/// The type of the first statement's expression, as ESTree names it.
fn expression_type(src: &str) -> String {
    let ast = parse_script(src).unwrap();
    let json: serde_json::Value = serde_json::from_str(&ast.to_estree(src)).unwrap();
    json["body"][0]["expression"]["type"]
        .as_str()
        .unwrap()
        .to_string()
}

// 13.2 Primary expressions.

#[test]
fn primary_expressions() {
    ok(&[
        "this",
        "a",
        "null; true; false",
        "1; 'a'; 1n",
        "[]",
        "({})",
        "(a)",
        "`a${b}`",
        "/a/g",
    ]);
    bad(
        "unexpected-token",
        &[")", "a b", "({a b})", "if (a) else b"],
    );
    bad("unexpected-character", &["@", "a #"]);
    bad("unexpected-end", &["a +", "(a", "{", "(", "[a"]);
}

#[test]
fn regex_or_division_is_the_parsers_choice() {
    assert_eq!(expression_type("/a/g"), "Literal");
    assert_eq!(expression_type("a / b / c"), "BinaryExpression");
    assert_eq!(expression_type("a\n/b/g"), "BinaryExpression");
    assert_eq!(expression_type("(a) / b"), "BinaryExpression");
    assert_eq!(expression_type("x = /=/"), "AssignmentExpression");
    ok(&[
        "if (a) /b/.test(c)",
        "{} /b/",
        "a => /b/",
        "[/a/, /b/]",
        "a ? /b/ : /c/",
        "typeof /a/",
    ]);
    ok(&["`${a}` / 2", "`${ {} }`", "`${`${a}`}`", "x = {} / 1"]);
}

#[test]
fn object_and_array_literals() {
    ok(&[
        "({a, b: 1, 'c': 2, 3: 4, [d]: 5, ...e})",
        "({get a() {}, set a(v) {}, async b() {}, *c() {}, async *d() {}})",
        "({get: 1, set: 2, async: 3, static: 4, if: 5})",
        "({get() {}, set() {}, async() {}})",
        "[, a, , ...b, ]",
    ]);
    bad("reserved-word", &["({if})", "({this})"]);
    bad(
        "shorthand-init",
        &["({a = 1})", "f({a = 1})", "({a = 1}).b", "[{a = 1}.b] = c"],
    );
    bad("duplicate-proto", &["({__proto__: 1, '__proto__': 2})"]);
    ok(&[
        "({__proto__: 1, ['__proto__']: 2, __proto__})",
        "({__proto__: a, __proto__: b} = c)",
    ]);
    bad(
        "accessor-params",
        &["({get a(b) {}})", "({set a() {}})", "({set a(...b) {}})"],
    );
}

#[test]
fn templates() {
    ok(&[
        "``",
        "`a${b}c${d}e`",
        "tag`\\unicode`",
        "tag`\\01`",
        "a`x`.b`y`",
    ]);
    bad(
        "invalid-template-escape",
        &["`\\unicode`", "`\\01`", "`\\xg`"],
    );
}

// 13.3 Left-hand-side expressions.

#[test]
fn member_call_and_new() {
    ok(&[
        "a.b.c",
        "a[b]",
        "a.if.class",
        "a()",
        "a(...b, c,)",
        "new A",
        "new A.b()",
        "new new A()()",
    ]);
    ok(&[
        "function f() { new.target }",
        "() => { function f() { new.target } }",
    ]);
    bad(
        "new-target-outside-function",
        &["new.target", "() => new.target"],
    );
    bad("unexpected-token", &["new.targe", "a(,)", "a.1"]);
}

#[test]
fn optional_chains() {
    assert_eq!(expression_type("a?.b"), "ChainExpression");
    assert_eq!(expression_type("(a?.b).c"), "MemberExpression");
    ok(&[
        "a?.b",
        "a?.[b]",
        "a?.()",
        "a?.b.c()",
        "a?.b?.c",
        "class A { #b; m(a) { a?.#b } }",
    ]);
    bad("tagged-template-in-chain", &["a?.b`c`", "a?.`b`"]);
    bad("optional-chain-in-new", &["new a?.b()"]);
    bad("invalid-target", &["a?.b = 1", "a?.b++"]);
}

#[test]
fn super_and_import() {
    ok(&[
        "class A extends B { constructor() { super(); } }",
        "({ m() { super.x } })",
    ]);
    ok(&["class A { m() { super.x; super[y] } static s() { super.z } }"]);
    bad(
        "super-outside-method",
        &[
            "super.x",
            "function f() { super.x }",
            "class A { constructor() { super() } }",
        ],
    );
    bad(
        "unexpected-token",
        &[
            "class A { m() { super } }",
            "class A extends B { m() { super?.x } }",
        ],
    );
    ok(&[
        "import('a')",
        "import('a', {with: {type: 'json'}})",
        "import(a,)",
    ]);
    bad(
        "unexpected-token",
        &["import()", "import(...a)", "(import)"],
    );
    bad("import-meta-outside-module", &["import.meta"]);
    bad("new-import", &["new import('a')"]);
}

// 13.4 to 13.13 Operators.

#[test]
fn update_and_unary() {
    ok(&[
        "a++",
        "a--",
        "++a",
        "--a",
        "++a.b",
        "(a)++",
        "-a",
        "!a",
        "~a",
        "typeof a",
        "void a",
        "delete a.b",
    ]);
    bad(
        "invalid-target",
        &["1++", "++(a + b)", "a + b = c", "++a++"],
    );
    // A line break before a postfix ++ ends the statement: `a; ++b`.
    ok(&["a\n++b"]);
    bad(
        "delete-identifier",
        &["'use strict'; delete a", "'use strict'; delete (a)"],
    );
    bad(
        "delete-private",
        &["class A { #a; m() { delete this.#a } }"],
    );
}

#[test]
fn binary_operators_and_precedence() {
    ok(&[
        "a + b * c ** d ** e",
        "a < b == c",
        "a instanceof b in c",
        "(-a) ** b",
        "a ** -b",
    ]);
    bad(
        "unary-before-exponent",
        &["-a ** b", "typeof a ** b", "!a ** b"],
    );
    ok(&[
        "a ?? b ?? c",
        "(a || b) ?? c",
        "a ?? (b && c)",
        "a || b && c",
    ]);
    bad(
        "mixed-coalesce",
        &["a ?? b || c", "a || b ?? c", "a && b ?? c", "a ?? b && c"],
    );
    ok(&["class A { #x; m(o) { return #x in o } }"]);
    bad(
        "lone-private-name",
        &[
            "class A { #x; m() { #x } }",
            "class A { #x; m() { #x + 1 } }",
            "class A { #x; m(o) { 1 + #x in o } }",
        ],
    );
}

#[test]
fn conditional_and_assignment() {
    ok(&[
        "a ? b : c",
        "a ? b : c ? d : e",
        "a = b = c",
        "a += 1",
        "a &&= b",
        "a ||= b",
        "a ??= b",
    ]);
    ok(&[
        "[a, b] = c",
        "({a, b} = c)",
        "[a.b, c[d]] = e",
        "(a) = 1",
        "[(a)] = 1",
    ]);
    bad(
        "invalid-target",
        &[
            "({a}) = 1",
            "([a]) = 1",
            "[a + b] = c",
            "({a: 1} = b)",
            "({...{a}} = b)",
        ],
    );
    bad("invalid-target", &["[a] += 1", "({a}) += 1"]);
    bad(
        "rest-not-last",
        &[
            "[...a, b] = c",
            "[...a,] = c",
            "({...a, b} = c)",
            "({...a,} = c)",
        ],
    );
    bad("rest-default", &["[...a = 1] = b", "async (...a = 1) => b"]);
    // B.3.6: a call as a target is a runtime error in sloppy code.
    ok(&["f() = 1", "f() += 1", "f()++", "for (f() in a);"]);
    bad(
        "invalid-target",
        &["'use strict'; f() = 1", "f() &&= 1", "[f()] = 1"],
    );
    bad(
        "strict-binding",
        &[
            "'use strict'; eval = 1",
            "'use strict'; [arguments] = 1",
            "'use strict'; arguments++",
        ],
    );
}

#[test]
fn arrows_and_their_cover() {
    ok(&[
        "a => a",
        "(a) => a",
        "(a, b) => a",
        "() => {}",
        "(...a) => a",
        "(a, ...[b, c]) => a",
    ]);
    ok(&[
        "(a = 1, [b], {c}) => a",
        "({a = 1}) => a",
        "a => b => c",
        "(a) => (b) => c",
        "x = (a,) => a",
    ]);
    ok(&[
        "async a => a",
        "async (a) => a",
        "async () => {}",
        "async(a)",
        "async",
        "async\n(a)",
    ]);
    bad("arrow-params-only", &["()", "(...a)", "(a,)", "(a, ...b)"]);
    bad(
        "invalid-target",
        &[
            "((a)) => a",
            "(a.b) => a",
            "([a.b]) => a",
            "(a + b) => a",
            "(1) => a",
        ],
    );
    bad("duplicate-binding", &["(a, a) => a", "([a], {a}) => a"]);
    bad(
        "unexpected-token",
        &["a\n=> a", "(a)\n=> a", "async a\n=> a"],
    );
    bad("arrow-operand", &["!a => a", "a + (b) => c"]);
    bad(
        "unexpected-token",
        &["a => {} + 1", "a => {}()", "a => {} ? b : c"],
    );
    bad(
        "await-in-params",
        &[
            "async (await) => 1",
            "async (a = await) => 1",
            "async await => 1",
        ],
    );
    bad("yield-in-params", &["function* g() { (a = yield) => 1 }"]);
    bad("comma-after-rest", &["async (...a,) => 1"]);
    bad(
        "use-strict-with-complex-params",
        &["(a = 1) => { 'use strict' }"],
    );
}

#[test]
fn yield_and_await() {
    ok(&[
        "function* g() { yield; yield a; yield* a; yield\na; (yield) }",
        "function* g() { yield /re/ }",
    ]);
    ok(&[
        "var yield = 1; yield / 2",
        "function* g() { function f() { yield } }",
        "await; var await = 1",
    ]);
    ok(&[
        "async function f() { await a; await await a; for await (x of y); }",
        "async () => await a",
    ]);
    bad(
        "reserved-word",
        &[
            "function* g() { var yield }",
            "'use strict'; yield",
            "async function f() { var await }",
        ],
    );
    bad(
        "for-await-outside-async",
        &[
            "for await (x of y);",
            "function f() { for await (x of y); }",
        ],
    );
    bad("yield-in-params", &["function* g(a = yield) {}"]);
    bad("await-in-params", &["async function f(a = await 1) {}"]);
}

// 14 Statements and declarations.

#[test]
fn declarations() {
    ok(&[
        "var a",
        "var a = 1, b",
        "let a",
        "let [a] = b",
        "const a = 1",
        "let\na = 1",
        "var [a, , ...b] = c",
    ]);
    ok(&["let = 1", "let.a", "var let", "if (a) let\nb"]);
    // A statement starting `let [` is a declaration, never an expression.
    bad("unexpected-token", &["let[0] = 1"]);
    bad("const-without-init", &["const a", "for (const a;;);"]);
    bad("pattern-without-init", &["let [a]", "var {a}"]);
    bad("let-binding", &["let let = 1", "const [let] = 1"]);
    bad("duplicate-binding", &["let a, a", "const [a, a] = b"]);
    ok(&["var a, a"]);
    bad("unexpected-token", &["var if", "var class", "let [if] = a"]);
    bad("reserved-word", &["var \\u0069f", "v\\u0061r = 1"]);
}

#[test]
fn blocks_and_control_flow() {
    ok(&[
        "{}",
        ";",
        "if (a) b",
        "if (a) b; else c",
        "if (a) {} else if (b) {} else {}",
        "while (a) b",
    ]);
    ok(&[
        "do a; while (b)",
        "do a; while (b) c",
        "do {} while (b);",
        "debugger",
    ]);
    ok(&[
        "switch (a) { case 1: b; default: c; case 2: }",
        "switch (a) { case 1: let b; }",
    ]);
    bad("duplicate-default", &["switch (a) { default: default: }"]);
    ok(&[
        "try {} catch (e) {}",
        "try {} catch {}",
        "try {} finally {}",
        "try {} catch ([a]) {} finally {}",
    ]);
    bad("unexpected-token", &["try {} a", "if (a) else b"]);
    bad("duplicate-binding", &["try {} catch ([a, a]) {}"]);
    bad("newline-after-throw", &["throw\na"]);
    ok(&["throw a"]);
    bad("with-in-strict", &["'use strict'; with (a) b"]);
    ok(&["with (a) b"]);
    // An else-if chain is a loop, not recursion: ten thousand deep parses.
    let src = "if (a) b; else ".repeat(10_000) + "c;";
    ok(&[&src]);
}

#[test]
fn for_statements() {
    ok(&[
        "for (;;);",
        "for (a; b; c);",
        "for (var a = 0, b; ;);",
        "for (let a = 0; ;);",
        "for (a in b);",
    ]);
    ok(&[
        "for (var a in b);",
        "for (let a in b);",
        "for (const a of b);",
        "for ([a, b] of c);",
        "for ({a} in b);",
    ]);
    ok(&[
        "for (a.b of c);",
        "for (let of of a);",
        "for (let in a);",
        "for (let.a in b);",
        "for (async of => {};;);",
    ]);
    ok(&[
        "async function f() { for await (async of a); }",
        "for ((a) in b);",
        "for (a in b, c);",
    ]);
    bad(
        "for-of-ambiguous",
        &["for (async of a);", "for (let.a of b);"],
    );
    bad("unexpected-token", &["for (let of a);"]);
    bad(
        "for-in-of-declarations",
        &["for (var a, b of c);", "for (let a, b in c);"],
    );
    bad(
        "for-in-of-initializer",
        &[
            "for (let a = 1 of b);",
            "for (var a = 1 of b);",
            "for (a = 0 in b);",
        ],
    );
    // B.3.5: one var with an initializer, in sloppy code.
    ok(&["for (var a = 1 in b);"]);
    bad(
        "for-in-of-initializer",
        &[
            "'use strict'; for (var a = 1 in b);",
            "for (var [a] = 1 in b);",
        ],
    );
    bad("unexpected-token", &["for (a of b, c);"]);
    bad(
        "invalid-target",
        &["for (a + b in c);", "for ((a, b) of c);"],
    );
    bad("shorthand-init", &["for ({a = 1};;);"]);
    ok(&["for ({a = 1} of b);"]);
}

#[test]
fn labels_break_continue_return() {
    ok(&[
        "a: b",
        "a: while (1) break a",
        "a: while (1) continue a",
        "a: b: while (1) continue a",
        "a: { break a }",
    ]);
    ok(&[
        "while (1) { break; continue }",
        "switch (a) { case 1: break }",
        "a: { b: { break a } }",
    ]);
    bad(
        "undefined-label",
        &[
            "break a",
            "while (1) continue a",
            "a: { function f() { break a } }",
        ],
    );
    bad(
        "continue-not-loop",
        &["a: { continue a }", "a: { while (1) continue a }"],
    );
    bad(
        "jump-outside-loop",
        &[
            "break",
            "continue",
            "switch (a) { case 1: continue }",
            "while (1) { function f() { break } }",
        ],
    );
    bad("duplicate-label", &["a: a: b", "a: { a: b }"]);
    ok(&["a: b; a: c", "await: 1", "yield: 1"]);
    bad("return-outside-function", &["return", "{ return 1 }"]);
    ok(&[
        "function f() { return }",
        "function f() { return\n1 }",
        "() => { return 1 }",
    ]);
}

#[test]
fn annex_b_function_declarations() {
    ok(&[
        "if (a) function f() {}",
        "if (a) function f() {} else function g() {}",
        "a: function f() {}",
    ]);
    bad(
        "declaration-not-allowed",
        &[
            "'use strict'; if (a) function f() {}",
            "'use strict'; a: function f() {}",
        ],
    );
    bad(
        "declaration-not-allowed",
        &[
            "if (a) function* g() {}",
            "a: function* g() {}",
            "while (a) function f() {}",
        ],
    );
    bad(
        "declaration-not-allowed",
        &[
            "while (a) b: function f() {}",
            "if (a) b: function f() {}",
            "if (a) class A {}",
        ],
    );
    bad(
        "declaration-not-allowed",
        &[
            "if (a) const b = 1",
            "while (a) let [b] = c",
            "if (a) async function f() {}",
        ],
    );
}

// 12.10 Automatic semicolon insertion.

#[test]
fn automatic_semicolons() {
    ok(&[
        "a\nb",
        "a\n;b",
        "{ a } b",
        "a; b",
        "var a\nlet b\nconst c = 1",
    ]);
    ok(&["a = b\n(c)", "a = b\n[c]", "a\n/b/g"]);
    bad("unexpected-token", &["a b", "{ a b }", "if (a) b c"]);
    // `return` and its argument, `break` and its label, are restricted.
    let ast = parse_script("function f() { return\n1 }").unwrap();
    let json = ast.to_estree("function f() { return\n1 }");
    assert!(json.contains(r#""type":"ReturnStatement","start":15,"end":21,"argument":null"#));
    // No semicolon is inserted in a `for` head.
    bad("unexpected-token", &["for (a\nb\n) c"]);
}

// 14.1 and 11.2 Directives and strict mode.

#[test]
fn directives_and_strict_mode() {
    ok(&["'use strict'; var a = 1", "\"use strict\"\nvar b"]);
    bad(
        "reserved-word",
        &[
            "'use strict'; var let",
            "'use strict'; var static",
            "'use strict'; implements",
        ],
    );
    bad(
        "legacy-octal-in-strict",
        &[
            "'use strict'; 017",
            "'use strict'; '\\01'",
            "'\\01'; 'use strict'",
        ],
    );
    bad(
        "legacy-octal-in-strict",
        &["function f() { 'use strict'; 08 }"],
    );
    // Escaped, it's not a directive.
    ok(&[
        "'use\\x20strict'; with (a) b",
        "('use strict'); with (a) b",
        "'use strict' + 1; with (a) b",
    ]);
    bad(
        "strict-binding",
        &[
            "function eval() { 'use strict' }",
            "function f(arguments) { 'use strict' }",
        ],
    );
    bad(
        "strict-binding",
        &[
            "'use strict'; function f(eval) {}",
            "'use strict'; var arguments",
        ],
    );
    bad(
        "duplicate-binding",
        &[
            "function f(a, a) { 'use strict' }",
            "'use strict'; function f(a, a) {}",
        ],
    );
    bad(
        "duplicate-binding",
        &["function f(a, [a]) {}", "({ m(a, a) {} })"],
    );
    ok(&["function f(a, a) {}", "function f(eval) {}"]);
    bad(
        "use-strict-with-complex-params",
        &[
            "function f(a = 1) { 'use strict' }",
            "function f({a}) { 'use strict' }",
        ],
    );
}

// 15 Functions and classes.

#[test]
fn functions() {
    ok(&[
        "function f() {}",
        "function* g() {}",
        "async function f() {}",
        "async function* g() {}",
    ]);
    ok(&[
        "function f(a, b = 1, [c], {d}, ...e) {}",
        "function f(a,) {}",
        "(function () {})",
        "(function* () {})",
    ]);
    bad(
        "unexpected-token",
        &[
            "function () {}",
            "function f(...a,) {}",
            "function f(...a = 1) {}",
            "function f(,) {}",
        ],
    );
    bad(
        "reserved-word",
        &[
            "(function* yield() {})",
            "(async function await() {})",
            "function* g(yield) {}",
        ],
    );
    ok(&[
        "function* yield() {}",
        "function* g() { (function yield() {}) }",
    ]);
    // A declaration's name is bound outside it, where `yield` is reserved.
    bad("reserved-word", &["function* g() { function yield() {} }"]);
    // `async` and `function` on two lines are two statements.
    ok(&["async\nfunction f() {}"]);
}

#[test]
fn classes() {
    ok(&[
        "class A {}",
        "class A extends B {}",
        "(class {})",
        "(class A extends (b, c) {})",
        "class A extends f() {}",
    ]);
    ok(&["class A { a; b = 1; static c; 'd' = 2; [e] = 3; 4 = 5; #f; static #g = 1 }"]);
    ok(&[
        "class A { m() {} static m() {} *g() {} async a() {} async *b() {} get x() {} set x(v) {} }",
    ]);
    ok(&[
        "class A { #a() {} get #b() {} set #b(v) {} static #c() {} static { this.d = 1 } static {} }",
    ]);
    ok(&[
        "class A { static; get; set; async; static = 1 }",
        "class A { get\n*a() {} }",
        "class A { async\nm() {} }",
    ]);
    ok(&["class A { constructor() {} ['constructor']() {} static constructor() {} }"]);
    bad(
        "duplicate-constructor",
        &[
            "class A { constructor() {} constructor() {} }",
            "class A { constructor() {} 'constructor'() {} }",
        ],
    );
    bad(
        "special-constructor",
        &[
            "class A { get constructor() {} }",
            "class A { *constructor() {} }",
            "class A { async constructor() {} }",
        ],
    );
    bad(
        "bad-field-name",
        &[
            "class A { constructor = 1 }",
            "class A { static prototype }",
            "class A { 'constructor' }",
        ],
    );
    bad("static-prototype", &["class A { static prototype() {} }"]);
    ok(&["class A { prototype() {} static ['prototype']() {} }"]);
    bad("private-constructor", &["class A { #constructor }"]);
    bad(
        "duplicate-private-name",
        &[
            "class A { #a; #a }",
            "class A { #a; #a() {} }",
            "class A { get #a() {} static set #a(v) {} }",
        ],
    );
    ok(&[
        "class A { get #a() {} set #a(v) {} }",
        "class A { m() { this.#a } #a }",
        "class A { #a; m() { class B { n() { this.#a } } } }",
    ]);
    bad(
        "undeclared-private-name",
        &["class A { m() { this.#a } }", "this.#a"],
    );
    bad(
        "arguments-in-class-init",
        &[
            "class A { a = arguments }",
            "class A { static { arguments } }",
            "class A { a = () => arguments }",
        ],
    );
    ok(&[
        "class A { a = function () { arguments } }",
        "class A { static { arguments: ; } }",
    ]);
    bad(
        "reserved-word",
        &[
            "class let {}",
            "class yield {}",
            "class A { m() { var static } }",
        ],
    );
    bad("arrow-operand", &["class A extends () => {} {}"]);
    bad(
        "reserved-word",
        &[
            "class A { static { var await } }",
            "class A { static { await } }",
        ],
    );
    ok(&[
        "class A { static { () => { var await } } }",
        "class A { static { (function await() {}) } }",
    ]);
    bad(
        "super-private",
        &["class A extends B { #a; m() { super.#a } }"],
    );
}

// Comments, warnings and the tree.

#[test]
fn comments_are_kept_beside_the_tree() {
    let src = "#!/usr/bin/env node\n// one\na /* two */ + b\n<!-- three\n--> four\n";
    let ast = parse_script(src).unwrap();
    let kinds: Vec<_> = ast.comments().iter().map(|c| c.kind).collect();
    assert_eq!(
        kinds,
        [
            CommentKind::Hashbang,
            CommentKind::Line,
            CommentKind::Block,
            CommentKind::HtmlOpen,
            CommentKind::HtmlClose
        ]
    );
    assert_eq!(ast.comments()[2].span.text(src), "/* two */");
}

#[test]
fn legacy_forms_warn_in_sloppy_code() {
    let ast = parse_script("a = 017 + '\\01'").unwrap();
    let codes: Vec<_> = ast
        .warnings()
        .iter()
        .map(|d| (d.code, d.severity))
        .collect();
    assert_eq!(
        codes,
        [
            ("legacy-octal-literal", Severity::Warning),
            ("legacy-octal-escape", Severity::Warning)
        ]
    );
}

#[test]
fn every_node_has_a_span_inside_its_parent() {
    let src = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/estree/expressions.js"
    ))
    .unwrap();
    let ast = parse_script(&src).unwrap();
    let root = ast.root();
    assert!(matches!(ast.node(root), Node::Program { .. }));
    assert_eq!(ast.span(root).end as usize, src.len());
    for id in ast.ids() {
        let span = ast.span(id);
        assert!(
            span.start <= span.end && span.end as usize <= src.len(),
            "{id:?}"
        );
    }
}

#[test]
fn errors_point_at_the_problem() {
    let err = parse("let x = 1;\nlet y = ;", &Options::default()).unwrap_err();
    assert_eq!(
        (err.code, err.span.start, err.span.end),
        ("unexpected-token", 19, 20)
    );
    let err = parse_script("a = 'unterminated").unwrap_err();
    assert_eq!(err.code, "unterminated-string");
}

// 16.2 Modules.

fn module(src: &str) -> Result<parsec_js::Ast, parsec_js::Diagnostic> {
    let options = Options {
        source_type: parsec_js::SourceType::Module,
        ..Options::default()
    };
    parse(src, &options)
}

fn ok_module(sources: &[&str]) {
    for src in sources {
        if let Err(d) = module(src) {
            panic!("{src:?} didn't parse as a module: {d}");
        }
    }
}

fn bad_module(code: &str, sources: &[&str]) {
    for src in sources {
        match module(src) {
            Ok(_) => panic!("{src:?} parsed as a module, but should fail with {code}"),
            Err(d) => assert_eq!(d.code, code, "{src:?}: {d}"),
        }
    }
}

#[test]
fn imports() {
    ok_module(&[
        "import 'a'",
        "import a from 'a'",
        "import * as a from 'a'",
        "import {a, b as c, 'd' as e, default as f, if as g} from 'a'",
        "import a, {b} from 'a'",
        "import a, * as b from 'a'",
        "import {} from 'a'",
        "import {a,} from 'a'",
        "import a from 'a' with {type: 'json'}",
        "import 'a' with {'type': 'json', b: 'c',}",
        "import a from 'a'\nwith {type: 'json'}",
    ]);
    bad_module(
        "unexpected-token",
        &[
            "import a, b from 'a'",
            "import {a} 'a'",
            "import * from 'a'",
            "import a from b",
        ],
    );
    bad_module(
        "import-needs-as",
        &["import {'a'} from 'a'", "import {if} from 'a'"],
    );
    bad_module(
        "reserved-word",
        &[
            "import {await} from 'a'",
            "import await from 'a'",
            "import {a as yield} from 'a'",
        ],
    );
    bad_module(
        "duplicate-attribute",
        &["import a from 'a' with {type: 'json', 'type': 'x'}"],
    );
    bad_module(
        "unexpected-token",
        &[
            "import a from 'a' with {type: json}",
            "import a from 'a' assert {type: 'json'}",
        ],
    );
    bad_module("lone-surrogate-name", &["import {'\\uD800' as a} from 'a'"]);
    // `import(...)` and `import.meta` are expressions, anywhere.
    ok_module(&[
        "import('a')",
        "import.meta.url",
        "function f() { import('a'); import.meta }",
    ]);
}

#[test]
fn exports() {
    ok_module(&[
        "export var a = 1, b",
        "export let a",
        "export const a = 1",
        "export function a() {}",
        "export async function a() {}",
        "export function* a() {}",
        "export class A {}",
        "var a, b; export {a, b as c, a as 'd e', b as default}",
        "export {a as b, 'c' as d, if, default} from 'a'",
        "export {} from 'a'",
        "export * from 'a'",
        "export * as a from 'a'",
        "export * as 'a b' from 'a' with {type: 'json'}",
        "export default function () {}",
        "export default function* a() {}",
        "export default async function () {}",
        "export default class {}",
        "export default 1 + 2",
        "export default async () => 1",
        "export default (function () {})",
    ]);
    bad_module(
        "duplicate-export",
        &[
            "export var a; export {a}",
            "export default 1; export default 2",
            "var a; export {a as default}; export default 1",
            "export * as a from 'a'; export * as a from 'b'",
            "export let [a] = b; export {c as a}",
        ],
    );
    bad_module(
        "export-string-local",
        &["export {'a'}", "export {'a' as b}"],
    );
    bad_module(
        "unexpected-token",
        &[
            "export {if}",
            "export a",
            "export async () => 1",
            "export * as a;",
        ],
    );
    bad_module("reserved-word", &["export {await}"]);
    // Declarations, not expressions, after `export default`.
    let ast = module("export default function () {}").unwrap();
    let json = ast.to_estree("export default function () {}");
    assert!(
        json.contains(r#""declaration":{"type":"FunctionDeclaration""#),
        "{json}"
    );
}

#[test]
fn modules_are_strict_and_top_level() {
    bad_module("with-in-strict", &["with (a) b"]);
    bad_module("legacy-octal-escape", &["'\\01'"]);
    bad_module(
        "reserved-word",
        &["var await", "function f() { var await }", "var let"],
    );
    bad_module("unexpected-token", &["<!-- not a comment in a module"]);
    ok_module(&["await a", "for await (a of b);", "await\n/x/g"]);
    bad_module(
        "module-item-not-allowed",
        &[
            "{ import 'a' }",
            "function f() { export var a }",
            "if (a) { export {} }",
        ],
    );
    bad(
        "module-item-not-allowed",
        &["import 'a'", "export var a", "import a from 'a'"],
    );
    ok(&["import('a')", "a = import('b')"]);
}

/// How long `f` takes, the best of three.
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

/// Ten times the input in at most about ten times the time, with room for
/// noise: a quadratic check would take a hundred.
fn assert_linear(what: &str, make: impl Fn(usize) -> String) {
    let (small, big) = (make(20_000), make(200_000));
    let parse = |src: &str| {
        module(src).unwrap();
    };
    let (ts, tb) = (
        best_of_three(|| parse(&small)),
        best_of_three(|| parse(&big)),
    );
    let ratio = tb.as_secs_f64() / ts.as_secs_f64().max(1e-6);
    assert!(
        ratio < 35.0,
        "{what}: 10x the input took {ratio:.1}x the time ({ts:?}, {tb:?})"
    );
}

#[test]
fn many_import_attributes_stay_linear() {
    let attributes = |n: usize| {
        (0..n)
            .map(|i| format!("k{i}:'v'"))
            .collect::<Vec<_>>()
            .join(",")
    };
    assert_linear("import with {…}", |n| {
        format!("import 'a' with {{{}}};", attributes(n))
    });
    assert_linear("export * from with {…}", |n| {
        format!("export * from 'a' with {{{}}};", attributes(n))
    });
}
