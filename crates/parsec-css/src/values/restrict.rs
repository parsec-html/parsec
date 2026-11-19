// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! What the specs' prose rules out that their grammars allow: names a
//! `<custom-ident>` can't be (`none` as a counter, `span` as a grid
//! line), counts (`steps(0)`), and strings with a syntax of their own
//! (a font feature's four-letter tag, SVG path data).

use super::matcher::Out;
use super::value::Component;
use crate::parser::ComponentValue;
use crate::tokenizer::Token;

pub(crate) const CSS_WIDE: &[&str] = &[
    "initial",
    "inherit",
    "unset",
    "revert",
    "revert-layer",
    "revert-rule",
    "default",
];

fn values<'o, 'a>(outs: &'o [Out<'a>]) -> impl Iterator<Item = &'o Component<'a>> {
    outs.iter().filter_map(|o| match o {
        Out::C(c) => Some(c),
        _ => None,
    })
}

/// Whether `c` is an identifier that's one of `words`, ignoring case.
fn named(c: &Component<'_>, words: &[&str]) -> bool {
    matches!(c, Component::Ident(s) if words.iter().any(|w| s.eq_ignore_ascii_case(w)))
}

pub(crate) const GENERIC_FAMILIES: &[&str] = &[
    "serif",
    "sans-serif",
    "cursive",
    "fantasy",
    "monospace",
    "system-ui",
    "math",
    "ui-serif",
    "ui-sans-serif",
    "ui-monospace",
    "ui-rounded",
];

/// The types with a restriction here, marked in a match for it.
pub(crate) const TYPES: &[&str] = &[
    "counter-style-name",
    "counter-name",
    "line-names",
    "grid-line",
    "animateable-feature",
    "single-transition-property",
    "font-family-name",
    "keyframes-name",
    "opentype-tag",
];

