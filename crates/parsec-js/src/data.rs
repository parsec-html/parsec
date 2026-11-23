// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Data from scripts: the state a page writes into a `<script>` as a
//! literal, read as JSON without running anything.
//!
//! [`literal`] turns an expression made only of literals into a [`Json`]
//! value, and refuses anything else with the span that stopped it.
//! [`page_state`] finds the shapes pages put their state in
//! (`window.__NUXT__ = …`, `var ytInitialData = …`,
//! `JSON.parse("…")`, Next.js's `self.__next_f.push(…)`) and reads each
//! one's value. [`json`] parses JSON text strictly, as `JSON.parse` does,
//! for state that's in a script as JSON, like Next.js's `__NEXT_DATA__`.
//!
//! Nothing is evaluated. No property is read, no function called and no
//! name looked up, with one exception: the arguments of a Nuxt function
//! that only returns a literal are put in place of its parameters.
//!
//! ```
//! use parsec_js::data::{Json, page_state};
//!
//! let src = r#"window.__STATE__ = {user: {name: "Ada", admin: !1}, cart: [1, , 3]};"#;
//! let found = page_state(src).unwrap();
//! assert_eq!(found[0].name, "window.__STATE__");
//! let value = found[0].value.as_ref().unwrap();
//! assert_eq!(
//!     value.to_json_string(),
//!     r#"{"user":{"name":"Ada","admin":false},"cart":[1,null,3]}"#
//! );
//! ```

mod json;
mod page;

use std::collections::HashMap;
use std::fmt;

use parsec_base::Span;

use crate::ast::{Ast, LiteralKind, Node, NodeId, PropertyKind, UnaryOp};
use crate::text;

pub use json::json;
pub use page::{Found, next_flight, page_state, page_state_in};

/// A JSON value. An object keeps its keys in the order they first
/// appear, each once.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    /// Never NaN; it may be infinite, from `1e400`, which JSON writes
    /// as `null` as `JSON.stringify` does.
    Number(f64),
    String(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    /// The value as JSON text, the way `JSON.stringify` writes it: no
    /// spaces, numbers as JavaScript prints them, `-0` as `0`, and an
    /// infinite number as `null`.
    pub fn to_json_string(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Number(x) => text::write_js_number(out, *x),
            Json::String(s) => text::write_json_str(out, s),
            Json::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write(out);
                }
                out.push(']');
            }
            Json::Object(entries) => {
                out.push('{');
                for (i, (key, value)) in entries.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    text::write_json_str(out, key);
                    out.push(':');
                    value.write(out);
                }
                out.push('}');
            }
        }
    }

    /// How much copying this value costs: a unit a value, plus its
    /// strings' bytes.
    fn weight(&self) -> usize {
        match self {
            Json::String(s) => 1 + s.len(),
            Json::Array(items) => 1 + items.iter().map(Json::weight).sum::<usize>(),
            Json::Object(entries) => {
                1 + entries
                    .iter()
                    .map(|(k, v)| k.len() + v.weight())
                    .sum::<usize>()
            }
            _ => 1,
        }
    }
}

/// Why a value isn't data, and where it stopped being.
///
/// `reason` is one of:
///
/// | Reason | What it was |
/// |---|---|
/// | `not-a-literal` | a name, a call, an operator, a function: anything that would have to run |
/// | `substitution` | a template's `${…}`; the span is the expression in it |
/// | `bigint` | a BigInt, which JSON has no way to write |
/// | `regexp` | a regular expression literal |
/// | `computed-key` | `{[k]: v}` |
/// | `method` | a method, getter or setter |
/// | `spread` | `...x` in an array, an object or arguments |
/// | `proto` | `__proto__: v`, which sets the prototype rather than a key |
/// | `not-a-number` | `-`, `+` or `!` on something other than a number (or, for `!`, a boolean) |
/// | `undefined` | the whole value is `undefined`, which JSON can't hold |
/// | `lone-surrogate` | a string with half a surrogate pair, which a Rust string can't hold |
/// | `parameters` | a Nuxt function's parameter that isn't a plain name |
/// | `function-body` | a Nuxt function that does more than return a value |
/// | `too-large` | substituting arguments would copy far more than the source holds |
/// | `json-syntax` | JSON text that `JSON.parse` would reject |
/// | `too-deep` | JSON nested deeper than [`DEFAULT_MAX_DEPTH`](crate::DEFAULT_MAX_DEPTH) |
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub span: Span,
    pub reason: &'static str,
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at {}..{}",
            self.reason, self.span.start, self.span.end
        )
    }
}

