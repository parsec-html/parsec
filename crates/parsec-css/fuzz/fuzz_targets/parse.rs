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

    // Every declaration's value typed for its property, written, read
    // back, and split if it's a shorthand; and the custom properties
    // resolved, whatever refers to whatever.
    let mut custom = values::CustomProperties::new();
    let mut blocks: Vec<&Block<'_>> = Vec::new();
    for rule in &deep.rules {
        match rule {
            Rule::Qualified(r) => blocks.push(&r.block),
            Rule::At(r) => blocks.extend(&r.block),
        }
    }
    while let Some(block) = blocks.pop() {
        for item in &block.items {
            match item {
                Item::Declaration(d) => {
                    if d.name.starts_with("--") {
                        custom.declare(&d.name, d.value.clone());
                    }
                    if let Ok(v) = values::parse_declaration_value(d) {
                        let written = v.to_string();
                        let again = parse_component_values(&written);
                        if let Ok(v2) = values::parse_value(&d.name, &again) {
                            let _ = v2.to_string();
                        }
                    }
                    let _ = values::expand(&d.name, &d.value);
                    // And as the properties with shortest forms and
                    // shorthands of their own, whatever it was given for.
                    for p in [
                        "font", "background", "mask", "grid", "grid-area", "animation",
                        "transition", "list-style", "border-radius", "clip-path", "offset",
                        "line-clamp", "grid-template-areas", "rule-inset", "animation-range",
                    ] {
                        if let Ok(v) = values::parse_value(p, &d.value) {
                            let _ = v.to_string();
                        }
                        let _ = values::expand(p, &d.value);
                    }
                }
                Item::Rule(Rule::Qualified(r)) => blocks.push(&r.block),
                Item::Rule(Rule::At(r)) => blocks.extend(&r.block),
            }
        }
    }
    // As a media query list, a supports condition, and a stylesheet for
    // a small page: parse, write, evaluate, cascade.
    let list = media::MediaQueryList::parse(&values);
    let _ = list.to_string();
    let _ = list.matches(&media::Environment::default());
    let _ = supports::supports(&values);
    let page = parsec::parse("<div id=a class=b style='container-type: size; writing-mode: vertical-rl'><style>@scope { p { color: blue } }</style><p style='color: red' title=t>x</p><ul><li><span></span></ul><input></div>", false);
    // Every at-rule as each kind it could be, written back.
    for rule in &deep.rules {
        if let Rule::At(a) = rule {
            let _ = rules::Keyframes::parse(a).map(|k| k.to_string());
            let _ = rules::CounterStyle::parse(a).map(|c| (c.to_string(), c.usable()));
            let _ = rules::PropertyRule::parse(a).map(|p| (p.to_string(), p.registers()));
            let _ = rules::Import::parse(a);
            let _ = rules::ScopeBounds::parse(&a.prelude).map(|s| s.to_string());
            if let Some(q) = rules::ContainerQuery::parse_list(&a.prelude) {
                for q in q {
                    let _ = q.to_string();
                    let lengths = media::QueryLengths::of(&media::Environment::default());
                    // Style ranges compare whatever the values give.
                    let number = |v: &str| Some(values::Leaf::Number(v.len() as f64));
                    let _ = q.evaluate(Some((100.0, 50.0)), &lengths, &|_| Some("1".into()), &number);
                    let _ = q.evaluate(None, &lengths, &|_| None, &|_| None);
                }
            }
        }
    }
    // Every `@import` fetches the sheet itself again, and every container
    // has a size: imports nest as deep as they're allowed to.
    let me = css.to_string();
    let mut styler = cascade::Styler::new(media::Environment::default())
        .with_document_styles(&page)
        .fetch(move |_| Some(me.clone()))
        .container_sizes(|_, _| Some((300.0, 200.0)));
    styler.add(css, cascade::Origin::Author);
    let styled = styler.style(&page);
    let _ = styler.at_rules();
    for p in ["color", "display", "margin", "font-size", "--x", "background"] {
        if let Some(root) = page.children(page.root()).next() {
            let _ = styled.get(root, p);
        }
    }
    // Every pseudo-element of a few elements, and the input's name as one.
    let pseudos = [
        cascade::Pseudo::Before,
        cascade::Pseudo::After,
        cascade::Pseudo::Marker,
        cascade::Pseudo::Placeholder,
        cascade::Pseudo::FirstLine,
        cascade::Pseudo::FirstLetter,
        cascade::Pseudo::Selection,
        cascade::Pseudo::Backdrop,
        cascade::Pseudo::FileSelectorButton,
        cascade::Pseudo::Highlight("x".into()),
    ];
    let named = cascade::Pseudo::parse(css);
    let some = parsec::css::SelectorList::parse("p, li, input").unwrap();
    for el in some.select(&page, page.root()) {
        for p in pseudos.iter().chain(&named) {
            let _ = styled.pseudo_exists(el, p);
            if let Some(c) = styled.computed_pseudo(el, p) {
                for property in ["content", "color", "display", "--x"] {
                    let _ = c.get(property);
                }
            }
        }
    }
    let resolved = custom.resolve(None);
    let _ = values::substitute_value("width", &values, &resolved);
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
