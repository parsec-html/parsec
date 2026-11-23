// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The shapes pages keep their state in, found at a script's top level.

use std::collections::HashMap;

use parsec_base::{Diagnostic, Span};

use super::{Json, Reader, Refused, json, refuse};
use crate::ast::{AssignOp, Ast, LiteralKind, Node, NodeId};
use crate::{parse_script, text};

/// A piece of page state: where a script put it, and its value, or why
/// it isn't data.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    /// What the script assigned: `window.__NUXT__`, `window.__NUXT__.config`,
    /// `ytInitialData` for a top-level `var`, `self.__next_f` for each of
    /// Next.js's pushes. A key that isn't a name is written as a JSON
    /// string in brackets, `window["a-b"]`.
    pub name: String,
    /// The value's expression in the source.
    pub span: Span,
    pub value: Result<Json, Refused>,
}

/// Parses `src` as a script, the text of one `<script>` element, and
/// finds the page state in it: see [`page_state_in`]. A syntax error is
/// the `Err`.
///
/// ```
/// use parsec_js::data::page_state;
///
/// let src = r#"window.__NUXT__=(function(a,b){return {layout:a,data:[{title:b}]}}("default","Home"));"#;
/// let found = page_state(src).unwrap();
/// assert_eq!(found[0].name, "window.__NUXT__");
/// assert_eq!(
///     found[0].value.as_ref().unwrap().to_json_string(),
///     r#"{"layout":"default","data":[{"title":"Home"}]}"#
/// );
/// ```
pub fn page_state(src: &str) -> Result<Vec<Found>, Diagnostic> {
    Ok(page_state_in(&parse_script(src)?, src))
}

/// The page state in a parsed script, in source order. Only statements
/// at the top level count, and of those:
///
/// - an assignment with `=` to a property of `window`, `self` or
///   `globalThis`, however deep (`window.a.b = …`, `window["a"] = …`),
///   alone or in a comma list;
/// - a `var`, `let` or `const` declaration, each name with a value;
/// - `self.__next_f.push(…)`, Next.js App Router's way of handing its
///   payload over a chunk at a time: [`next_flight`] puts them together.
///
/// The value is read with [`literal`](super::literal), with two more
/// shapes:
///
/// - `JSON.parse("…")` with one string literal or template without
///   substitutions: the string is parsed with [`json`], and a refusal's
///   span is the string's;
/// - a function called at once, the way Nuxt 2 writes
///   `window.__NUXT__=(function(a,b){return {…}}(1,"x"))`: a function
///   or arrow, not async or a generator, with plain parameters, whose
///   body only returns a value. Each parameter takes its argument's
///   value, which must be a literal, or `undefined` past the last one.
///   Substitution may copy at most 16 times the source's size.
pub fn page_state_in(ast: &Ast, src: &str) -> Vec<Found> {
    let mut found = Vec::new();
    let Node::Program { body, .. } = *ast.node(ast.root()) else {
        return found;
    };
    for &stmt in ast.get(body) {
        match *ast.node(stmt) {
            Node::Expression { expression, .. } => {
                let e = ast.unparenthesized(expression);
                match *ast.node(e) {
                    Node::Sequence { expressions } => {
                        for &e in ast.get(expressions) {
                            statement(ast, src, ast.unparenthesized(e), &mut found);
                        }
                    }
                    _ => statement(ast, src, e, &mut found),
                }
            }
            Node::VariableDeclaration { declarations, .. } => {
                for &d in ast.get(declarations) {
                    let Node::VariableDeclarator {
                        id,
                        init: Some(init),
                    } = *ast.node(d)
                    else {
                        continue;
                    };
                    if let Node::Identifier = ast.node(id) {
                        let name = text::ident_name(ast.span(id).text(src)).into_owned();
                        found.push(read(ast, src, name, init));
                    }
                }
            }
            _ => {}
        }
    }
    found
}

/// The payload Next.js App Router streamed into the page, put back
/// together from the `self.__next_f` pushes in `found`, in order.
///
/// Each push is an array: `[0]` starts the stream, `[1, "…"]` is a piece
/// of the payload, `[2, "…"]` is form state and `[3, "…"]` a piece in
/// base64. Only the type 1 pieces are text, and only together: a row can
/// start in one push and end in the next. What they make is React's
/// Flight format, rows of `id:value` ending in newlines; most values are
/// JSON, and reading them is up to the caller.
///
/// ```
/// use parsec_js::data::{next_flight, page_state};
///
/// let src = r#"self.__next_f.push([1,"0:[\"$\",\"div\""]);self.__next_f.push([1,",null,{}]\n"]);"#;
/// let flight = next_flight(&page_state(src).unwrap());
/// assert_eq!(flight, "0:[\"$\",\"div\",null,{}]\n");
/// ```
pub fn next_flight(found: &[Found]) -> String {
    let mut out = String::new();
    for f in found.iter().filter(|f| f.name == "self.__next_f") {
        if let Ok(Json::Array(items)) = &f.value
            && let [Json::Number(n), Json::String(s)] = items.as_slice()
            && *n == 1.0
        {
            out.push_str(s);
        }
    }
    out
}

fn statement(ast: &Ast, src: &str, e: NodeId, found: &mut Vec<Found>) {
    match *ast.node(e) {
        Node::Assign {
            operator: AssignOp::Assign,
            left,
            right,
        } => {
            if let Some(name) = global(ast, src, left) {
                found.push(read(ast, src, name, right));
            }
        }
        Node::Call {
            callee, arguments, ..
        } => {
            let [arg] = ast.get(arguments) else { return };
            let Node::Member {
                object, property, ..
            } = *ast.node(callee)
            else {
                return;
            };
            if name_of(ast, src, property) == Some("push")
                && member_text(ast, src, object) == Some(("self", "__next_f"))
            {
                let value = match ast.node(*arg) {
                    Node::Spread { .. } => refuse(ast.span(*arg), "spread"),
                    _ => super::literal(ast, src, *arg),
                };
                found.push(Found {
                    name: "self.__next_f".to_string(),
                    span: ast.span(*arg),
                    value,
                });
            }
        }
        _ => {}
    }
}

