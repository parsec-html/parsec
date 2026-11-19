// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! A declaration's value, typed against its property's grammar: what
//! `div.style[property] = value` would accept, and how CSSOM writes it
//! back. Shorthands expand into their longhands.

use std::borrow::Cow;
use std::fmt::{self, Write};

use super::color::Color;
use super::grammar::Grammar;
use super::matcher::{Matcher, Out, canonical};
use super::math::Math;
use super::properties::{Property, property};
use super::units::{Unit, write_number};
use crate::parser::{ComponentValue, Declaration};
use crate::serialize::{write_identifier, write_string, write_values};
use crate::tokenizer::Token;

/// One typed value in a property's value.
#[derive(Clone, Debug, PartialEq)]
pub enum Component<'a> {
    /// A keyword from the grammar, in lowercase.
    Keyword(&'static str),
    /// An identifier the grammar leaves open, as written: `<custom-ident>`.
    Ident(Cow<'a, str>),
    Number {
        value: f64,
        integer: bool,
    },
    Percentage(f64),
    Dimension(f64, Unit),
    Math(Math),
    Color(Color),
    String(Cow<'a, str>),
    Url(Cow<'a, str>),
    /// `,`, `/` and the like, where the grammar has them.
    Literal(char),
    Function {
        /// In lowercase.
        name: &'static str,
        arguments: Vec<Component<'a>>,
    },
    /// `[ … ]`, as grid line names are written.
    Block(Vec<Component<'a>>),
    /// `( … )`.
    Parens(Vec<Component<'a>>),
    /// `<declaration-value>`: tokens the grammar takes as they are.
    Tokens(Vec<ComponentValue<'a>>),
}

/// The CSS-wide keywords, which every property takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CssWide {
    Initial,
    Inherit,
    Unset,
    Revert,
    RevertLayer,
}

impl CssWide {
    fn parse(word: &str) -> Option<CssWide> {
        let mut buf = [0; 32];
        Some(match super::units::lowercase(word, &mut buf)? {
            "initial" => CssWide::Initial,
            "inherit" => CssWide::Inherit,
            "unset" => CssWide::Unset,
            "revert" => CssWide::Revert,
            "revert-layer" => CssWide::RevertLayer,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            CssWide::Initial => "initial",
            CssWide::Inherit => "inherit",
            CssWide::Unset => "unset",
            CssWide::Revert => "revert",
            CssWide::RevertLayer => "revert-layer",
        }
    }
}

/// A property's specified value.
#[derive(Clone, Debug, PartialEq)]
pub enum Specified<'a> {
    CssWide(CssWide),
    /// A value with `var()` or `env()` in it: valid until substitution
    /// says otherwise, so kept as written, to be parsed then.
    Unparsed(Vec<ComponentValue<'a>>),
    /// A custom property's value, as written.
    Custom(Vec<ComponentValue<'a>>),
    /// A value that matched the property's grammar.
    Value(Vec<Component<'a>>),
}

/// The value didn't match the property's grammar, or the property isn't
/// one the specs define.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidValue;

impl fmt::Display for InvalidValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid value")
    }
}

impl std::error::Error for InvalidValue {}

fn trim<'v, 'a>(values: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = values
        .iter()
        .position(|v| !v.is_whitespace())
        .unwrap_or(values.len());
    let end = values
        .iter()
        .rposition(|v| !v.is_whitespace())
        .map_or(start, |i| i + 1);
    &values[start..end]
}

/// Whether `var()` or `env()` is anywhere in the values.
pub(crate) fn has_substitution(values: &[ComponentValue<'_>]) -> bool {
    // Most values have no functions or blocks, and need no stack.
    let mut stack = Vec::new();
    let mut list = values;
    loop {
        for v in list {
            match v {
                ComponentValue::Function(f) => {
                    if f.name.eq_ignore_ascii_case("var") || f.name.eq_ignore_ascii_case("env") {
                        return true;
                    }
                    stack.push(&f.arguments[..]);
                }
                ComponentValue::Block(b) => stack.push(&b.contents[..]),
                ComponentValue::Token(..) => {}
            }
        }
        match stack.pop() {
            Some(next) => list = next,
            None => return false,
        }
    }
}

/// Parses `value` as a value of `property`, `--` custom properties
/// included.
///
/// ```
/// use parsec_css::values::parse_value;
///
/// let v = parsec_css::parse_component_values("1PX  calc(2px + 3px) AUTO");
/// assert_eq!(parse_value("margin", &v).unwrap().to_string(), "1px calc(5px) auto");
/// let v = parsec_css::parse_component_values("solid");
/// assert!(parse_value("margin", &v).is_err());
/// ```
pub fn parse_value<'a>(
    property_name: &str,
    value: &[ComponentValue<'a>],
) -> Result<Specified<'a>, InvalidValue> {
    let value = trim(value);
    if property_name.starts_with("--") {
        return Ok(Specified::Custom(value.to_vec()));
    }
    let p = property(property_name).ok_or(InvalidValue)?;
    parse_for(p, value)
}

fn parse_for<'a>(
    p: &'static Property,
    value: &[ComponentValue<'a>],
) -> Result<Specified<'a>, InvalidValue> {
    if let [ComponentValue::Token(Token::Ident(w), _)] = value
        && let Some(k) = CssWide::parse(w)
    {
        return Ok(Specified::CssWide(k));
    }
    if value.is_empty() {
        return Err(InvalidValue);
    }
    if has_substitution(value) {
        return Ok(Specified::Unparsed(value.to_vec()));
    }
    let g = p.grammar().ok_or(InvalidValue)?;
    // A keyword the grammar has elsewhere is taken as that keyword where it
    // can be (`span` in `grid-column`), and as a name only if nothing
    // else reads.
    let mut m = Matcher::new(value).for_property(p);
    if !m.whole(g) {
        m = Matcher::new(value);
        if !m.whole(g) {
            return Err(InvalidValue);
        }
    }
    let outs = canonical(m.out);
    let mut c = match same_pair(g, &outs) {
        Some(first) => components(first),
        None => components(outs),
    };
    shortest(g, &mut c);
    // `first baseline` is `baseline`, CSS Box Alignment §4.2.
    if let Some(i) = c
        .windows(2)
        .position(|w| w[0] == Component::Keyword("first") && w[1] == Component::Keyword("baseline"))
    {
        c.remove(i);
    }
    match p.name {
        "display" => shortest_display(&mut c),
        "white-space" => shortest_white_space(&mut c),
        "flex" => shortest_flex(&mut c),
        _ => {}
    }
    Ok(Specified::Value(c))
}

/// For `<'a'> <'b'>?`, as `place-items` is, the first part alone when the
/// second is the same: `center center` is `center`.
fn same_pair<'a>(g: &Grammar, outs: &[Out<'a>]) -> Option<Vec<Out<'a>>> {
    let Grammar::Sequence(items) = g else {
        return None;
    };
    let [
        Grammar::Property(_),
        Grammar::Repeat {
            item,
            min: 0,
            max: 1,
            ..
        },
    ] = &items[..]
    else {
        return None;
    };
    if !matches!(**item, Grammar::Property(_)) {
        return None;
    }
    let parts = segments(outs);
    let [(Some(_), a), (Some(_), b)] = &parts[..] else {
        return None;
    };
    let text = |o: &[Out<'a>]| Specified::Value(components(o.to_vec())).to_string();
    (text(a) == text(b)).then(|| a.to_vec())
}

/// `flex` in full, as browsers write it: grow, shrink and basis, with
/// `1`, `1` and `0%` where they're left out, and `none` as `0 0 auto`.
fn shortest_flex(c: &mut Vec<Component<'_>>) {
    if *c == [Component::Keyword("none")] {
        *c = vec![
            Component::Number {
                value: 0.0,
                integer: true,
            },
            Component::Number {
                value: 0.0,
                integer: true,
            },
            Component::Keyword("auto"),
        ];
        return;
    }
    let mut numbers = Vec::new();
    let mut basis = None;
    for x in c.drain(..) {
        match x {
            Component::Number { .. } if numbers.len() < 2 => numbers.push(x),
            Component::Math(ref m) if numbers.len() < 2 && m.fits(super::math::Expect::Number) => {
                numbers.push(x)
            }
            other => basis = Some(other),
        }
    }
    let one = Component::Number {
        value: 1.0,
        integer: true,
    };
    let grow = numbers.first().cloned().unwrap_or(one.clone());
    let shrink = numbers.get(1).cloned().unwrap_or(one);
    let basis = basis.unwrap_or(Component::Percentage(0.0));
    *c = vec![grow, shrink, basis];
}

fn keywords(c: &[Component<'_>]) -> Option<Vec<&'static str>> {
    c.iter()
        .map(|c| match c {
            Component::Keyword(k) => Some(*k),
            _ => None,
        })
        .collect()
}

/// `display` in its shortest form, CSS Display 3 §2: `block flow` is
/// `block`, `inline flex` is `inline-flex`, `block flow list-item` is
/// `list-item`.
fn shortest_display(c: &mut Vec<Component<'_>>) {
    let Some(words) = keywords(c) else { return };
    if words.len() < 2 {
        return;
    }
    let outer = ["block", "inline", "run-in"];
    let out = words.iter().copied().find(|w| outer.contains(w));
    let list_item = words.contains(&"list-item");
    let inner = words
        .iter()
        .copied()
        .find(|w| !outer.contains(w) && *w != "list-item");
    let out_word = out.unwrap_or("block");
    let inner_word = inner.unwrap_or("flow");
    let written: Vec<&'static str> = if list_item {
        let mut v = Vec::new();
        if out_word != "block" {
            v.push(out_word);
        }
        if inner_word != "flow" {
            v.push(inner_word);
        }
        v.push("list-item");
        v
    } else {
        match (out_word, inner_word) {
            ("block", "flow") => vec!["block"],
            ("inline", "flow") => vec!["inline"],
            ("run-in", "flow") => vec!["run-in"],
            ("block", "flow-root") => vec!["flow-root"],
            ("inline", "flow-root") => vec!["inline-block"],
            ("block", "table") => vec!["table"],
            ("inline", "table") => vec!["inline-table"],
            ("block", "flex") => vec!["flex"],
            ("inline", "flex") => vec!["inline-flex"],
            ("block", "grid") => vec!["grid"],
            ("inline", "grid") => vec!["inline-grid"],
            ("inline", "ruby") => vec!["ruby"],
            (o, i) => vec![o, i],
        }
    };
    *c = written.into_iter().map(Component::Keyword).collect();
}

/// `white-space` as its legacy keywords where one says it, and otherwise
/// without what's at its initial value.
fn shortest_white_space(c: &mut Vec<Component<'_>>) {
    let Some(words) = keywords(c) else { return };
    let collapse = [
        "collapse",
        "preserve",
        "preserve-breaks",
        "preserve-spaces",
        "break-spaces",
    ];
    let wrap = ["wrap", "nowrap"];
    if words
        .iter()
        .any(|w| ["normal", "pre", "pre-wrap", "pre-line"].contains(w))
    {
        return;
    }
    let col = words
        .iter()
        .copied()
        .find(|w| collapse.contains(w))
        .unwrap_or("collapse");
    let wr = words
        .iter()
        .copied()
        .find(|w| wrap.contains(w))
        .unwrap_or("wrap");
    let trim: Vec<&'static str> = words
        .iter()
        .copied()
        .filter(|w| !collapse.contains(w) && !wrap.contains(w) && *w != "none")
        .collect();
    let mut written: Vec<&'static str> = Vec::new();
    if trim.is_empty() {
        let legacy = match (col, wr) {
            ("collapse", "wrap") => Some("normal"),
            ("preserve", "nowrap") => Some("pre"),
            ("preserve", "wrap") => Some("pre-wrap"),
            ("preserve-breaks", "wrap") => Some("pre-line"),
            ("collapse", "nowrap") => Some("nowrap"),
            ("break-spaces", "wrap") => Some("break-spaces"),
            _ => None,
        };
        if let Some(l) = legacy {
            *c = vec![Component::Keyword(l)];
            return;
        }
    }
    if col != "collapse" {
        written.push(col);
    }
    if wr != "wrap" {
        written.push(wr);
    }
    written.extend(trim);
    if written.is_empty() {
        written.push("normal");
    }
    *c = written.into_iter().map(Component::Keyword).collect();
}

/// Leaves out what a box shorthand repeats, as CSSOM writes them:
/// `1px 2px 1px 2px` is `1px 2px`, `3px 3px` is `3px`.
fn shortest(g: &Grammar, c: &mut Vec<Component<'_>>) {
    let Grammar::Repeat {
        item,
        min: 1,
        max: max @ (2 | 4),
        commas: false,
    } = g
    else {
        return;
    };
    // Only where each item is one value.
    if !matches!(**item, Grammar::Property(_) | Grammar::Type(_)) || c.len() > *max as usize {
        return;
    }
    if c.len() == 4 && c[3] == c[1] {
        c.pop();
    }
    if c.len() == 3 && c[2] == c[0] {
        c.pop();
    }
    if c.len() == 2 && c[1] == c[0] {
        c.pop();
    }
}

fn components(out: Vec<Out<'_>>) -> Vec<Component<'_>> {
    out.into_iter()
        .filter_map(|o| match o {
            Out::C(c) => Some(c),
            _ => None,
        })
        .collect()
}

/// Matches `values` against the grammar the specs give a function, as
/// `<color-mix()>`, for the colors that are matched that way.
pub(crate) fn match_function<'a>(
    name: &str,
    values: &[ComponentValue<'a>],
) -> Option<Vec<Component<'a>>> {
    let g = super::properties::function_grammar(name)?;
    let mut m = Matcher::new(values);
    if !m.whole(g) {
        return None;
    }
    Some(components(canonical(m.out)))
}

/// Parses a declaration's value for its property.
pub fn parse_declaration_value<'a>(d: &Declaration<'a>) -> Result<Specified<'a>, InvalidValue> {
    parse_value(&d.name, &d.value)
}

/// Parses a shorthand's value and splits it into a value for each of its
/// longhands, in the order the spec lists them; a longhand the value
/// leaves out gets its initial value. A longhand gives itself.
///
/// ```
/// use parsec_css::values::expand;
///
/// let v = parsec_css::parse_component_values("1px 2px");
/// let longhands: Vec<String> = expand("margin", &v)
///     .unwrap()
///     .iter()
///     .map(|(name, value)| format!("{name}: {value}"))
///     .collect();
/// assert_eq!(
///     longhands,
///     ["margin-top: 1px", "margin-right: 2px", "margin-bottom: 1px", "margin-left: 2px"]
/// );
/// ```
pub fn expand<'a>(
    shorthand: &str,
    value: &[ComponentValue<'a>],
) -> Result<Vec<(&'static str, Specified<'a>)>, InvalidValue> {
    let p = property(shorthand).ok_or(InvalidValue)?;
    let value = trim(value);
    if !p.is_shorthand() {
        return Ok(vec![(p.name, parse_for(p, value)?)]);
    }
    match parse_for(p, value)? {
        Specified::Value(_) => {}
        // Every longhand takes the keyword, or waits for substitution.
        other => return Ok(p.longhands.iter().map(|l| (*l, other.clone())).collect()),
    }
    if let Some(special) = expand_special(p, value) {
        return Ok(special);
    }
    let g = p.grammar().ok_or(InvalidValue)?;
    let mut m = Matcher::new(value).for_property(p);
    if !m.whole(g) {
        m = Matcher::new(value);
        m.whole(g);
    }
    let outs = canonical(m.out);
    let mut assigned: Vec<Option<Vec<Out<'a>>>> = vec![None; p.longhands.len()];
    split(p, g, &outs, &mut assigned).ok_or(InvalidValue)?;
    let mut result = Vec::with_capacity(p.longhands.len());
    for (name, part) in p.longhands.iter().zip(assigned) {
        let value = match part {
            Some(outs) => Specified::Value(components(outs)),
            None => initial(name),
        };
        result.push((*name, value));
    }
    Ok(result)
}

/// The shorthands whose left-out parts aren't their longhands' initial
/// values: `flex`, where they're `1 1 0%` (CSS Flexbox §7.1), and
/// `white-space`'s legacy keywords (CSS Text 4 §3).
fn expand_special<'a>(
    p: &'static Property,
    value: &[ComponentValue<'a>],
) -> Option<Vec<(&'static str, Specified<'a>)>> {
    let keyword = match value {
        [ComponentValue::Token(Token::Ident(w), _)] => Some(w.to_ascii_lowercase()),
        _ => None,
    };
    let text = |s: &'static str| -> Specified<'a> {
        let values = crate::parse_component_values(s);
        let Ok(Specified::Value(c)) = parse_value_static(s, &values) else {
            unreachable!("{s}")
        };
        Specified::Value(c)
    };
    match p.name {
        "white-space" => {
            let (collapse, wrap) = match keyword.as_deref()? {
                "normal" => ("collapse", "wrap"),
                "pre" => ("preserve", "nowrap"),
                "pre-wrap" => ("preserve", "wrap"),
                "pre-line" => ("preserve-breaks", "wrap"),
                _ => return None,
            };
            let kw = |k: &'static str| Specified::Value(vec![Component::Keyword(k)]);
            Some(vec![
                ("white-space-collapse", kw(collapse)),
                ("text-wrap-mode", kw(wrap)),
            ])
        }
        "flex" => {
            let (grow, shrink, basis) = match keyword.as_deref() {
                Some("none") => ("0", "0", "auto"),
                Some("auto") => ("1", "1", "auto"),
                _ => {
                    // Split as the grammar does, then fill in 1, 1 and 0%.
                    let g = p.grammar()?;
                    let mut m = Matcher::new(value).for_property(p);
                    if !m.whole(g) {
                        return None;
                    }
                    let outs = canonical(m.out);
                    let mut assigned: Vec<Option<Vec<Out<'a>>>> = vec![None; 3];
                    split(p, g, &outs, &mut assigned)?;
                    let mut fill = |i: usize, d: &'static str| match assigned[i].take() {
                        Some(o) => Specified::Value(components(o)),
                        None => text(d),
                    };
                    let (gw, sh, ba) = (fill(0, "1"), fill(1, "1"), fill(2, "0%"));
                    return Some(vec![
                        ("flex-grow", gw),
                        ("flex-shrink", sh),
                        ("flex-basis", ba),
                    ]);
                }
            };
            Some(vec![
                ("flex-grow", text(grow)),
                ("flex-shrink", text(shrink)),
                ("flex-basis", text(basis)),
            ])
        }
        _ => None,
    }
}

/// A value from text that lives for the program: for the defaults above.
fn parse_value_static<'a>(
    s: &'static str,
    values: &[ComponentValue<'static>],
) -> Result<Specified<'a>, InvalidValue> {
    // A number alone is `flex-grow`'s or `flex-shrink`'s; anything else
    // is a basis.
    let name = if s.parse::<f64>().is_ok() {
        "flex-grow"
    } else {
        "flex-basis"
    };
    parse_value(name, values)
}

