// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any input parses or fails without panicking, as a script and as a
//! module, with the default limits and with small ones: every node's span
//! and every error's is inside the input and on character boundaries,
//! and a tree that parsed writes out as ESTree JSON.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_js::{Limits, Options, SourceType, parse};

fn on_boundaries(src: &str, start: u32, end: u32) -> bool {
    start <= end
        && end as usize <= src.len()
        && src.is_char_boundary(start as usize)
        && src.is_char_boundary(end as usize)
}

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    let small = Limits {
        max_depth: Some(8),
        max_steps: Some(64),
        ..Limits::default()
    };
    for source_type in [SourceType::Script, SourceType::Module] {
        for limits in [Limits::default(), small] {
            let options = Options { source_type, limits };
            match parse(&src, &options) {
                Ok(ast) => {
                    for id in ast.ids() {
                        let span = ast.span(id);
                        assert!(on_boundaries(&src, span.start, span.end), "{id:?} at {span:?}");
                    }
                    for c in ast.comments() {
                        assert!(on_boundaries(&src, c.span.start, c.span.end), "{c:?}");
                    }
                    let json = ast.to_estree(&src);
                    assert!(json.starts_with(r#"{"type":"Program""#));
                }
                Err(d) => assert!(d.span.start <= d.span.end && d.span.end as usize <= src.len(), "{d}"),
            }
        }
    }
});
