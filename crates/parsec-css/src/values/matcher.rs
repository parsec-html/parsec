// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Matching component values against a [`Grammar`]: a backtracking
//! matcher, so `<length>{1,4}` or `a || b || c` find a way through the
//! input if there is one, with the values it matched typed as it goes.
//!
//! The primitive types (numbers, dimensions and their math functions,
//! colors, strings, URLs, identifiers) are matched here; every other type
//! and function is matched by its grammar from the specs. A step budget
//! keeps a hostile value from taking exponential time: past it, the value
//! is invalid.

use super::color::Color;
use super::grammar::{Grammar, TypeRef};
use super::math::{Expect, Math};
use super::properties::{function_grammar, property, type_grammar};
use super::units::{Kind, Unit};
use super::value::Component;
use crate::parser::{Bracket, ComponentValue};
use crate::tokenizer::Token;

/// How many matching steps one value may take.
const BUDGET: usize = 200_000;
/// How deep grammars may refer to grammars.
const MAX_DEPTH: usize = 48;

/// What a match produces: typed values, with markers around the parts
/// that matched another property's grammar, which is how a shorthand is
/// split into its longhands.
#[derive(Clone, Debug)]
pub(crate) enum Out<'a> {
    C(Component<'a>),
    Start(&'static str),
    End,
    /// Around what one alternative of a `||` or `&&` matched: which group
    /// and which alternative, for writing them in the grammar's order.
    Alt(usize, usize, bool),
    AltEnd,
}

/// One thing in a match, with what it contains.
enum Tree<'a> {
    Leaf(Out<'a>),
    Group(Out<'a>, Vec<Tree<'a>>),
}

fn tree<'a>(outs: &mut std::iter::Peekable<std::vec::IntoIter<Out<'a>>>) -> Vec<Tree<'a>> {
    let mut items = Vec::new();
    while let Some(o) = outs.next() {
        match o {
            Out::End | Out::AltEnd => break,
            o @ (Out::Start(_) | Out::Alt(..)) => {
                let inner = tree(outs);
                items.push(Tree::Group(o, inner));
            }
            o => items.push(Tree::Leaf(o)),
        }
    }
    items
}

fn flatten<'a>(items: Vec<Tree<'a>>, out: &mut Vec<Out<'a>>) {
    let mut items = items;
    // Runs of one group's alternatives go in the grammar's order, which
    // is the canonical order CSSOM writes them in.
    let mut i = 0;
    while i < items.len() {
        let set = match &items[i] {
            Tree::Group(Out::Alt(set, _, _), _) => *set,
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i;
        while matches!(items.get(j), Some(Tree::Group(Out::Alt(s, _, _), _)) if *s == set) {
            j += 1;
        }
        items[i..j].sort_by_key(|t| match t {
            Tree::Group(Out::Alt(_, idx, _), _) => *idx,
            _ => 0,
        });
        // In a `||` group, what says only the default is left out, as the
        // shortest serialization does: `font: normal bold 1em x` is
        // `bold 1em x`. If that's all of it, the first stays.
        let any = matches!(&items[i], Tree::Group(Out::Alt(_, _, true), _));
        if any && j - i > 1 {
            let defaults: Vec<bool> = items[i..j].iter().map(is_default).collect();
            if defaults.iter().any(|d| !d) {
                let mut k = i;
                for d in defaults {
                    if d {
                        items.remove(k);
                        j -= 1;
                    } else {
                        k += 1;
                    }
                }
            }
        }
        i = j;
    }
    // `/ normal` for a line height left at its initial value, likewise.
    let mut i = 0;
    while i + 1 < items.len() {
        if matches!(items[i], Tree::Leaf(Out::C(Component::Literal('/'))))
            && let Tree::Group(Out::Start(p), inner) = &items[i + 1]
            && is_initial(p, inner)
            && text(inner) == "normal"
        {
            items.drain(i..i + 2);
            continue;
        }
        i += 1;
    }
    for t in items {
        match t {
            Tree::Leaf(o) => out.push(o),
            Tree::Group(Out::Alt(..), inner) => flatten(inner, out),
            Tree::Group(start, inner) => {
                out.push(start);
                flatten(inner, out);
                out.push(Out::End);
            }
        }
    }
}