/// The text of a plain identifier.
fn name_of<'a>(ast: &Ast, src: &'a str, id: NodeId) -> Option<&'a str> {
    let span = ast.span(id);
    matches!(ast.node(id), Node::Identifier).then(|| span.text(src))
}

/// `a.b` with both plain names, as `("a", "b")`.
fn member_text<'a>(ast: &Ast, src: &'a str, id: NodeId) -> Option<(&'a str, &'a str)> {
    match *ast.node(id) {
        Node::Member {
            object,
            property,
            computed: false,
            optional: false,
        } => Some((name_of(ast, src, object)?, name_of(ast, src, property)?)),
        _ => None,
    }
}

/// `window.a.b`'s name, for a chain of properties from `window`, `self`
/// or `globalThis`, each a name or a string.
fn global(ast: &Ast, src: &str, left: NodeId) -> Option<String> {
    let mut keys = Vec::new();
    let mut id = left;
    while let Node::Member {
        object,
        property,
        computed,
        optional: false,
    } = *ast.node(id)
    {
        let span = ast.span(property);
        let key = match ast.node(property) {
            Node::Identifier if !computed => text::ident_name(span.text(src)).into_owned(),
            Node::Literal(LiteralKind::String) if computed => {
                let body = Span::new(span.start + 1, span.end - 1).text(src);
                String::from_utf16(&text::string_value(body)).ok()?
            }
            _ => return None,
        };
        keys.push(key);
        id = object;
    }
    let root = name_of(ast, src, id)?;
    if keys.is_empty() || !matches!(root, "window" | "self" | "globalThis") {
        return None;
    }
    let mut name = root.to_string();
    for key in keys.iter().rev() {
        let plain = key.chars().enumerate().all(|(i, c)| {
            c == '_' || c == '$' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
        });
        if plain && !key.is_empty() {
            name.push('.');
            name.push_str(key);
        } else {
            name.push('[');
            text::write_json_str(&mut name, key);
            name.push(']');
        }
    }
    Some(name)
}

fn read(ast: &Ast, src: &str, name: String, value: NodeId) -> Found {
    Found {
        name,
        span: ast.span(value),
        value: state(ast, src, value),
    }
}

/// The value assigned: `JSON.parse` of a string, a function called with
/// literals, or a literal.
fn state(ast: &Ast, src: &str, id: NodeId) -> Result<Json, Refused> {
    let id = ast.unparenthesized(id);
    if let Node::Call {
        callee,
        arguments,
        optional: false,
    } = *ast.node(id)
    {
        let callee = ast.unparenthesized(callee);
        if member_text(ast, src, callee) == Some(("JSON", "parse")) {
            return json_parse(ast, src, ast.get(arguments))
                .unwrap_or_else(|| refuse(ast.span(id), "not-a-literal"));
        }
        if let Node::Function(f) = *ast.node(callee)
            && !f.is_async
            && !f.generator
        {
            return call(ast, src, f, ast.get(arguments));
        }
    }
    super::literal(ast, src, id)
}

/// `JSON.parse(text)`'s value, `None` unless `args` is one string.
fn json_parse(ast: &Ast, src: &str, args: &[NodeId]) -> Option<Result<Json, Refused>> {
    let &[arg] = args else { return None };
    let arg = ast.unparenthesized(arg);
    if !matches!(
        ast.node(arg),
        Node::Literal(LiteralKind::String) | Node::Template { .. }
    ) {
        return None;
    }
    // The text's offsets don't map back through the string's escapes, so
    // a refusal points at the string.
    let span = ast.span(arg);
    Some(match super::literal(ast, src, arg) {
        Ok(Json::String(s)) => json(&s).map_err(|r| Refused { span, ..r }),
        Ok(_) => refuse(span, "not-a-literal"),
        Err(r) => Err(r),
    })
}

/// The value a function returns, called with `args`.
fn call(ast: &Ast, src: &str, f: crate::ast::Function, args: &[NodeId]) -> Result<Json, Refused> {
    let mut reader = Reader::new(ast, src);
    let mut values = Vec::with_capacity(args.len());
    for &arg in args {
        if let Node::Spread { .. } = ast.node(arg) {
            return refuse(ast.span(arg), "spread");
        }
        values.push(reader.value(arg)?);
    }
    let mut env = HashMap::new();
    let mut values = values.into_iter();
    for &param in ast.get(f.params) {
        if !matches!(ast.node(param), Node::Identifier) {
            return refuse(ast.span(param), "parameters");
        }
        let name = text::ident_name(ast.span(param).text(src)).into_owned();
        env.insert(name, values.next().flatten());
    }
    let returned = if f.expression {
        f.body
    } else {
        let Node::Block { body } = *ast.node(f.body) else {
            return refuse(ast.span(f.body), "function-body");
        };
        match ast.get(body) {
            [stmt] => match *ast.node(*stmt) {
                Node::Return {
                    argument: Some(arg),
                } => arg,
                _ => return refuse(ast.span(f.body), "function-body"),
            },
            _ => return refuse(ast.span(f.body), "function-body"),
        }
    };
    reader.env = env;
    // shortcut: a flat cap of 16 bytes copied per source byte, which Nuxt's
    // output stays far below; a caller-set limit if one ever needs more.
    reader.budget = src.len().saturating_mul(16);
    reader.whole(returned)
}
