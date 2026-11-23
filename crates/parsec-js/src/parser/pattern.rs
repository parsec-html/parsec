// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Patterns: binding patterns parsed as themselves, and expressions
//! turned into assignment patterns or arrow parameters in place.

use parsec_base::Span;

use super::{Parser, R};
use crate::ast::{AssignOp, Node, NodeId, PropertyKind};
use crate::token::{Kind, Punct};

impl Parser<'_> {
    /// Turns `id`, parsed as an expression, into the pattern it covers:
    /// an assignment target, or with `binding`, a binding pattern.
    pub(super) fn reinterpret(&mut self, id: NodeId, binding: bool) -> R<()> {
        self.enter()?;
        match self.node(id) {
            Node::Identifier => self.check_target_name(id)?,
            Node::Member { .. } if !binding => {}
            Node::Paren { .. } if !binding => {
                let inner = self.ast.unparenthesized(id);
                match self.node(inner) {
                    Node::Identifier => self.check_target_name(inner)?,
                    Node::Member { .. } => {}
                    _ => return self.invalid_target(id),
                }
            }
            Node::Object { properties, .. } | Node::ObjectPattern { properties } => {
                let comma_after_spread = matches!(
                    self.node(id),
                    Node::Object {
                        comma_after_spread: true,
                        ..
                    }
                );
                for i in 0..properties.len() {
                    let p = self.ast.get(properties)[i];
                    match self.node(p) {
                        Node::Property {
                            value,
                            kind,
                            method,
                            ..
                        } => {
                            if method || kind != PropertyKind::Init {
                                return self.invalid_target(p);
                            }
                            self.reinterpret(value, binding)?;
                        }
                        Node::Spread { argument } | Node::Rest { argument } => {
                            if i + 1 != properties.len() || comma_after_spread {
                                return self.err(
                                    "rest-not-last",
                                    self.span(p),
                                    "a rest element has to be last",
                                );
                            }
                            // An object's rest is a name, or for an
                            // assignment, a member too: never a pattern.
                            let inner = self.ast.unparenthesized(argument);
                            let ok = match self.node(inner) {
                                Node::Identifier => true,
                                Node::Member { .. } => !binding,
                                _ => false,
                            };
                            if !ok || binding && inner != argument {
                                return self.invalid_target(argument);
                            }
                            self.reinterpret(argument, binding)?;
                            *self.ast.node_mut(p) = Node::Rest { argument };
                        }
                        _ => return self.invalid_target(p),
                    }
                }
                *self.ast.node_mut(id) = Node::ObjectPattern { properties };
            }
            Node::Array {
                elements,
                comma_after_spread,
            } => {
                for i in 0..elements.len() {
                    let e = self.ast.get(elements)[i];
                    match self.node(e) {
                        Node::Elision => {}
                        Node::Spread { argument } => {
                            if i + 1 != elements.len() || comma_after_spread {
                                return self.err(
                                    "rest-not-last",
                                    self.span(e),
                                    "a rest element has to be last",
                                );
                            }
                            self.rest_argument(argument, binding)?;
                            *self.ast.node_mut(e) = Node::Rest { argument };
                        }
                        _ => self.reinterpret(e, binding)?,
                    }
                }
                *self.ast.node_mut(id) = Node::ArrayPattern { elements };
            }
            Node::ArrayPattern { elements } => {
                for i in 0..elements.len() {
                    let e = self.ast.get(elements)[i];
                    match self.node(e) {
                        Node::Elision => {}
                        Node::Rest { argument } => self.rest_argument(argument, binding)?,
                        _ => self.reinterpret(e, binding)?,
                    }
                }
            }
            Node::Assign {
                operator: AssignOp::Assign,
                left,
                right,
            } => {
                self.reinterpret(left, binding)?;
                *self.ast.node_mut(id) = Node::AssignmentPattern { left, right };
            }
            Node::AssignmentPattern { left, .. } => self.reinterpret(left, binding)?,
            _ => return self.invalid_target(id),
        }
        self.leave();
        Ok(())
    }

    /// An array's rest element: any target, but with no default.
    fn rest_argument(&mut self, argument: NodeId, binding: bool) -> R<()> {
        if matches!(
            self.node(argument),
            Node::Assign { .. } | Node::AssignmentPattern { .. }
        ) {
            return self.err(
                "rest-default",
                self.span(argument),
                "a rest element can't have a default",
            );
        }
        self.reinterpret(argument, binding)
    }

    fn invalid_target<T>(&mut self, id: NodeId) -> R<T> {
        let span = self.span(id);
        self.err("invalid-target", span, "this can't be assigned to or bound")
    }

    /// Strict code can't assign or bind `eval` or `arguments`.
    fn check_target_name(&mut self, id: NodeId) -> R<()> {
        if !self.ctx.strict {
            return Ok(());
        }
        let span = self.span(id);
        if self.name_is(span, "eval") || self.name_is(span, "arguments") {
            let name = self.name(span).into_owned();
            return self.err(
                "strict-binding",
                span,
                format!("strict code can't bind or assign `{name}`"),
            );
        }
        Ok(())
    }

    /// The target of `++`, `--` or a compound assignment: a name or a
    /// member, maybe in parentheses.
    ///
    /// With `call`, a call too, in sloppy code: B.3.6 keeps `f() = 1` a
    /// ReferenceError when it runs, not a SyntaxError, as browsers have it.
    pub(super) fn simple_target(&mut self, id: NodeId, call: bool) -> R<()> {
        let inner = self.ast.unparenthesized(id);
        match self.node(inner) {
            Node::Identifier => self.check_target_name(inner),
            Node::Member { .. } => Ok(()),
            Node::Call { .. } if call && !self.ctx.strict => Ok(()),
            _ => self.invalid_target(id),
        }
    }

    /// The names a pattern binds, in order.
    pub(super) fn bound_names(&self, id: NodeId, out: &mut Vec<Span>) {
        match self.node(id) {
            Node::Identifier => out.push(self.span(id)),
            Node::ObjectPattern { properties } => {
                for &p in self.ast.get(properties) {
                    match self.node(p) {
                        Node::Property { value, .. } => self.bound_names(value, out),
                        Node::Rest { argument } => self.bound_names(argument, out),
                        _ => {}
                    }
                }
            }
            Node::ArrayPattern { elements } => {
                for &e in self.ast.get(elements) {
                    self.bound_names(e, out);
                }
            }
            Node::Rest { argument } => self.bound_names(argument, out),
            Node::AssignmentPattern { left, .. } => self.bound_names(left, out),
            _ => {}
        }
    }

    /// BindingIdentifier or BindingPattern.
    pub(super) fn binding_target(&mut self) -> R<NodeId> {
        self.enter()?;
        let start = self.tok.span.start;
        let node = match self.tok.kind {
            Kind::Punct(Punct::LBracket) => {
                self.bump()?;
                let mark = self.scratch.len();
                while !self.at(Punct::RBracket) {
                    if self.at(Punct::Comma) {
                        let t = self.bump()?;
                        let hole = self.ast.push(Span::at(t.span.start), Node::Elision);
                        self.scratch.push(hole);
                        continue;
                    }
                    if self.at(Punct::Ellipsis) {
                        let rstart = self.tok.span.start;
                        self.bump()?;
                        let argument = self.binding_target()?;
                        let rest = self.finish(rstart, Node::Rest { argument });
                        self.scratch.push(rest);
                        break;
                    }
                    let e = self.binding_element()?;
                    self.scratch.push(e);
                    if !self.at(Punct::RBracket) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.expect(Punct::RBracket)?;
                let elements = self.take_list(mark);
                self.finish(start, Node::ArrayPattern { elements })
            }
            Kind::Punct(Punct::LBrace) => {
                self.bump()?;
                let mark = self.scratch.len();
                while !self.at(Punct::RBrace) {
                    let pstart = self.tok.span.start;
                    if self.eat(Punct::Ellipsis)? {
                        let argument = self.identifier(true)?;
                        let rest = self.finish(pstart, Node::Rest { argument });
                        self.scratch.push(rest);
                        break;
                    }
                    let key_tok = self.tok;
                    let (key, computed) = self.property_name()?;
                    let p = if self.eat(Punct::Colon)? {
                        let value = self.binding_element()?;
                        Node::Property {
                            key,
                            value,
                            kind: PropertyKind::Init,
                            method: false,
                            shorthand: false,
                            computed,
                        }
                    } else {
                        if computed || !matches!(key_tok.kind, Kind::Ident | Kind::Keyword(_)) {
                            return self.unexpected();
                        }
                        self.check_identifier(key_tok, true)?;
                        let value = if self.eat(Punct::Eq)? {
                            let right = self.assign(false)?;
                            self.finish(pstart, Node::AssignmentPattern { left: key, right })
                        } else {
                            key
                        };
                        Node::Property {
                            key,
                            value,
                            kind: PropertyKind::Init,
                            method: false,
                            shorthand: true,
                            computed: false,
                        }
                    };
                    let p = self.finish(pstart, p);
                    self.scratch.push(p);
                    if !self.at(Punct::RBrace) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.expect(Punct::RBrace)?;
                let properties = self.take_list(mark);
                self.finish(start, Node::ObjectPattern { properties })
            }
            _ => self.identifier(true)?,
        };
        self.leave();
        Ok(node)
    }

    /// A binding target with an optional `= default`.
    pub(super) fn binding_element(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let target = self.binding_target()?;
        if !self.eat(Punct::Eq)? {
            return Ok(target);
        }
        let right = self.assign(false)?;
        Ok(self.finish(
            start,
            Node::AssignmentPattern {
                left: target,
                right,
            },
        ))
    }
}
