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
use super::properties::{PropertyData, function_grammar, property, type_grammar};
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
    Start(&'static PropertyData),
    End,
    /// Around what one alternative of a `||` or `&&` matched: which group,
    /// which alternative, whether it's `||`, and the group's alternatives,
    /// for writing them in the grammar's order.
    Alt(usize, usize, bool, &'static [Grammar]),
    AltEnd,
    /// Around what a type with a shortest form of its own matched.
    Type(&'static str),
    TypeEnd,
}

/// One thing in a match, with what it contains.
enum Tree<'a> {
    Leaf(Out<'a>),
    Group(Out<'a>, Vec<Tree<'a>>),
}

fn tree<'a>(outs: &mut std::vec::IntoIter<Out<'a>>) -> Vec<Tree<'a>> {
    // Iterative, with a stack of open groups: a value can nest deeply.
    let mut stack: Vec<(Option<Out<'a>>, Vec<Tree<'a>>)> = vec![(None, Vec::new())];
    for o in outs {
        match o {
            Out::End | Out::AltEnd | Out::TypeEnd => {
                if stack.len() > 1 {
                    let (open, items) = stack.pop().unwrap();
                    let group = Tree::Group(open.unwrap(), items);
                    stack.last_mut().unwrap().1.push(group);
                }
            }
            o @ (Out::Start(_) | Out::Alt(..) | Out::Type(_)) => stack.push((Some(o), Vec::new())),
            o => stack.last_mut().unwrap().1.push(Tree::Leaf(o)),
        }
    }
    while stack.len() > 1 {
        let (open, items) = stack.pop().unwrap();
        stack
            .last_mut()
            .unwrap()
            .1
            .push(Tree::Group(open.unwrap(), items));
    }
    stack.pop().unwrap().1
}

/// The values in a part of a match, in order.
fn leaves<'t, 'a>(items: &'t [Tree<'a>], out: &mut Vec<&'t Component<'a>>) {
    for t in items {
        match t {
            Tree::Leaf(Out::C(c)) => out.push(c),
            Tree::Group(_, inner) => leaves(inner, out),
            _ => {}
        }
    }
}

