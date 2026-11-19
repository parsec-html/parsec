// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `@supports` and `CSS.supports()`, CSS Conditional Rules Level 5: a
//! condition parsed and evaluated against what this crate parses. A
//! declaration is supported when its value parses for its property, a
//! `selector()` when Parsec's selector engine parses it, and
//! `font-format()` and `font-tech()` when they name a format or
//! technology the specs define.

use crate::parser::{Bracket, ComponentValue};
use crate::tokenizer::Token;

/// The at-rules `at-rule()` says are supported: those this crate reads.
const AT_RULES: &[&str] = &[
    "media",
    "supports",
    "layer",
    "import",
    "container",
    "scope",
    "property",
    "keyframes",
    "counter-style",
];

const FORMATS: &[&str] = &[
    "collection",
    "embedded-opentype",
    "opentype",
    "svg",
    "truetype",
    "woff",
    "woff2",
];
const TECHS: &[&str] = &[
    "color-cbdt",
    "color-colrv0",
    "color-colrv1",
    "color-sbix",
    "color-svg",
    "features-aat",
    "features-graphite",
    "features-opentype",
    "incremental",
    "palettes",
    "variations",
];

/// How deep conditions may nest.
const MAX_DEPTH: usize = 64;

fn solid<'v, 'a>(values: &'v [ComponentValue<'a>]) -> Vec<&'v ComponentValue<'a>> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

fn word(v: Option<&&ComponentValue<'_>>, w: &str) -> bool {
    matches!(v, Some(ComponentValue::Token(Token::Ident(s), _)) if s.eq_ignore_ascii_case(w))
}

/// Whether a `<supports-condition>` holds, or `None` if it doesn't parse,
/// which makes an `@supports` rule invalid.
///
/// ```
/// use parsec_css::supports::supports;
///
/// let c = parsec_css::parse_component_values("(display: grid) and (not (color: nonsense))");
/// assert_eq!(supports(&c), Some(true));
/// let c = parsec_css::parse_component_values("selector(a > b) or (width: 1fr)");
/// assert_eq!(supports(&c), Some(true));
/// assert_eq!(supports(&parsec_css::parse_component_values("color: red")), None);
/// ```
pub fn supports(condition: &[ComponentValue<'_>]) -> Option<bool> {
    condition_at(&solid(condition), 0)
}

/// `CSS.supports(property, value)`.
pub fn supports_declaration(property: &str, value: &[ComponentValue<'_>]) -> bool {
    crate::values::parse_value(property, value).is_ok()
}

/// `CSS.supports(conditionText)`: a condition, or a declaration without
/// its parentheses.
pub fn supports_text(css: &str) -> bool {
    let values = crate::parse_component_values(css);
    if let Some(r) = supports(&values) {
        return r;
    }
    let wrapped = format!("({css})");
    supports(&crate::parse_component_values(&wrapped)).unwrap_or(false)
}

fn condition_at(v: &[&ComponentValue<'_>], depth: usize) -> Option<bool> {
    if depth > MAX_DEPTH || v.is_empty() {
        return None;
    }
    if word(v.first(), "not") {
        if v.len() != 2 {
            return None;
        }
        return in_parens(v[1], depth).map(|b| !b);
    }
    let first = in_parens(v[0], depth)?;
    if v.len() == 1 {
        return Some(first);
    }
    let and = word(v.get(1), "and");
    if !and && !word(v.get(1), "or") {
        return None;
    }
    let joiner = if and { "and" } else { "or" };
    let mut result = first;
    let mut i = 1;
    while i < v.len() {
        if !word(v.get(i), joiner) {
            return None;
        }
        let next = in_parens(v.get(i + 1)?, depth)?;
        result = if and { result && next } else { result || next };
        i += 2;
    }
    Some(result)
}

fn in_parens(v: &ComponentValue<'_>, depth: usize) -> Option<bool> {
    match v {
        ComponentValue::Block(b) if b.bracket == Bracket::Paren => {
            let inner = solid(&b.contents);
            if let Some(r) = condition_at(&inner, depth + 1) {
                return Some(r);
            }
            // A declaration, `(property: value)`.
            if let Ok(d) = crate::parse_declaration(&crate::to_css(&b.contents)) {
                return Some(crate::values::parse_value(&d.name, &d.value).is_ok());
            }
            // `<general-enclosed>`: false.
            Some(false)
        }
        ComponentValue::Function(f) => {
            let args = solid(&f.arguments);
            let one = |list: &[&str]| match args[..] {
                [ComponentValue::Token(Token::Ident(w), _)] => {
                    list.iter().any(|l| w.eq_ignore_ascii_case(l))
                }
                _ => false,
            };
            Some(match f.name.to_ascii_lowercase().as_str() {
                "selector" => {
                    parsec::css::SelectorList::parse(crate::to_css(&f.arguments).trim()).is_ok()
                }
                "font-format" => one(FORMATS),
                "font-tech" => one(TECHS),
                // CSS Conditional 5 §2.5: an at-rule this crate reads.
                "at-rule" => match args[..] {
                    [ComponentValue::Token(Token::AtKeyword(w), _)] => {
                        AT_RULES.iter().any(|a| w.eq_ignore_ascii_case(a))
                    }
                    _ => false,
                },
                _ => false,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditions() {
        for (css, want) in [
            ("(color: red)", true),
            ("color: red", true),
            ("(color: red) and (color: blue)", true),
            ("not (foobar)", true),
            ("color: something-pointless var(--foo)", true),
            ("unknownproperty: blah", false),
            ("width: blah", false),
            ("--foo: blah", true),
            ("selector(div > p)", true),
            ("selector(:::)", false),
            ("font-format(opentype)", true),
            ("font-format(xyzzy)", false),
            ("font-tech(color-COLRv1)", true),
            ("(color: red) or (width: blah)", true),
            ("(color: red) and (width: blah)", false),
        ] {
            assert_eq!(supports_text(css), want, "{css}");
        }
        // `and` and `or` don't mix without parentheses.
        let c = crate::parse_component_values("(a: b) and (c: d) or (e: f)");
        assert_eq!(supports(&c), None);
    }
}
