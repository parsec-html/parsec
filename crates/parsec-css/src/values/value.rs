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
        // By length first: most words are none of these.
        let k = match word.len() {
            7 if word.eq_ignore_ascii_case("initial") => CssWide::Initial,
            7 if word.eq_ignore_ascii_case("inherit") => CssWide::Inherit,
            5 if word.eq_ignore_ascii_case("unset") => CssWide::Unset,
            6 if word.eq_ignore_ascii_case("revert") => CssWide::Revert,
            12 if word.eq_ignore_ascii_case("revert-layer") => CssWide::RevertLayer,
            _ => return None,
        };
        Some(k)
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
    if let [ComponentValue::Token(Token::Ident(w), _)] = value {
        let Some(lone) = lone(p, w) else {
            if !p.takes_names() {
                return Err(InvalidValue);
            }
            return parse_matched(p, value);
        };
        return match lone {
            Lone::Invalid => Err(InvalidValue),
            Lone::Keywords(k) => Ok(Specified::Value(
                k.iter().map(|k| Component::Keyword(k)).collect(),
            )),
            Lone::Other => parse_matched(p, value),
        };
    }
    if has_substitution(value) {
        return Ok(Specified::Unparsed(value.to_vec()));
    }
    if p.name == "font-family"
        && let Some(found) = font_family(value)
    {
        return found;
    }
    // One token, `1px` or `#fff`, for a grammar of plain alternatives.
    if let [v @ ComponentValue::Token(..)] = value {
        let f = flags(p);
        if !f.form
            && !f.restricted
            && let Some(g) = p.grammar()
            && let Some(found) = super::matcher::single(g, v)
        {
            return found.map(|c| Specified::Value(vec![c])).ok_or(InvalidValue);
        }
    }
    parse_matched(p, value)
}

/// `font-family` without the matcher, which takes a while over a long
/// list: each name a string, identifiers, or a generic family's keyword,
/// as CSS Fonts 4 §2.1 says. `None` for anything else, `generic(kai)` say.
fn font_family<'a>(value: &[ComponentValue<'a>]) -> Option<Result<Specified<'a>, InvalidValue>> {
    use super::restrict::{CSS_WIDE, GENERIC_FAMILIES};
    let mut c = Vec::with_capacity(value.len());
    for item in value.split(|v| matches!(v, ComponentValue::Token(Token::Comma, _))) {
        if !c.is_empty() {
            c.push(Component::Literal(','));
        }
        let mut words = item.iter().filter(|v| !v.is_whitespace()).peekable();
        match words.peek() {
            None => return Some(Err(InvalidValue)),
            Some(ComponentValue::Token(Token::String(s), _)) => {
                c.push(Component::String(s.clone()));
                words.next();
                if words.next().is_some() {
                    return Some(Err(InvalidValue));
                }
            }
            Some(_) => {
                let start = c.len();
                for w in words {
                    let ComponentValue::Token(Token::Ident(s), _) = w else {
                        return None;
                    };
                    c.push(Component::Ident(s.clone()));
                }
                // One word that's a generic family is its keyword; a
                // generic family or a CSS-wide keyword in a name is no
                // name at all.
                let generic = |x: &Component<'_>| {
                    GENERIC_FAMILIES
                        .iter()
                        .find(|g| matches!(x, Component::Ident(s) if s.eq_ignore_ascii_case(g)))
                };
                if c.len() - start == 1
                    && let Some(g) = generic(&c[start])
                {
                    c[start] = Component::Keyword(g);
                    continue;
                }
                let reserved = c[start..].iter().any(|x| {
                    generic(x).is_some()
                        || matches!(x, Component::Ident(s) if CSS_WIDE.iter().any(|w| s.eq_ignore_ascii_case(w)))
                });
                if reserved {
                    return Some(Err(InvalidValue));
                }
            }
        }
    }
    super::canonical::form("font-family", &mut c);
    Some(Ok(Specified::Value(c)))
}