fn is_initial(property: &str, inner: &[Tree<'_>]) -> bool {
    let Some(initial) = super::value::initial_cached(property) else {
        return false;
    };
    let mut cs = Vec::new();
    leaves(inner, &mut cs);
    cs.len() == initial.len() && cs.iter().zip(initial).all(|(a, b)| *a == b)
}

/// Whether `values` are the initial value of `property`.
fn initial_values(property: &str, values: &[Out<'_>]) -> bool {
    let Some(initial) = super::value::initial_cached(property) else {
        return false;
    };
    values.len() == initial.len()
        && values
            .iter()
            .zip(initial)
            .all(|(a, b)| matches!(a, Out::C(c) if c == b))
}

/// A longhand at `i` with only values inside, through longhands that are
/// each other's whole value (`outline-color` as `<'border-top-color'>`):
/// where its values are and where it ends.
fn longhand<'o, 'a>(outs: &'o [Out<'a>], i: usize) -> Option<Longhand<'o, 'a>> {
    let mut j = i;
    let mut any_form = false;
    while let Some(Out::Start(p)) = outs.get(j) {
        any_form |= needs_form(p);
        j += 1;
    }
    let chain = &outs[i..j];
    let start = j;
    while matches!(outs.get(j), Some(Out::C(_))) {
        j += 1;
    }
    let values = &outs[start..j];
    for _ in 0..chain.len() {
        if !matches!(outs.get(j)?, Out::End) {
            return None;
        }
        j += 1;
    }
    if !any_form {
        return Some(Longhand {
            values: std::borrow::Cow::Borrowed(values),
            whole: None,
            end: j,
        });
    }
    // Written in each longhand's shortest form, the innermost first.
    let mut c: Vec<Component<'a>> = values
        .iter()
        .filter_map(|o| match o {
            Out::C(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    for o in chain.iter().rev() {
        if let Out::Start(p) = o {
            let (form, repeats) = forms(p);
            if repeats {
                super::value::shortest_of(p.name, &mut c);
            }
            if form {
                super::canonical::form(p.name, &mut c);
            }
        }
    }
    let values: Vec<Out<'a>> = c.into_iter().map(Out::C).collect();
    let mut whole = Vec::with_capacity(values.len() + 2 * chain.len());
    whole.extend(chain.iter().cloned());
    whole.extend(values.iter().cloned());
    whole.extend(chain.iter().map(|_| Out::End));
    Some(Longhand {
        values: std::borrow::Cow::Owned(values),
        whole: Some(whole),
        end: j,
    })
}

/// A longhand in a match, from [`longhand`].
struct Longhand<'o, 'a> {
    /// Its values, in their shortest form.
    values: std::borrow::Cow<'o, [Out<'a>]>,
    /// All of it, markers too, where its shortest form changed it.
    whole: Option<Vec<Out<'a>>>,
    /// Where it ends in the match.
    end: usize,
}

/// [`canonical`] without a tree, for the common shape: `||` and `&&`
/// alternatives side by side, each one value or one longhand of values,
/// and longhands of values. `None` for anything else, or a group all at
/// its defaults, which the tree handles.
fn plan<'a>(outs: &[Out<'a>], keep_order: bool) -> Option<Vec<Item<'a>>> {
    let mut items: Vec<Item<'a>> = Vec::with_capacity(8);
    let mut i = 0;
    while i < outs.len() {
        match &outs[i] {
            // A last `/` part at its initial value is left out: `/ normal`.
            Out::C(Component::Literal('/')) if matches!(outs.get(i + 1), Some(Out::Start(p)) if SLASHED.contains(&p.name)) =>
            {
                let Some(Out::Start(p)) = outs.get(i + 1) else {
                    return None;
                };
                let l = longhand(outs, i + 1)?;
                let end = l.end;
                let last = !outs[end..]
                    .iter()
                    .any(|o| matches!(o, Out::C(Component::Literal('/'))));
                if !(last && initial_values(p.name, &l.values)) {
                    items.push(Item {
                        start: i,
                        end,
                        alt: None,
                        default: false,
                        owned: l.whole.map(|w| {
                            let mut v = vec![outs[i].clone()];
                            v.extend(w);
                            v
                        }),
                    });
                }
                i = end;
            }
            Out::Type(_) => {
                let (owned, end) = typed(outs, i, keep_order)?;
                items.push(Item {
                    start: i,
                    end,
                    alt: None,
                    default: false,
                    owned: Some(owned),
                });
                i = end;
            }
            Out::C(_) => {
                items.push(Item {
                    start: i,
                    end: i + 1,
                    alt: None,
                    default: false,
                    owned: None,
                });
                i += 1;
            }
            Out::Start(_) => {
                let l = longhand(outs, i)?;
                let end = l.end;
                items.push(Item {
                    start: i,
                    end,
                    alt: None,
                    default: false,
                    owned: l.whole,
                });
                i = end;
            }
            Out::Alt(set, idx, any, _) if matches!(outs.get(i + 1), Some(Out::Type(_))) => {
                let (owned, end) = typed(outs, i + 1, keep_order)?;
                if !matches!(outs.get(end)?, Out::AltEnd) {
                    return None;
                }
                items.push(Item {
                    start: i + 1,
                    end,
                    alt: Some((*set, *idx, *any)),
                    default: false,
                    owned: Some(owned),
                });
                i = end + 1;
            }
            // An alternative of values and alternatives: `span && [ <integer>
            // || <custom-ident> ]`.
            Out::Alt(set, idx, any, _)
                if !matches!(outs.get(i + 1), Some(Out::Start(_)))
                    && !matches!(outs.get(i + 2), Some(Out::AltEnd)) =>
            {
                let mut depth = 0;
                let mut end = None;
                for (j, o) in outs.iter().enumerate().skip(i + 1) {
                    match o {
                        Out::C(_) => {}
                        Out::Alt(..) => depth += 1,
                        Out::AltEnd if depth == 0 => {
                            end = Some(j);
                            break;
                        }
                        Out::AltEnd => depth -= 1,
                        _ => return None,
                    }
                }
                let end = end?;
                let owned = flat(&outs[i + 1..end], keep_order)?;
                items.push(Item {
                    start: i + 1,
                    end,
                    alt: Some((*set, *idx, *any)),
                    default: false,
                    owned: Some(owned),
                });
                i = end + 1;
            }
            Out::Alt(set, idx, any, _) => {
                let mut whole = None;
                let (inner, close) = match outs.get(i + 1)? {
                    Out::Start(p) => {
                        if !drops_default(p.name) {
                            return None;
                        }
                        let l = longhand(outs, i + 1)?;
                        let end = l.end;
                        if !matches!(outs.get(end)?, Out::AltEnd) {
                            return None;
                        }
                        let default = *any && initial_values(p.name, &l.values);
                        whole = l.whole;
                        ((i + 1, end, default), end + 1)
                    }
                    Out::C(c) => {
                        if !matches!(outs.get(i + 2)?, Out::AltEnd) {
                            return None;
                        }
                        let default = *any && *c == Component::Keyword("normal");
                        ((i + 1, i + 2, default), i + 3)
                    }
                    _ => return None,
                };
                items.push(Item {
                    start: inner.0,
                    end: inner.1,
                    alt: Some((*set, *idx, *any)),
                    default: inner.2,
                    owned: whole,
                });
                i = close;
            }
            _ => return None,
        }
    }
    // Runs of one group: in the grammar's order, defaults left out.
    let mut k = 0;
    while k < items.len() {
        let Some((set, _, any)) = items[k].alt else {
            k += 1;
            continue;
        };
        let mut j = k;
        while matches!(items.get(j), Some(Item { alt: Some((s, ..)), .. }) if *s == set) {
            j += 1;
        }
        if !keep_order {
            items[k..j].sort_by_key(|t| t.alt.map_or(0, |a| a.1));
        }
        if any {
            if items[k..j].iter().all(|t| t.default) {
                return None;
            }
            let before = items.len();
            let mut n = 0;
            items.retain(|t| {
                let inside = n >= k && n < j;
                n += 1;
                !(inside && t.default)
            });
            j -= before - items.len();
        }
        k = j;
    }
    Some(items)
}