/// The values in a part of a match, written out.
fn text(items: &[Tree<'_>]) -> String {
    fn collect<'t, 'a>(items: &'t [Tree<'a>], out: &mut Vec<&'t Component<'a>>) {
        for t in items {
            match t {
                Tree::Leaf(Out::C(c)) => out.push(c),
                Tree::Group(_, inner) => collect(inner, out),
                _ => {}
            }
        }
    }
    let mut cs = Vec::new();
    collect(items, &mut cs);
    let owned: Vec<Component<'_>> = cs.into_iter().cloned().collect();
    let mut s = String::new();
    super::value::write_components(&mut s, &owned).unwrap();
    s
}

fn is_initial(property: &str, inner: &[Tree<'_>]) -> bool {
    super::properties::property(property)
        .and_then(|p| p.initial)
        .is_some_and(|i| i.eq_ignore_ascii_case(&text(inner)))
}

/// Whether one alternative of a `||` group is its default, `normal`.
fn is_default(t: &Tree<'_>) -> bool {
    let Tree::Group(_, inner) = t else {
        return false;
    };
    match &inner[..] {
        [Tree::Leaf(Out::C(Component::Keyword("normal")))] => true,
        // Only `normal`: other initial values, `auto` in `flex` or `none`
        // in `list-style`, are written by browsers.
        [Tree::Group(Out::Start(p), value)] => is_initial(p, value) && text(value) == "normal",
        _ => false,
    }
}

/// The match with every `||` and `&&` group in the grammar's order, and
/// the markers for them gone.
pub(crate) fn canonical(outs: Vec<Out<'_>>) -> Vec<Out<'_>> {
    if !outs.iter().any(|o| matches!(o, Out::Alt(..))) {
        return outs;
    }
    let mut it = outs.into_iter().peekable();
    let items = tree(&mut it);
    let mut out = Vec::new();
    flatten(items, &mut out);
    out
}

pub(crate) struct Matcher<'v, 'a> {
    input: Input<'v, 'a>,
    pub out: Vec<Out<'a>>,
    steps: usize,
    depth: usize,
    groups: usize,
    /// Keywords a `<custom-ident>` can't be here.
    reserved: Option<&'static std::collections::HashSet<&'static str>>,
}

/// The values being matched, whitespace aside: the slice itself when
/// there's none in it, as for most one-word values, so nothing's copied.
enum Input<'v, 'a> {
    Slice(&'v [ComponentValue<'a>]),
    Filtered(Vec<&'v ComponentValue<'a>>),
}

impl<'v, 'a> Input<'v, 'a> {
    fn new(values: &'v [ComponentValue<'a>]) -> Input<'v, 'a> {
        if values.iter().any(ComponentValue::is_whitespace) {
            Input::Filtered(values.iter().filter(|v| !v.is_whitespace()).collect())
        } else {
            Input::Slice(values)
        }
    }

    #[inline]
    fn get(&self, i: usize) -> Option<&'v ComponentValue<'a>> {
        match self {
            Input::Slice(s) => s.get(i),
            Input::Filtered(v) => v.get(i).copied(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Input::Slice(s) => s.len(),
            Input::Filtered(v) => v.len(),
        }
    }
}

type K<'k, 'v, 'a> = &'k mut dyn FnMut(&mut Matcher<'v, 'a>, usize) -> bool;

const CSS_WIDE: [&str; 6] = [
    "initial",
    "inherit",
    "unset",
    "revert",
    "revert-layer",
    "default",
];

impl<'v, 'a> Matcher<'v, 'a> {
    pub fn new(values: &'v [ComponentValue<'a>]) -> Matcher<'v, 'a> {
        Matcher {
            input: Input::new(values),
            out: Vec::with_capacity(8),
            steps: 0,
            depth: 0,
            groups: 0,
            reserved: None,
        }
    }

    /// Matching a value of `p`: its grammar's keywords aren't identifiers.
    pub fn for_property(mut self, p: &'static super::properties::PropertyData) -> Self {
        self.reserved = Some(p.keywords());
        self
    }

    /// Matches all of the input against `g`.
    pub fn whole(&mut self, g: &'static Grammar) -> bool {
        let len = self.input.len();
        self.m(g, 0, &mut |_, p| p == len)
    }

    fn emit(&mut self, c: Component<'a>, pos: usize, k: K<'_, 'v, 'a>) -> bool {
        self.out.push(Out::C(c));
        if k(self, pos) {
            return true;
        }
        self.out.pop();
        false
    }

    fn m(&mut self, g: &'static Grammar, pos: usize, k: K<'_, 'v, 'a>) -> bool {
        self.steps += 1;
        if self.steps > BUDGET {
            return false;
        }
        match g {
            Grammar::Never => false,
            Grammar::Keyword(w) => match self.input.get(pos) {
                Some(ComponentValue::Token(Token::Ident(i), _)) if i.eq_ignore_ascii_case(w) => {
                    self.emit(Component::Keyword(w), pos + 1, k)
                }
                _ => false,
            },
            Grammar::Literal(c) => {
                let ok = match self.input.get(pos) {
                    Some(ComponentValue::Token(t, _)) => match (c, t) {
                        (',', Token::Comma) | (':', Token::Colon) | (';', Token::Semicolon) => true,
                        (c, Token::Delim(d)) => c == d,
                        _ => false,
                    },
                    _ => false,
                };
                ok && self.emit(Component::Literal(*c), pos + 1, k)
            }
            Grammar::Type(t) => self.typed(t, pos, k),
            Grammar::Property(r) => {
                let found = r
                    .target
                    .get_or_init(|| property(&r.name).and_then(|p| Some((p, p.grammar()?))));
                let Some((p, g)) = *found else {
                    return false;
                };
                self.nested(|s| {
                    s.out.push(Out::Start(p.name));
                    let ok = s.m(g, pos, &mut |s, q| {
                        s.out.push(Out::End);
                        if k(s, q) {
                            return true;
                        }
                        s.out.pop();
                        false
                    });
                    if !ok {
                        s.out.pop();
                    }
                    ok
                })
            }
            Grammar::Function(name, inner) => {
                let Some(ComponentValue::Function(f)) = self.input.get(pos) else {
                    return false;
                };
                if !f.name.eq_ignore_ascii_case(name) {
                    return false;
                }
                let Some(mut arguments) = self.inside(&f.arguments, inner) else {
                    return false;
                };
                if name.ends_with("gradient") {
                    tidy_gradient(&mut arguments);
                } else if **name == *"color-mix" || **name == *"palette-mix" {
                    tidy_mix(&mut arguments, **name == *"color-mix");
                } else if **name == *"color-layers"
                    && arguments.first() == Some(&Component::Keyword("normal"))
                    && arguments.get(1) == Some(&Component::Literal(','))
                {
                    // `normal` is the default blend mode.
                    arguments.drain(..2);
                }
                let c = Component::Function { name, arguments };
                self.emit(c, pos + 1, k)
            }
            Grammar::SquareBlock(inner) | Grammar::ParenBlock(inner) => {
                let want = match g {
                    Grammar::SquareBlock(_) => Bracket::Square,
                    _ => Bracket::Paren,
                };
                let Some(ComponentValue::Block(b)) = self.input.get(pos) else {
                    return false;
                };
                if b.bracket != want {
                    return false;
                }
                let Some(inside) = self.inside(&b.contents, inner) else {
                    return false;
                };
                let c = match want {
                    Bracket::Square => Component::Block(inside),
                    _ => Component::Parens(inside),
                };
                self.emit(c, pos + 1, k)
            }
            Grammar::Sequence(items) => self.sequence(items, pos, true, k),
            Grammar::One(alts) => {
                for alt in alts {
                    let mark = self.out.len();
                    if self.m(alt, pos, k) {
                        return true;
                    }
                    self.out.truncate(mark);
                }
                false
            }
            Grammar::All(alts) | Grammar::Any(alts) => {
                self.groups += 1;
                let set = self.groups;
                self.permute(alts, 0, pos, matches!(g, Grammar::All(_)), set, k)
            }
            Grammar::Repeat {
                item,
                min,
                max,
                commas,
            } => self.repeat(item, *min, *max, *commas, 0, pos, k),
            Grammar::NonEmpty(inner) => self.m(inner, pos, &mut |s, q| q > pos && k(s, q)),
        }
    }

    fn nested(&mut self, f: impl FnOnce(&mut Self) -> bool) -> bool {
        if self.depth >= MAX_DEPTH {
            return false;
        }
        self.depth += 1;
        let ok = f(self);
        self.depth -= 1;
        ok
    }

    /// Matches a function's arguments or a block's contents, all of them,
    /// as one unit: the first way through is the one kept.
    fn inside(
        &mut self,
        values: &'v [ComponentValue<'a>],
        g: &'static Grammar,
    ) -> Option<Vec<Component<'a>>> {
        if self.depth >= MAX_DEPTH {
            return None;
        }
        let mut sub = Matcher::new(values);
        sub.reserved = self.reserved;
        sub.steps = self.steps;
        sub.depth = self.depth + 1;
        let ok = sub.whole(g);
        self.steps = sub.steps;
        ok.then(|| {
            canonical(sub.out)
                .into_iter()
                .filter_map(|o| match o {
                    Out::C(c) => Some(c),
                    _ => None,
                })
                .collect()
        })
    }

    /// Juxtaposed items, in order. A comma next to an item that matched
    /// nothing may be left out, as Values 4 §2.6 says: `[ a? , b ]`
    /// matches `b`.
    fn sequence(
        &mut self,
        items: &'static [Grammar],
        pos: usize,
        prev_empty: bool,
        k: K<'_, 'v, 'a>,
    ) -> bool {
        let Some((first, rest)) = items.split_first() else {
            return k(self, pos);
        };
        if *first == Grammar::Literal(',') {
            let mark = self.out.len();
            if self.m(first, pos, &mut |s, q| s.sequence(rest, q, false, k)) {
                return true;
            }
            self.out.truncate(mark);
            if prev_empty {
                return self.sequence(rest, pos, true, k);
            }
            // Before items that all match nothing.
            return self.sequence(rest, pos, true, &mut |s, q| q == pos && k(s, q));
        }
        self.m(first, pos, &mut |s, q| s.sequence(rest, q, q == pos, k))
    }

    #[allow(clippy::too_many_arguments)]
    fn permute(
        &mut self,
        alts: &'static [Grammar],
        used: u64,
        pos: usize,
        all: bool,
        set: usize,
        k: K<'_, 'v, 'a>,
    ) -> bool {
        for (i, alt) in alts.iter().enumerate().take(64) {
            if used & (1 << i) != 0 {
                continue;
            }
            let mark = self.out.len();
            let next = used | (1 << i);
            self.out.push(Out::Alt(set, i, !all));
            if self.m(alt, pos, &mut |s, q| {
                if q <= pos {
                    return false;
                }
                s.out.push(Out::AltEnd);
                if s.permute(alts, next, q, all, set, k) {
                    return true;
                }
                s.out.pop();
                false
            }) {
                return true;
            }
            self.out.truncate(mark);
        }
        let full = used.count_ones() as usize == alts.len().min(64);
        if (all && full) || (!all && used != 0) {
            return k(self, pos);
        }
        // `&&` and `||` items that can match nothing count as matched.
        if all
            && alts
                .iter()
                .enumerate()
                .all(|(i, a)| used & (1 << i) != 0 || optional(a))
        {
            return k(self, pos);
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &mut self,
        item: &'static Grammar,
        min: u32,
        max: u32,
        commas: bool,
        n: u32,
        pos: usize,
        k: K<'_, 'v, 'a>,
    ) -> bool {
        if n < max {
            let mark = self.out.len();
            let mut start = pos;
            let mut comma_ok = true;
            if commas && n > 0 {
                match self.input.get(pos) {
                    Some(ComponentValue::Token(Token::Comma, _)) => {
                        self.out.push(Out::C(Component::Literal(',')));
                        start = pos + 1;
                    }
                    _ => comma_ok = false,
                }
            }
            if comma_ok
                && self.m(item, start, &mut |s, q| {
                    q > start && s.repeat(item, min, max, commas, n + 1, q, k)
                })
            {
                return true;
            }
            self.out.truncate(mark);
        }
        n >= min && k(self, pos)
    }

    fn typed(&mut self, t: &'static TypeRef, pos: usize, k: K<'_, 'v, 'a>) -> bool {
        let v = self.input.get(pos);
        let one = |c: Option<Component<'a>>| c.map(|c| (c, pos + 1));
        let found = match t.target.get_or_init(|| target(&t.name)) {
            Target::Numeric(n) => one(v.and_then(|v| n.matches(v, t.range))),
            Target::Color => one(v.and_then(Color::parse).map(Component::Color)),
            Target::String => one(match v {
                Some(ComponentValue::Token(Token::String(s), _)) => {
                    Some(Component::String(s.clone()))
                }
                _ => None,
            }),
            Target::Url => one(v.and_then(url)),
            Target::Ident => one(match v {
                Some(ComponentValue::Token(Token::Ident(s), _)) => {
                    Some(Component::Ident(s.clone()))
                }
                _ => None,
            }),
            Target::CustomIdent => one(match v {
                Some(ComponentValue::Token(Token::Ident(s), _))
                    if !CSS_WIDE.iter().any(|w| s.eq_ignore_ascii_case(w))
                        && !self.reserved.is_some_and(|r| {
                            let mut buf = [0; 32];
                            super::units::lowercase(s, &mut buf).is_some_and(|l| r.contains(l))
                        }) =>
                {
                    Some(Component::Ident(s.clone()))
                }
                _ => None,
            }),
            Target::DashedIdent => one(match v {
                Some(ComponentValue::Token(Token::Ident(s), _))
                    if s.starts_with("--") && s.len() > 2 =>
                {
                    Some(Component::Ident(s.clone()))
                }
                _ => None,
            }),
            Target::Zero => one(match v {
                Some(ComponentValue::Token(Token::Number(n), _)) if n.value == 0.0 => {
                    Some(Component::Number {
                        value: 0.0,
                        integer: true,
                    })
                }
                _ => None,
            }),
            Target::Dimension => one(match v {
                Some(ComponentValue::Token(Token::Dimension { number, unit }, _)) => {
                    Unit::parse(unit).map(|u| Component::Dimension(number.value, u))
                }
                _ => None,
            }),
            Target::Tokens => {
                // Longest first, down to one value.
                for end in (pos + 1..=self.input.len()).rev() {
                    let tokens: Vec<ComponentValue<'a>> = (pos..end)
                        .filter_map(|i| self.input.get(i).cloned())
                        .collect();
                    if self.emit(Component::Tokens(tokens), end, k) {
                        return true;
                    }
                }
                return false;
            }
            Target::Grammar(g) => return self.nested(|s| s.m(g, pos, k)),
            Target::Missing => return false,
        };
        match found {
            Some((c, next)) => self.emit(c, next, k),
            None => false,
        }
    }
}

