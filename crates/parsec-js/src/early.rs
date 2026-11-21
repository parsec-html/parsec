// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The spec's early errors: what makes a program a SyntaxError though
//! its grammar is fine.
//!
//! Most are the parser's, since it knows enough where it stands to
//! check them as it goes: [`parse`](crate::parse) fails with the first.
//! The rest need the whole program or a grammar of their own, and are
//! here, opt-in, so a parse costs nothing for them: [`early_errors`]
//! gives every one in a tree the parser accepted.
//!
//! ```
//! use parsec_js::{early_errors, parse_script};
//!
//! let src = "let total = 0;\nvar total;\nconst digits = /[9-0]/;";
//! let ast = parse_script(src).unwrap();
//! let codes: Vec<_> = early_errors(&ast, src).iter().map(|d| d.code).collect();
//! assert_eq!(codes, ["lexical-var-conflict", "regexp-range-order"]);
//! ```
//!
//! Where each rule is checked, and its code:
//!
//! | Rule | Code | Where |
//! |---|---|---|
//! | A lexical name declared twice in a scope, or as a parameter too | `duplicate-declaration` | here, from [`Scopes`] |
//! | A `var` and a lexical name of one name in a scope | `lexical-var-conflict` | here, from [`Scopes`] |
//! | A module exports a name it doesn't declare | `export-undeclared` | here |
//! | A regular expression's pattern doesn't compile | `regexp-…`, see [`regexp`] | here |
//! | A parameter given twice in strict code, an arrow, a method, or a list with defaults or patterns | `duplicate-binding` | the parser |
//! | `break` or `continue` with nothing to leave or go on with | `jump-outside-loop` | the parser |
//! | `continue` to a label that isn't on a loop | `continue-not-loop` | the parser |
//! | A label that isn't around the jump, or is used twice | `undefined-label`, `duplicate-label` | the parser |
//! | An assignment or update of something that can't be assigned | `invalid-target` | the parser |
//! | `with` in strict code | `with-in-strict` | the parser |
//! | A legacy octal literal or escape in strict code | `legacy-octal-in-strict` | the parser |
//! | `eval` or `arguments` bound or assigned in strict code | `strict-binding` | the parser |
//! | A word reserved where it's used | `reserved-word` | the parser |
//! | `delete` of a plain name in strict code | `delete-identifier` | the parser |
//! | "use strict" in a function with defaults, patterns or rest | `use-strict-with-complex-params` | the parser |
//! | `super` outside a method, `super()` outside a derived constructor | `super-outside-method` | the parser |
//! | `new.target` outside a function | `new-target-outside-function` | the parser |
//! | `yield` or `await` in parameters | `yield-in-params`, `await-in-params` | the parser |
//! | `return` outside a function | `return-outside-function` | the parser |
//! | A name exported twice | `duplicate-export` | the parser |
//! | `__proto__` twice in an object literal, not a pattern | `duplicate-proto` | the parser |
//! | A private name not declared in a class around it, or declared twice | `undeclared-private-name`, `duplicate-private-name` | the parser |
//! | `arguments` in a class field or static block | `arguments-in-class-init` | the parser |

use parsec_base::{Diagnostic, Span};

use crate::ast::{Ast, LiteralKind, Node};
use crate::regexp;
use crate::scope::Scopes;

/// Every early error in `ast`, parsed from `src`, that the parser leaves
/// to a later pass, in source order. Empty when there's none.
pub fn early_errors(ast: &Ast, src: &str) -> Vec<Diagnostic> {
    early_errors_in(ast, src, &Scopes::analyze(ast, src))
}

/// [`early_errors`] with the [`Scopes`] already analyzed.
pub fn early_errors_in(ast: &Ast, src: &str, scopes: &Scopes) -> Vec<Diagnostic> {
    let mut out = scopes.diagnostics().to_vec();
    let Node::Program { body, .. } = *ast.node(ast.root()) else {
        unreachable!("the root is a program")
    };
    // ExportedBindings have to be declared at the top (16.2.1.1). An
    // export's name is a reference in the module's scope, so a free one
    // is declared nowhere there.
    for &item in ast.get(body) {
        if let Node::ExportNamed {
            declaration: None,
            source: None,
            specifiers,
            ..
        } = *ast.node(item)
        {
            for &spec in ast.get(specifiers) {
                if let Node::ExportSpecifier { local, .. } = *ast.node(spec)
                    && scopes.reference(local).is_some_and(|r| r.binding.is_none())
                {
                    let name = scopes.reference(local).map(|r| r.name(ast, src));
                    out.push(Diagnostic::error(
                        "export-undeclared",
                        ast.span(local),
                        format!(
                            "`{}` is exported, but this module doesn't declare it",
                            name.unwrap_or_default()
                        ),
                    ));
                }
            }
        }
    }
    for id in ast.ids() {
        if let Node::Literal(LiteralKind::RegExp) = *ast.node(id) {
            let span = ast.span(id);
            let text = span.text(src);
            let slash = text.rfind('/').unwrap_or(0) as u32;
            let pattern = Span::new(span.start + 1, span.start + slash);
            let flags = Span::new(span.start + slash + 1, span.end);
            if let Err(mut d) = regexp::validate(pattern.text(src), flags.text(src)) {
                let base = if d.code == "regexp-flags" {
                    flags.start
                } else {
                    pattern.start
                };
                d.span = Span::new(base + d.span.start, base + d.span.end);
                out.push(d);
            }
        }
    }
    out.sort_by_key(|d| (d.span.start, d.span.end));
    out
}