/// One part of a match, for [`plan`]: its range in the match, and for an
/// alternative its group, index and whether it's at its default.
struct Item<'a> {
    start: usize,
    end: usize,
    alt: Option<(usize, usize, bool)>,
    default: bool,
    /// Values in their shortest form, in place of the range.
    owned: Option<Vec<Out<'a>>>,
}

fn flat<'a>(outs: &[Out<'a>], keep_order: bool) -> Option<Vec<Out<'a>>> {
    let mut out = Vec::with_capacity(outs.len());
    for t in plan(outs, keep_order)? {
        match t.owned {
            Some(owned) => out.extend(owned),
            None => out.extend(outs[t.start..t.end].iter().cloned()),
        }
    }
    Some(out)
}

/// Whether `items` leave out any value of `outs`.
fn dropped(outs: &[Out<'_>], items: &[Item<'_>]) -> bool {
    let kept: usize = items
        .iter()
        .map(|t| {
            outs[t.start..t.end]
                .iter()
                .filter(|o| matches!(o, Out::C(_)))
                .count()
        })
        .sum();
    kept != outs.iter().filter(|o| matches!(o, Out::C(_))).count()
}

/// [`flat`] for a whole match, moving rather than copying, and giving the
/// match back as it is when nothing changes.
fn flat_owned<'a>(mut outs: Vec<Out<'a>>, keep_order: bool) -> Result<Vec<Out<'a>>, Vec<Out<'a>>> {
    let Some(items) = plan(&outs, keep_order) else {
        return Err(outs);
    };
    // In order, nothing left out or rewritten: the match as it is, the
    // markers around alternatives being ignored after this anyway.
    let same = items.iter().all(|t| t.owned.is_none() && !t.default)
        && items.windows(2).all(|w| w[0].end <= w[1].start)
        && !dropped(&outs, &items);
    if same {
        return Ok(outs);
    }
    let mut out = Vec::with_capacity(outs.len());
    for t in items {
        match t.owned {
            Some(owned) => out.extend(owned),
            None => out.extend(
                outs[t.start..t.end]
                    .iter_mut()
                    .map(|o| std::mem::replace(o, Out::End)),
            ),
        }
    }
    Ok(out)
}

/// A type's group at `i` with only values and alternatives in it, in its
/// shortest form, and where it ends.
fn typed<'a>(outs: &[Out<'a>], i: usize, keep_order: bool) -> Option<(Vec<Out<'a>>, usize)> {
    let Out::Type(name) = outs[i] else {
        return None;
    };
    let len = outs[i + 1..]
        .iter()
        .position(|o| !matches!(o, Out::C(_) | Out::Alt(..) | Out::AltEnd))?;
    let end = i + 1 + len;
    if !matches!(outs[end], Out::TypeEnd) {
        return None;
    }
    let keep = keep_order || name == "try-tactic";
    let inner = flat(&outs[i + 1..end], keep)?;
    let mut c: Vec<Component<'a>> = inner
        .into_iter()
        .filter_map(|o| match o {
            Out::C(c) => Some(c),
            _ => None,
        })
        .collect();
    super::canonical::of_type(name, &mut c);
    Some((c.into_iter().map(Out::C).collect(), end + 1))
}

/// Whether a shorthand leaves this longhand out at its initial value:
/// `flex`, `list-style` and `line-clamp` have shortest forms of their own.
fn drops_default(name: &str) -> bool {
    !(matches!(
        name,
        "flex-grow" | "flex-shrink" | "flex-basis" | "block-ellipsis"
    ) || name.starts_with("list-style-"))
}

/// Whether one alternative of a `||` group says only its default: the
/// initial value of the longhand it sets, or `normal`.
fn is_default(t: &Tree<'_>) -> bool {
    let Tree::Group(_, inner) = t else {
        return false;
    };
    match &inner[..] {
        [Tree::Leaf(Out::C(Component::Keyword("normal")))] => true,
        [Tree::Group(Out::Start(p), value)] => drops_default(p.name) && is_initial(p.name, value),
        _ => false,
    }
}

