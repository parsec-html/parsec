// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Custom properties and `var()`, CSS Custom Properties Level 1 and CSS
//! Values 5's substitution: an element's custom properties resolved
//! against each other, cycles found and made the guaranteed-invalid
//! value, and `var()` in any value replaced from them.
//!
//! Inheriting from a parent is what the cascade does (#30); here you hand
//! in the parent's resolved properties if there is one.

use std::collections::HashMap;

use super::value::{InvalidValue, Specified, parse_value};
use crate::parser::{ComponentValue, Function, SimpleBlock};
use crate::tokenizer::Token;

/// How many component values a substituted value may grow to. Substitution
/// can grow a value exponentially (`--b: var(--a) var(--a)`, and so on);
/// past this the value is invalid, as the spec lets a browser decide.
const MAX_VALUES: usize = 100_000;
/// How deep `var()` fallbacks and functions may nest while substituting.
const MAX_DEPTH: usize = 128;

/// An element's custom properties, as declared.
///
/// ```
/// use parsec_css::values::CustomProperties;
///
/// let mut props = CustomProperties::new();
/// props.declare("--gap", parsec_css::parse_component_values("4px"));
/// props.declare("--pad", parsec_css::parse_component_values("calc(var(--gap) * 2)"));
/// props.declare("--a", parsec_css::parse_component_values("var(--b)"));
/// props.declare("--b", parsec_css::parse_component_values("var(--a)"));
/// let resolved = props.resolve(None);
/// assert_eq!(resolved.get_css("--pad").as_deref(), Some("calc(4px * 2)"));
/// assert_eq!(resolved.get_css("--a"), None); // a cycle: guaranteed-invalid
/// ```
#[derive(Clone, Debug, Default)]
pub struct CustomProperties<'a> {
    declared: Vec<(String, Vec<ComponentValue<'a>>)>,
    index: HashMap<String, usize>,
}

/// Custom properties with every `var()` in them substituted. A name that
/// maps to `None` holds the guaranteed-invalid value.
#[derive(Clone, Debug, Default)]
pub struct Resolved<'a> {
    values: HashMap<String, Option<Vec<ComponentValue<'a>>>>,
}

impl<'a> CustomProperties<'a> {
    pub fn new() -> CustomProperties<'a> {
        CustomProperties::default()
    }

    /// Declares `name` (with its `--`), replacing an earlier declaration.
    pub fn declare(&mut self, name: &str, value: Vec<ComponentValue<'a>>) {
        let value = trim(value);
        match self.index.get(name) {
            Some(&i) => self.declared[i].1 = value,
            None => {
                self.index.insert(name.to_string(), self.declared.len());
                self.declared.push((name.to_string(), value));
            }
        }
    }

    /// Resolves every declared property: `var()` substituted, cycles made
    /// guaranteed-invalid. What isn't declared here comes from `parent`.
    pub fn resolve(&self, parent: Option<&Resolved<'a>>) -> Resolved<'a> {
        let n = self.declared.len();
        let index = &self.index;
        // Who each property refers to, fallbacks included.
        let edges: Vec<Vec<usize>> = self
            .declared
            .iter()
            .map(|(_, v)| {
                let mut out = Vec::new();
                references(v, &mut |name| {
                    if let Some(&j) = index.get(name) {
                        out.push(j);
                    }
                });
                out
            })
            .collect();
        let mut resolved = Resolved {
            values: parent.map(|p| p.values.clone()).unwrap_or_default(),
        };
        // Tarjan's strongly connected components, without recursion: each
        // component completes after those it refers to, so its references
        // are resolved by the time it's substituted.
        let mut order = vec![usize::MAX; n];
        let mut low = vec![0; n];
        let mut on_stack = vec![false; n];
        let mut stack = Vec::new();
        let mut counter = 0;
        for root in 0..n {
            if order[root] != usize::MAX {
                continue;
            }
            let mut work = vec![(root, 0usize)];
            while let Some(&mut (v, ref mut next)) = work.last_mut() {
                if *next == 0 {
                    order[v] = counter;
                    low[v] = counter;
                    counter += 1;
                    stack.push(v);
                    on_stack[v] = true;
                }
                if let Some(&w) = edges[v].get(*next) {
                    *next += 1;
                    if order[w] == usize::MAX {
                        work.push((w, 0));
                    } else if on_stack[w] {
                        low[v] = low[v].min(order[w]);
                    }
                    continue;
                }
                work.pop();
                if let Some(&(parent, _)) = work.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] != order[v] {
                    continue;
                }
                let mut component = Vec::new();
                loop {
                    let w = stack.pop().unwrap();
                    on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                let cyclic = component.len() > 1 || edges[v].contains(&v);
                for w in component {
                    let (name, value) = &self.declared[w];
                    let result = if cyclic {
                        None
                    } else {
                        custom_value(value, &resolved)
                    };
                    resolved.values.insert(name.clone(), result);
                }
            }
        }
        resolved
    }
}

