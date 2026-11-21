// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! JavaScript for Parsec: a parser for ECMAScript 2025 scripts, with
//! Annex B's additions, and modules, and the lexer under it.
//!
//! It's a parser, not an engine: nothing here runs code. [`parse`] gives
//! an [`Ast`], every node in one arena with a [`Span`] beside it and the
//! comments kept on the side, or the first syntax error as a
//! [`Diagnostic`]. [`Ast::to_estree`] writes the tree as ESTree JSON in
//! the shape acorn gives, so tools written for acorn or espree can read
//! it.
//!
//! ```
//! use parsec_js::{Node, parse_script};
//!
//! let src = "const total = items.reduce((sum, x) => sum + x, 0);";
//! let ast = parse_script(src).unwrap();
//! let Node::Program { body, .. } = *ast.node(ast.root()) else { unreachable!() };
//! let first = ast.get(body)[0];
//! assert_eq!(ast.span(first).text(src), src);
//! assert!(ast.to_estree(src).contains(r#""type":"ArrowFunctionExpression""#));
//! ```
//!
//! [`scope`] works out which declaration each name refers to: the
//! scopes, their bindings, every reference resolved or left free, and
//! the names declared twice where the spec forbids it.
//!
//! [`regexp`] checks a regular expression's pattern by its own grammar,
//! the error a `RegExp` built from it would throw.
//!
//! [`data`] reads the state pages keep in scripts, `window.__NUXT__ = …`
//! and its kin, as JSON, without running anything.
//!
//! Recursion is bounded by [`Limits::max_depth`] ([`DEFAULT_MAX_DEPTH`]
//! without one) and work by [`Limits::max_steps`], so hostile input gets
//! an error, never a stack overflow.
//!
//! The lexer works alone too. Every token carries its [`Span`] in the
//! source, and nothing is allocated per token; what a token says (a
//! string's value, a number's) is in its text, and [`text`] reads it.
//! `/` means division after an expression and starts a regular expression
//! anywhere else, and `}` either closes a block or carries on a template.
//! The lexer doesn't guess which: [`Lexer::next_token`] takes a [`Goal`],
//! as the spec's InputElementDiv and InputElementRegExp goals do, and the
//! parser, which knows, passes it.
//!
//! ```
//! use parsec_js::{Goal, Kind, Lexer, SourceType};
//!
//! let src = "#!/usr/bin/env node\nlet big = 1_000n; // a BigInt\n";
//! let mut lexer = Lexer::new(src, SourceType::Script);
//! let mut words = Vec::new();
//! loop {
//!     let token = lexer.next_token(Goal::RegExp);
//!     if token.kind == Kind::Eof {
//!         break;
//!     }
//!     if !token.is_trivia() {
//!         words.push(token.text(src));
//!     }
//! }
//! assert_eq!(words, ["let", "big", "=", "1_000n", ";"]);
//! assert!(lexer.diagnostics().is_empty());
//! ```

pub mod ast;
pub mod data;
mod estree;
mod lexer;
mod parser;
pub mod regexp;
pub mod scope;
pub mod text;
mod token;
mod unicode;

pub use ast::{Ast, Comment, CommentKind, List, Node, NodeId};
pub use estree::EstreeOptions;
pub use lexer::{Goal, Lexer, SourceType};
pub use parsec_base::{Diagnostic, Limits, LineIndex, Severity, Span};
pub use parser::{DEFAULT_MAX_DEPTH, Options, parse, parse_script};
pub use token::{Flags, Keyword, Kind, Punct, Token};