/// What a type reference names, worked out once per reference.
#[derive(Debug)]
pub(crate) enum Target {
    Numeric(Numeric),
    Color,
    String,
    Url,
    Ident,
    CustomIdent,
    DashedIdent,
    Zero,
    Dimension,
    /// `<declaration-value>` and `<any-value>`: tokens as they are.
    Tokens,
    /// A type or function the specs give a grammar for.
    Grammar(&'static Grammar),
    /// Defined in prose only, so nothing matches it.
    Missing,
}

fn target(name: &str) -> Target {
    let numeric = |n| Target::Numeric(n);
    match name {
        "number" => numeric(Numeric::Number),
        "integer" => numeric(Numeric::Integer),
        "percentage" => numeric(Numeric::Percentage),
        "length" => numeric(Numeric::Dim(Kind::Length, false)),
        "angle" => numeric(Numeric::Dim(Kind::Angle, false)),
        "time" => numeric(Numeric::Dim(Kind::Time, false)),
        "frequency" => numeric(Numeric::Dim(Kind::Frequency, false)),
        "resolution" => numeric(Numeric::Dim(Kind::Resolution, false)),
        "flex" => numeric(Numeric::Dim(Kind::Flex, false)),
        "length-percentage" => numeric(Numeric::Dim(Kind::Length, true)),
        "angle-percentage" => numeric(Numeric::Dim(Kind::Angle, true)),
        "time-percentage" => numeric(Numeric::Dim(Kind::Time, true)),
        "frequency-percentage" => numeric(Numeric::Dim(Kind::Frequency, true)),
        "color" => Target::Color,
        "string" => Target::String,
        "url" => Target::Url,
        "ident" => Target::Ident,
        "custom-ident" => Target::CustomIdent,
        "dashed-ident" | "custom-property-name" => Target::DashedIdent,
        "zero" => Target::Zero,
        "dimension" => Target::Dimension,
        "declaration-value" | "any-value" => Target::Tokens,
        _ => {
            let g = match name.strip_suffix("()") {
                Some(f) => function_grammar(f),
                None => type_grammar(name),
            };
            g.map_or(Target::Missing, Target::Grammar)
        }
    }
}

/// Whether a grammar can match nothing at all.
fn optional(g: &'static Grammar) -> bool {
    match g {
        Grammar::Repeat { min: 0, .. } => true,
        Grammar::Sequence(items) | Grammar::All(items) => items.iter().all(optional),
        Grammar::One(alts) | Grammar::Any(alts) => alts.iter().any(optional),
        _ => false,
    }
}

fn url<'a>(v: &ComponentValue<'a>) -> Option<Component<'a>> {
    match v {
        ComponentValue::Token(Token::Url(u), _) => Some(Component::Url(u.clone())),
        ComponentValue::Function(f)
            if f.name.eq_ignore_ascii_case("url") || f.name.eq_ignore_ascii_case("src") =>
        {
            let mut args = f.arguments.iter().filter(|a| !a.is_whitespace());
            match (args.next(), args.next()) {
                (Some(ComponentValue::Token(Token::String(s), _)), None) => {
                    Some(Component::Url(s.clone()))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Numeric {
    Number,
    Integer,
    Percentage,
    /// A dimension of a kind, and whether a percentage may stand for one.
    Dim(Kind, bool),
}

impl Numeric {
    fn matches<'a>(
        self,
        v: &ComponentValue<'a>,
        range: Option<(f64, f64)>,
    ) -> Option<Component<'a>> {
        let in_range = |x: f64| range.is_none_or(|(lo, hi)| x >= lo && x <= hi);
        let c = match (self, v) {
            (Numeric::Number, ComponentValue::Token(Token::Number(n), _)) => Component::Number {
                value: n.value,
                integer: false,
            },
            (Numeric::Integer, ComponentValue::Token(Token::Number(n), _)) if n.integer => {
                Component::Number {
                    value: n.value,
                    integer: true,
                }
            }
            (
                Numeric::Percentage | Numeric::Dim(_, true),
                ComponentValue::Token(Token::Percentage(n), _),
            ) => Component::Percentage(n.value),
            (
                Numeric::Dim(kind, _),
                ComponentValue::Token(Token::Dimension { number, unit }, _),
            ) => {
                let u = Unit::parse(unit)?;
                if u.kind() != kind {
                    return None;
                }
                Component::Dimension(number.value, u)
            }
            // A unitless zero is a length.
            (Numeric::Dim(Kind::Length, _), ComponentValue::Token(Token::Number(n), _))
                if n.value == 0.0 =>
            {
                Component::Dimension(0.0, Unit::Px)
            }
            (_, ComponentValue::Function(f)) if Math::is_math_function(&f.name) => {
                let m = Math::parse(f)?;
                let expect = match self {
                    Numeric::Number | Numeric::Integer => Expect::Number,
                    Numeric::Percentage => Expect::Percentage,
                    Numeric::Dim(k, false) => Expect::Dimension(k),
                    Numeric::Dim(k, true) => Expect::DimensionPercentage(k),
                };
                // Out-of-range math is clamped when it's used, not invalid.
                return m.fits(expect).then_some(Component::Math(m));
            }
            _ => return None,
        };
        let value = match &c {
            Component::Number { value, .. } | Component::Percentage(value) => *value,
            Component::Dimension(value, _) => *value,
            _ => 0.0,
        };
        in_range(value).then_some(c)
    }
}

/// A gradient's arguments as CSS Images 4 writes them: the default color
/// interpolation left out (sRGB when every stop is a legacy color, Oklab
/// otherwise), `shorter hue` left out as the default, `xyz` as `xyz-d65`,
/// and `ellipse` left out before two sizes, which say it.
fn tidy_gradient(args: &mut Vec<Component<'_>>) {
    let end = args
        .iter()
        .position(|c| *c == Component::Literal(','))
        .unwrap_or(args.len());
    let legacy = args[end..].iter().all(|c| match c {
        Component::Color(Color::Channels(ch)) => {
            matches!(
                ch.model,
                super::color::Model::Rgb | super::color::Model::Hsl | super::color::Model::Hwb
            )
        }
        _ => true,
    });
    let mut end = end;
    if let Some(i) = args[..end]
        .iter()
        .position(|c| *c == Component::Keyword("in"))
    {
        if args.get(i + 1) == Some(&Component::Keyword("xyz")) {
            args[i + 1] = Component::Keyword("xyz-d65");
        }
        if args.get(i + 2) == Some(&Component::Keyword("shorter"))
            && args.get(i + 3) == Some(&Component::Keyword("hue"))
        {
            args.drain(i + 2..i + 4);
            end -= 2;
        }
        let default = if legacy { "srgb" } else { "oklab" };
        let hue = matches!(args.get(i + 3), Some(Component::Keyword("hue")));
        if !hue && args.get(i + 1) == Some(&Component::Keyword(default)) {
            args.drain(i..i + 2);
            end -= 2;
        }
    }
    if let Some(i) = args[..end]
        .iter()
        .position(|c| *c == Component::Keyword("ellipse"))
        && args[i + 1..end]
            .iter()
            .take(2)
            .filter(|c| {
                matches!(
                    c,
                    Component::Percentage(_) | Component::Dimension(..) | Component::Math(_)
                )
            })
            .count()
            == 2
    {
        args.remove(i);
        end -= 1;
    }
    // Nothing left before the stops: no comma either.
    if end == 0 && args.first() == Some(&Component::Literal(',')) {
        args.remove(0);
    }
}

/// `color-mix()`'s and `palette-mix()`'s arguments as CSS Color 5 and
/// CSS Fonts write them: `in xyz` as `in xyz-d65`, `shorter hue` left out
/// as the default, and for `color-mix()` `in oklab` too; and the
/// percentages filled in: those left out share what the others leave of
/// 100%, and two at 50% are left out.
fn tidy_mix(args: &mut Vec<Component<'_>>, color_mix: bool) {
    if args.first() == Some(&Component::Keyword("in")) {
        if args.get(1) == Some(&Component::Keyword("xyz")) {
            args[1] = Component::Keyword("xyz-d65");
        }
        if args.get(2) == Some(&Component::Keyword("shorter"))
            && args.get(3) == Some(&Component::Keyword("hue"))
        {
            args.drain(2..4);
        }
        let hue = matches!(args.get(3), Some(Component::Keyword("hue")));
        if color_mix && !hue && args.get(1) == Some(&Component::Keyword("oklab")) {
            args.drain(0..2);
            if args.first() == Some(&Component::Literal(',')) {
                args.remove(0);
            }
        }
    }
    let start = match args.first() {
        Some(Component::Keyword("in")) => {
            match args.iter().position(|c| *c == Component::Literal(',')) {
                Some(i) => i + 1,
                None => return,
            }
        }
        _ => 0,
    };
    let parts: Vec<Vec<Component<'_>>> = args[start..]
        .split(|c| *c == Component::Literal(','))
        .map(<[_]>::to_vec)
        .collect();
    let mut items = Vec::new();
    for p in parts {
        match &p[..] {
            [c, Component::Percentage(v)] => items.push((c.clone(), Some(*v))),
            [c] => items.push((c.clone(), None)),
            // Math or anything else: leave the whole thing as it is.
            _ => return,
        }
    }
    let given: f64 = items.iter().filter_map(|(_, p)| *p).sum();
    let missing = items.iter().filter(|(_, p)| p.is_none()).count();
    if missing == items.len() {
        return;
    }
    if missing > 0 {
        let share = ((100.0 - given) / missing as f64).max(0.0);
        for (_, p) in &mut items {
            p.get_or_insert(share);
        }
    }
    if items.len() == 2 && items.iter().all(|(_, p)| *p == Some(50.0)) {
        for (_, p) in &mut items {
            *p = None;
        }
    }
    args.truncate(start);
    for (i, (c, p)) in items.into_iter().enumerate() {
        if i > 0 {
            args.push(Component::Literal(','));
        }
        args.push(c);
        args.extend(p.map(Component::Percentage));
    }
}