/// A longhand's initial value, parsed, or `initial` where the spec's is
/// prose. The text is the table's, so the value borrows nothing.
fn initial(name: &str) -> Specified<'static> {
    let parsed = property(name).and_then(|p| {
        let values = crate::parse_component_values(p.initial?);
        match parse_for(p, &values) {
            Ok(v @ Specified::Value(_)) => Some(v),
            _ => None,
        }
    });
    parsed.unwrap_or(Specified::CssWide(CssWide::Initial))
}

/// The top-level parts of a match: each `(property, outs)` that matched a
/// property reference, and the values outside any.
fn segments<'o, 'a>(outs: &'o [Out<'a>]) -> Vec<(Option<&'static str>, &'o [Out<'a>])> {
    let mut parts = Vec::new();
    let mut i = 0;
    while i < outs.len() {
        match &outs[i] {
            Out::Start(name) => {
                let mut depth = 0;
                let mut j = i;
                loop {
                    match outs[j] {
                        Out::Start(_) => depth += 1,
                        Out::End => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    j += 1;
                }
                parts.push((Some(*name), &outs[i + 1..j]));
                i = j + 1;
            }
            Out::C(_) => {
                parts.push((None, &outs[i..i + 1]));
                i += 1;
            }
            _ => i += 1,
        }
    }
    parts
}

/// Assigns the parts of `outs`, a match of `p`'s grammar `g`, to `p`'s
/// longhands in `assigned`.
fn split<'a>(
    p: &'static Property,
    g: &'static Grammar,
    outs: &[Out<'a>],
    assigned: &mut [Option<Vec<Out<'a>>>],
) -> Option<()> {
    let parts = segments(outs);
    let refs: Vec<&'static str> = parts.iter().filter_map(|(n, _)| *n).collect();
    if refs.is_empty() && split_by_position(p, g, outs, assigned).is_some() {
        return Some(());
    }
    let slot = |name: &str| p.longhands.iter().position(|l| *l == name);
    // `<'margin-top'>{1,4}`: one property written one to four times, over
    // four sides, or `{1,2}` over two.
    if !refs.is_empty() && refs.iter().all(|r| *r == refs[0]) && refs.len() > 1
        || (refs.len() == 1 && slot(refs[0]).is_none() && matches!(p.longhands.len(), 2 | 4))
    {
        let values: Vec<&[Out<'a>]> = parts
            .iter()
            .filter(|(n, _)| n.is_some())
            .map(|(_, o)| *o)
            .collect();
        let order: &[usize] = match (p.longhands.len(), values.len()) {
            (4, 1) => &[0, 0, 0, 0],
            (4, 2) => &[0, 1, 0, 1],
            (4, 3) => &[0, 1, 2, 1],
            (4, 4) => &[0, 1, 2, 3],
            (2, 1) => &[0, 0],
            (2, 2) => &[0, 1],
            _ => return None,
        };
        for (slot, i) in assigned.iter_mut().zip(order) {
            *slot = Some(values[*i].to_vec());
        }
        return Some(());
    }
    for (name, part) in &parts {
        let Some(name) = name else { continue };
        if let Some(i) = slot(name) {
            assigned[i] = Some(part.to_vec());
            continue;
        }
        // A shorthand inside a shorthand: `border-width` in `border`.
        let sub = property(name)?;
        if sub.is_shorthand() {
            let mut inner = vec![None; sub.longhands.len()];
            split(sub, sub.grammar()?, part, &mut inner)?;
            for (l, v) in sub.longhands.iter().zip(inner) {
                if let (Some(i), Some(v)) = (slot(l), v) {
                    assigned[i] = Some(v);
                }
            }
        }
    }
    // `<'row-gap'> <'column-gap'>?`: the second, left out, is the first.
    if let Grammar::Sequence(items) = g
        && let [
            Grammar::Property(a),
            Grammar::Repeat {
                item,
                min: 0,
                max: 1,
                ..
            },
        ] = &items[..]
        && let Grammar::Property(b) = &**item
        && p.longhands == [&*a.name, &*b.name]
        && assigned[1].is_none()
    {
        assigned[1] = assigned[0].clone();
    }
    // Values outside any property reference go to each longhand that
    // takes them: `border: 1px solid red`.
    for (name, part) in &parts {
        if name.is_some() {
            continue;
        }
        let [Out::C(c)] = part else { continue };
        if matches!(c, Component::Literal(_)) {
            continue;
        }
        let text = Specified::Value(vec![c.clone()]).to_string();
        let values = crate::parse_component_values(&text);
        let mut taken = false;
        for (i, l) in p.longhands.iter().enumerate() {
            if assigned[i].is_some() {
                continue;
            }
            let lp = property(l)?;
            if lp.is_shorthand() {
                continue;
            }
            if matches!(parse_for(lp, &values), Ok(Specified::Value(_))) {
                assigned[i] = Some(part.to_vec());
                taken = true;
            }
        }
        if !taken {
            return None;
        }
    }
    Some(())
}

/// Shorthands whose grammar is positions rather than longhands: one to
/// four values over four sides (`border-color: red blue`), one or two
/// over a pair, or two separated by `/` (`grid-column: 1 / 3`), where a
/// second left out is the first if that's a name, and initial if not.
fn split_by_position<'a>(
    p: &'static Property,
    g: &Grammar,
    outs: &[Out<'a>],
    assigned: &mut [Option<Vec<Out<'a>>>],
) -> Option<()> {
    let values: Vec<&Out<'a>> = outs.iter().filter(|o| matches!(o, Out::C(_))).collect();
    match g {
        Grammar::Repeat {
            min: 1,
            max: max @ (2 | 4),
            commas: false,
            ..
        } if p.longhands.len() == *max as usize && values.len() <= *max as usize => {
            let order: &[usize] = match (max, values.len()) {
                (4, 1) => &[0, 0, 0, 0],
                (4, 2) => &[0, 1, 0, 1],
                (4, 3) => &[0, 1, 2, 1],
                (4, 4) => &[0, 1, 2, 3],
                (2, 1) => &[0, 0],
                (2, 2) => &[0, 1],
                _ => return None,
            };
            for (slot, i) in assigned.iter_mut().zip(order) {
                *slot = Some(vec![values[*i].clone()]);
            }
            Some(())
        }
        Grammar::Sequence(items) if p.longhands.len() == 2 => {
            let [
                _,
                Grammar::Repeat {
                    item,
                    min: 0,
                    max: 1,
                    ..
                },
            ] = &items[..]
            else {
                return None;
            };
            let Grammar::Sequence(inner) = &**item else {
                return None;
            };
            if inner.first() != Some(&Grammar::Literal('/')) {
                return None;
            }
            let slash = outs
                .iter()
                .position(|o| matches!(o, Out::C(Component::Literal('/'))));
            let (first, second) = match slash {
                Some(i) => (&outs[..i], Some(&outs[i + 1..])),
                None => (outs, None),
            };
            assigned[0] = Some(first.to_vec());
            assigned[1] = match second {
                Some(s) => Some(s.to_vec()),
                None if matches!(first, [Out::C(Component::Ident(_))]) => Some(first.to_vec()),
                None => None,
            };
            Some(())
        }
        _ => None,
    }
}

/// Writes typed values as CSSOM serializes a specified value: separated
/// by spaces, commas followed by one.
pub fn write_components(out: &mut impl Write, components: &[Component<'_>]) -> fmt::Result {
    for (i, c) in components.iter().enumerate() {
        if i > 0 && *c != Component::Literal(',') {
            out.write_char(' ')?;
        }
        write!(out, "{c}")?;
    }
    Ok(())
}

impl fmt::Display for Component<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Component::Keyword(k) => f.write_str(k),
            Component::Ident(i) => write_identifier(f, i),
            Component::Number { value, .. } => write_number(f, *value),
            Component::Percentage(v) => {
                write_number(f, *v)?;
                f.write_char('%')
            }
            Component::Dimension(v, u) => {
                write_number(f, *v)?;
                f.write_str(u.as_str())
            }
            Component::Math(m) => write!(f, "{m}"),
            Component::Color(c) => write!(f, "{c}"),
            Component::String(s) => write_string(f, s),
            Component::Url(u) => {
                f.write_str("url(")?;
                write_string(f, u)?;
                f.write_char(')')
            }
            Component::Literal(c) => f.write_char(*c),
            Component::Function { name, arguments } => {
                write!(f, "{name}(")?;
                write_components(f, arguments)?;
                f.write_char(')')
            }
            Component::Block(inside) => {
                f.write_char('[')?;
                write_components(f, inside)?;
                f.write_char(']')
            }
            Component::Parens(inside) => {
                f.write_char('(')?;
                write_components(f, inside)?;
                f.write_char(')')
            }
            Component::Tokens(t) => write_values(f, trim(t)),
        }
    }
}

impl fmt::Display for Specified<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Specified::CssWide(k) => f.write_str(k.as_str()),
            Specified::Unparsed(v) | Specified::Custom(v) => write_values(f, trim(v)),
            Specified::Value(c) => write_components(f, c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written(property: &str, css: &str) -> Option<String> {
        let v = crate::parse_component_values(css);
        parse_value(property, &v).ok().map(|s| s.to_string())
    }

    #[test]
    fn shortest_forms() {
        let cases = [
            ("display", "inline flex", "inline-flex"),
            ("display", "block flow list-item", "list-item"),
            ("white-space", "preserve nowrap", "pre"),
            ("flex", "1", "1 1 0%"),
            ("flex", "none", "0 0 auto"),
            ("place-items", "center center", "center"),
            ("margin", "1px 2px 1px 2px", "1px 2px"),
            ("font", "normal bold 12px/normal serif", "bold 12px serif"),
            ("text-decoration-line", "blink underline", "underline blink"),
            (
                "background-image",
                "linear-gradient(in srgb, red, blue)",
                "linear-gradient(red, blue)",
            ),
            (
                "color",
                "color-mix(in oklab, 25% red, blue)",
                "color-mix(red 25%, blue 75%)",
            ),
            ("font-family", "Monospace", "monospace"),
            ("grid-column-start", "span 2", "span 2"),
        ];
        for (p, css, want) in cases {
            assert_eq!(written(p, css).as_deref(), Some(want), "{p}: {css}");
        }
    }

    #[test]
    fn what_every_property_takes() {
        assert_eq!(written("color", "INHERIT").as_deref(), Some("inherit"));
        assert_eq!(
            written("width", " var(--w,  10px) ").as_deref(),
            Some("var(--w, 10px)")
        );
        assert_eq!(written("--x", " { a } ").as_deref(), Some("{ a }"));
        assert_eq!(written("not-a-property", "1px"), None);
        assert_eq!(written("width", ""), None);
    }

    #[test]
    fn shorthands_split() {
        let split = |p: &str, css: &str| -> Vec<String> {
            let v = crate::parse_component_values(css);
            expand(p, &v)
                .unwrap()
                .into_iter()
                .map(|(n, v)| format!("{n}: {v}"))
                .collect()
        };
        assert_eq!(
            split("border-color", "red blue"),
            [
                "border-top-color: red",
                "border-right-color: blue",
                "border-bottom-color: red",
                "border-left-color: blue"
            ]
        );
        assert_eq!(
            split("grid-column", "a"),
            ["grid-column-start: a", "grid-column-end: a"]
        );
        assert_eq!(split("gap", "1px"), ["row-gap: 1px", "column-gap: 1px"]);
        let border = split("border-top", "1px solid");
        assert_eq!(
            border,
            [
                "border-top-width: 1px",
                "border-top-style: solid",
                "border-top-color: currentcolor"
            ]
        );
        assert!(
            split("margin", "inherit")
                .iter()
                .all(|l| l.ends_with(": inherit"))
        );
    }
}