/// What [`parse_matched`] has to do for a property, worked out once.
#[derive(Clone, Copy)]
struct Flags {
    keep_order: bool,
    restricted: bool,
    form: bool,
}

fn flags(p: &'static Property) -> Flags {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<OnceLock<Flags>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        super::properties::properties()
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let work = || {
        let name = p.alias_of.unwrap_or(p.name);
        Flags {
            keep_order: super::canonical::KEEP_ORDER.contains(&name),
            restricted: super::restrict::PROPERTIES.contains(&name),
            form: super::canonical::has_form(name),
        }
    };
    match p.position().and_then(|i| all.get(i)) {
        Some(cell) => *cell.get_or_init(work),
        None => work(),
    }
}

/// What one of a property's keywords alone parses to.
enum Lone {
    Invalid,
    Keywords(Vec<&'static str>),
    /// Something else, `0 0 auto` for `flex: none`: worked out each time.
    Other,
}

type LoneTable = std::collections::HashMap<&'static [u8], Lone, super::properties::FnvBuild>;

/// A value of one keyword, worked out the long way once for each keyword
/// of each property, the first time the property's value is parsed: most
/// declarations are a keyword alone (`display: none`), and its grammar
/// can take many steps to say so. `None` for a word that's no keyword of
/// the property's, or one that parses to something else, a name.
fn lone(p: &'static Property, word: &str) -> Option<&'static Lone> {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<OnceLock<LoneTable>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        super::properties::properties()
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let table = all.get(p.position()?)?.get_or_init(|| {
        p.keywords()
            .iter()
            .map(|k| {
                let values = crate::parse_component_values(k);
                let lone = match parse_matched(p, &values) {
                    Err(_) => Lone::Invalid,
                    Ok(Specified::Value(c)) => c
                        .iter()
                        .map(|c| match c {
                            Component::Keyword(k) => Some(*k),
                            _ => None,
                        })
                        .collect::<Option<_>>()
                        .map_or(Lone::Other, Lone::Keywords),
                    Ok(_) => Lone::Other,
                };
                (k.as_bytes(), lone)
            })
            .collect()
    });
    // Lowercased as bytes: no need to check it's still UTF-8.
    let mut buf = [0u8; 32];
    let out = buf.get_mut(..word.len())?;
    out.copy_from_slice(word.as_bytes());
    out.make_ascii_lowercase();
    table.get(&*out)
}

/// [`parse_for`] after its shortcuts: the value matched against the
/// grammar.
fn parse_matched<'a>(
    p: &'static Property,
    value: &[ComponentValue<'a>],
) -> Result<Specified<'a>, InvalidValue> {
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
    let name = p.alias_of.unwrap_or(p.name);
    let f = flags(p);
    let outs = canonical(m.out, f.keep_order);
    let mut c = match same_pair(g, &outs) {
        Some(first) => components(first),
        None => components(outs),
    };
    if f.restricted && !super::restrict::property(name, value, &c) {
        return Err(InvalidValue);
    }
    shortest(g, &mut c);
    if f.form {
        super::canonical::form(name, &mut c);
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
        Grammar::Property(_) | Grammar::Type(_),
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
    if let (Grammar::Type(a), Grammar::Type(b)) = (&items[0], &**item)
        && a == b
        && single(&items[0])
    {
        // `<inset-value> <inset-value>?`: the second left out is the first.
        let values: Vec<&Out<'a>> = outs.iter().filter(|o| matches!(o, Out::C(_))).collect();
        if let [Out::C(x), Out::C(y)] = values[..]
            && x == y
        {
            return Some(vec![Out::C(x.clone())]);
        }
        return None;
    }
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
    if !single(item) || c.len() > *max as usize {
        return;
    }
    super::canonical::sides(c);
}

/// [`shortest`] for a property by name.
pub(crate) fn shortest_of(name: &str, c: &mut Vec<Component<'_>>) {
    if let Some(g) = property(name).and_then(|p| p.grammar()) {
        shortest(g, c);
    }
}

/// Whether a grammar always matches exactly one value.
fn single(g: &Grammar) -> bool {
    match g {
        Grammar::Keyword(_) | Grammar::Property(_) | Grammar::Function(..) => true,
        Grammar::Type(t) => !matches!(&*t.name, "position" | "bg-position" | "track-list"),
        Grammar::One(alts) => alts.iter().all(single),
        _ => false,
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
    Some(components(canonical(m.out, false)))
}

/// Parses a declaration's value for its property.
pub fn parse_declaration_value<'a>(d: &Declaration<'a>) -> Result<Specified<'a>, InvalidValue> {
    parse_value(&d.name, &d.value)
}

/// Parses a shorthand's value and splits it into a value for each of its
/// longhands, in the order the spec lists them; a longhand the value
/// leaves out gets its initial value. A longhand that is itself a
/// shorthand (`border-width` in `border`) is followed by its own. A
/// longhand gives itself.
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
        other => {
            let mut leaves = Vec::new();
            leaf_longhands(p, &mut leaves);
            return Ok(leaves.into_iter().map(|l| (l, other.clone())).collect());
        }
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
    let outs = canonical(m.out, false);
    let mut result = Vec::with_capacity(p.longhands.len());
    assign(p, g, &outs, &mut result)?;
    // CSS Masking 1 §7.10 and CSS Backgrounds 3 §4.4: `mask` resets
    // `mask-border` too, and `border` `border-image`.
    let resets = match p.name {
        "mask" => Some("mask-border"),
        "border" => Some("border-image"),
        _ => None,
    };
    if let Some(border) = resets.and_then(property) {
        result.push((border.name, initial(border.name)));
        result.extend(border.longhands.iter().map(|l| (*l, initial(l))));
    }
    Ok(result)
}