/// A custom property's own value, substituted: `initial` is the
/// guaranteed-invalid value, `inherit` and `unset` the parent's (already
/// in `resolved`, since custom properties inherit).
fn custom_value<'a>(
    value: &[ComponentValue<'a>],
    resolved: &Resolved<'a>,
) -> Option<Vec<ComponentValue<'a>>> {
    if let [ComponentValue::Token(Token::Ident(w), _)] = value {
        if w.eq_ignore_ascii_case("initial") {
            return None;
        }
        if ["inherit", "unset", "revert", "revert-layer"]
            .iter()
            .any(|k| w.eq_ignore_ascii_case(k))
        {
            return None;
        }
    }
    substitute(value, resolved).ok()
}

impl<'a> Resolved<'a> {
    /// A resolved custom property's value; `None` if it's guaranteed-invalid
    /// or not there at all.
    pub fn get(&self, name: &str) -> Option<&[ComponentValue<'a>]> {
        self.values.get(name)?.as_deref()
    }

    /// [`Resolved::get`], written as CSS.
    pub fn get_css(&self, name: &str) -> Option<String> {
        self.get(name).map(crate::to_css)
    }
}

/// The guaranteed-invalid value turned up where a value was needed: a
/// `var()` with nothing to stand for and no fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GuaranteedInvalid;

impl std::fmt::Display for GuaranteedInvalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a var() refers to the guaranteed-invalid value")
    }
}

impl std::error::Error for GuaranteedInvalid {}

/// Replaces every `var()` in `value` from `custom`.
pub fn substitute<'a>(
    value: &[ComponentValue<'a>],
    custom: &Resolved<'a>,
) -> Result<Vec<ComponentValue<'a>>, GuaranteedInvalid> {
    let mut out = Vec::new();
    let mut count = 0;
    substitute_into(value, custom, &mut out, &mut count, 0)?;
    Ok(out)
}

/// Substitutes `var()` in a property's value and parses what results, as
/// computing a value with `var()` in it does. An error is "invalid at
/// computed-value time", which the cascade turns into `unset`.
pub fn substitute_value<'a>(
    property: &str,
    value: &[ComponentValue<'a>],
    custom: &Resolved<'a>,
) -> Result<Specified<'a>, InvalidValue> {
    let values = substitute(value, custom).map_err(|_| InvalidValue)?;
    // Substituted tokens stay the tokens they were: `var(--n)px` with `--n:
    // 1` is a number and an ident, never a dimension, so no re-reading.
    // `env()` isn't substituted: no environment is given here.
    if super::value::has_substitution(&values) {
        return Err(InvalidValue);
    }
    parse_value(property, &values)
}

fn substitute_into<'a>(
    value: &[ComponentValue<'a>],
    custom: &Resolved<'a>,
    out: &mut Vec<ComponentValue<'a>>,
    count: &mut usize,
    depth: usize,
) -> Result<(), GuaranteedInvalid> {
    if depth > MAX_DEPTH {
        return Err(GuaranteedInvalid);
    }
    for v in value {
        *count += 1;
        if *count > MAX_VALUES {
            return Err(GuaranteedInvalid);
        }
        match v {
            ComponentValue::Function(f) if f.name.eq_ignore_ascii_case("var") => {
                let (name, fallback) = var_parts(f).ok_or(GuaranteedInvalid)?;
                match custom.get(name) {
                    Some(found) => {
                        *count += found.len();
                        if *count > MAX_VALUES {
                            return Err(GuaranteedInvalid);
                        }
                        out.extend(found.iter().cloned());
                    }
                    None => {
                        let fallback = fallback.ok_or(GuaranteedInvalid)?;
                        substitute_into(trim_slice(fallback), custom, out, count, depth + 1)?;
                    }
                }
            }
            ComponentValue::Function(f) => {
                let mut arguments = Vec::new();
                substitute_into(&f.arguments, custom, &mut arguments, count, depth + 1)?;
                out.push(ComponentValue::Function(Function {
                    name: f.name.clone(),
                    arguments,
                    span: f.span,
                }));
            }
            ComponentValue::Block(b) => {
                let mut contents = Vec::new();
                substitute_into(&b.contents, custom, &mut contents, count, depth + 1)?;
                out.push(ComponentValue::Block(SimpleBlock {
                    bracket: b.bracket,
                    contents,
                    span: b.span,
                }));
            }
            v => out.push(v.clone()),
        }
    }
    Ok(())
}