/// A group whose contents are all values, through `||` and `&&` groups,
/// rewritten by `f`; one with types or properties inside is left as it is.
fn tidy_leaves(inner: &mut Vec<Tree<'_>>, f: impl FnOnce(&mut Vec<Component<'_>>)) {
    fn plain(items: &[Tree<'_>]) -> bool {
        items.iter().all(|t| match t {
            Tree::Leaf(Out::C(_)) => true,
            Tree::Group(Out::Alt(..), inner) => plain(inner),
            _ => false,
        })
    }
    fn take<'a>(items: Vec<Tree<'a>>, out: &mut Vec<Component<'a>>) {
        for t in items {
            match t {
                Tree::Leaf(Out::C(c)) => out.push(c),
                Tree::Group(_, inner) => take(inner, out),
                _ => {}
            }
        }
    }
    if !plain(inner) {
        return;
    }
    let mut c = Vec::new();
    take(std::mem::take(inner), &mut c);
    f(&mut c);
    inner.extend(c.into_iter().map(|c| Tree::Leaf(Out::C(c))));
}

/// One level of a match in canonical form, its groups first.
fn process(items: Vec<Tree<'_>>, keep_order: bool) -> Vec<Tree<'_>> {
    let mut items: Vec<Tree<'_>> = items
        .into_iter()
        .map(|t| match t {
            Tree::Group(o, inner) => {
                // Flips in `position-try` apply in the order written.
                let keep = keep_order || matches!(o, Out::Type("try-tactic"));
                let mut inner = process(inner, keep);
                match o {
                    Out::Start(p) => tidy_leaves(&mut inner, |c| {
                        super::value::shortest_of(p.name, c);
                        super::canonical::property(p.name, c)
                    }),
                    Out::Type(t) => tidy_leaves(&mut inner, |c| super::canonical::of_type(t, c)),
                    _ => {}
                }
                Tree::Group(o, inner)
            }
            leaf => leaf,
        })
        .collect();
    // Runs of one group's alternatives go in the grammar's order, which
    // is the canonical order CSSOM writes them in.
    let mut i = 0;
    while i < items.len() {
        let (set, alts) = match &items[i] {
            Tree::Group(Out::Alt(set, _, _, alts), _) => (*set, *alts),
            _ => {
                i += 1;
                continue;
            }
        };
        let mut j = i;
        while matches!(items.get(j), Some(Tree::Group(Out::Alt(s, ..), _)) if *s == set) {
            j += 1;
        }
        if !keep_order {
            items[i..j].sort_by_key(|t| match t {
                Tree::Group(Out::Alt(_, idx, ..), _) => *idx,
                _ => 0,
            });
        }
        // In a `||` group, what says only the default is left out, as the
        // shortest serialization does: `font: normal bold 1em x` is
        // `bold 1em x`.
        let any = matches!(&items[i], Tree::Group(Out::Alt(_, _, true, _), _));
        if any {
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
            } else if alone(&items, i, j) {
                // All of it at defaults, and nothing else beside it: the
                // first longhand the group can set says so, as `none` for
                // `text-decoration: solid`.
                let first = alts
                    .iter()
                    .position(|a| matches!(a, Grammar::Property(_)))
                    .unwrap_or(usize::MAX);
                let has = |t: &Tree<'_>| matches!(t, Tree::Group(Out::Alt(_, idx, ..), _) if *idx == first);
                if let Some(keep) = items[i..j].iter().position(has) {
                    items.drain(i + keep + 1..j);
                    items.drain(i..i + keep);
                    j = i + 1;
                } else if let Some(Grammar::Property(r)) = alts.get(first)
                    && let Some(p) = property(&r.name)
                    && let Some(initial) = super::value::initial_components(&r.name)
                {
                    let value = initial.into_iter().map(|c| Tree::Leaf(Out::C(c))).collect();
                    let group = vec![Tree::Group(Out::Start(p), value)];
                    let group = Tree::Group(Out::Alt(set, first, true, alts), group);
                    items.splice(i..j, [group]);
                    j = i + 1;
                } else {
                    items.drain(i + 1..j);
                    j = i + 1;
                }
            } else {
                items.drain(i..j);
                j = i;
            }
        }
        // Past the run, or at what follows a run left out entirely.
        i = j;
    }
    // `/ normal` for a line height left at its initial value, and the
    // like: a last `/` part at its longhand's initial value.
    let mut i = items.len();
    while i >= 2 {
        i -= 1;
        if matches!(items[i - 1], Tree::Leaf(Out::C(Component::Literal('/'))))
            && let Tree::Group(Out::Start(p), inner) = &items[i]
            && SLASHED.contains(&p.name)
            && is_initial(p.name, inner)
            && !items[i + 1..]
                .iter()
                .any(|t| matches!(t, Tree::Leaf(Out::C(Component::Literal('/')))))
        {
            items.drain(i - 1..=i);
            i -= 1;
        }
    }
    items
}