/// Splits a match of shorthand `p`'s grammar `g` into its longhands, and
/// those that are shorthands into theirs after them, onto `result`.
fn assign<'a>(
    p: &'static Property,
    g: &'static Grammar,
    outs: &[Out<'a>],
    result: &mut Vec<(&'static str, Specified<'a>)>,
) -> Result<(), InvalidValue> {
    let mut assigned: Vec<Option<Vec<Out<'a>>>> = vec![None; p.longhands.len()];
    // Lists of layers, `background` or `transition`, layer by layer.
    let listed = matches!(g, Grammar::Repeat { commas: true, .. }) || p.name == "background";
    let by_layer = listed && (p.longhands.len() > 2 || p.name == "animation-range");
    if by_layer || split(p, g, outs, &mut assigned).is_none() {
        assigned = layers(p, outs).ok_or(InvalidValue)?;
    }
    for (name, part) in p.longhands.iter().zip(assigned) {
        let Some(lp) = property(name) else { continue };
        if lp.is_shorthand() {
            // The shorthand itself, then its own longhands.
            match (part, lp.grammar()) {
                (Some(outs), Some(sg)) => {
                    let mut c = components(outs.clone());
                    super::canonical::property(name, &mut c);
                    result.push((*name, Specified::Value(c)));
                    assign(lp, sg, &outs, result)?
                }
                _ => {
                    result.push((*name, initial(name)));
                    let mut leaves = Vec::new();
                    leaf_longhands(lp, &mut leaves);
                    result.extend(leaves.into_iter().map(|l| (l, initial(l))));
                }
            }
            continue;
        }
        let value = match part {
            Some(outs) => {
                let mut c = components(outs);
                shortest_of(name, &mut c);
                super::canonical::property(name, &mut c);
                Specified::Value(c)
            }
            None => initial(name),
        };
        result.push((*name, value));
    }
    Ok(())
}

/// A shorthand's longhands, those that are shorthands by theirs.
fn leaf_longhands(p: &'static Property, out: &mut Vec<&'static str>) {
    for l in p.longhands {
        out.push(l);
        if let Some(lp) = property(l)
            && lp.is_shorthand()
        {
            leaf_longhands(lp, out);
        }
    }
}