/// `var(--name)` or `var(--name, fallback)`: the name, and the fallback if
/// there's a comma. `None` if it isn't a well-formed `var()`.
fn var_parts<'v, 'a>(f: &'v Function<'a>) -> Option<(&'v str, Option<&'v [ComponentValue<'a>]>)> {
    let args = &f.arguments;
    let start = args.iter().position(|v| !v.is_whitespace())?;
    let ComponentValue::Token(Token::Ident(name), _) = &args[start] else {
        return None;
    };
    if !name.starts_with("--") {
        return None;
    }
    let rest = &args[start + 1..];
    let comma = rest
        .iter()
        .position(|v| matches!(v, ComponentValue::Token(Token::Comma, _)));
    match comma {
        Some(i) if rest[..i].iter().all(ComponentValue::is_whitespace) => {
            Some((name, Some(&rest[i + 1..])))
        }
        None if rest.iter().all(ComponentValue::is_whitespace) => Some((name, None)),
        _ => None,
    }
}

/// Every custom property name a `var()` in `value` refers to.
fn references<'v>(value: &'v [ComponentValue<'_>], found: &mut impl FnMut(&'v str)) {
    let mut stack = vec![value];
    while let Some(list) = stack.pop() {
        for v in list {
            match v {
                ComponentValue::Function(f) => {
                    if f.name.eq_ignore_ascii_case("var")
                        && let Some((name, _)) = var_parts(f)
                    {
                        found(name);
                    }
                    stack.push(&f.arguments);
                }
                ComponentValue::Block(b) => stack.push(&b.contents),
                ComponentValue::Token(..) => {}
            }
        }
    }
}

fn trim(mut v: Vec<ComponentValue<'_>>) -> Vec<ComponentValue<'_>> {
    while v.last().is_some_and(ComponentValue::is_whitespace) {
        v.pop();
    }
    let lead = v.iter().take_while(|c| c.is_whitespace()).count();
    v.drain(..lead);
    v
}

fn trim_slice<'v, 'a>(v: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = v.iter().position(|c| !c.is_whitespace()).unwrap_or(v.len());
    let end = v
        .iter()
        .rposition(|c| !c.is_whitespace())
        .map_or(start, |i| i + 1);
    &v[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(decls: &[(&'static str, &'static str)]) -> Resolved<'static> {
        let mut p = CustomProperties::new();
        for (n, v) in decls {
            p.declare(n, crate::parse_component_values(v));
        }
        p.resolve(None)
    }

    #[test]
    fn fallbacks_chains_and_cycles() {
        let r = props(&[
            ("--a", "1px"),
            ("--b", "var(--a) var(--missing, 2px)"),
            ("--c", "var(--missing)"),
            ("--d", "var(--c, var(--a))"),
            ("--x", "var(--y)"),
            ("--y", "var(--z, 1)"),
            ("--z", "var(--x)"),
            ("--w", "var(--x, ok)"),
            ("--self", "var(--self)"),
            ("--i", "initial"),
        ]);
        assert_eq!(r.get_css("--b").as_deref(), Some("1px 2px"));
        assert_eq!(r.get_css("--c"), None);
        assert_eq!(r.get_css("--d").as_deref(), Some("1px"));
        // A cycle through a fallback is still a cycle.
        for n in ["--x", "--y", "--z", "--self", "--i"] {
            assert_eq!(r.get_css(n), None, "{n}");
        }
        // Referring to a cycle isn't being in one.
        assert_eq!(r.get_css("--w").as_deref(), Some("ok"));
    }

    #[test]
    fn exponential_growth_is_cut_off() {
        let mut decls = vec![("--l0".to_string(), "x x x x x x x x x x".to_string())];
        for i in 1..12 {
            let prev = format!("var(--l{})", i - 1);
            decls.push((format!("--l{i}"), [prev.as_str(); 10].join(" ")));
        }
        let texts: Vec<(&'static str, &'static str)> = decls
            .into_iter()
            .map(|(n, v)| {
                (
                    &*Box::leak(n.into_boxed_str()),
                    &*Box::leak(v.into_boxed_str()),
                )
            })
            .collect();
        let r = props(&texts);
        assert!(r.get("--l3").is_some());
        assert_eq!(r.get("--l11"), None);
    }

    #[test]
    fn long_chains_resolve_without_recursion() {
        let names: Vec<String> = (0..50_000).map(|i| format!("--v{i}")).collect();
        let texts: Vec<String> = (0..50_000)
            .map(|i| {
                if i == 0 {
                    "7".into()
                } else {
                    format!("var(--v{})", i - 1)
                }
            })
            .collect();
        let mut p = CustomProperties::new();
        for (n, t) in names.iter().zip(&texts) {
            p.declare(n, crate::parse_component_values(t));
        }
        let r = p.resolve(None);
        assert_eq!(r.get_css("--v49999").as_deref(), Some("7"));
    }

    #[test]
    fn parent_values_and_property_values() {
        let parent = props(&[("--color", "red")]);
        let mut child = CustomProperties::new();
        child.declare(
            "--border",
            crate::parse_component_values("1px solid var(--color)"),
        );
        let r = child.resolve(Some(&parent));
        let value = crate::parse_component_values("var(--border)");
        let v = substitute_value("border-top", &value, &r).unwrap();
        assert_eq!(v.to_string(), "1px solid red");
        let bad = crate::parse_component_values("var(--nope)");
        assert!(substitute_value("color", &bad, &r).is_err());
    }
}