/// Whether a longhand in a match has a shortest form to be written in.
fn needs_form(p: &'static PropertyData) -> bool {
    let f = forms(p);
    f.0 || f.1
}

/// Whether a longhand has a shortest form of its own, and whether its
/// grammar repeats values a box shorthand's way, worked out once.
fn forms(p: &'static PropertyData) -> (bool, bool) {
    use std::sync::OnceLock;
    static ALL: OnceLock<Vec<OnceLock<(bool, bool)>>> = OnceLock::new();
    let all = ALL.get_or_init(|| {
        super::properties::properties()
            .iter()
            .map(|_| OnceLock::new())
            .collect()
    });
    let work = || {
        (
            super::canonical::has_form(p.name),
            p.grammar().is_some_and(|g| {
                matches!(
                    g,
                    Grammar::Repeat {
                        min: 1,
                        max: 2 | 4,
                        commas: false,
                        ..
                    }
                )
            }),
        )
    };
    match p.position().and_then(|i| all.get(i)) {
        Some(cell) => *cell.get_or_init(work),
        None => work(),
    }
}

/// The longhands after a `/` that the shortest form leaves out at their
/// initial values.
const SLASHED: &[&str] = &[
    "line-height",
    "column-height",
    "offset-anchor",
    "border-image-width",
    "border-image-outset",
    "mask-border-width",
    "mask-border-outset",
];

/// Whether the run `i..j` is all there is between commas or slashes.
fn alone(items: &[Tree<'_>], i: usize, j: usize) -> bool {
    let comma = |t: &Tree<'_>| matches!(t, Tree::Leaf(Out::C(Component::Literal(',' | '/'))));
    let before = items[..i].iter().rev().take_while(|t| !comma(t)).count();
    let after = items[j..].iter().take_while(|t| !comma(t)).count();
    before == 0 && after == 0
}

fn flatten<'a>(items: Vec<Tree<'a>>, out: &mut Vec<Out<'a>>) {
    let mut stack = vec![items.into_iter()];
    let mut ends: Vec<bool> = Vec::new();
    while let Some(top) = stack.last_mut() {
        match top.next() {
            Some(Tree::Leaf(o)) => out.push(o),
            Some(Tree::Group(o, inner)) => {
                let marked = matches!(o, Out::Start(_));
                if marked {
                    out.push(o);
                }
                ends.push(marked);
                stack.push(inner.into_iter());
            }
            None => {
                stack.pop();
                if let Some(marked) = ends.pop()
                    && marked
                {
                    out.push(Out::End);
                }
            }
        }
    }
}

/// The match with every `||` and `&&` group in the grammar's order, the
/// shortest forms of types and longhands written, and the markers for
/// groups gone; `keep_order` keeps groups in the order written.
pub(crate) fn canonical(outs: Vec<Out<'_>>, keep_order: bool) -> Vec<Out<'_>> {
    if !outs
        .iter()
        .any(|o| matches!(o, Out::Alt(..) | Out::Type(_)))
    {
        // Only longhands, as in `margin: 0 1px`: those without a shortest
        // form of their own need no tree.
        let plain = outs.iter().all(|o| match o {
            Out::Start(p) => !needs_form(p),
            _ => true,
        });
        if plain {
            return outs;
        }
    }
    let outs = match flat_owned(outs, keep_order) {
        Ok(out) => return out,
        Err(outs) => outs,
    };
    let mut it = outs.into_iter();
    let items = process(tree(&mut it), keep_order);
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
    reserved: Option<&'static super::properties::Keywords>,
    /// Where the last comma in the input is.
    last_comma: Option<usize>,
}

/// The values being matched, whitespace aside: the slice itself when
/// there's none in it, as for most one-word values, so nothing's copied.
enum Input<'v, 'a> {
    Slice(&'v [ComponentValue<'a>]),
    /// Up to [`SHORT`] values, kept without an allocation.
    Short([Option<&'v ComponentValue<'a>>; SHORT], usize),
    Filtered(Vec<&'v ComponentValue<'a>>),
}

/// How many values a value with whitespace in it can have and be matched
/// without allocating: most have a few.
const SHORT: usize = 12;

impl<'v, 'a> Input<'v, 'a> {
    fn new(values: &'v [ComponentValue<'a>]) -> Input<'v, 'a> {
        if !values.iter().any(ComponentValue::is_whitespace) {
            return Input::Slice(values);
        }
        let mut short = [None; SHORT];
        let mut n = 0;
        for v in values.iter().filter(|v| !v.is_whitespace()) {
            if n == SHORT {
                return Input::Filtered(values.iter().filter(|v| !v.is_whitespace()).collect());
            }
            short[n] = Some(v);
            n += 1;
        }
        Input::Short(short, n)
    }

    #[inline]
    fn get(&self, i: usize) -> Option<&'v ComponentValue<'a>> {
        match self {
            Input::Slice(s) => s.get(i),
            Input::Short(s, n) => s[..*n].get(i).copied().flatten(),
            Input::Filtered(v) => v.get(i).copied(),
        }
    }

    fn len(&self) -> usize {
        match self {
            Input::Slice(s) => s.len(),
            Input::Short(_, n) => *n,
            Input::Filtered(v) => v.len(),
        }
    }
}

