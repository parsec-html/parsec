// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Every error and warning this crate reports, by its code.
//!
//! A [`Diagnostic`](crate::Diagnostic)'s `code` is stable: a code isn't
//! renamed or reused for something else, so you can match on it, count
//! it or turn it off. The message beside it is for people and may be
//! reworded. Where a second place matters, like the first of two
//! declarations or the bracket left open, a label points at it.
//!
//! [`parse`](crate::parse) stops at the first error;
//! [`parse_recovering`](crate::parse_recovering) goes on and reports
//! each; [`early_errors`](crate::early_errors) gives the ones checked on
//! the finished tree. [`Diagnostic::render`](crate::Diagnostic::render)
//! prints any of them as a code frame:
//!
//! ```
//! use parsec_js::{LineIndex, Options, RenderOptions, parse_recovering};
//!
//! let src = "let total = 0;\nouter: outer: total++;\n";
//! let errors = parse_recovering(src, &Options::default()).unwrap_err();
//! let lines = LineIndex::ecmascript(src);
//! let frame = errors[0].render(src, &lines, &RenderOptions::default());
//! assert_eq!(frame, "\
//! error[duplicate-label]: the label `outer` is already in use here
//!  --> 2:8
//!   |
//! 2 | outer: outer: total++;
//!   | -----  ^^^^^
//!   | |
//!   | the label is defined here
//! ");
//! ```
//!
//! The section in each row is ECMAScript 2025's, where the rule is.
//!
//! # The lexer's
//!
//! | Code | What's wrong | Spec |
//! |---|---|---|
//! | `input-too-large` | the source is longer than [`Limits::max_input`](crate::Limits::max_input), or 4 GiB | |
//! | `unexpected-character` | a character that starts no token, like `@` or `ⸯ` | [12.6] |
//! | `unterminated-comment` | a `/*` with no `*/` | [12.4] |
//! | `invalid-escape` | a `\` in a name that doesn't start `\u`, or a `\x` or `\u` escape in a string without its hex digits | [12.7], [12.9.4] |
//! | `invalid-identifier-escape` | a `\u` escape in a name for a character a name can't hold there | [12.7.1.1] |
//! | `missing-digits` | `0x`, `0o` or `0b` with no digits after it | [12.9.3] |
//! | `missing-exponent` | an `e` with no digits after it, like `1e` | [12.9.3] |
//! | `identifier-after-number` | a letter or digit straight after a number, like `3in` | [12.9.3] |
//! | `invalid-numeric-separator` | a `_` in a number that isn't between two digits | [12.9.3] |
//! | `legacy-octal-literal` | `017` in strict code; a warning in sloppy code | [12.9.3.1] |
//! | `leading-zero-literal` | `08` in strict code; a warning in sloppy code | [12.9.3.1] |
//! | `unterminated-string` | a string with no closing quote on its line | [12.9.4] |
//! | `legacy-octal-escape` | `"\01"` or `"\8"` in strict code; a warning in sloppy code | [12.9.4.1] |
//! | `unterminated-template` | a template with no closing `` ` `` | [12.9.6] |
//! | `unterminated-regexp` | a regular expression with no closing `/` on its line | [12.9.5] |
//! | `invalid-regexp-flags` | a regular expression's flags written with an escape | [12.9.5] |
//!
//! # The parser's
//!
//! Syntax errors, and the early errors the parser checks as it goes.
//!
//! | Code | What's wrong | Spec |
//! |---|---|---|
//! | `unexpected-token` | a token the grammar doesn't allow where it is | |
//! | `unexpected-end` | the source ends in the middle of something | |
//! | `too-deep` | the tree nests deeper than [`Limits::max_depth`](crate::Limits::max_depth) | |
//! | `step-budget` | the parse read more tokens than [`Limits::max_steps`](crate::Limits::max_steps) | |
//! | `reserved-word` | a reserved word, or one strict code, a generator, an async function or a module reserves, as a name | [13.1.1] |
//! | `strict-binding` | `eval` or `arguments` declared or assigned in strict code | [13.1.1] |
//! | `legacy-octal-in-strict` | a legacy octal literal or escape in code a later `"use strict"` makes strict | [12.9.3.1], [12.9.4.1] |
//! | `arguments-in-class-init` | `arguments` in a class field's initializer or a static block | [15.7.1] |
//! | `await-in-class-init` | `await` in a class field's initializer or a static block | [15.7.1] |
//! | `shorthand-init` | `{a = 1}` as an object literal, not a pattern | [13.2.5.1] |
//! | `duplicate-proto` | `__proto__` set twice in an object literal | [13.2.5.1] |
//! | `invalid-template-escape` | a bad escape in a template that isn't tagged | [13.2.8.1] |
//! | `tagged-template-in-chain` | an optional chain as a template's tag, like ``a?.b`c` `` | [13.3.1.1] |
//! | `optional-chain-in-new` | `new a?.b()` | [13.3.9] |
//! | `super-outside-method` | `super.x` outside a method, or `super()` outside a derived class's constructor | [15.2.1], [15.7.1] |
//! | `super-private` | `super.#x` | [13.3.7] |
//! | `new-target-outside-function` | `new.target` outside a function | [16.1.1], [16.2.1.1] |
//! | `import-meta-outside-module` | `import.meta` in a script | [13.3.1.1] |
//! | `new-import` | `new import(...)` | [13.3.10] |
//! | `invalid-target` | an assignment, update or binding of something that can't be assigned, like `1 = a` or `a() ++` | [13.4.1], [13.15.1] |
//! | `delete-identifier` | `delete x` of a plain name in strict code | [13.5.1.1] |
//! | `delete-private` | `delete this.#x` | [13.5.1.1] |
//! | `unary-before-exponent` | `-a ** b` without parentheses | [13.6] |
//! | `mixed-coalesce` | `??` beside `\|\|` or `&&` without parentheses | [13.13] |
//! | `lone-private-name` | `#x` alone, anywhere but before `in` | [13.10] |
//! | `arrow-operand` | an arrow function as an operand without parentheses | [15.3] |
//! | `arrow-params-only` | `()`, `(...a)` or a trailing comma kept by an expression that isn't an arrow's parameters | [13.2.9.1] |
//! | `rest-not-last` | a rest element or parameter that isn't last | [14.3.3], [15.1.1] |
//! | `rest-default` | a rest element with a default, like `[...a = 1]` | [14.3.3] |
//! | `comma-after-rest` | a comma after a rest parameter | [15.1.1] |
//! | `yield-in-params` | `yield` in a generator's parameters | [15.5.1] |
//! | `await-in-params` | `await` in an async function's parameters | [15.8.1], [15.9.1] |
//! | `duplicate-binding` | a parameter given twice where that's not allowed, or a name a `let` or `const` declares twice in one declaration | [14.3.1.1], [15.1.1], [15.2.1] |
//! | `use-strict-with-complex-params` | `"use strict"` in a function with defaults, patterns or a rest parameter | [15.2.1] |
//! | `accessor-params` | a getter with parameters, or a setter without exactly one | [15.4.1] |
//! | `special-constructor` | a constructor that's a getter, setter, generator or async | [15.7.1] |
//! | `duplicate-constructor` | a class with two constructors | [15.7.1] |
//! | `static-prototype` | a static method or accessor named `prototype` | [15.7.1] |
//! | `bad-field-name` | a field named `constructor`, or a static one `prototype` | [15.7.1] |
//! | `private-constructor` | `#constructor` | [15.7.1] |
//! | `private-outside-class` | a private name declared outside a class | [15.7.1] |
//! | `duplicate-private-name` | a private name declared twice in a class, other than a getter and a setter | [15.7.1] |
//! | `undeclared-private-name` | a private name no class around it declares | [15.7.1], [16.1.1] |
//! | `declaration-not-allowed` | a declaration as the body of an `if`, loop, label or `with` | [14.6.1], [B.3.1] |
//! | `const-without-init` | a `const` without an initializer | [14.3.1.1] |
//! | `pattern-without-init` | a destructuring declaration without an initializer | [14.3.1], [14.3.2] |
//! | `let-binding` | `let` as a name a `let` or `const` declares | [14.3.1.1] |
//! | `for-await-outside-async` | `for await` outside an async function or a module | [14.7.5] |
//! | `for-in-of-declarations` | a for-in or for-of head that declares more than one binding | [14.7.5] |
//! | `for-in-of-initializer` | an initializer in a for-in or for-of head, except Annex B's `for (var a = 1 in b)` | [14.7.5], [B.3.5] |
//! | `for-of-ambiguous` | a for-of head that starts with `let`, or is `async of` | [14.7.5] |
//! | `jump-outside-loop` | `break` outside a loop or a switch, or `continue` outside a loop | [14.8.1], [14.9.1] |
//! | `continue-not-loop` | `continue a` where `a` isn't on a loop | [15.2.1], [16.1.1] |
//! | `undefined-label` | a `break` or `continue` to a label that isn't around it | [15.2.1], [16.1.1] |
//! | `duplicate-label` | a label inside another of the same name | [15.2.1], [16.1.1] |
//! | `return-outside-function` | `return` outside a function | [14.10] |
//! | `with-in-strict` | `with` in strict code | [14.11.1] |
//! | `newline-after-throw` | a line break between `throw` and its expression | [14.14] |
//! | `duplicate-default` | a switch with two `default`s | [14.12] |
//! | `module-item-not-allowed` | `import` or `export` anywhere but a module's top level | [16.2] |
//! | `duplicate-export` | a name a module exports twice | [16.2.1.1] |
//! | `duplicate-attribute` | an import attribute given twice | [16.2.2.1] |
//! | `import-needs-as` | a string imported without `as` a name | [16.2.2] |
//! | `export-string-local` | a string as the local name in an export that isn't a re-export | [16.2.3.1] |
//! | `lone-surrogate-name` | an import or export name, as a string, with a lone surrogate | [16.2.1.1] |
//!
//! # From [`early_errors`](crate::early_errors)
//!
//! | Code | What's wrong | Spec |
//! |---|---|---|
//! | `duplicate-declaration` | a lexical name declared twice in a scope, or as a parameter too | [14.2.1], [15.2.1], [16.1.1] |
//! | `lexical-var-conflict` | a `var` and a lexical declaration of one name in a scope | [14.2.1], [16.1.1] |
//! | `export-undeclared` | a module exports a name it doesn't declare | [16.2.1.1] |
//! | `regexp-…` | a regular expression's pattern or flags that don't compile; the [`regexp`](crate::regexp) module has each | [13.2.7.1], [22.2.1.1] |
//!
//! # From [`Ast::reparse`](crate::Ast::reparse)
//!
//! Besides the new source's syntax errors:
//!
//! | Code | What's wrong | Spec |
//! |---|---|---|
//! | `edit-out-of-range` | the edit's range is past the end of the source, backwards, or splits a character | |
//!
//! [12.4]: https://tc39.es/ecma262/2025/#sec-comments
//! [12.6]: https://tc39.es/ecma262/2025/#sec-tokens
//! [12.7]: https://tc39.es/ecma262/2025/#sec-names-and-keywords
//! [12.7.1.1]: https://tc39.es/ecma262/2025/#sec-identifier-names-static-semantics-early-errors
//! [12.9.3]: https://tc39.es/ecma262/2025/#sec-literals-numeric-literals
//! [12.9.3.1]: https://tc39.es/ecma262/2025/#sec-numeric-literals-early-errors
//! [12.9.4]: https://tc39.es/ecma262/2025/#sec-literals-string-literals
//! [12.9.4.1]: https://tc39.es/ecma262/2025/#sec-string-literals-early-errors
//! [12.9.5]: https://tc39.es/ecma262/2025/#sec-literals-regular-expression-literals
//! [12.9.6]: https://tc39.es/ecma262/2025/#sec-template-literal-lexical-components
//! [12.10]: https://tc39.es/ecma262/2025/#sec-automatic-semicolon-insertion
//! [13.1.1]: https://tc39.es/ecma262/2025/#sec-identifiers-static-semantics-early-errors
//! [13.2.5.1]: https://tc39.es/ecma262/2025/#sec-object-initializer-static-semantics-early-errors
//! [13.2.7.1]: https://tc39.es/ecma262/2025/#sec-primary-expression-regular-expression-literals-static-semantics-early-errors
//! [13.2.8.1]: https://tc39.es/ecma262/2025/#sec-primary-expression-template-literals-static-semantics-early-errors
//! [13.2.9.1]: https://tc39.es/ecma262/2025/#sec-grouping-operator-static-semantics-early-errors
//! [13.3.1.1]: https://tc39.es/ecma262/2025/#sec-left-hand-side-expressions-static-semantics-early-errors
//! [13.3.9]: https://tc39.es/ecma262/2025/#sec-optional-chains
//! [13.3.7]: https://tc39.es/ecma262/2025/#sec-super-keyword
//! [13.3.10]: https://tc39.es/ecma262/2025/#sec-import-calls
//! [13.4.1]: https://tc39.es/ecma262/2025/#sec-update-expressions-static-semantics-early-errors
//! [13.5.1.1]: https://tc39.es/ecma262/2025/#sec-delete-operator-static-semantics-early-errors
//! [13.6]: https://tc39.es/ecma262/2025/#sec-exp-operator
//! [13.10]: https://tc39.es/ecma262/2025/#sec-relational-operators
//! [13.13]: https://tc39.es/ecma262/2025/#sec-binary-logical-operators
//! [13.15.1]: https://tc39.es/ecma262/2025/#sec-assignment-operators-static-semantics-early-errors
//! [14.2.1]: https://tc39.es/ecma262/2025/#sec-block-static-semantics-early-errors
//! [14.3.1.1]: https://tc39.es/ecma262/2025/#sec-let-and-const-declarations-static-semantics-early-errors
//! [14.3.1]: https://tc39.es/ecma262/2025/#sec-let-and-const-declarations
//! [14.3.2]: https://tc39.es/ecma262/2025/#sec-variable-statement
//! [14.3.3]: https://tc39.es/ecma262/2025/#sec-destructuring-binding-patterns
//! [14.6.1]: https://tc39.es/ecma262/2025/#sec-if-statement-static-semantics-early-errors
//! [14.7.5]: https://tc39.es/ecma262/2025/#sec-for-in-and-for-of-statements
//! [14.7.5.1]: https://tc39.es/ecma262/2025/#sec-for-in-and-for-of-statements-static-semantics-early-errors
//! [14.8.1]: https://tc39.es/ecma262/2025/#sec-continue-statement-static-semantics-early-errors
//! [14.9.1]: https://tc39.es/ecma262/2025/#sec-break-statement-static-semantics-early-errors
//! [14.10]: https://tc39.es/ecma262/2025/#sec-return-statement
//! [14.11.1]: https://tc39.es/ecma262/2025/#sec-with-statement-static-semantics-early-errors
//! [14.12]: https://tc39.es/ecma262/2025/#sec-switch-statement
//! [14.12.1]: https://tc39.es/ecma262/2025/#sec-switch-statement-static-semantics-early-errors
//! [14.13.1]: https://tc39.es/ecma262/2025/#sec-labelled-statements-static-semantics-early-errors
//! [14.14]: https://tc39.es/ecma262/2025/#sec-throw-statement
//! [15.1.1]: https://tc39.es/ecma262/2025/#sec-parameter-lists-static-semantics-early-errors
//! [15.2.1]: https://tc39.es/ecma262/2025/#sec-function-definitions-static-semantics-early-errors
//! [15.3]: https://tc39.es/ecma262/2025/#sec-arrow-function-definitions
//! [15.4.1]: https://tc39.es/ecma262/2025/#sec-method-definitions-static-semantics-early-errors
//! [15.5.1]: https://tc39.es/ecma262/2025/#sec-generator-function-definitions-static-semantics-early-errors
//! [15.7.1]: https://tc39.es/ecma262/2025/#sec-class-definitions-static-semantics-early-errors
//! [15.8.1]: https://tc39.es/ecma262/2025/#sec-async-function-definitions-static-semantics-early-errors
//! [15.9.1]: https://tc39.es/ecma262/2025/#sec-async-arrow-function-definitions-static-semantics-early-errors
//! [16.1.1]: https://tc39.es/ecma262/2025/#sec-scripts-static-semantics-early-errors
//! [16.2]: https://tc39.es/ecma262/2025/#sec-modules
//! [16.2.1.1]: https://tc39.es/ecma262/2025/#sec-module-semantics-static-semantics-early-errors
//! [16.2.2]: https://tc39.es/ecma262/2025/#sec-imports
//! [16.2.2.1]: https://tc39.es/ecma262/2025/#sec-imports-static-semantics-early-errors
//! [16.2.3.1]: https://tc39.es/ecma262/2025/#sec-exports-static-semantics-early-errors
//! [22.2.1.1]: https://tc39.es/ecma262/2025/#sec-patterns-static-semantics-early-errors
//! [B.3.1]: https://tc39.es/ecma262/2025/#sec-labelled-function-declarations
//! [B.3.5]: https://tc39.es/ecma262/2025/#sec-initializers-in-forin-statement-heads