/// Whether what a type matched, `outs`, is one of its values.
pub(crate) fn of_type(name: &str, outs: &[Out<'_>]) -> bool {
    let mut v = values(outs);
    match name {
        // CSS Counter Styles 3 §1.1 and CSS Lists 3 §4.
        "counter-style-name" | "counter-name" => !v.any(|c| named(c, &["none"])),
        // CSS Grid 2 §7.2 and §8.3.
        "line-names" => !v.any(|c| match c {
            Component::Block(names) => names.iter().any(|n| named(n, &["span", "auto"])),
            _ => false,
        }),
        "grid-line" => !v.any(|c| named(c, &["span", "auto"])),
        // CSS Will Change 1 §2.
        "animateable-feature" => {
            !v.any(|c| named(c, &["will-change", "none", "all", "auto"]) || named(c, CSS_WIDE))
        }
        // CSS Transitions 1 §2.1: `none` only alone.
        "single-transition-property" => !v.any(|c| named(c, &["none"])),
        // CSS Fonts 4 §2.1.1: generic names and the CSS-wide keywords
        // have to be quoted.
        "font-family-name" => !v.any(|c| named(c, GENERIC_FAMILIES) || named(c, CSS_WIDE)),
        "keyframes-name" => !v.any(|c| matches!(c, Component::String(s) if s.is_empty())),
        // CSS Fonts 4 §6.12: four printable ASCII characters.
        "opentype-tag" => v.all(|c| match c {
            Component::String(s) => s.len() == 4 && s.bytes().all(|b| (0x20..=0x7e).contains(&b)),
            _ => true,
        }),
        _ => true,
    }
}

/// Whether a function's arguments, as matched, are allowed.
pub(crate) fn function(name: &str, args: &[Component<'_>]) -> bool {
    match name {
        // CSS Easing 1 §2.3: at least one step, two for `jump-none`.
        "steps" => match args.first() {
            Some(Component::Number { value, .. }) => {
                let none = args
                    .last()
                    .is_some_and(|a| *a == Component::Keyword("jump-none"));
                *value >= if none { 2.0 } else { 1.0 }
            }
            _ => true,
        },
        // CSS Counter Styles 3 §6.1: `numeric` and `alphabetic` need two
        // symbols; image symbols aren't supported.
        "symbols" => {
            let symbols = args
                .iter()
                .filter(|a| !matches!(a, Component::Keyword(_)))
                .count();
            let two = args
                .first()
                .is_some_and(|a| matches!(a, Component::Keyword("numeric" | "alphabetic")));
            let images = args
                .iter()
                .any(|a| matches!(a, Component::Url(_) | Component::Function { .. }));
            !images && (!two || symbols >= 2)
        }
        "path" => match args.last() {
            Some(Component::String(s)) => super::path::normalize(s).is_some(),
            _ => false,
        },
        // CSS Images 3 §3.2.1: a circle has one size, an ellipse two.
        n if n.ends_with("radial-gradient") => {
            let end = args
                .iter()
                .position(|a| matches!(a, Component::Literal(',') | Component::Keyword("at")))
                .unwrap_or(args.len());
            let sizes = args[..end]
                .iter()
                .filter(|a| {
                    matches!(
                        a,
                        Component::Dimension(..) | Component::Percentage(_) | Component::Math(_)
                    )
                })
                .count();
            let has = |k: &'static str| args[..end].contains(&Component::Keyword(k));
            !(has("circle") && sizes == 2) && !(has("ellipse") && sizes == 1)
        }
        _ => true,
    }
}

/// The properties [`property`] has a restriction for.
pub(crate) const PROPERTIES: &[&str] = &[
    "color-scheme",
    "view-transition-class",
    "flex",
    "flex-basis",
    "font-language-override",
    "grid-template-areas",
    "cursor",
    "offset-path",
];

/// Whether a value of `name`, as written (`raw`) and matched (`c`), is
/// allowed.
pub(crate) fn property(name: &str, raw: &[ComponentValue<'_>], c: &[Component<'_>]) -> bool {
    match name {
        // CSS Color Adjust 1 §2.1.
        "color-scheme" => !c
            .iter()
            .any(|x| named(x, &["normal", "light", "dark", "only"])),
        // CSS View Transitions 2 §3.
        "view-transition-class" => !c.iter().any(|x| named(x, &["none"])),
        // CSS Flexbox 1 §7.1.1: a unitless zero not after two flex
        // factors is a factor, so a third number is a basis only if it's
        // zero.
        "flex" => {
            let numbers: Vec<f64> = raw
                .iter()
                .filter_map(|v| match v {
                    ComponentValue::Token(Token::Number(n), _) => Some(n.value),
                    _ => None,
                })
                .collect();
            numbers.len() < 3 || numbers[2] == 0.0
        }
        // CSS Anchor Positioning 1 §7.1: sizing, inset and margin
        // properties only.
        "flex-basis" => !c.iter().any(|x| {
            matches!(
                x,
                Component::Function {
                    name: "anchor-size",
                    ..
                }
            )
        }),
        // CSS Fonts 4 §6.13: an OpenType language tag, up to four ASCII
        // letters, spaces at the end aside.
        "font-language-override" => c.iter().all(|x| match x {
            Component::String(s) => {
                (1..=4).contains(&s.len())
                    && s.bytes().all(|b| (0x20..=0x7e).contains(&b))
                    && !s.trim_end_matches(' ').is_empty()
            }
            _ => true,
        }),
        "grid-template-areas" => areas(c),
        // CSS UI 4 §5.1.1: a cursor's `image-set()` takes URLs only.
        "cursor" => !c.iter().any(|x| match x {
            Component::Function {
                name: "image-set",
                arguments,
            } => arguments
                .iter()
                .any(|a| matches!(a, Component::Function { .. })),
            _ => false,
        }),
        // CSS Motion 1 §2.1: a path's fill rule means nothing here.
        "offset-path" => !raw.iter().any(|v| match v {
            ComponentValue::Function(f) if f.name.eq_ignore_ascii_case("path") => f
                .arguments
                .iter()
                .any(|a| matches!(a, ComponentValue::Token(Token::Comma, _))),
            _ => false,
        }),
        _ => true,
    }
}

/// CSS Grid 2 §7.3: rows with cells, as many in each, and each named area
/// a rectangle.
fn areas(c: &[Component<'_>]) -> bool {
    let rows: Vec<Vec<String>> = c
        .iter()
        .filter_map(|x| match x {
            Component::String(s) => Some(
                s.split([' ', '\t', '\n', '\r', '\x0c'])
                    .filter(|w| !w.is_empty())
                    .flat_map(|w| {
                        // A run of dots is one null cell.
                        let mut out = Vec::new();
                        let mut rest = w;
                        while !rest.is_empty() {
                            let dots = rest.starts_with('.');
                            let end = rest.find(|ch| (ch == '.') != dots).unwrap_or(rest.len());
                            out.push(if dots {
                                ".".to_string()
                            } else {
                                rest[..end].to_string()
                            });
                            rest = &rest[end..];
                        }
                        out
                    })
                    .collect(),
            ),
            _ => None,
        })
        .collect();
    if rows.is_empty() || rows.iter().any(Vec::is_empty) {
        return c.iter().all(|x| !matches!(x, Component::String(_)));
    }
    let width = rows[0].len();
    if rows.iter().any(|r| r.len() != width) {
        return false;
    }
    // Each name's bounds, and how many cells it has: a rectangle has as
    // many as its bounds hold. Linear in the cells, however many names.
    let mut seen: std::collections::HashMap<&str, (usize, usize, usize, usize, usize)> =
        std::collections::HashMap::new();
    for (y, row) in rows.iter().enumerate() {
        for (x, cell) in row.iter().enumerate() {
            if cell == "." {
                continue;
            }
            let s = seen.entry(cell).or_insert((x, x, y, y, 0));
            s.0 = s.0.min(x);
            s.1 = s.1.max(x);
            s.2 = s.2.min(y);
            s.3 = s.3.max(y);
            s.4 += 1;
        }
    }
    seen.values()
        .all(|&(x0, x1, y0, y1, n)| n == (x1 - x0 + 1) * (y1 - y0 + 1))
}