/// What a grammar's match can start with: a set of kinds of component
/// value, and whether it can match nothing. Where the next value is of no
/// kind it can start with, it isn't tried, which saves descending into
/// grammars like `<position>` for `#fff`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct First {
    kinds: u16,
    empty: bool,
}

const IDENT: u16 = 1;
const FUNCTION: u16 = 1 << 1;
const NUMBER: u16 = 1 << 2;
const PERCENTAGE: u16 = 1 << 3;
const DIMENSION: u16 = 1 << 4;
const STRING: u16 = 1 << 5;
const URL: u16 = 1 << 6;
const HASH: u16 = 1 << 7;
const DELIM: u16 = 1 << 8;
const COMMA: u16 = 1 << 9;
const BLOCK: u16 = 1 << 10;
const OTHER: u16 = 1 << 11;
const ANY: u16 = u16::MAX;
const NUMERIC: u16 = NUMBER | PERCENTAGE | DIMENSION | FUNCTION;

fn kind(v: &ComponentValue<'_>) -> u16 {
    match v {
        ComponentValue::Function(_) => FUNCTION,
        ComponentValue::Block(_) => BLOCK,
        ComponentValue::Token(t, _) => match t {
            Token::Ident(_) => IDENT,
            Token::Number(_) => NUMBER,
            Token::Percentage(_) => PERCENTAGE,
            Token::Dimension { .. } => DIMENSION,
            Token::String(_) => STRING,
            Token::Url(_) => URL,
            Token::Hash { .. } => HASH,
            Token::Delim(_) => DELIM,
            Token::Comma => COMMA,
            _ => OTHER,
        },
    }
}

impl First {
    const ALL: First = First {
        kinds: ANY,
        empty: true,
    };

    fn admits(self, v: Option<&ComponentValue<'_>>) -> bool {
        self.empty || v.is_some_and(|v| self.kinds & kind(v) != 0)
    }

    /// Works out `g`'s, through the types and properties it names, without
    /// their cached targets: one may be what's being worked out.
    fn of(g: &'static Grammar, depth: usize) -> First {
        if depth > MAX_DEPTH {
            return First::ALL;
        }
        let one = |kinds| First {
            kinds,
            empty: false,
        };
        match g {
            Grammar::Never => one(0),
            Grammar::Keyword(_) => one(IDENT),
            Grammar::Literal(',') => one(COMMA),
            Grammar::Literal(':' | ';') => one(OTHER),
            Grammar::Literal(_) => one(DELIM),
            Grammar::Function(..) => one(FUNCTION),
            Grammar::SquareBlock(_) | Grammar::ParenBlock(_) => one(BLOCK),
            Grammar::Type(t) => First::of_type(&t.name, depth),
            Grammar::Property(r) => match property(&r.name).and_then(|p| p.grammar()) {
                Some(g) => First::of(g, depth + 1),
                None => one(0),
            },
            Grammar::Sequence(items) => {
                let mut f = First {
                    kinds: 0,
                    empty: true,
                };
                for item in items {
                    let i = First::of(item, depth + 1);
                    f.kinds |= i.kinds;
                    // A comma can be left out with what's beside it.
                    if !i.empty && *item != Grammar::Literal(',') {
                        f.empty = false;
                        break;
                    }
                }
                f
            }
            Grammar::One(alts) | Grammar::Any(alts) => {
                let mut f = one(0);
                for a in alts {
                    let i = First::of(a, depth + 1);
                    f.kinds |= i.kinds;
                    f.empty |= i.empty;
                }
                f
            }
            Grammar::All(alts) => {
                let mut f = First {
                    kinds: 0,
                    empty: true,
                };
                for a in alts {
                    let i = First::of(a, depth + 1);
                    f.kinds |= i.kinds;
                    f.empty &= i.empty;
                }
                f
            }
            Grammar::Repeat { item, min, .. } => {
                let mut f = First::of(item, depth + 1);
                f.empty |= *min == 0;
                f
            }
            Grammar::NonEmpty(inner) => First {
                empty: false,
                ..First::of(inner, depth + 1)
            },
        }
    }

