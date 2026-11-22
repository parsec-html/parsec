// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! JavaScript for Parsec. For now, the lexer: ECMAScript 2025's lexical
//! grammar, with Annex B's additions for scripts.
//!
//! It's a parser's lexer, not an engine's: nothing here runs code. Every
//! token carries its [`Span`] in the source, and nothing is allocated per
//! token; what a token says (a string's value, a number's) is in its text.
//!
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

mod lexer;
mod token;
mod unicode;

pub use lexer::{Goal, Lexer, SourceType};
pub use parsec_base::{Diagnostic, Limits, LineIndex, Severity, Span};
pub use token::{Flags, Keyword, Kind, Punct, Token};
