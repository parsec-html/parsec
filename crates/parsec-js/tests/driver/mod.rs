// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Tokenizes a whole source without a parser, for the tests, test262 and
//! the fuzz target. A parser knows which goal each token needs; this
//! guesses it from the token before.
//!
//! shortcut: `/` after `]`, `}`, an identifier or a `)` that doesn't close
//! `if (...)`, `while (...)`, `for (...)` or `with (...)` is read as
//! division, so a regex starting a statement after a block's `}` lexes
//! wrongly; upgrade to the parser's goals once parsec-js has a parser
//! (M10).

#![allow(dead_code)]

use parsec_js::{Diagnostic, Goal, Keyword, Kind, Lexer, Punct, SourceType, Token};

/// Every token, trivia included, and the diagnostics.
pub fn tokenize(src: &str, source: SourceType) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer::new(src, source);
    let mut out = Vec::new();
    let mut regexp = true;
    let mut last: Option<Kind> = None;
    // For each `{` or template substitution open, whether it's a
    // substitution, whose `}` carries the template on.
    let mut braces: Vec<bool> = Vec::new();
    // For each `(` open, whether it's a control statement's, after which
    // an expression statement can start with a regex.
    let mut parens: Vec<bool> = Vec::new();
    loop {
        let tail = braces.last() == Some(&true);
        let goal = match (regexp, tail) {
            (false, false) => Goal::Div,
            (true, false) => Goal::RegExp,
            (false, true) => Goal::TemplateTail,
            (true, true) => Goal::RegExpOrTemplateTail,
        };
        let token = lexer.next_token(goal);
        if token.kind == Kind::Eof {
            break;
        }
        out.push(token);
        if token.is_trivia() {
            continue;
        }
        regexp = regexp_after(token.kind);
        match token.kind {
            Kind::Punct(Punct::LBrace) => braces.push(false),
            Kind::Punct(Punct::RBrace) | Kind::TemplateTail => {
                braces.pop();
            }
            Kind::TemplateHead => braces.push(true),
            Kind::Punct(Punct::LParen) => parens.push(matches!(
                last,
                Some(Kind::Keyword(
                    Keyword::If | Keyword::While | Keyword::For | Keyword::With
                ))
            )),
            Kind::Punct(Punct::RParen) => regexp = parens.pop().unwrap_or(false),
            _ => {}
        }
        last = Some(token.kind);
    }
    (out, lexer.take_diagnostics())
}

/// Whether a `/` after a token of this kind starts a regular expression.
fn regexp_after(kind: Kind) -> bool {
    match kind {
        Kind::Punct(p) => !matches!(
            p,
            Punct::RParen | Punct::RBracket | Punct::RBrace | Punct::PlusPlus | Punct::MinusMinus
        ),
        Kind::Keyword(k) => {
            !matches!(
                k,
                Keyword::This | Keyword::Super | Keyword::Null | Keyword::True | Keyword::False
            ) && k.is_reserved()
        }
        Kind::TemplateHead | Kind::TemplateMiddle => true,
        Kind::Invalid => true,
        _ => false,
    }
}
