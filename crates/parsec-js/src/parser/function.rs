// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Functions, arrows, methods and classes (ECMA-262 clause 15).

use parsec_base::Span;

use super::{ClassScope, Ctx, Parser, Private, R};
use crate::ast::{Class, Function, FunctionKind, List, LiteralKind, MethodKind, Node, NodeId};
use crate::text;
use crate::token::{Keyword, Kind, Punct, Token};

/// What a method is, for its parameters and `super`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum MethodType {
    Method,
    Get,
    Set,
    Constructor,
}

impl Parser<'_> {
    /// The context inside a new non-arrow function.
    fn function_ctx(&self, generator: bool, is_async: bool) -> Ctx {
        Ctx {
            strict: self.ctx.strict,
            yield_: generator,
            await_: is_async,
            ret: true,
            new_target: true,
            labels: self.labels.len(),
            ..Ctx::default()
        }
    }

    /// A function declaration or expression, from `function`; `start` is
    /// at `async` if it's async. A declaration's name is optional only in
    /// `export default`.
    pub(super) fn function(
        &mut self,
        start: u32,
        is_async: bool,
        kind: FunctionKind,
        optional_name: bool,
    ) -> R<NodeId> {
        self.enter()?;
        self.expect_kw(Keyword::Function)?;
        let generator = self.eat(Punct::Star)?;
        let outer = self.ctx;
        let mut name = None;
        let id = if self.at(Punct::LParen) {
            if kind == FunctionKind::Declaration && !optional_name {
                return self.unexpected();
            }
            None
        } else {
            name = Some(self.tok);
            // An expression's name is bound inside it, a declaration's
            // outside.
            if kind == FunctionKind::Expression {
                self.ctx.yield_ = generator;
                self.ctx.await_ = is_async;
                self.ctx.static_block = false;
            }
            let id = self.identifier(true);
            self.ctx = outer;
            Some(id?)
        };
        self.ctx = self.function_ctx(generator, is_async);
        let params_start = self.tok.span.start;
        let (params, simple) = self.formal_params()?;
        self.check_params_operators(params_start, generator, is_async)?;
        let body = self.function_body(params, simple, false, name)?;
        self.ctx = outer;
        let f = Function {
            kind,
            id,
            params,
            body,
            is_async,
            generator,
            expression: false,
        };
        self.leave();
        Ok(self.finish(start, Node::Function(f)))
    }

    /// A generator's parameters can't hold `yield`, nor an async
    /// function's `await`.
    fn check_params_operators(&mut self, start: u32, generator: bool, is_async: bool) -> R<()> {
        if generator && self.last_yield.is_some_and(|at| at >= start) {
            let span = Span::at(self.last_yield.unwrap_or(start));
            return self.err(
                "yield-in-params",
                span,
                "a generator's parameters can't use `yield`",
            );
        }
        if is_async && self.last_await.is_some_and(|at| at >= start) {
            let span = Span::at(self.last_await.unwrap_or(start));
            return self.err(
                "await-in-params",
                span,
                "an async function's parameters can't use `await`",
            );
        }
        Ok(())
    }

    /// `( FormalParameters )`, and whether they're all plain identifiers.
    pub(super) fn formal_params(&mut self) -> R<(List, bool)> {
        self.expect(Punct::LParen)?;
        let mark = self.scratch.len();
        let mut simple = true;
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let start = self.tok.span.start;
                self.bump()?;
                let argument = self.binding_target()?;
                let rest = self.finish(start, Node::Rest { argument });
                self.scratch.push(rest);
                simple = false;
                break;
            }
            let param = self.binding_element()?;
            simple &= matches!(self.node(param), Node::Identifier);
            self.scratch.push(param);
            if !self.at(Punct::RParen) {
                self.expect(Punct::Comma)?;
            }
        }
        self.expect(Punct::RParen)?;
        Ok((self.take_list(mark), simple))
    }

    /// `{ FunctionBody }`, checking the parameters against what the body
    /// turned out to be: strict, or not.
    pub(super) fn function_body(
        &mut self,
        params: List,
        simple: bool,
        unique: bool,
        name: Option<Token>,
    ) -> R<NodeId> {
        let start = self.tok.span.start;
        self.expect(Punct::LBrace)?;
        let was_strict = self.ctx.strict;
        let mark = self.scratch.len();
        let use_strict = self.statement_list(true)?;
        if use_strict && !simple {
            return self.err(
                "use-strict-with-complex-params",
                Span::at(start),
                "a function with defaults, patterns or a rest parameter can't say \"use strict\"",
            );
        }
        self.expect(Punct::RBrace)?;
        let body = self.take_list(mark);
        let block = self.finish(start, Node::Block { body });
        self.check_params(params, simple, unique, was_strict, name)?;
        Ok(block)
    }

    /// The early errors on parameters: duplicates where they're not
    /// allowed, and in a body that became strict, names strict code
    /// reserves.
    pub(super) fn check_params(
        &mut self,
        params: List,
        simple: bool,
        unique: bool,
        was_strict: bool,
        name: Option<Token>,
    ) -> R<()> {
        let params = self.ast.get(params).to_vec();
        if self.ctx.strict && !was_strict {
            let mut names = Vec::new();
            for &p in &params {
                self.bound_names(p, &mut names);
            }
            if let Some(t) = name {
                names.push(t.span);
            }
            for span in names {
                let n = self.name(span);
                let reserved = n == "eval"
                    || n == "arguments"
                    || Keyword::from_str(&n)
                        .is_some_and(|k| k.is_strict_reserved() || k == Keyword::Yield);
                if reserved {
                    let n = n.into_owned();
                    return self.err(
                        "strict-binding",
                        span,
                        format!("strict code can't bind `{n}`"),
                    );
                }
            }
        }
        if self.ctx.strict || !simple || unique {
            self.check_duplicates(&params)?;
        }
        Ok(())
    }

    /// No name is bound twice by `patterns`.
    pub(super) fn check_duplicates(&mut self, patterns: &[NodeId]) -> R<()> {
        let mut spans = Vec::new();
        for &p in patterns {
            self.bound_names(p, &mut spans);
        }
        if spans.len() < 2 {
            return Ok(());
        }
        let mut names: Vec<(std::borrow::Cow<str>, Span)> =
            spans.iter().map(|&s| (self.name(s), s)).collect();
        names.sort();
        if let Some(w) = names.windows(2).find(|w| w[0].0 == w[1].0) {
            let (name, span) = (w[1].0.to_string(), w[1].1);
            return self.err(
                "duplicate-binding",
                span,
                format!("`{name}` is bound twice here"),
            );
        }
        Ok(())
    }

    /// An arrow's `=>` and body, its parameters converted already.
    pub(super) fn arrow(
        &mut self,
        start: u32,
        params: List,
        simple: bool,
        is_async: bool,
        no_in: bool,
    ) -> R<NodeId> {
        self.expect(Punct::Arrow)?;
        let outer = self.ctx;
        self.ctx = Ctx {
            yield_: false,
            await_: is_async,
            ret: true,
            static_block: false,
            loops: 0,
            breakable: 0,
            labels: self.labels.len(),
            ..outer
        };
        let (body, expression) = if self.at(Punct::LBrace) {
            (self.function_body(params, simple, true, None)?, false)
        } else {
            let body = self.assign(no_in)?;
            self.check_params(params, simple, true, outer.strict, None)?;
            (body, true)
        };
        self.ctx = outer;
        let f = Function {
            kind: FunctionKind::Arrow,
            id: None,
            params,
            body,
            is_async,
            generator: false,
            expression,
        };
        Ok(self.finish(start, Node::Function(f)))
    }

    /// A method's parameters and body, from `(`, as a function expression.
    pub(super) fn method(
        &mut self,
        kind: MethodType,
        is_async: bool,
        generator: bool,
        super_call: bool,
    ) -> R<NodeId> {
        let start = self.tok.span.start;
        let outer = self.ctx;
        self.ctx = Ctx {
            super_prop: true,
            super_call,
            ..self.function_ctx(generator, is_async)
        };
        let (params, simple) = self.formal_params()?;
        let ids = self.ast.get(params);
        let bad = match kind {
            MethodType::Get => !ids.is_empty(),
            MethodType::Set => ids.len() != 1 || matches!(self.node(ids[0]), Node::Rest { .. }),
            _ => false,
        };
        if bad {
            return self.err(
                "accessor-params",
                Span::new(start, self.prev_end),
                if kind == MethodType::Get {
                    "a getter takes no parameters"
                } else {
                    "a setter takes exactly one parameter"
                },
            );
        }
        self.check_params_operators(start, generator, is_async)?;
        let body = self.function_body(params, simple, true, None)?;
        self.ctx = outer;
        let f = Function {
            kind: FunctionKind::Expression,
            id: None,
            params,
            body,
            is_async,
            generator,
            expression: false,
        };
        Ok(self.finish(start, Node::Function(f)))
    }

    /// A property name: an IdentifierName, a string, a number, or
    /// `[expression]`. Whether it's computed.
    pub(super) fn property_name(&mut self) -> R<(NodeId, bool)> {
        let t = self.tok;
        let node = match t.kind {
            Kind::Ident | Kind::Keyword(_) => Node::Identifier,
            Kind::String => Node::Literal(LiteralKind::String),
            Kind::Number => Node::Literal(LiteralKind::Number),
            Kind::BigInt => Node::Literal(LiteralKind::BigInt),
            Kind::Punct(Punct::LBracket) => {
                self.bump()?;
                let key = self.assign(false)?;
                self.expect(Punct::RBracket)?;
                return Ok((key, true));
            }
            _ => return self.unexpected(),
        };
        self.bump()?;
        Ok((self.ast.push(t.span, node), false))
    }

    /// The name a non-computed key says, for `constructor`, `prototype`
    /// and `__proto__`.
    pub(super) fn key_is(&self, key: NodeId, name: &str) -> bool {
        let span = self.span(key);
        match self.node(key) {
            Node::Identifier => self.name(span) == name,
            Node::Literal(LiteralKind::String) => {
                let body = Span::new(span.start + 1, span.end - 1).text(self.src);
                text::string_value(body)
                    .iter()
                    .copied()
                    .eq(name.encode_utf16())
            }
            _ => false,
        }
    }

    /// A class declaration or expression, from `class`.
    pub(super) fn class(
        &mut self,
        start: u32,
        declaration: bool,
        optional_name: bool,
    ) -> R<NodeId> {
        self.enter()?;
        self.bump()?;
        let outer_strict = self.ctx.strict;
        self.ctx.strict = true;
        let id = if self.at_identifier() {
            Some(self.identifier(true)?)
        } else {
            if declaration && !optional_name {
                return self.unexpected();
            }
            None
        };
        let super_class = if self.at_kw(Keyword::Extends) {
            self.bump()?;
            let heritage = self.lhs()?;
            if self.is_arrow(heritage) {
                let span = self.span(heritage);
                return self.err(
                    "arrow-operand",
                    span,
                    "an arrow function needs parentheses here",
                );
            }
            Some(heritage)
        } else {
            None
        };
        let bstart = self.tok.span.start;
        self.expect(Punct::LBrace)?;
        self.classes.push(ClassScope::default());
        let mark = self.scratch.len();
        let mut constructor = false;
        while !self.at(Punct::RBrace) {
            if self.eat(Punct::Semicolon)? {
                continue;
            }
            let element = self.class_element(super_class.is_some(), &mut constructor)?;
            self.scratch.push(element);
        }
        self.bump()?;
        self.end_class_scope()?;
        let body = self.take_list(mark);
        let body = self.finish(bstart, Node::ClassBody { body });
        self.ctx.strict = outer_strict;
        let c = Class {
            declaration,
            id,
            super_class,
            body,
        };
        self.leave();
        Ok(self.finish(start, Node::Class(c)))
    }

    /// Whether the token after a modifier (`static`, `get`, `async`) says
    /// the modifier is the element's name instead.
    fn modifier_is_name(&self, async_like: bool) -> bool {
        let next = self.peek();
        matches!(
            next.kind,
            Kind::Punct(Punct::LParen | Punct::Eq | Punct::Semicolon | Punct::RBrace | Punct::Colon | Punct::Comma)
                | Kind::Eof
        ) || async_like && next.newline_before()
            // `get *x() {}` is no getter, so `get` ⟨newline⟩ `*x() {}` is
            // a field named `get` and a generator.
            || (self.at_kw(Keyword::Get) || self.at_kw(Keyword::Set)) && next.kind == Kind::Punct(Punct::Star)
    }

    fn class_element(&mut self, derived: bool, constructor: &mut bool) -> R<NodeId> {
        let start = self.tok.span.start;
        let mut is_static = false;
        if self.at_kw(Keyword::Static) && !self.modifier_is_name(false) {
            self.bump()?;
            if self.at(Punct::LBrace) {
                return self.static_block(start);
            }
            is_static = true;
        }
        let mut is_async = false;
        let mut generator = false;
        let mut kind = MethodType::Method;
        if self.at_kw(Keyword::Async) && !self.modifier_is_name(true) {
            self.bump()?;
            is_async = true;
        }
        if self.eat(Punct::Star)? {
            generator = true;
        }
        if !is_async
            && !generator
            && (self.at_kw(Keyword::Get) || self.at_kw(Keyword::Set))
            && !self.modifier_is_name(false)
        {
            kind = if self.at_kw(Keyword::Get) {
                MethodType::Get
            } else {
                MethodType::Set
            };
            self.bump()?;
        }
        let (key, computed, private) = if self.tok.kind == Kind::PrivateName {
            let t = self.bump()?;
            (self.ast.push(t.span, Node::PrivateIdentifier), false, true)
        } else {
            let (key, computed) = self.property_name()?;
            (key, computed, false)
        };
        let named = |p: &Self, name: &str| !computed && !private && p.key_is(key, name);
        if self.at(Punct::LParen) || is_async || generator || kind != MethodType::Method {
            let is_ctor = !is_static && named(self, "constructor");
            if is_ctor {
                if kind != MethodType::Method || generator || is_async {
                    let span = self.span(key);
                    return self.err(
                        "special-constructor",
                        span,
                        "a constructor can't be a getter, setter, generator or async",
                    );
                }
                if *constructor {
                    let span = self.span(key);
                    return self.err(
                        "duplicate-constructor",
                        span,
                        "a class can have only one constructor",
                    );
                }
                *constructor = true;
                kind = MethodType::Constructor;
            }
            if is_static && named(self, "prototype") {
                let span = self.span(key);
                return self.err(
                    "static-prototype",
                    span,
                    "a static member can't be called `prototype`",
                );
            }
            if private {
                let p = match kind {
                    MethodType::Get => Private::Get,
                    MethodType::Set => Private::Set,
                    _ => Private::Method,
                };
                let span = self.span(key);
                self.declare_private(span, p, is_static)?;
            }
            let value = self.method(kind, is_async, generator, is_ctor && derived)?;
            let kind = match kind {
                MethodType::Method => MethodKind::Method,
                MethodType::Get => MethodKind::Get,
                MethodType::Set => MethodKind::Set,
                MethodType::Constructor => MethodKind::Constructor,
            };
            return Ok(self.finish(
                start,
                Node::Method {
                    key,
                    value,
                    kind,
                    computed,
                    is_static,
                },
            ));
        }
        if named(self, "constructor") || is_static && named(self, "prototype") {
            let span = self.span(key);
            return self.err(
                "bad-field-name",
                span,
                "a field can't be called `constructor`, nor a static one `prototype`",
            );
        }
        if private {
            let span = self.span(key);
            self.declare_private(span, Private::Field, is_static)?;
        }
        let value = if self.eat(Punct::Eq)? {
            let outer = self.ctx;
            self.ctx = Ctx {
                yield_: false,
                await_: false,
                ret: false,
                new_target: true,
                super_prop: true,
                super_call: false,
                no_arguments: true,
                static_block: false,
                loops: 0,
                breakable: 0,
                labels: self.labels.len(),
                ..outer
            };
            let value = self.assign(false);
            self.ctx = outer;
            Some(value?)
        } else {
            None
        };
        self.semicolon()?;
        Ok(self.finish(
            start,
            Node::Field {
                key,
                value,
                computed,
                is_static,
            },
        ))
    }

    /// `static { ... }`, from the `{`.
    fn static_block(&mut self, start: u32) -> R<NodeId> {
        self.bump()?;
        let outer = self.ctx;
        self.ctx = Ctx {
            strict: true,
            new_target: true,
            super_prop: true,
            no_arguments: true,
            static_block: true,
            labels: self.labels.len(),
            ..Ctx::default()
        };
        let mark = self.scratch.len();
        let result = self.statement_list(false);
        self.ctx = outer;
        result?;
        self.expect(Punct::RBrace)?;
        let body = self.take_list(mark);
        Ok(self.finish(start, Node::StaticBlock { body }))
    }
}
