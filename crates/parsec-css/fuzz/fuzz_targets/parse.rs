// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any input tokenizes and parses through every entry point without
//! panicking, with spans inside the input, and serializes to text that
//! parses back to the same values.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_css::*;

/// The values without their spans, whitespace runs as one, for comparing
/// a parse with the parse of its serialization.
fn shape(values: &[ComponentValue<'_>], out: &mut Vec<String>) {
    for v in values {
        match v {
            ComponentValue::Token(Token::Whitespace, _) => {
                if out.last().is_none_or(|l| l != " ") {
                    out.push(" ".into());
                }
            }
            ComponentValue::Token(t, _) => out.push(format!("{t:?}")),
            ComponentValue::Function(f) => {
                out.push(format!("{}(", f.name));
                shape(&f.arguments, out);
                out.push(")".into());
            }
            ComponentValue::Block(b) => {
                out.push(b.bracket.open().into());
                shape(&b.contents, out);
                out.push(b.bracket.close().into());
            }
        }
    }
}

fn spans_inside(values: &[ComponentValue<'_>], len: usize) {
    for v in values {
        let s = v.span();
        assert!(s.start <= s.end && s.end <= len, "{s:?} outside 0..{len}");
    }
}

/// `r` with its block's contents parsed, and theirs.
fn deepen<'a>(r: Rule<'a>, css: &'a str, errors: &mut Vec<ParseError>) -> Rule<'a, Block<'a>> {
    let mut block = |b: SimpleBlock<'a>| {
        let span = b.span;
        let contents = b.contents(css);
        errors.extend(contents.errors);
        let items = contents
            .items
            .into_iter()
            .map(|i| match i {
                Item::Declaration(d) => Item::Declaration(d),
                Item::Rule(r) => Item::Rule(deepen(r, css, errors)),
            })
            .collect();
        Block { items, span }
    };
    match r {
        Rule::At(r) => Rule::At(AtRule {
            name: r.name,
            prelude: r.prelude,
            block: r.block.map(&mut block),
            span: r.span,
        }),
        Rule::Qualified(r) => Rule::Qualified(QualifiedRule {
            prelude: r.prelude,
            block: block(r.block),
            span: r.span,
        }),
    }
}

fuzz_target!(|data: &[u8]| {
    let css = String::from_utf8_lossy(data);
    let css = css.as_ref();

    let mut end = 0;
    for (_, span) in Tokenizer::new(css) {
        assert_eq!(span.start >= end && span.end > span.start, true);
        end = span.end;
    }

    let values = parse_component_values(css);
    spans_inside(&values, css.len());
    let again = to_css(&values);
    let (mut before, mut after) = (Vec::new(), Vec::new());
    shape(&values, &mut before);
    shape(&parse_component_values(&again), &mut after);
    assert_eq!(before, after, "{css:?} serialized as {again:?}");

    // The one-pass parse makes what parsing each block later does.
    let sheet = parse_stylesheet(css);
    let mut errors = sheet.errors;
    let rules: Vec<_> = sheet
        .rules
        .into_iter()
        .map(|r| deepen(r, css, &mut errors))
        .collect();
    let mut deep = parse_stylesheet_deep(css);
    errors.sort_by_key(|e| (e.span.start, e.span.end));
    deep.errors.sort_by_key(|e| (e.span.start, e.span.end));
    assert_eq!(deep, Stylesheet { rules, errors }, "{css:?}");
    // Not checked to read back the same: a nested rule like `a: {}`, left
    // when a broken declaration is read again as a rule, writes the text
    // of a declaration. No selector looks like that.
    let _ = deep.to_string();
    let _ = parse_rule_list(css).to_string();
    let _ = parse_block_contents(css).to_string();
    let _ = parse_declaration_list(css);
    let _ = parse_rule(css);
    let _ = parse_declaration(css);
    let _ = parse_component_value(css);
    let _ = parse_comma_separated(css);
    let _ = parse_anb(css);
    let _ = parse_unicode_ranges(css);
    let _ = decode_stylesheet(data, None, None);
});