    fn of_type(name: &str, depth: usize) -> First {
        let one = |kinds| First {
            kinds,
            empty: false,
        };
        match name {
            "number"
            | "integer"
            | "percentage"
            | "length"
            | "angle"
            | "time"
            | "frequency"
            | "resolution"
            | "flex"
            | "length-percentage"
            | "angle-percentage"
            | "time-percentage"
            | "frequency-percentage" => one(NUMERIC),
            "color" => one(IDENT | HASH | FUNCTION),
            "string" => one(STRING),
            "url" => one(URL | FUNCTION),
            "ident" | "custom-ident" | "dashed-ident" | "custom-property-name" => one(IDENT),
            "zero" => one(NUMBER),
            "dimension" => one(DIMENSION),
            "declaration-value" | "any-value" => First::ALL,
            _ => {
                let g = match name.strip_suffix("()") {
                    Some(f) => function_grammar(f),
                    None => type_grammar(name),
                };
                match g {
                    Some(g) => First::of(g, depth + 1),
                    None => one(0),
                }
            }
        }
    }
}

/// What came before an item in a sequence, for its commas.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Before {
    Start,
    Nothing,
    Something,
}

type K<'k, 'v, 'a> = &'k mut dyn FnMut(&mut Matcher<'v, 'a>, usize) -> bool;

const CSS_WIDE: [&str; 7] = [
    "initial",
    "inherit",
    "unset",
    "revert",
    "revert-layer",
    "revert-rule",
    "default",
];

impl<'v, 'a> Matcher<'v, 'a> {
    pub fn new(values: &'v [ComponentValue<'a>]) -> Matcher<'v, 'a> {
        let input = Input::new(values);
        let last_comma = (0..input.len())
            .rev()
            .find(|&i| matches!(input.get(i), Some(ComponentValue::Token(Token::Comma, _))));
        Matcher {
            input,
            out: Vec::with_capacity(8),
            steps: 0,
            depth: 0,
            groups: 0,
            reserved: None,
            last_comma,
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
                let first = r.first.get_or_init(|| First::of(g, 0));
                if !first.admits(self.input.get(pos)) {
                    return false;
                }
                let found = r
                    .target
                    .get_or_init(|| property(&r.name).and_then(|p| Some((p, p.grammar()?))));
                let Some((p, g)) = *found else {
                    return false;
                };
                self.nested(|s| {
                    s.out.push(Out::Start(p));
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
                if !super::restrict::function(name, &arguments) {
                    return false;
                }
                let name = super::canonical::function(name, &mut arguments);
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
            Grammar::Sequence(items) => self.sequence(items, pos, Before::Start, k),
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
            canonical(sub.out, false)
                .into_iter()
                .filter_map(|o| match o {
                    Out::C(c) => Some(c),
                    _ => None,
                })
                .collect()
        })
    }

    /// Juxtaposed items, in order, with the commas Values 4 §2.6 says to
    /// leave out: those with only omitted items before them, or after
    /// them, or between them and another comma. `[ a? , b ]` matches `b`,
    /// and not `, b`.
    fn sequence(
        &mut self,
        items: &'static [Grammar],
        pos: usize,
        before: Before,
        k: K<'_, 'v, 'a>,
    ) -> bool {
        let Some((first, rest)) = items.split_first() else {
            return k(self, pos);
        };
        // `<bg-layer>#? , <final-bg-layer>` with no comma left to match:
        // what's before the comma can only be left out, which saves trying
        // every way it could match.
        if rest.first() == Some(&Grammar::Literal(','))
            && !rest[1..].iter().all(optional)
            && self.last_comma.is_none_or(|c| c < pos)
        {
            if !optional(first) {
                return false;
            }
            let next = if before == Before::Something {
                Before::Something
            } else {
                Before::Nothing
            };
            return self.sequence(rest, pos, next, k);
        }
        if *first == Grammar::Literal(',') {
            if before == Before::Nothing {
                return self.sequence(rest, pos, Before::Nothing, k);
            }
            let mark = self.out.len();
            if self.m(first, pos, &mut |s, q| {
                // Something has to follow it, if anything in the sequence does.
                s.sequence(rest, q, Before::Nothing, &mut |s, r| {
                    (r > q || rest.is_empty()) && k(s, r)
                })
            }) {
                return true;
            }
            self.out.truncate(mark);
            if before == Before::Start {
                return self.sequence(rest, pos, Before::Nothing, k);
            }
            // After something, the comma goes only if all that follows
            // matches nothing.
            if rest.is_empty() {
                return false;
            }
            return self.sequence(rest, pos, Before::Nothing, &mut |s, q| q == pos && k(s, q));
        }
        self.m(first, pos, &mut |s, q| {
            let next = match (q == pos, before) {
                (false, _) => Before::Something,
                (true, Before::Something) => Before::Something,
                (true, _) => Before::Nothing,
            };
            s.sequence(rest, q, next, k)
        })
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
            self.out.push(Out::Alt(set, i, !all, alts));
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
        if !t.first.get_or_init(|| First::of_type(&t.name, 0)).admits(v) {
            return false;
        }
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
            Target::Tidy(name, g) if !super::canonical::TYPES.contains(name) => {
                // Only a restriction: checked on what it matched, unmarked.
                let name: &'static str = name;
                return self.nested(|s| {
                    let mark = s.out.len();
                    s.m(g, pos, &mut |s, q| {
                        super::restrict::of_type(name, &s.out[mark..]) && k(s, q)
                    })
                });
            }
            Target::Tidy(name, g) => {
                let name: &'static str = name;
                return self.nested(|s| {
                    let mark = s.out.len();
                    s.out.push(Out::Type(name));
                    let ok = s.m(g, pos, &mut |s, q| {
                        if !super::restrict::of_type(name, &s.out[mark + 1..]) {
                            return false;
                        }
                        s.out.push(Out::TypeEnd);
                        if k(s, q) {
                            return true;
                        }
                        s.out.pop();
                        false
                    });
                    if !ok {
                        s.out.truncate(mark);
                    }
                    ok
                });
            }
            Target::Missing => return false,
        };
        match found {
            Some((c, next)) => self.emit(c, next, k),
            None => false,
        }
    }
}

