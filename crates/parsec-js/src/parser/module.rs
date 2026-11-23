// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Modules' imports and exports (ECMA-262 clause 16.2), with import
//! attributes.

use parsec_base::Span;

use super::{Parser, R};
use crate::ast::{FunctionKind, List, LiteralKind, Node, NodeId, VarKind};
use crate::text;
use crate::token::{Keyword, Kind, Punct, Token};

impl Parser<'_> {
    /// Whether an `import` being looked at starts a declaration rather
    /// than `import(...)` or `import.meta`.
    pub(super) fn at_import_declaration(&self) -> bool {
        self.at_kw(Keyword::Import)
            && !matches!(self.peek().kind, Kind::Punct(Punct::LParen | Punct::Dot))
    }

    /// An `import` or `export` declaration, which only a module's top
    /// level may have.
    pub(super) fn module_item(&mut self) -> R<NodeId> {
        if !self.module || self.depth > 0 {
            let span = self.tok.span;
            return self.err(
                "module-item-not-allowed",
                span,
                "`import` and `export` declarations are only allowed at the top level of a module",
            );
        }
        if self.at_kw(Keyword::Import) {
            self.import_declaration()
        } else {
            self.export_declaration()
        }
    }

    /// The token as a string literal node.
    fn string_literal(&mut self) -> R<NodeId> {
        if self.tok.kind != Kind::String {
            return self.unexpected();
        }
        let t = self.bump()?;
        Ok(self.ast.push(t.span, Node::Literal(LiteralKind::String)))
    }

    /// A ModuleExportName: an IdentifierName, or a string with no lone
    /// surrogate in it.
    fn module_export_name(&mut self) -> R<(NodeId, Token)> {
        let t = self.tok;
        if t.kind == Kind::String {
            let body = Span::new(t.span.start + 1, t.span.end - 1).text(self.src);
            if char::decode_utf16(text::string_value(body)).any(|c| c.is_err()) {
                return self.err(
                    "lone-surrogate-name",
                    t.span,
                    "an import or export name can't have a lone surrogate",
                );
            }
            return Ok((self.string_literal()?, t));
        }
        Ok((self.identifier_name()?, t))
    }

    /// The name a ModuleExportName says.
    fn export_name(&self, id: NodeId) -> String {
        let span = self.span(id);
        match self.node(id) {
            Node::Literal(_) => {
                let body = Span::new(span.start + 1, span.end - 1).text(self.src);
                String::from_utf16_lossy(&text::string_value(body))
            }
            _ => self.name(span).into_owned(),
        }
    }

    /// Records an exported name, which a module may export only once.
    fn export(&mut self, name: String, span: Span) -> R<()> {
        if !self.exports.insert(name.clone()) {
            return self.err(
                "duplicate-export",
                span,
                format!("`{name}` is exported twice"),
            );
        }
        Ok(())
    }

    /// `from "source"`, and its `with` clause.
    fn source_clause(&mut self) -> R<(NodeId, List)> {
        self.expect_kw(Keyword::From)?;
        let source = self.string_literal()?;
        let attributes = self.with_clause()?;
        Ok((source, attributes))
    }

    /// `with { key: "value", ... }`, or nothing.
    fn with_clause(&mut self) -> R<List> {
        let mark = self.scratch.len();
        if !self.at_kw(Keyword::With) {
            return Ok(List::default());
        }
        self.bump()?;
        self.expect(Punct::LBrace)?;
        // A set, not a list: a crafted module can give thousands of keys.
        let mut keys = std::collections::HashSet::new();
        while !self.at(Punct::RBrace) {
            let start = self.tok.span.start;
            let key = match self.tok.kind {
                Kind::String => self.module_export_name()?.0,
                _ => self.identifier_name()?,
            };
            let name = self.export_name(key);
            if !keys.insert(name.clone()) {
                let span = self.span(key);
                return self.err(
                    "duplicate-attribute",
                    span,
                    format!("the attribute `{name}` is given twice"),
                );
            }
            self.expect(Punct::Colon)?;
            let value = self.string_literal()?;
            let a = self.finish(start, Node::ImportAttribute { key, value });
            self.scratch.push(a);
            if !self.at(Punct::RBrace) {
                self.expect(Punct::Comma)?;
            }
        }
        self.bump()?;
        Ok(self.take_list(mark))
    }

    fn import_declaration(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mark = self.scratch.len();
        if self.tok.kind == Kind::String {
            let source = self.string_literal()?;
            let attributes = self.with_clause()?;
            self.semicolon()?;
            let specifiers = self.take_list(mark);
            return Ok(self.finish(
                start,
                Node::ImportDeclaration {
                    specifiers,
                    source,
                    attributes,
                },
            ));
        }
        let mut more = true;
        if self.at_identifier() {
            let local = self.identifier(true)?;
            let span = self.span(local);
            let d = self.ast.push(span, Node::ImportDefaultSpecifier { local });
            self.scratch.push(d);
            more = self.eat(Punct::Comma)?;
        }
        if more {
            if self.at(Punct::Star) {
                let sstart = self.tok.span.start;
                self.bump()?;
                self.expect_kw(Keyword::As)?;
                let local = self.identifier(true)?;
                let n = self.finish(sstart, Node::ImportNamespaceSpecifier { local });
                self.scratch.push(n);
            } else if self.eat(Punct::LBrace)? {
                while !self.at(Punct::RBrace) {
                    let sstart = self.tok.span.start;
                    let (imported, t) = self.module_export_name()?;
                    let local = if self.at_kw(Keyword::As) {
                        self.bump()?;
                        self.identifier(true)?
                    } else {
                        // Without `as`, the name is bound as it is.
                        if t.kind == Kind::String
                            || matches!(t.kind, Kind::Keyword(k) if k.is_reserved() && !matches!(k, Keyword::Yield | Keyword::Await))
                        {
                            return self.err(
                                "import-needs-as",
                                t.span,
                                "this name can be imported only `as` another",
                            );
                        }
                        self.check_identifier(t, true)?;
                        imported
                    };
                    let s = self.finish(sstart, Node::ImportSpecifier { imported, local });
                    self.scratch.push(s);
                    if !self.at(Punct::RBrace) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.bump()?;
            } else {
                return self.unexpected();
            }
        }
        let (source, attributes) = self.source_clause()?;
        self.semicolon()?;
        let specifiers = self.take_list(mark);
        Ok(self.finish(
            start,
            Node::ImportDeclaration {
                specifiers,
                source,
                attributes,
            },
        ))
    }

    fn export_declaration(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let dstart = self.tok.span.start;
        match self.tok.kind {
            Kind::Punct(Punct::Star) => {
                self.bump()?;
                let exported = if self.at_kw(Keyword::As) {
                    self.bump()?;
                    let (name, _) = self.module_export_name()?;
                    let n = self.export_name(name);
                    self.export(n, self.span(name))?;
                    Some(name)
                } else {
                    None
                };
                let (source, attributes) = self.source_clause()?;
                self.semicolon()?;
                Ok(self.finish(
                    start,
                    Node::ExportAll {
                        exported,
                        source,
                        attributes,
                    },
                ))
            }
            Kind::Punct(Punct::LBrace) => {
                self.bump()?;
                let mark = self.scratch.len();
                let mut locals = Vec::new();
                while !self.at(Punct::RBrace) {
                    let sstart = self.tok.span.start;
                    let (local, t) = self.module_export_name()?;
                    let exported = if self.at_kw(Keyword::As) {
                        self.bump()?;
                        self.module_export_name()?.0
                    } else {
                        local
                    };
                    let n = self.export_name(exported);
                    self.export(n, self.span(exported))?;
                    locals.push(t);
                    let s = self.finish(sstart, Node::ExportSpecifier { local, exported });
                    self.scratch.push(s);
                    if !self.at(Punct::RBrace) {
                        self.expect(Punct::Comma)?;
                    }
                }
                self.bump()?;
                let (source, attributes) = if self.at_kw(Keyword::From) {
                    let (s, a) = self.source_clause()?;
                    (Some(s), a)
                } else {
                    // Exporting what's here, each name has to be one a
                    // reference could be.
                    for t in locals {
                        if t.kind == Kind::String {
                            return self.err(
                                "export-string-local",
                                t.span,
                                "only a re-export can name a string here",
                            );
                        }
                        if matches!(t.kind, Kind::Keyword(k) if k.is_reserved() && !matches!(k, Keyword::Yield | Keyword::Await))
                        {
                            return self.unexpected_at(t);
                        }
                        self.check_identifier(t, false)?;
                    }
                    (None, List::default())
                };
                self.semicolon()?;
                let specifiers = self.take_list(mark);
                Ok(self.finish(
                    start,
                    Node::ExportNamed {
                        declaration: None,
                        specifiers,
                        source,
                        attributes,
                    },
                ))
            }
            Kind::Keyword(Keyword::Default) => {
                let t = self.bump()?;
                self.export("default".into(), t.span)?;
                let dstart = self.tok.span.start;
                let declaration = if self.at_kw(Keyword::Function) {
                    self.function(dstart, false, FunctionKind::Declaration, true)?
                } else if self.at_kw(Keyword::Async) && {
                    let next = self.peek();
                    next.kind == Kind::Keyword(Keyword::Function) && !next.newline_before()
                } {
                    self.bump()?;
                    self.function(dstart, true, FunctionKind::Declaration, true)?
                } else if self.at_kw(Keyword::Class) {
                    self.class(dstart, true, true)?
                } else {
                    let e = self.assign(false)?;
                    self.semicolon()?;
                    e
                };
                Ok(self.finish(start, Node::ExportDefault { declaration }))
            }
            _ => {
                let declaration = match self.tok.kind {
                    Kind::Keyword(Keyword::Var) => self.declaration_statement(VarKind::Var)?,
                    Kind::Keyword(Keyword::Let) => self.declaration_statement(VarKind::Let)?,
                    Kind::Keyword(Keyword::Const) => self.declaration_statement(VarKind::Const)?,
                    Kind::Keyword(Keyword::Function) => {
                        self.function(dstart, false, FunctionKind::Declaration, false)?
                    }
                    Kind::Keyword(Keyword::Class) => self.class(dstart, true, false)?,
                    Kind::Keyword(Keyword::Async) => {
                        let next = self.peek();
                        if next.kind != Kind::Keyword(Keyword::Function) || next.newline_before() {
                            return self.unexpected();
                        }
                        self.bump()?;
                        self.function(dstart, true, FunctionKind::Declaration, false)?
                    }
                    _ => return self.unexpected(),
                };
                for (name, span) in self.declared_names(declaration) {
                    self.export(name, span)?;
                }
                Ok(self.finish(
                    start,
                    Node::ExportNamed {
                        declaration: Some(declaration),
                        specifiers: List::default(),
                        source: None,
                        attributes: List::default(),
                    },
                ))
            }
        }
    }

    /// The names a declaration declares, with where.
    fn declared_names(&self, declaration: NodeId) -> Vec<(String, Span)> {
        let mut spans = Vec::new();
        match self.node(declaration) {
            Node::VariableDeclaration { declarations, .. } => {
                for &d in self.ast.get(declarations) {
                    if let Node::VariableDeclarator { id, .. } = self.node(d) {
                        self.bound_names(id, &mut spans);
                    }
                }
            }
            Node::Function(f) => spans.extend(f.id.map(|id| self.span(id))),
            Node::Class(c) => spans.extend(c.id.map(|id| self.span(id))),
            _ => {}
        }
        spans
            .into_iter()
            .map(|s| (self.name(s).into_owned(), s))
            .collect()
    }

    fn unexpected_at<T>(&mut self, t: Token) -> R<T> {
        let shown: String = t.text(self.src).chars().take(40).collect();
        self.err(
            "unexpected-token",
            t.span,
            format!("`{shown}` can't go here"),
        )
    }
}