/// Splits a comma-separated list of layers, `background` or `transition`
/// say, by finding for each run of values in a layer the longhand that
/// takes it, longest runs first, in the longhands' order; what follows a
/// `/` goes first. A longhand with no list (`background-color`) takes its
/// value from the layer that has it.
fn layers<'a>(p: &'static Property, outs: &[Out<'a>]) -> Option<Vec<Option<Vec<Out<'a>>>>> {
    let all = components(outs.to_vec());
    let n = p.longhands.len();
    let mut per: Vec<Vec<Option<Vec<Component<'a>>>>> = Vec::new();
    for layer in all.split(|c| *c == Component::Literal(',')) {
        let mut got: Vec<Option<Vec<Component<'a>>>> = vec![None; n];
        let mut rest: Vec<Component<'a>> = layer.to_vec();
        let mut slashed = None;
        if let Some(s) = rest.iter().position(|c| *c == Component::Literal('/')) {
            let after = rest.split_off(s);
            slashed = Some(after[1..].to_vec());
        }
        if let Some(after) = slashed {
            // The run before the `/` and the run after it, for the two
            // longhands that take them.
            let (i, before) = take_run(p, &mut rest, &got, true)?;
            got[i] = Some(before);
            let mut after = after;
            let (j, run) = take_run(p, &mut after, &got, false)?;
            got[j] = Some(run);
            if !after.is_empty() {
                rest.extend(after);
            }
        }
        while !rest.is_empty() {
            let (i, run) = take_run(p, &mut rest, &got, false)?;
            got[i] = Some(run);
        }
        // Scroll Animations 1 §4.4: an end left out is the start's range
        // name, at its own default, or `normal`.
        if p.name == "animation-range"
            && got[1].is_none()
            && let Some(start) = &got[0]
            && let Some(Component::Keyword(name)) = start.first()
            && *name != "normal"
        {
            got[1] = Some(vec![Component::Keyword(name)]);
        }
        // One box is both the origin and the clip.
        let origin = p.longhands.iter().position(|l| l.ends_with("-origin"));
        let clip = p.longhands.iter().position(|l| l.ends_with("-clip"));
        if let (Some(o), Some(c)) = (origin, clip)
            && got[c].is_none()
            && let Some(b) = got[o].clone()
        {
            got[c] = Some(b);
        }
        per.push(got);
    }
    let mut out = vec![None; n];
    for (i, name) in p.longhands.iter().enumerate() {
        let listed = property(name).and_then(|lp| lp.grammar()).is_some_and(|g| {
            let list = |g: &Grammar| matches!(g, Grammar::Repeat { commas: true, .. });
            match g {
                // `none | <single-transition-property>#`
                Grammar::One(alts) => alts.iter().any(list),
                g => list(g),
            }
        });
        if !listed {
            out[i] = per
                .iter()
                .rev()
                .find_map(|l| l[i].clone())
                .map(|c| c.into_iter().map(Out::C).collect());
            continue;
        }
        let mut joined = Vec::new();
        for (k, l) in per.iter().enumerate() {
            if k > 0 {
                joined.push(Out::C(Component::Literal(',')));
            }
            match &l[i] {
                Some(c) => joined.extend(c.iter().cloned().map(Out::C)),
                None => joined.extend(initial_components(name)?.into_iter().map(Out::C)),
            }
        }
        out[i] = Some(joined);
    }
    Some(out)
}

/// The longest run at the start of `rest` (or all of it, for `whole`)
/// that a longhand not yet given a value takes, and which one.
fn take_run<'a>(
    p: &'static Property,
    rest: &mut Vec<Component<'a>>,
    got: &[Option<Vec<Component<'a>>>],
    whole: bool,
) -> Option<(usize, Vec<Component<'a>>)> {
    // No longhand takes more than a few values (a position takes four), so
    // runs are capped: a long hostile layer stays linear.
    let ends: Vec<usize> = if whole {
        vec![rest.len()]
    } else {
        (1..=rest.len().min(8)).rev().collect()
    };
    for end in ends {
        let run = &rest[..end];
        let text = Specified::Value(run.to_vec()).to_string();
        let values = crate::parse_component_values(&text);
        for (i, name) in p.longhands.iter().enumerate() {
            if got[i].is_some() {
                continue;
            }
            let Some(lp) = property(name) else { continue };
            let Ok(Specified::Value(c)) = parse_for(lp, &values) else {
                continue;
            };
            // A keyword stays one: `paused` isn't an animation's name.
            let renamed = run.iter().any(|r| match r {
                Component::Keyword(k) => c
                    .iter()
                    .any(|x| matches!(x, Component::Ident(s) if s.eq_ignore_ascii_case(k))),
                _ => false,
            });
            if renamed || c.contains(&Component::Literal(',')) {
                continue;
            }
            let run: Vec<Component<'a>> = rest.drain(..end).collect();
            return Some((i, run));
        }
    }
    None
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
        "border-radius" | "grid-area" | "grid-row" | "grid-column" | "place-content" => {
            let Ok(Specified::Value(c)) = parse_for(p, value) else {
                return None;
            };
            // The value before its shortest form: what each part says.
            let c = match p.name {
                "place-content" => c,
                _ => {
                    let g = p.grammar()?;
                    let mut m = Matcher::new(value).for_property(p);
                    if !m.whole(g) {
                        m = Matcher::new(value);
                        m.whole(g);
                    }
                    components(canonical(m.out, false))
                }
            };
            let parts: Vec<Vec<Component<'a>>> = c
                .split(|x| *x == Component::Literal('/'))
                .map(<[_]>::to_vec)
                .collect();
            let value = |c: Vec<Component<'a>>| Specified::Value(c);
            match p.name {
                // CSS Backgrounds 3 §5.1: one to four corners, then as many
                // vertical radii, the same if left out.
                "border-radius" => {
                    let corner = |part: &[Component<'a>], i: usize| -> Option<Component<'a>> {
                        let at = match (part.len(), i) {
                            (1, _) => 0,
                            (2, i) => i % 2,
                            (3, 3) => 1,
                            (3, i) => i,
                            (4, i) => i,
                            _ => return None,
                        };
                        part.get(at).cloned()
                    };
                    let v = parts.get(1).unwrap_or(&parts[0]);
                    (0..4)
                        .map(|i| {
                            let (a, b) = (corner(&parts[0], i)?, corner(v, i)?);
                            let c = if a == b { vec![a] } else { vec![a, b] };
                            Some((p.longhands[i], value(c)))
                        })
                        .collect()
                }
                // CSS Box Alignment §5.1: with a baseline, `start` across.
                "place-content" => {
                    // In its shortest form, the baseline and then what's
                    // across: `start` if nothing was given.
                    let at = c
                        .iter()
                        .position(|x| *x == Component::Keyword("baseline"))?;
                    let mut align = c;
                    let mut justify = align.split_off(at + 1);
                    if justify.is_empty() {
                        justify.push(Component::Keyword("start"));
                    }
                    Some(vec![
                        (p.longhands[0], value(align)),
                        (p.longhands[1], value(justify)),
                    ])
                }
                // CSS Grid 2 §8.4: a line left out is the one it pairs
                // with if that's a name, `auto` if not.
                _ => {
                    let default = |of: &Vec<Component<'a>>| match &of[..] {
                        [Component::Ident(_)] => of.clone(),
                        _ => vec![Component::Keyword("auto")],
                    };
                    let n = p.longhands.len();
                    let mut lines: Vec<Vec<Component<'a>>> = parts.clone();
                    let pairs: &[usize] = if n == 4 { &[0, 0, 0, 1] } else { &[0, 0] };
                    while lines.len() < n {
                        let i = lines.len();
                        let d = default(&lines[pairs[i]]);
                        lines.push(d);
                    }
                    // `grid-area`'s order is row start, column start, row
                    // end, column end; its longhands' differs.
                    Some(
                        p.longhands
                            .iter()
                            .enumerate()
                            .map(|(i, l)| {
                                let at = match (n, *l) {
                                    (4, "grid-row-start") => 0,
                                    (4, "grid-column-start") => 1,
                                    (4, "grid-row-end") => 2,
                                    (4, "grid-column-end") => 3,
                                    _ => i,
                                };
                                let mut c = lines[at].clone();
                                super::canonical::span_one(&mut c);
                                (*l, value(c))
                            })
                            .collect(),
                    )
                }
            }
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
                    let outs = canonical(m.out, false);
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

/// A longhand's initial value as its shortest form writes it, worked out
/// once: what a shorthand leaves out when a longhand has it.
pub(crate) fn initial_text(name: &str) -> Option<&'static str> {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<OnceLock<Option<String>>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        super::properties::properties()
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let p = property(name)?;
    let cell = all.get(p.position()?)?;
    cell.get_or_init(|| {
        if p.is_shorthand() {
            // Its longhands', in order: `0% 0%` for `background-position`.
            let parts: Option<Vec<&str>> = p.longhands.iter().map(|l| initial_text(l)).collect();
            let text = parts?.join(" ");
            let values = crate::parse_component_values(&text);
            return match parse_for(p, &values) {
                Ok(v @ Specified::Value(_)) => Some(v.to_string()),
                _ => None,
            };
        }
        match initial(p.name) {
            v @ Specified::Value(_) => Some(v.to_string()),
            _ => p.initial.map(str::to_ascii_lowercase),
        }
    })
    .as_deref()
}

/// A longhand's initial value, typed, worked out once.
pub(crate) fn initial_cached(name: &str) -> Option<&'static [Component<'static>]> {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<OnceLock<Option<Vec<Component<'static>>>>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        super::properties::properties()
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let p = property(name)?;
    all.get(p.position()?)?
        .get_or_init(|| initial_components(p.name))
        .as_deref()
}

/// A longhand's initial value, typed, if the spec gives one as a value.
pub(crate) fn initial_components(name: &str) -> Option<Vec<Component<'static>>> {
    match initial(name) {
        Specified::Value(c) => Some(c),
        _ => None,
    }
}

