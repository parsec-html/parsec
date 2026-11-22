// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! An edit to any source that parses, reparsed, gives the tree a full
//! parse of the new source gives, with its comments and warnings, or the
//! same error, leaving the tree as it was. The input's first four bytes
//! say where the edit is and how much it takes out, the rest up to a NUL
//! is what it puts in, and what's after is the source.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_js::{Edit, Limits, Options, SourceType, Span, parse};

fuzz_target!(|data: &[u8]| {
    let Some((head, rest)) = data.split_first_chunk::<4>() else {
        return;
    };
    let rest = String::from_utf8_lossy(rest);
    let (text, src) = rest.split_once('\0').unwrap_or(("", &rest));
    let at = u16::from_le_bytes([head[0], head[1]]) as usize % (src.len() + 1);
    let mut start = at;
    while !src.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (start + head[2] as usize % 8).min(src.len());
    while !src.is_char_boundary(end) {
        end += 1;
    }
    let source_type = if head[3] & 1 == 0 {
        SourceType::Script
    } else {
        SourceType::Module
    };
    let limits = Limits {
        max_depth: (head[3] & 2 != 0).then_some(12),
        ..Limits::default()
    };
    let options = Options { source_type, limits };
    let Ok(mut ast) = parse(src, &options) else {
        return;
    };
    let before = ast.to_estree(src);
    let edit = Edit {
        range: Span::new(start as u32, end as u32),
        text,
    };
    let new = format!("{}{}{}", &src[..start], text, &src[end..]);
    match (ast.reparse(src, &edit, &options), parse(&new, &options)) {
        (Ok(done), Ok(full)) => {
            assert_eq!(done.source, new);
            assert_eq!(ast.to_estree(&new), full.to_estree(&new));
            assert_eq!(ast.comments(), full.comments());
            assert_eq!(ast.warnings(), full.warnings());
        }
        (Err(a), Err(b)) => {
            assert_eq!(a, b);
            assert_eq!(ast.to_estree(src), before);
        }
        (r, f) => panic!("reparse {:?}, parse {:?}", r.map(|d| d.reparsed), f.map(|_| ())),
    }
});
