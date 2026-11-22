// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Tokenizes a whole source without a parser, for the tests, test262 and
//! the fuzz target. A parser knows which goal each token needs; this
//! guesses it from the token before.
//!
//! shortcut: `/` after `)`, `]`, `}` or a word is read as division, so
//! `if (a) /re/.test(s)`, a regex after a block statement's `}`, and
//! `yield /re/` lex wrongly; upgrade to the parser's goals once
//! parsec-js has a parser (M10).

#![allow(dead_code)]

use parsec_js::{Diagnostic, Goal, Keyword, Kind, Lexer, Punct, SourceType, Token};

/// Every token, trivia included, and the diagnostics.
pub fn tokenize(src: &str, source: SourceType) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer::new(src, source);
    let mut out = Vec::new();
    let mut last: Option<Kind> = None;
    // For each `{` or template substitution open, whether it's a
    // substitution, whose `}` carries the template on.
    let mut braces: Vec<bool> = Vec::new();
    loop {
        let regexp = last.is_none_or(regexp_after);
        let tail = braces.last() == Some(&true);
        let goal = match (regexp, tail) {
            (false, false) => Goal::Div,
            (true, false) => Goal::RegExp,
            (false, true) => Goal::TemplateTail,
            (true, true) => Goal::RegExpOrTemplateTail,
        };
        let token = lexer.next_token(goal);
        match token.kind {
            Kind::Eof => break,
            Kind::Punct(Punct::LBrace) => braces.push(false),
            Kind::Punct(Punct::RBrace) | Kind::TemplateTail => {
                braces.pop();
            }
            Kind::TemplateHead => braces.push(true),
            _ => {}
        }
        if !token.is_trivia() {
            last = Some(token.kind);
        }
        out.push(token);
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
            ) && (k.is_reserved() || k == Keyword::Of)
        }
        Kind::TemplateHead | Kind::TemplateMiddle => true,
        Kind::Invalid => true,
        _ => false,
    }
}