/// A value of one token (not an identifier, a function or a block)
/// matched against `g` without the matcher, where `g` is alternatives of
/// keywords and primitive types: `Some(Some(_))` for a match, `Some(None)`
/// for none, and `None` where an alternative needs the matcher.
pub(crate) fn single<'a>(
    g: &'static Grammar,
    v: &ComponentValue<'a>,
) -> Option<Option<Component<'a>>> {
    single_at(g, v, 0)
}

fn single_at<'a>(
    g: &'static Grammar,
    v: &ComponentValue<'a>,
    depth: usize,
) -> Option<Option<Component<'a>>> {
    if depth > MAX_DEPTH {
        return None;
    }
    let alts = match g {
        Grammar::One(alts) => &alts[..],
        other => std::slice::from_ref(other),
    };
    for alt in alts {
        let found = match alt {
            Grammar::Keyword(_) | Grammar::Function(..) => continue,
            // One token is one of a repeat's items.
            Grammar::Repeat {
                item, min: 0 | 1, ..
            } => single_at(item, v, depth + 1)?,
            // Or the first of a sequence whose others can be left out.
            Grammar::Sequence(items)
                if items.len() > 1 && items[1..].iter().all(|i| optional(i) && !has_comma(i)) =>
            {
                single_at(&items[0], v, depth + 1)?
            }
            // Or one of a `||` group's, unless it's a longhand's initial
            // value, which the shortest form may write otherwise.
            Grammar::Any(items) => {
                let mut found = None;
                for item in items {
                    if let Some(c) = single_at(item, v, depth + 1)? {
                        if let Grammar::Property(r) = item
                            && super::value::initial_cached(&r.name)
                                .is_some_and(|i| i.len() == 1 && i[0] == c)
                        {
                            return None;
                        }
                        found = Some(c);
                        break;
                    }
                }
                found
            }
            // A longhand's grammar, where it has no shortest form to keep.
            Grammar::Property(r) => {
                let p = property(&r.name)?;
                if needs_form(p) {
                    return None;
                }
                single_at(p.grammar()?, v, depth + 1)?
            }
            Grammar::Type(t) => {
                if !t
                    .first
                    .get_or_init(|| First::of_type(&t.name, 0))
                    .admits(Some(v))
                {
                    continue;
                }
                match t.target.get_or_init(|| target(&t.name)) {
                    Target::Numeric(n) => n.matches(v, t.range),
                    Target::Color => Color::parse(v).map(Component::Color),
                    Target::String => match v {
                        ComponentValue::Token(Token::String(s), _) => {
                            Some(Component::String(s.clone()))
                        }
                        _ => None,
                    },
                    Target::Url => url(v),
                    Target::Missing => None,
                    Target::Grammar(g) => single_at(g, v, depth + 1)?,
                    _ => return None,
                }
            }
            _ => return None,
        };
        if found.is_some() {
            return Some(found);
        }
    }
    Some(None)
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
    /// One with a shortest form or a restriction of its own, marked in the
    /// match for it.
    Tidy(&'static str, &'static Grammar),
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
            match g {
                Some(g) => match super::canonical::TYPES
                    .iter()
                    .chain(super::restrict::TYPES)
                    .find(|t| **t == name)
                {
                    Some(t) => Target::Tidy(t, g),
                    None => Target::Grammar(g),
                },
                None => Target::Missing,
            }
        }
    }
}

/// Whether a grammar has a comma of its own, which can't be left out
/// beside a value that's there.
fn has_comma(g: &Grammar) -> bool {
    match g {
        Grammar::Literal(',') => true,
        Grammar::Sequence(items)
        | Grammar::All(items)
        | Grammar::Any(items)
        | Grammar::One(items) => items.iter().any(has_comma),
        Grammar::Repeat { item, .. } | Grammar::NonEmpty(item) => has_comma(item),
        _ => false,
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