impl std::error::Error for Refused {}

fn refuse<T>(span: Span, reason: &'static str) -> Result<T, Refused> {
    Err(Refused { span, reason })
}

/// The value of `expr`, an expression in `ast` parsed from `src`, if it's
/// made only of literals:
///
/// - objects with plain keys (names, strings, numbers), arrays, strings,
///   numbers, `true`, `false`, `null`, and templates without
///   substitutions;
/// - `-` and `+` on a number, `!` on a number or boolean (minifiers write
///   `!0` for `true` and `!1` for `false`), and parentheses;
/// - `undefined` and `void` on a literal, which, as in `JSON.stringify`,
///   leave a key out of an object and are `null` in an array; a hole in
///   an array is `null` too.
///
/// A key given twice keeps its first place and its last value, as in
/// JavaScript. Keys stay in source order; JavaScript would put the ones
/// that are array indices first. Anything else is refused: see
/// [`Refused`].
///
/// ```
/// use parsec_js::{Node, parse_script};
/// use parsec_js::data::literal;
///
/// let src = "x = {a: [1, -2.5, `t`], b: void 0, 'c': !0}";
/// let ast = parse_script(src).unwrap();
/// let Node::Program { body, .. } = *ast.node(ast.root()) else { unreachable!() };
/// let Node::Expression { expression, .. } = *ast.node(ast.get(body)[0]) else { unreachable!() };
/// let Node::Assign { right, .. } = *ast.node(expression) else { unreachable!() };
/// let value = literal(&ast, src, right).unwrap();
/// assert_eq!(value.to_json_string(), r#"{"a":[1,-2.5,"t"],"c":true}"#);
/// ```
pub fn literal(ast: &Ast, src: &str, expr: NodeId) -> Result<Json, Refused> {
    Reader::new(ast, src).whole(expr)
}

/// Reads literals out of a tree, with the parameters of a Nuxt function
/// bound to their arguments' values.
struct Reader<'a> {
    ast: &'a Ast,
    src: &'a str,
    /// A parameter's value, `None` for `undefined`.
    env: HashMap<String, Option<Json>>,
    /// What substituting may still copy, by [`Json::weight`].
    budget: usize,
}