#[cfg(test)]
mod tests {
    /// Every code this crate's sources give a diagnostic is in the tables
    /// above, and every code in the tables is one they give.
    #[test]
    fn every_code_is_documented() {
        let sources = [
            include_str!("lexer.rs"),
            include_str!("parser/mod.rs"),
            include_str!("parser/expr.rs"),
            include_str!("parser/function.rs"),
            include_str!("parser/module.rs"),
            include_str!("parser/pattern.rs"),
            include_str!("parser/stmt.rs"),
            include_str!("early.rs"),
            include_str!("scope.rs"),
            include_str!("reparse.rs"),
        ];
        let docs = include_str!("diagnostics.rs");
        let table: Vec<&str> = docs
            .lines()
            .filter_map(|l| l.strip_prefix("//! | `"))
            .filter_map(|l| l.split('`').next())
            .filter(|c| *c != "regexp-…")
            .collect();
        let mut used = std::collections::BTreeSet::new();
        for src in sources {
            for (i, _) in src.match_indices('"') {
                let rest = &src[i + 1..];
                let Some(end) = rest.find('"') else { continue };
                let word = &rest[..end];
                let is_code = word.contains('-')
                    && word.split('-').all(|p| {
                        !p.is_empty()
                            && p.bytes()
                                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
                    });
                if is_code && !word.starts_with("regexp-") {
                    used.insert(word);
                }
            }
        }
        // Codes from parsec-base the parser passes on.
        used.insert("input-too-large");
        for code in &used {
            assert!(table.contains(code), "`{code}` isn't in the table");
        }
        for code in &table {
            assert!(
                used.contains(code),
                "`{code}` is in the table, but nothing gives it"
            );
        }
    }
}
