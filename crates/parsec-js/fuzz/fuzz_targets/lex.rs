// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any input lexes without panicking, as a script and as a module, with
//! the goals the test driver picks and with goals the input picks: every
//! token is non-empty, on character boundaries, after the one before, and
//! inside the input, and so is every diagnostic.

#![no_main]

#[path = "../../tests/driver/mod.rs"]
mod driver;

use libfuzzer_sys::fuzz_target;
use parsec_js::{Goal, Kind, Lexer, SourceType, Token};

const GOALS: [Goal; 4] = [
    Goal::Div,
    Goal::RegExp,
    Goal::TemplateTail,
    Goal::RegExpOrTemplateTail,
];

fn check(src: &str, tokens: &[Token], diagnostics: &[parsec_js::Diagnostic]) {
    let mut end = 0;
    for t in tokens {
        assert!(
            t.span.start >= end && t.span.end > t.span.start,
            "{t:?} in {src:?}"
        );
        assert!(
            src.is_char_boundary(t.span.start as usize)
                && src.is_char_boundary(t.span.end as usize)
        );
        end = t.span.end;
    }
    assert!(end as usize <= src.len());
    for d in diagnostics {
        assert!(
            d.span.start <= d.span.end && d.span.end as usize <= src.len(),
            "{d:?}"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    // The first byte picks the goals; the rest is the source.
    let Some((&seed, rest)) = data.split_first() else {
        return;
    };
    let src = String::from_utf8_lossy(rest);
    for source in [SourceType::Script, SourceType::Module] {
        let (tokens, diagnostics) = driver::tokenize(&src, source);
        check(&src, &tokens, &diagnostics);

        let mut lexer = Lexer::new(&src, source);
        let mut tokens = Vec::new();
        let mut state = u32::from(seed) | 1;
        loop {
            // A xorshift, so the goals change from token to token.
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            let token = lexer.next_token(GOALS[state as usize % 4]);
            if token.kind == Kind::Eof {
                break;
            }
            tokens.push(token);
            assert!(tokens.len() <= src.len(), "more tokens than bytes");
        }
        check(&src, &tokens, lexer.diagnostics());
    }
});