impl<'a> Reader<'a> {
    fn new(ast: &'a Ast, src: &'a str) -> Reader<'a> {
        Reader {
            ast,
            src,
            env: HashMap::new(),
            budget: 0,
        }
    }

    /// The value of `id`, which may not be `undefined`.
    fn whole(&mut self, id: NodeId) -> Result<Json, Refused> {
        match self.value(id)? {
            Some(v) => Ok(v),
            None => refuse(self.ast.span(id), "undefined"),
        }
    }

    fn string(&self, span: Span, units: &[u16]) -> Result<String, Refused> {
        String::from_utf16(units).or_else(|_| refuse(span, "lone-surrogate"))
    }

    /// The value of `id`, `None` for `undefined`.
    fn value(&mut self, id: NodeId) -> Result<Option<Json>, Refused> {
        let span = self.ast.span(id);
        let v = match *self.ast.node(id) {
            Node::Paren { expression } => return self.value(expression),
            Node::Literal(kind) => match kind {
                LiteralKind::String => {
                    let body = Span::new(span.start + 1, span.end - 1).text(self.src);
                    Json::String(self.string(span, &text::string_value(body))?)
                }
                LiteralKind::Number => Json::Number(text::number_value(span.text(self.src))),
                LiteralKind::Boolean => Json::Bool(span.text(self.src) == "true"),
                LiteralKind::Null => Json::Null,
                LiteralKind::BigInt => return refuse(span, "bigint"),
                LiteralKind::RegExp => return refuse(span, "regexp"),
            },
            Node::Template {
                quasis,
                expressions,
            } => {
                if let Some(&first) = self.ast.get(expressions).first() {
                    return refuse(self.ast.span(first), "substitution");
                }
                let quasi = self.ast.get(quasis)[0];
                let body = self.ast.span(quasi).text(self.src);
                // An untagged template's escapes are all valid, or it
                // wouldn't have parsed.
                let units = text::template_cooked(body).unwrap_or_default();
                Json::String(self.string(span, &units)?)
            }
            Node::Identifier => {
                let name = text::ident_name(span.text(self.src));
                if let Some(v) = self.env.get(name.as_ref()) {
                    let weight = v.as_ref().map_or(1, Json::weight);
                    if weight > self.budget {
                        return refuse(span, "too-large");
                    }
                    self.budget -= weight;
                    return Ok(v.clone());
                }
                if name == "undefined" {
                    return Ok(None);
                }
                return refuse(span, "not-a-literal");
            }
            Node::Unary { operator, argument } => {
                let arg = self.value(argument)?;
                return match (operator, arg) {
                    (UnaryOp::Void, _) => Ok(None),
                    (UnaryOp::Minus, Some(Json::Number(x))) => Ok(Some(Json::Number(-x))),
                    (UnaryOp::Plus, Some(Json::Number(x))) => Ok(Some(Json::Number(x))),
                    (UnaryOp::Not, Some(Json::Number(x))) => {
                        Ok(Some(Json::Bool(x == 0.0 || x.is_nan())))
                    }
                    (UnaryOp::Not, Some(Json::Bool(b))) => Ok(Some(Json::Bool(!b))),
                    (UnaryOp::Minus | UnaryOp::Plus | UnaryOp::Not, _) => {
                        refuse(self.ast.span(argument), "not-a-number")
                    }
                    _ => refuse(span, "not-a-literal"),
                };
            }
            Node::Array { elements, .. } => {
                let mut items = Vec::with_capacity(elements.len());
                for &e in self.ast.get(elements) {
                    match *self.ast.node(e) {
                        Node::Elision => items.push(Json::Null),
                        Node::Spread { .. } => return refuse(self.ast.span(e), "spread"),
                        _ => items.push(self.value(e)?.unwrap_or(Json::Null)),
                    }
                }
                Json::Array(items)
            }
            Node::Object { properties, .. } => {
                let mut object = Object::default();
                for &p in self.ast.get(properties) {
                    let Node::Property {
                        key,
                        value,
                        kind,
                        method,
                        shorthand,
                        computed,
                    } = *self.ast.node(p)
                    else {
                        return refuse(self.ast.span(p), "spread");
                    };
                    if method || kind != PropertyKind::Init {
                        return refuse(self.ast.span(p), "method");
                    }
                    if computed {
                        return refuse(self.ast.span(key), "computed-key");
                    }
                    let name = self.key(key)?;
                    if name == "__proto__" && !shorthand {
                        return refuse(self.ast.span(key), "proto");
                    }
                    let v = self.value(value)?;
                    object.set(name, v);
                }
                object.finish()
            }
            _ => return refuse(span, "not-a-literal"),
        };
        Ok(Some(v))
    }

    /// A property key that isn't computed: a name, a string or a number.
    fn key(&self, key: NodeId) -> Result<String, Refused> {
        let span = self.ast.span(key);
        let raw = span.text(self.src);
        match *self.ast.node(key) {
            Node::Literal(LiteralKind::String) => {
                let body = Span::new(span.start + 1, span.end - 1).text(self.src);
                self.string(span, &text::string_value(body))
            }
            Node::Literal(LiteralKind::Number) => {
                let mut s = String::new();
                text::write_js_number(&mut s, text::number_value(raw));
                Ok(s)
            }
            Node::Literal(LiteralKind::BigInt) => refuse(span, "bigint"),
            _ => Ok(text::ident_name(raw).into_owned()),
        }
    }
}

/// An object being built: a key given again keeps its place and takes
/// the new value, and a key whose value is `undefined` is left out.
#[derive(Default)]
struct Object {
    entries: Vec<(String, Option<Json>)>,
    index: HashMap<String, usize>,
}

impl Object {
    fn set(&mut self, key: String, value: Option<Json>) {
        match self.index.get(&key) {
            Some(&i) => self.entries[i].1 = value,
            None => {
                self.index.insert(key.clone(), self.entries.len());
                self.entries.push((key, value));
            }
        }
    }

    fn finish(self) -> Json {
        Json::Object(
            self.entries
                .into_iter()
                .filter_map(|(k, v)| Some((k, v?)))
                .collect(),
        )
    }
}
