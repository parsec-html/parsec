// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Expressions (ECMA-262 clause 13), by precedence climbing from
//! assignment down to primary expressions, with the arrow and pattern
//! covers read once.

use parsec_base::Span;

use super::function::MethodType;
use super::{Parser, R, starts_expression};
use crate::ast::{
    AssignOp, BinaryOp, FunctionKind, List, LiteralKind, LogicalOp, Node, NodeId, PropertyKind,
    UnaryOp, UpdateOp,
};
use crate::lexer::Goal;
use crate::token::{Flags, Keyword, Kind, Punct};

/// A binary operator and its precedence, from `??` at 1 to `**` at 12.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Binary(BinaryOp),
    Logical(LogicalOp),
}

const RELATIONAL: u8 = 8;

impl Parser<'_> {
    /// Expression: assignments separated by commas, with no pattern-only
    /// syntax left over.
    pub(super) fn expression(&mut self, no_in: bool) -> R<NodeId> {
        let mark = self.pending.len();
        let e = self.expression_cover(no_in)?;
        self.check_pending(mark)?;
        Ok(e)
    }

    /// An Expression that may yet turn into a pattern (a `for` head's).
    pub(super) fn expression_cover(&mut self, no_in: bool) -> R<NodeId> {
        let start = self.tok.span.start;
        let first = self.assign_cover(no_in)?;
        if !self.at(Punct::Comma) {
            return Ok(first);
        }
        let mark = self.scratch.len();
        self.scratch.push(first);
        while self.eat(Punct::Comma)? {
            let e = self.assign_cover(no_in)?;
            self.scratch.push(e);
        }
        let expressions = self.take_list(mark);
        Ok(self.finish(start, Node::Sequence { expressions }))
    }

    /// AssignmentExpression, checked: nothing in it is left only a
    /// pattern could have.
    pub(super) fn assign(&mut self, no_in: bool) -> R<NodeId> {
        let mark = self.pending.len();
        let e = self.assign_cover(no_in)?;
        self.check_pending(mark)?;
        Ok(e)
    }

    /// AssignmentExpression where the caller may still turn it into a
    /// pattern: an array element, a property value, a parenthesized one.
    pub(super) fn assign_cover(&mut self, no_in: bool) -> R<NodeId> {
        self.enter()?;
        if self.at_kw(Keyword::Yield) && self.ctx.yield_ {
            let e = self.yield_expression(no_in)?;
            self.leave();
            return Ok(e);
        }
        let start = self.tok.span.start;
        let left = self.conditional(no_in)?;
        let Some(operator) = assign_op(self.tok.kind) else {
            self.leave();
            return Ok(left);
        };
        if self.is_arrow(left) {
            return self.unexpected();
        }
        if operator == AssignOp::Assign
            && matches!(self.node(left), Node::Object { .. } | Node::Array { .. })
        {
            self.reinterpret(left, false)?;
        } else {
            let logical = matches!(operator, AssignOp::And | AssignOp::Or | AssignOp::Coalesce);
            self.simple_target(left, !logical)?;
        }
        self.bump()?;
        let right = self.assign(no_in)?;
        self.leave();
        Ok(self.finish(
            start,
            Node::Assign {
                operator,
                left,
                right,
            },
        ))
    }

    pub(super) fn is_arrow(&self, id: NodeId) -> bool {
        matches!(self.node(id), Node::Function(f) if f.kind == FunctionKind::Arrow)
    }

    fn yield_expression(&mut self, no_in: bool) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        self.last_yield = Some(start);
        let (argument, delegate) = if self.tok.newline_before() {
            (None, false)
        } else if self.eat(Punct::Star)? {
            (Some(self.assign(no_in)?), true)
        } else if starts_expression(self.tok.kind) {
            (Some(self.assign(no_in)?), false)
        } else {
            (None, false)
        };
        Ok(self.finish(start, Node::Yield { argument, delegate }))
    }

    fn conditional(&mut self, no_in: bool) -> R<NodeId> {
        let start = self.tok.span.start;
        let test = self.binary(0, no_in)?;
        if !self.at(Punct::Question) {
            return Ok(test);
        }
        if self.is_arrow(test) {
            return self.unexpected();
        }
        self.bump()?;
        let consequent = self.assign(false)?;
        self.expect(Punct::Colon)?;
        let alternate = self.assign(no_in)?;
        Ok(self.finish(
            start,
            Node::Conditional {
                test,
                consequent,
                alternate,
            },
        ))
    }

    /// The binary operator being looked at, and its precedence.
    fn binary_op(&self, no_in: bool) -> Option<(Op, u8)> {
        use BinaryOp as B;
        let (op, prec) = match self.tok.kind {
            Kind::Punct(p) => match p {
                Punct::QuestionQuestion => (Op::Logical(LogicalOp::Coalesce), 1),
                Punct::PipePipe => (Op::Logical(LogicalOp::Or), 2),
                Punct::AmpAmp => (Op::Logical(LogicalOp::And), 3),
                Punct::Pipe => (Op::Binary(B::BitOr), 4),
                Punct::Caret => (Op::Binary(B::BitXor), 5),
                Punct::Amp => (Op::Binary(B::BitAnd), 6),
                Punct::EqEq => (Op::Binary(B::Eq), 7),
                Punct::NotEq => (Op::Binary(B::NotEq), 7),
                Punct::EqEqEq => (Op::Binary(B::StrictEq), 7),
                Punct::NotEqEq => (Op::Binary(B::StrictNotEq), 7),
                Punct::Lt => (Op::Binary(B::Lt), RELATIONAL),
                Punct::Gt => (Op::Binary(B::Gt), RELATIONAL),
                Punct::LtEq => (Op::Binary(B::LtEq), RELATIONAL),
                Punct::GtEq => (Op::Binary(B::GtEq), RELATIONAL),
                Punct::Shl => (Op::Binary(B::Shl), 9),
                Punct::Shr => (Op::Binary(B::Shr), 9),
                Punct::UShr => (Op::Binary(B::UShr), 9),
                Punct::Plus => (Op::Binary(B::Add), 10),
                Punct::Minus => (Op::Binary(B::Sub), 10),
                Punct::Star => (Op::Binary(B::Mul), 11),
                Punct::Slash => (Op::Binary(B::Div), 11),
                Punct::Percent => (Op::Binary(B::Rem), 11),
                Punct::StarStar => (Op::Binary(B::Exp), 12),
                _ => return None,
            },
            Kind::Keyword(Keyword::Instanceof) => (Op::Binary(B::Instanceof), RELATIONAL),
            Kind::Keyword(Keyword::In) if !no_in => (Op::Binary(B::In), RELATIONAL),
            _ => return None,
        };
        Some((op, prec))
    }

    /// Binary operators binding tighter than `min`.
    fn binary(&mut self, min: u8, no_in: bool) -> R<NodeId> {
        let start = self.tok.span.start;
        let mut left = if self.tok.kind == Kind::PrivateName {
            // `#x in o`, the one place a private name stands alone.
            let t = self.bump()?;
            if !self.at_kw(Keyword::In) || no_in || min >= RELATIONAL {
                return self.err(
                    "lone-private-name",
                    t.span,
                    "a private name stands alone only before `in`",
                );
            }
            self.use_private(t.span)?;
            self.ast.push(t.span, Node::PrivateIdentifier)
        } else {
            self.unary()?
        };
        while let Some((op, prec)) = self.binary_op(no_in) {
            if prec <= min {
                break;
            }
            if self.is_arrow(left) {
                return self.unexpected();
            }
            if matches!(self.node(left), Node::PrivateIdentifier) && op != Op::Binary(BinaryOp::In)
            {
                let span = self.span(left);
                return self.err(
                    "lone-private-name",
                    span,
                    "a private name stands alone only before `in`",
                );
            }
            let op_span = self.tok.span;
            self.bump()?;
            // `**` is right-associative, and the one operator whose right
            // operand can nest without end; the rest loop here.
            let right = if op == Op::Binary(BinaryOp::Exp) {
                self.enter()?;
                let right = self.binary(prec - 1, no_in)?;
                self.leave();
                right
            } else {
                self.binary(prec, no_in)?
            };
            if self.is_arrow(right) {
                let span = self.span(right);
                return self.err(
                    "arrow-operand",
                    span,
                    "an arrow function needs parentheses here",
                );
            }
            left = match op {
                Op::Binary(BinaryOp::Exp) => {
                    if matches!(self.node(left), Node::Unary { .. } | Node::Await { .. }) {
                        return self.err(
                            "unary-before-exponent",
                            op_span,
                            "a unary operator before `**` needs parentheses, like `(-a) ** b`",
                        );
                    }
                    self.finish(
                        start,
                        Node::Binary {
                            operator: BinaryOp::Exp,
                            left,
                            right,
                        },
                    )
                }
                Op::Binary(operator) => self.finish(
                    start,
                    Node::Binary {
                        operator,
                        left,
                        right,
                    },
                ),
                Op::Logical(operator) => {
                    let mixes = |p: &Self, id: NodeId| match p.node(id) {
                        Node::Logical { operator: o, .. } => {
                            (o == LogicalOp::Coalesce) != (operator == LogicalOp::Coalesce)
                        }
                        _ => false,
                    };
                    if mixes(self, left) || mixes(self, right) {
                        return self.err(
                            "mixed-coalesce",
                            op_span,
                            "`??` can't be mixed with `||` or `&&` without parentheses",
                        );
                    }
                    self.finish(
                        start,
                        Node::Logical {
                            operator,
                            left,
                            right,
                        },
                    )
                }
            };
        }
        Ok(left)
    }

    /// The operand of a prefix operator, one level deeper.
    fn operand(&mut self) -> R<NodeId> {
        self.enter()?;
        let argument = self.unary()?;
        self.leave();
        Ok(argument)
    }

    fn unary(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let operator = match self.tok.kind {
            Kind::Punct(Punct::Minus) => Some(UnaryOp::Minus),
            Kind::Punct(Punct::Plus) => Some(UnaryOp::Plus),
            Kind::Punct(Punct::Bang) => Some(UnaryOp::Not),
            Kind::Punct(Punct::Tilde) => Some(UnaryOp::BitNot),
            Kind::Keyword(Keyword::Typeof) => Some(UnaryOp::Typeof),
            Kind::Keyword(Keyword::Void) => Some(UnaryOp::Void),
            Kind::Keyword(Keyword::Delete) => Some(UnaryOp::Delete),
            _ => None,
        };
        let node = if let Some(operator) = operator {
            self.bump()?;
            let argument = self.operand()?;
            if self.is_arrow(argument) {
                let span = self.span(argument);
                return self.err(
                    "arrow-operand",
                    span,
                    "an arrow function needs parentheses here",
                );
            }
            if operator == UnaryOp::Delete {
                self.check_delete(argument)?;
            }
            self.finish(start, Node::Unary { operator, argument })
        } else if self.at(Punct::PlusPlus) || self.at(Punct::MinusMinus) {
            let operator = if self.at(Punct::PlusPlus) {
                UpdateOp::Increment
            } else {
                UpdateOp::Decrement
            };
            self.bump()?;
            let argument = self.operand()?;
            self.simple_target(argument, true)?;
            self.finish(
                start,
                Node::Update {
                    operator,
                    prefix: true,
                    argument,
                },
            )
        } else if self.at_kw(Keyword::Await) && self.ctx.await_ {
            if self.ctx.static_block {
                let span = self.tok.span;
                return self.err(
                    "await-in-class-init",
                    span,
                    "`await` isn't allowed in a class field or static block",
                );
            }
            self.bump()?;
            self.last_await = Some(start);
            let argument = self.operand()?;
            self.finish(start, Node::Await { argument })
        } else {
            let e = self.lhs()?;
            if (self.at(Punct::PlusPlus) || self.at(Punct::MinusMinus))
                && !self.tok.newline_before()
            {
                if self.is_arrow(e) {
                    return self.unexpected();
                }
                let operator = if self.at(Punct::PlusPlus) {
                    UpdateOp::Increment
                } else {
                    UpdateOp::Decrement
                };
                self.simple_target(e, true)?;
                self.bump()?;
                self.finish(
                    start,
                    Node::Update {
                        operator,
                        prefix: false,
                        argument: e,
                    },
                )
            } else {
                e
            }
        };
        Ok(node)
    }

    /// `delete x` is an error in strict code, and `delete this.#x`
    /// anywhere.
    fn check_delete(&mut self, argument: NodeId) -> R<()> {
        let inner = self.ast.unparenthesized(argument);
        let target = match self.node(inner) {
            Node::Chain { expression } => expression,
            _ => inner,
        };
        match self.node(target) {
            Node::Identifier if self.ctx.strict => {
                let span = self.span(target);
                self.err(
                    "delete-identifier",
                    span,
                    "strict code can't delete a plain name",
                )
            }
            Node::Member { property, .. }
                if matches!(self.node(property), Node::PrivateIdentifier) =>
            {
                let span = self.span(property);
                self.err("delete-private", span, "a private field can't be deleted")
            }
            _ => Ok(()),
        }
    }

    /// LeftHandSideExpression: a primary expression and what follows it.
    pub(super) fn lhs(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let e = self.primary()?;
        if self.is_arrow(e) {
            return Ok(e);
        }
        self.subscripts(e, start, false)
    }

    /// Member accesses, calls, tagged templates and optional chains after
    /// `e`; with `no_call`, no calls, for `new`'s callee.
    fn subscripts(&mut self, mut e: NodeId, start: u32, no_call: bool) -> R<NodeId> {
        let mut chain = false;
        loop {
            match self.tok.kind {
                Kind::Punct(Punct::Dot) => {
                    self.bump()?;
                    let property = self.member_name(e)?;
                    e = self.finish(
                        start,
                        Node::Member {
                            object: e,
                            property,
                            computed: false,
                            optional: false,
                        },
                    );
                }
                Kind::Punct(Punct::QuestionDot) => {
                    if no_call {
                        return self.err(
                            "optional-chain-in-new",
                            self.tok.span,
                            "`new` can't take an optional chain; put it in parentheses",
                        );
                    }
                    self.bump()?;
                    chain = true;
                    if self.at(Punct::LParen) {
                        let (arguments, _) = self.arguments(false)?;
                        e = self.finish(
                            start,
                            Node::Call {
                                callee: e,
                                arguments,
                                optional: true,
                            },
                        );
                    } else if self.eat(Punct::LBracket)? {
                        let property = self.expression(false)?;
                        self.expect(Punct::RBracket)?;
                        e = self.finish(
                            start,
                            Node::Member {
                                object: e,
                                property,
                                computed: true,
                                optional: true,
                            },
                        );
                    } else if matches!(self.tok.kind, Kind::Template | Kind::TemplateHead) {
                        return self.tagged_in_chain();
                    } else {
                        let property = self.member_name(e)?;
                        e = self.finish(
                            start,
                            Node::Member {
                                object: e,
                                property,
                                computed: false,
                                optional: true,
                            },
                        );
                    }
                }
                Kind::Punct(Punct::LBracket) => {
                    self.bump()?;
                    let property = self.expression(false)?;
                    self.expect(Punct::RBracket)?;
                    e = self.finish(
                        start,
                        Node::Member {
                            object: e,
                            property,
                            computed: true,
                            optional: false,
                        },
                    );
                }
                Kind::Template | Kind::TemplateHead => {
                    if chain {
                        return self.tagged_in_chain();
                    }
                    let quasi = self.template(true)?;
                    e = self.finish(start, Node::TaggedTemplate { tag: e, quasi });
                }
                Kind::Punct(Punct::LParen) if !no_call => {
                    let (arguments, _) = self.arguments(false)?;
                    e = self.finish(
                        start,
                        Node::Call {
                            callee: e,
                            arguments,
                            optional: false,
                        },
                    );
                }
                _ => break,
            }
        }
        if chain {
            e = self.finish(start, Node::Chain { expression: e });
        }
        Ok(e)
    }

    fn tagged_in_chain<T>(&mut self) -> R<T> {
        let span = self.tok.span;
        self.err(
            "tagged-template-in-chain",
            span,
            "an optional chain can't be a template's tag",
        )
    }

    /// The name after `.` or `?.`: any IdentifierName, or a private name.
    fn member_name(&mut self, object: NodeId) -> R<NodeId> {
        if self.tok.kind == Kind::PrivateName {
            let t = self.bump()?;
            if matches!(self.node(object), Node::Super) {
                return self.err("super-private", t.span, "`super` has no private names");
            }
            self.use_private(t.span)?;
            return Ok(self.ast.push(t.span, Node::PrivateIdentifier));
        }
        self.identifier_name()
    }

    /// `( arguments )`, and where a comma follows a spread last, for an
    /// async arrow's parameters. With `cover`, they may turn into those.
    pub(super) fn arguments(&mut self, cover: bool) -> R<(List, Option<Span>)> {
        self.expect(Punct::LParen)?;
        let mark = self.scratch.len();
        let mut comma_after_spread = None;
        while !self.at(Punct::RParen) {
            let start = self.tok.span.start;
            let spread = self.eat(Punct::Ellipsis)?;
            let mut e = if cover {
                self.assign_cover(false)?
            } else {
                self.assign(false)?
            };
            if spread {
                e = self.finish(start, Node::Spread { argument: e });
            }
            self.scratch.push(e);
            if !self.at(Punct::RParen) {
                let comma = self.expect(Punct::Comma)?;
                comma_after_spread = spread.then_some(comma.span);
            } else {
                comma_after_spread = None;
            }
        }
        self.expect(Punct::RParen)?;
        Ok((self.take_list(mark), comma_after_spread))
    }

    fn literal(&mut self, kind: LiteralKind) -> R<NodeId> {
        let t = self.bump()?;
        Ok(self.ast.push(t.span, Node::Literal(kind)))
    }

    fn primary(&mut self) -> R<NodeId> {
        let t = self.tok;
        let start = t.span.start;
        match t.kind {
            Kind::Keyword(Keyword::This) => {
                self.bump()?;
                Ok(self.ast.push(t.span, Node::This))
            }
            Kind::Keyword(Keyword::Null) => self.literal(LiteralKind::Null),
            Kind::Keyword(Keyword::True | Keyword::False) => self.literal(LiteralKind::Boolean),
            Kind::Keyword(Keyword::Function) => {
                self.function(start, false, FunctionKind::Expression, true)
            }
            Kind::Keyword(Keyword::Class) => self.class(start, false, true),
            Kind::Keyword(Keyword::Async) => self.async_primary(),
            Kind::Keyword(Keyword::New) => self.new_expression(),
            Kind::Keyword(Keyword::Super) => self.super_expression(),
            Kind::Keyword(Keyword::Import) => self.import_expression(),
            Kind::Ident | Kind::Keyword(_) => {
                let id = self.identifier(false)?;
                if !(self.at(Punct::Colon) && self.label_candidate == Some(start)) {
                    self.check_arguments(t.span)?;
                }
                if self.at(Punct::Arrow) && !self.tok.newline_before() {
                    self.reinterpret(id, true)?;
                    let params = self.ast.list([id]);
                    return self.arrow(start, params, true, false, false);
                }
                Ok(id)
            }
            Kind::Number => self.literal(LiteralKind::Number),
            Kind::BigInt => self.literal(LiteralKind::BigInt),
            Kind::String => self.literal(LiteralKind::String),
            Kind::Template | Kind::TemplateHead => self.template(false),
            Kind::Punct(Punct::Slash | Punct::SlashEq) => {
                self.rescan(Goal::RegExp)?;
                self.literal(LiteralKind::RegExp)
            }
            Kind::Punct(Punct::LParen) => self.parenthesized(),
            Kind::Punct(Punct::LBracket) => self.array(),
            Kind::Punct(Punct::LBrace) => self.object(),
            _ => self.unexpected(),
        }
    }

    /// `async` as an async function, an async arrow, a call to something
    /// named `async`, or just that name.
    fn async_primary(&mut self) -> R<NodeId> {
        let t = self.tok;
        let start = t.span.start;
        let next = self.peek();
        if next.kind == Kind::Keyword(Keyword::Function) && !next.newline_before() {
            self.bump()?;
            return self.function(start, true, FunctionKind::Expression, true);
        }
        let id = self.identifier(false)?;
        if self.tok.newline_before() {
            return Ok(id);
        }
        if self.at_identifier() && self.peek().kind == Kind::Punct(Punct::Arrow) {
            // `async x => ...`
            if self.at_kw(Keyword::Await) {
                let span = self.tok.span;
                return self.err(
                    "await-in-params",
                    span,
                    "an async arrow's parameter can't be `await`",
                );
            }
            let param = self.identifier(true)?;
            if !self.at(Punct::Arrow) || self.tok.newline_before() {
                return self.unexpected();
            }
            let params = self.ast.list([param]);
            return self.arrow(start, params, true, true, false);
        }
        if self.at(Punct::LParen) {
            let args_start = self.tok.span.start;
            let (arguments, comma_after_spread) = self.arguments(true)?;
            if self.at(Punct::Arrow) && !self.tok.newline_before() {
                if let Some(span) = comma_after_spread {
                    return self.err(
                        "comma-after-rest",
                        span,
                        "a rest parameter has to be last, with no comma after it",
                    );
                }
                let simple = self.params_from_cover(arguments)?;
                let awaits = [self.last_await, self.last_await_ident];
                if let Some(at) = awaits
                    .into_iter()
                    .flatten()
                    .filter(|&at| at >= args_start)
                    .max()
                {
                    return self.err(
                        "await-in-params",
                        Span::at(at),
                        "an async arrow's parameters can't use `await`",
                    );
                }
                if self.ctx.yield_ && self.last_yield.is_some_and(|at| at >= args_start) {
                    let span = Span::at(args_start);
                    return self.err(
                        "yield-in-params",
                        span,
                        "an arrow's parameters can't use `yield` here",
                    );
                }
                return self.arrow(start, arguments, simple, true, false);
            }
            return Ok(self.finish(
                start,
                Node::Call {
                    callee: id,
                    arguments,
                    optional: false,
                },
            ));
        }
        if self.at(Punct::Arrow) {
            self.reinterpret(id, true)?;
            let params = self.ast.list([id]);
            return self.arrow(start, params, true, false, false);
        }
        Ok(id)
    }

    /// `(`: a parenthesized expression, or an arrow's parameters.
    fn parenthesized(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mark = self.scratch.len();
        let mut only_arrow: Option<Span> = None;
        if self.at(Punct::RParen) {
            only_arrow = Some(self.tok.span);
        }
        while !self.at(Punct::RParen) {
            if self.at(Punct::Ellipsis) {
                let rstart = self.tok.span.start;
                only_arrow = Some(self.tok.span);
                self.bump()?;
                let argument = self.binding_target()?;
                let rest = self.finish(rstart, Node::Rest { argument });
                self.scratch.push(rest);
                break;
            }
            let e = self.assign_cover(false)?;
            self.scratch.push(e);
            if !self.at(Punct::RParen) {
                self.expect(Punct::Comma)?;
                if self.at(Punct::RParen) {
                    only_arrow = Some(Span::at(self.prev_end - 1));
                }
            }
        }
        self.expect(Punct::RParen)?;
        if self.at(Punct::Arrow) && !self.tok.newline_before() {
            let params = self.take_list(mark);
            let simple = self.params_from_cover(params)?;
            if self.ctx.yield_ && self.last_yield.is_some_and(|at| at >= start) {
                return self.err(
                    "yield-in-params",
                    Span::at(start),
                    "an arrow's parameters can't use `yield` here",
                );
            }
            if self.ctx.await_ && self.last_await.is_some_and(|at| at >= start) {
                return self.err(
                    "await-in-params",
                    Span::at(start),
                    "an arrow's parameters can't use `await` here",
                );
            }
            return self.arrow(start, params, simple, false, false);
        }
        if let Some(span) = only_arrow {
            return self.err(
                "arrow-params-only",
                span,
                "this is only allowed in an arrow function's parameters",
            );
        }
        let n = self.scratch.len() - mark;
        let expression = if n == 1 {
            self.scratch.pop().expect("one expression")
        } else {
            let first = self.span(self.scratch[mark]).start;
            let last = self.span(self.scratch[self.scratch.len() - 1]).end;
            let expressions = self.take_list(mark);
            self.ast
                .push(Span::new(first, last), Node::Sequence { expressions })
        };
        Ok(self.finish(start, Node::Paren { expression }))
    }

    /// Turns each of `params`, parsed as expressions, into a parameter.
    /// Whether they're all plain identifiers.
    fn params_from_cover(&mut self, params: List) -> R<bool> {
        let mut simple = true;
        for i in 0..params.len() {
            let p = self.ast.get(params)[i];
            match self.node(p) {
                Node::Spread { argument } => {
                    if i + 1 != params.len() {
                        let span = self.span(p);
                        return self.err("rest-not-last", span, "a rest parameter has to be last");
                    }
                    self.reinterpret(argument, true)?;
                    if matches!(
                        self.node(argument),
                        Node::Assign { .. } | Node::AssignmentPattern { .. }
                    ) {
                        let span = self.span(argument);
                        return self.err(
                            "rest-default",
                            span,
                            "a rest parameter can't have a default",
                        );
                    }
                    *self.ast.node_mut(p) = Node::Rest { argument };
                    simple = false;
                }
                Node::Rest { .. } => simple = false,
                _ => {
                    self.reinterpret(p, true)?;
                    simple &= matches!(self.node(p), Node::Identifier);
                }
            }
        }
        Ok(simple)
    }

    fn array(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mark = self.scratch.len();
        let mut comma_after_spread = false;
        while !self.at(Punct::RBracket) {
            if self.at(Punct::Comma) {
                let t = self.bump()?;
                let hole = self.ast.push(Span::at(t.span.start), Node::Elision);
                self.scratch.push(hole);
                continue;
            }
            let estart = self.tok.span.start;
            let spread = self.eat(Punct::Ellipsis)?;
            let mut e = self.assign_cover(false)?;
            if spread {
                e = self.finish(estart, Node::Spread { argument: e });
            }
            self.scratch.push(e);
            if !self.at(Punct::RBracket) {
                self.expect(Punct::Comma)?;
                comma_after_spread |= spread;
            }
        }
        self.bump()?;
        let elements = self.take_list(mark);
        Ok(self.finish(
            start,
            Node::Array {
                elements,
                comma_after_spread,
            },
        ))
    }

    fn object(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mark = self.scratch.len();
        let mut proto = false;
        let mut pending = Vec::new();
        let mut comma_after_spread = false;
        while !self.at(Punct::RBrace) {
            let p = self.property(&mut proto, &mut pending)?;
            self.scratch.push(p);
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
                comma_after_spread |= matches!(self.node(p), Node::Spread { .. });
            }
        }
        self.bump()?;
        let properties = self.take_list(mark);
        let id = self.finish(
            start,
            Node::Object {
                properties,
                comma_after_spread,
            },
        );
        for (span, code) in pending {
            self.pending.push((id, span, code));
        }
        Ok(id)
    }

    fn property(&mut self, proto: &mut bool, pending: &mut Vec<(Span, &'static str)>) -> R<NodeId> {
        let start = self.tok.span.start;
        if self.eat(Punct::Ellipsis)? {
            let argument = self.assign_cover(false)?;
            return Ok(self.finish(start, Node::Spread { argument }));
        }
        let mut is_async = false;
        let mut generator = false;
        let mut kind = PropertyKind::Init;
        if self.at_kw(Keyword::Async) && !self.modifier_is_name_in_object(true) {
            self.bump()?;
            is_async = true;
        }
        if self.eat(Punct::Star)? {
            generator = true;
        }
        if !is_async
            && !generator
            && (self.at_kw(Keyword::Get) || self.at_kw(Keyword::Set))
            && !self.modifier_is_name_in_object(false)
        {
            kind = if self.at_kw(Keyword::Get) {
                PropertyKind::Get
            } else {
                PropertyKind::Set
            };
            self.bump()?;
        }
        let key_tok = self.tok;
        let (key, computed) = self.property_name()?;
        if is_async || generator || kind != PropertyKind::Init || self.at(Punct::LParen) {
            let mt = match kind {
                PropertyKind::Get => MethodType::Get,
                PropertyKind::Set => MethodType::Set,
                PropertyKind::Init => MethodType::Method,
            };
            let value = self.method(mt, is_async, generator, false)?;
            let method = kind == PropertyKind::Init;
            return Ok(self.finish(
                start,
                Node::Property {
                    key,
                    value,
                    kind,
                    method,
                    shorthand: false,
                    computed,
                },
            ));
        }
        if self.eat(Punct::Colon)? {
            if !computed && self.key_is(key, "__proto__") {
                if *proto {
                    pending.push((self.span(key), "duplicate-proto"));
                }
                *proto = true;
            }
            let value = self.assign_cover(false)?;
            return Ok(self.finish(
                start,
                Node::Property {
                    key,
                    value,
                    kind,
                    method: false,
                    shorthand: false,
                    computed,
                },
            ));
        }
        if computed || !matches!(key_tok.kind, Kind::Ident | Kind::Keyword(_)) {
            return self.unexpected();
        }
        self.check_identifier(key_tok, false)?;
        self.check_arguments(key_tok.span)?;
        let value = if self.at(Punct::Eq) {
            let eq = self.bump()?;
            let right = self.assign(false)?;
            pending.push((eq.span, "shorthand-init"));
            self.finish(
                start,
                Node::Assign {
                    operator: AssignOp::Assign,
                    left: key,
                    right,
                },
            )
        } else {
            key
        };
        Ok(self.finish(
            start,
            Node::Property {
                key,
                value,
                kind,
                method: false,
                shorthand: true,
                computed,
            },
        ))
    }

    fn modifier_is_name_in_object(&self, async_like: bool) -> bool {
        let next = self.peek();
        matches!(
            next.kind,
            Kind::Punct(Punct::LParen | Punct::Eq | Punct::RBrace | Punct::Colon | Punct::Comma)
                | Kind::Eof
        ) || async_like && next.newline_before()
    }

    /// A template, from its first part. Untagged, an escape that isn't
    /// one is an error; tagged, its cooked value is undefined.
    pub(super) fn template(&mut self, tagged: bool) -> R<NodeId> {
        let start = self.tok.span.start;
        let mark = self.scratch.len();
        let mut quasis = Vec::new();
        loop {
            let t = self.tok;
            let tail = matches!(t.kind, Kind::Template | Kind::TemplateTail);
            let invalid = t.flags.contains(Flags::INVALID_ESCAPE);
            if invalid && !tagged {
                return self.err(
                    "invalid-template-escape",
                    t.span,
                    "this escape isn't one; only a tagged template allows that",
                );
            }
            let end = t.span.end - if tail { 1 } else { 2 };
            let span = Span::new(t.span.start + 1, end.max(t.span.start + 1));
            quasis.push(self.ast.push(
                span,
                Node::TemplateElement {
                    tail,
                    cooked: !invalid,
                },
            ));
            self.bump()?;
            if tail {
                break;
            }
            let e = self.expression(false)?;
            self.scratch.push(e);
            if !self.at(Punct::RBrace) {
                return self.unexpected();
            }
            self.rescan(Goal::TemplateTail)?;
        }
        let expressions = self.take_list(mark);
        let quasis = self.ast.list(quasis);
        Ok(self.finish(
            start,
            Node::Template {
                quasis,
                expressions,
            },
        ))
    }

    fn new_expression(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let new = self.bump()?;
        if self.at(Punct::Dot) {
            self.bump()?;
            if !self.at_kw(Keyword::Target) {
                return self.unexpected();
            }
            let t = self.bump()?;
            if !self.ctx.new_target {
                return self.err(
                    "new-target-outside-function",
                    Span::new(start, t.span.end),
                    "`new.target` is only allowed in a function",
                );
            }
            let meta = self.ast.push(new.span, Node::Identifier);
            let property = self.ast.push(t.span, Node::Identifier);
            return Ok(self.finish(start, Node::MetaProperty { meta, property }));
        }
        self.enter()?;
        let cstart = self.tok.span.start;
        if self.at_kw(Keyword::Import) && self.peek().kind != Kind::Punct(Punct::Dot) {
            let span = self.tok.span;
            return self.err("new-import", span, "`import()` can't be called with `new`");
        }
        let callee = self.primary()?;
        if self.is_arrow(callee) {
            let span = self.span(callee);
            return self.err(
                "arrow-operand",
                span,
                "an arrow function needs parentheses here",
            );
        }
        let callee = self.subscripts(callee, cstart, true)?;
        let arguments = if self.at(Punct::LParen) {
            self.arguments(false)?.0
        } else {
            List::default()
        };
        self.leave();
        Ok(self.finish(start, Node::New { callee, arguments }))
    }

    fn super_expression(&mut self) -> R<NodeId> {
        let t = self.bump()?;
        let ok = match self.tok.kind {
            Kind::Punct(Punct::LParen) => self.ctx.super_call,
            Kind::Punct(Punct::Dot | Punct::LBracket) => self.ctx.super_prop,
            _ => return self.unexpected(),
        };
        if !ok {
            return self.err("super-outside-method", t.span, "`super` isn't allowed here");
        }
        Ok(self.ast.push(t.span, Node::Super))
    }

    /// `import(...)` or `import.meta`.
    fn import_expression(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let import = self.bump()?;
        if self.at(Punct::Dot) {
            self.bump()?;
            if !self.at_kw(Keyword::Meta) {
                return self.unexpected();
            }
            let t = self.bump()?;
            if !self.module {
                return self.err(
                    "import-meta-outside-module",
                    Span::new(start, t.span.end),
                    "`import.meta` is only allowed in a module",
                );
            }
            let meta = self.ast.push(import.span, Node::Identifier);
            let property = self.ast.push(t.span, Node::Identifier);
            return Ok(self.finish(start, Node::MetaProperty { meta, property }));
        }
        self.expect(Punct::LParen)?;
        let source = self.assign(false)?;
        let mut options = None;
        if self.eat(Punct::Comma)? && !self.at(Punct::RParen) {
            options = Some(self.assign(false)?);
            self.eat(Punct::Comma)?;
        }
        self.expect(Punct::RParen)?;
        Ok(self.finish(start, Node::Import { source, options }))
    }
}

fn assign_op(kind: Kind) -> Option<AssignOp> {
    use AssignOp as A;
    let Kind::Punct(p) = kind else { return None };
    Some(match p {
        Punct::Eq => A::Assign,
        Punct::PlusEq => A::Add,
        Punct::MinusEq => A::Sub,
        Punct::StarEq => A::Mul,
        Punct::SlashEq => A::Div,
        Punct::PercentEq => A::Rem,
        Punct::StarStarEq => A::Exp,
        Punct::ShlEq => A::Shl,
        Punct::ShrEq => A::Shr,
        Punct::UShrEq => A::UShr,
        Punct::PipeEq => A::BitOr,
        Punct::CaretEq => A::BitXor,
        Punct::AmpEq => A::BitAnd,
        Punct::PipePipeEq => A::Or,
        Punct::AmpAmpEq => A::And,
        Punct::QuestionQuestionEq => A::Coalesce,
        _ => return None,
    })
}