/// A longhand's initial value, parsed, or `initial` where the spec's is
/// prose. The text is the table's, so the value borrows nothing.
fn initial(name: &str) -> Specified<'static> {
    let parsed = property(name).and_then(|p| {
        let text = match p.is_shorthand() {
            true => initial_text(p.name)?,
            false => p.initial?,
        };
        let values = crate::parse_component_values(text);
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
            Out::Start(p) => {
                let name = &p.name;
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
    let repeat_of_one = matches!(g, Grammar::Repeat { item, min: 1, max: 2 | 4, commas: false }
        if matches!(**item, Grammar::Property(_)));
    if !refs.is_empty() && refs.iter().all(|r| *r == refs[0]) && (refs.len() > 1 || repeat_of_one)
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
    // `rule: <'column-rule'>`: one value for each of its longhands.
    if let Grammar::Property(_) = g
        && let [(Some(_), part)] = &parts[..]
    {
        for slot in assigned.iter_mut() {
            *slot = Some(part.to_vec());
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
    fn font_family_without_the_matcher() {
        let p = property("font-family").unwrap();
        let cases = [
            "a, b",
            "\"New Century Schoolbook\", serif",
            "Serif",
            "cursive serif",
            "x, inherit",
            "a b c, 'd'",
            "MONOSPACE, ui-serif",
            ",a",
            "a,",
            "\"a\" b",
            "default",
            "revert-rule",
            "\"serif\"",
            "Emoji",
        ];
        for css in cases {
            let v = crate::parse_component_values(css);
            let v = trim(&v);
            let fast = font_family(v).expect(css).map(|s| s.to_string());
            let slow = parse_matched(p, v).map(|s| s.to_string());
            assert_eq!(fast, slow, "{css}");
        }
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
