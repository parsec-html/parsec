// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Statements and declarations (ECMA-262 clause 14), with Annex B's
//! function declarations as `if` bodies and labelled items in sloppy
//! code.

use parsec_base::{Diagnostic, Span};

use super::{Ctx, Label, Parser, R, Stop};
use crate::ast::{FunctionKind, Node, NodeId, VarKind};
use crate::token::{Flags, Keyword, Kind, Punct};

impl Parser<'_> {
    /// StatementListItems into the scratch list, up to a `}` or the end,
    /// which it leaves. With `prologue`, the directive prologue first;
    /// where it said `"use strict"`, if it did.
    pub(super) fn statement_list(&mut self, prologue: bool) -> R<Option<Span>> {
        let mut use_strict = None;
        let mut octal: Option<Span> = None;
        if prologue {
            while self.tok.kind == Kind::String {
                let t = self.tok;
                let Some(item) = self.list_item()? else {
                    break;
                };
                self.scratch.push(item);
                let Node::Expression { expression, .. } = self.node(item) else {
                    break;
                };
                if self.span(expression) != t.span {
                    break;
                }
                *self.ast.node_mut(item) = Node::Expression {
                    expression,
                    directive: true,
                };
                if t.flags.contains(Flags::LEGACY_OCTAL) {
                    octal.get_or_insert(t.span);
                }
                let raw = &self.src[t.span.start as usize + 1..t.span.end as usize - 1];
                if raw == "use strict" {
                    use_strict = Some(t.span);
                    if !self.ctx.strict {
                        self.ctx.strict = true;
                        if let Some(span) = octal {
                            return self.fail(
                                Diagnostic::error(
                                    "legacy-octal-in-strict",
                                    span,
                                    "legacy octal escapes aren't allowed in strict code",
                                )
                                .with_label(t.span, "this makes the code strict"),
                            );
                        }
                    }
                }
            }
        }
        while !self.at(Punct::RBrace) && self.tok.kind != Kind::Eof {
            if let Some(item) = self.list_item()? {
                self.scratch.push(item);
            }
        }
        Ok(use_strict)
    }

    /// A StatementListItem, or when recovering from an error in it,
    /// `None`, the error kept and the tokens up to the next statement
    /// skipped.
    fn list_item(&mut self) -> R<Option<NodeId>> {
        if !self.recover {
            return self.statement_list_item().map(Some);
        }
        let saved = Saved {
            ctx: self.ctx,
            depth: self.depth,
            scratch: self.scratch.len(),
            labels: self.labels.len(),
            if_chain: self.if_chain.len(),
            pending: self.pending.len(),
            classes: self.classes.len(),
            start: self.tok.span.start,
        };
        match self.statement_list_item() {
            Ok(item) => Ok(Some(item)),
            Err(Stop) => {
                self.recover_from(saved)?;
                Ok(None)
            }
        }
    }

    /// Keeps the error that stopped a statement, puts the parser back as
    /// it was before it, and skips to where the next statement starts.
    /// An error from a limit stops the parse.
    #[cold]
    #[inline(never)]
    fn recover_from(&mut self, saved: Saved) -> R<()> {
        let d = self.error.take().expect("a stop has a diagnostic");
        if matches!(d.code, "too-deep" | "step-budget" | "input-too-large") {
            return self.fail(d);
        }
        self.errors.push(d);
        self.ctx = saved.ctx;
        self.depth = saved.depth;
        self.scratch.truncate(saved.scratch);
        self.labels.truncate(saved.labels);
        self.if_chain.truncate(saved.if_chain);
        self.pending.truncate(saved.pending);
        self.classes.truncate(saved.classes);
        self.label_chain = None;
        self.label_candidate = None;
        self.in_body = false;
        // Whatever comes next, a statement that took no token gets one
        // skipped, so the same error isn't found again.
        let mut first = self.tok.span.start == saved.start;
        // The `{` and template substitutions open: those the statement
        // opened before its error, read again, and those met while
        // skipping. Inside them, nothing ends the statement.
        let mut open: Vec<Kind> = Vec::new();
        let mut last = Kind::Eof;
        let mut lexer = self.lexer.starting_at(saved.start, false);
        loop {
            let t = lexer.next_token(crate::lexer::Goal::Div);
            if t.kind == Kind::Eof || t.span.start >= self.tok.span.start {
                break;
            }
            if self.budget.spend(1).is_err() {
                return self.err(
                    "step-budget",
                    t.span,
                    "the parse used up its step budget here",
                );
            }
            if !t.is_trivia() {
                let t = match t.kind {
                    Kind::Punct(Punct::RBrace) if open.last() == Some(&Kind::TemplateHead) => {
                        lexer.rescan(t, crate::lexer::Goal::TemplateTail)
                    }
                    Kind::Punct(Punct::Slash | Punct::SlashEq) if regexp_after(last) => {
                        lexer.rescan(t, crate::lexer::Goal::RegExp)
                    }
                    _ => t,
                };
                track(&mut open, t.kind);
                last = t.kind;
            }
        }
        loop {
            let t = self.tok;
            match t.kind {
                Kind::Eof => return Ok(()),
                _ if first => {}
                Kind::Punct(Punct::Semicolon) if open.is_empty() => {
                    self.skip_token()?;
                    return Ok(());
                }
                Kind::Punct(Punct::RBrace) if open.is_empty() => return Ok(()),
                Kind::Keyword(k)
                    if open.is_empty() && t.newline_before() && starts_statement(k) =>
                {
                    return Ok(());
                }
                _ => {}
            }
            first = false;
            match t.kind {
                Kind::Punct(Punct::RBrace)
                    if open.last() == Some(&Kind::TemplateHead) && self.rescannable(t) =>
                {
                    self.tok = self.lexer.rescan(t, crate::lexer::Goal::TemplateTail);
                }
                Kind::Punct(Punct::Slash | Punct::SlashEq)
                    if regexp_after(last) && self.rescannable(t) =>
                {
                    self.tok = self.lexer.rescan(t, crate::lexer::Goal::RegExp);
                }
                _ => {}
            }
            track(&mut open, self.tok.kind);
            last = self.tok.kind;
            self.skip_token()?;
        }
    }

    /// Whether `t` is the last token the lexer read, which an error can
    /// leave it not to be.
    fn rescannable(&self, t: crate::token::Token) -> bool {
        self.lexer.position() == t.span.end
    }

    /// The token being looked at, skipped while recovering, with the
    /// comments after it. The lexer's errors in what's skipped are left
    /// out, since they'd only follow from reading it wrong.
    pub(super) fn skip_token(&mut self) -> R<()> {
        let t = self.tok;
        if self.budget.spend(1).is_err() {
            return self.err(
                "step-budget",
                t.span,
                "the parse used up its step budget here",
            );
        }
        self.prev_end = t.span.end;
        let mut next = self.lexer.next_token(crate::lexer::Goal::Div);
        while next.is_trivia() {
            next = self.lexer.next_token(crate::lexer::Goal::Div);
        }
        self.seen = self.lexer.diagnostics().len();
        self.tok = next;
        Ok(())
    }

    /// Whether a `let` being looked at starts a lexical declaration: it
    /// does before a binding identifier or pattern.
    fn let_declaration(&self) -> bool {
        let next = self.peek();
        match next.kind {
            Kind::Ident | Kind::Punct(Punct::LBracket | Punct::LBrace) => true,
            Kind::Keyword(k) => {
                !k.is_reserved() || matches!(k, Keyword::Yield | Keyword::Await | Keyword::Let)
            }
            _ => false,
        }
    }

    /// The token is `async` and `function` follows on the same line.
    fn at_async_function(&self) -> bool {
        if !self.at_kw(Keyword::Async) {
            return false;
        }
        let next = self.peek();
        next.kind == Kind::Keyword(Keyword::Function) && !next.newline_before()
    }

    pub(super) fn statement_list_item(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        match self.tok.kind {
            Kind::Keyword(Keyword::Function) => {
                self.function(start, false, FunctionKind::Declaration, false)
            }
            Kind::Keyword(Keyword::Class) => self.class(start, true, false),
            Kind::Keyword(Keyword::Const) => self.declaration_statement(VarKind::Const),
            Kind::Keyword(Keyword::Let) if self.let_declaration() => {
                self.declaration_statement(VarKind::Let)
            }
            Kind::Keyword(Keyword::Async) if self.at_async_function() => {
                self.bump()?;
                self.function(start, true, FunctionKind::Declaration, false)
            }
            Kind::Keyword(Keyword::Export) => self.module_item(),
            Kind::Keyword(Keyword::Import) if self.at_import_declaration() => self.module_item(),
            _ => self.statement(),
        }
    }

    pub(super) fn declaration_statement(&mut self, kind: VarKind) -> R<NodeId> {
        let decl = self.variable_declaration(kind, false, false)?;
        self.semicolon()?;
        let span = Span::new(self.span(decl).start, self.prev_end);
        self.ast.set_span(decl, span);
        Ok(decl)
    }

    /// A Statement: no declarations but `var`.
    pub(super) fn statement(&mut self) -> R<NodeId> {
        self.enter()?;
        let chain = self.label_chain.take();
        let in_body = std::mem::take(&mut self.in_body);
        let start = self.tok.span.start;
        let node = match self.tok.kind {
            Kind::Punct(Punct::LBrace) => self.block()?,
            Kind::Punct(Punct::Semicolon) => {
                self.bump()?;
                self.finish(start, Node::Empty)
            }
            Kind::Keyword(Keyword::Var) => self.declaration_statement(VarKind::Var)?,
            Kind::Keyword(Keyword::If) => self.if_statement()?,
            Kind::Keyword(k @ (Keyword::For | Keyword::While | Keyword::Do)) => {
                if let Some(chain) = chain {
                    for label in &mut self.labels[chain..] {
                        label.is_loop = true;
                    }
                }
                match k {
                    Keyword::For => self.for_statement()?,
                    Keyword::While => {
                        self.bump()?;
                        let open = self.expect(Punct::LParen)?;
                        let test = self.expression(false)?;
                        self.expect_in(Punct::RParen, open)?;
                        let body = self.loop_body()?;
                        self.finish(start, Node::While { test, body })
                    }
                    _ => {
                        self.bump()?;
                        let body = self.loop_body()?;
                        self.expect_kw(Keyword::While)?;
                        let open = self.expect(Punct::LParen)?;
                        let test = self.expression(false)?;
                        self.expect_in(Punct::RParen, open)?;
                        // A semicolon is always inserted after a do-while.
                        self.eat(Punct::Semicolon)?;
                        self.finish(start, Node::DoWhile { body, test })
                    }
                }
            }
            Kind::Keyword(Keyword::Continue | Keyword::Break) => self.jump()?,
            Kind::Keyword(Keyword::Return) => {
                if !self.ctx.ret {
                    let span = self.tok.span;
                    return self.err(
                        "return-outside-function",
                        span,
                        "`return` is only allowed in a function",
                    );
                }
                self.bump()?;
                let argument = if self.at(Punct::Semicolon) || self.can_insert_semicolon() {
                    None
                } else {
                    Some(self.expression(false)?)
                };
                self.semicolon()?;
                self.finish(start, Node::Return { argument })
            }
            Kind::Keyword(Keyword::With) => {
                if self.ctx.strict {
                    let span = self.tok.span;
                    return self.err("with-in-strict", span, "strict code can't use `with`");
                }
                self.bump()?;
                let open = self.expect(Punct::LParen)?;
                let object = self.expression(false)?;
                self.expect_in(Punct::RParen, open)?;
                self.in_body = true;
                let body = self.statement()?;
                self.finish(start, Node::With { object, body })
            }
            Kind::Keyword(Keyword::Switch) => self.switch_statement()?,
            Kind::Keyword(Keyword::Throw) => {
                let throw = self.bump()?;
                if self.tok.newline_before() {
                    let span = self.tok.span;
                    return self.fail(
                        Diagnostic::error(
                            "newline-after-throw",
                            span,
                            "a `throw` and its expression have to be on the same line",
                        )
                        .with_label(throw.span, "the line ends after this `throw`"),
                    );
                }
                let argument = self.expression(false)?;
                self.semicolon()?;
                self.finish(start, Node::Throw { argument })
            }
            Kind::Keyword(Keyword::Try) => self.try_statement()?,
            Kind::Keyword(Keyword::Debugger) => {
                self.bump()?;
                self.semicolon()?;
                self.finish(start, Node::Debugger)
            }
            Kind::Keyword(Keyword::Function | Keyword::Class | Keyword::Const) => {
                return self.declaration_here();
            }
            Kind::Keyword(Keyword::Let) if self.peek().kind == Kind::Punct(Punct::LBracket) => {
                return self.declaration_here();
            }
            Kind::Keyword(Keyword::Async) if self.at_async_function() => {
                return self.declaration_here();
            }
            _ => self.expression_statement(chain, in_body)?,
        };
        self.leave();
        Ok(node)
    }

    fn declaration_here<T>(&mut self) -> R<T> {
        let span = self.tok.span;
        self.err(
            "declaration-not-allowed",
            span,
            "a declaration can't be the body of a statement; put it in a block",
        )
    }

    fn expression_statement(&mut self, chain: Option<usize>, in_body: bool) -> R<NodeId> {
        let t = self.tok;
        let start = t.span.start;
        self.label_candidate = Some(start);
        let expression = self.expression(false)?;
        if self.at(Punct::Colon)
            && self.span(expression) == t.span
            && matches!(self.node(expression), Node::Identifier)
        {
            return self.labeled(start, expression, chain, in_body);
        }
        self.semicolon()?;
        Ok(self.finish(
            start,
            Node::Expression {
                expression,
                directive: false,
            },
        ))
    }

    fn labeled(
        &mut self,
        start: u32,
        label: NodeId,
        chain: Option<usize>,
        in_body: bool,
    ) -> R<NodeId> {
        let span = self.span(label);
        let name = self.name(span).into_owned();
        if let Some(first) = self.labels[self.ctx.labels..]
            .iter()
            .find(|l| l.name == name)
        {
            let first = first.span;
            return self.fail(
                Diagnostic::error(
                    "duplicate-label",
                    span,
                    format!("the label `{name}` is already in use here"),
                )
                .with_label(first, "the label is defined here"),
            );
        }
        self.bump()?;
        self.label_chain = Some(chain.unwrap_or(self.labels.len()));
        self.labels.push(Label {
            name,
            span,
            is_loop: false,
        });
        let body = if self.at_kw(Keyword::Function) {
            self.label_chain = None;
            let fstart = self.tok.span.start;
            if in_body || self.ctx.strict || self.peek().kind == Kind::Punct(Punct::Star) {
                return self.declaration_here();
            }
            self.function(fstart, false, FunctionKind::Declaration, false)?
        } else {
            self.in_body = in_body;
            self.statement()?
        };
        self.labels.pop();
        Ok(self.finish(start, Node::Labeled { label, body }))
    }

    fn jump(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let t = self.bump()?;
        let is_break = t.kind == Kind::Keyword(Keyword::Break);
        let label = if !self.tok.newline_before() && self.at_identifier() {
            let label = self.identifier(false)?;
            let span = self.span(label);
            let name = self.name(span);
            let found = self.labels[self.ctx.labels..]
                .iter()
                .rev()
                .find(|l| l.name == name);
            match found {
                Some(l) if is_break || l.is_loop => {}
                Some(l) => {
                    let defined = l.span;
                    return self.fail(
                        Diagnostic::error(
                            "continue-not-loop",
                            span,
                            format!("`continue {name}` needs `{name}` to label a loop"),
                        )
                        .with_label(defined, "the label is defined here, not on a loop"),
                    );
                }
                None => {
                    return self.err(
                        "undefined-label",
                        span,
                        format!("there's no label `{name}` here"),
                    );
                }
            }
            Some(label)
        } else {
            let ok = if is_break {
                self.ctx.breakable > 0
            } else {
                self.ctx.loops > 0
            };
            if !ok {
                return self.err(
                    "jump-outside-loop",
                    t.span,
                    if is_break {
                        "`break` is only allowed in a loop or a switch"
                    } else {
                        "`continue` is only allowed in a loop"
                    },
                );
            }
            None
        };
        self.semicolon()?;
        Ok(self.finish(
            start,
            if is_break {
                Node::Break { label }
            } else {
                Node::Continue { label }
            },
        ))
    }

    pub(super) fn block(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        let open = self.expect(Punct::LBrace)?;
        let mark = self.scratch.len();
        self.statement_list(false)?;
        self.expect_in(Punct::RBrace, open)?;
        let body = self.take_list(mark);
        Ok(self.finish(start, Node::Block { body }))
    }

    /// `if`, with an `else if` chain read in a loop, not by recursion.
    fn if_statement(&mut self) -> R<NodeId> {
        let mark = self.if_chain.len();
        let alternate = loop {
            let start = self.tok.span.start;
            self.expect_kw(Keyword::If)?;
            let open = self.expect(Punct::LParen)?;
            let test = self.expression(false)?;
            self.expect_in(Punct::RParen, open)?;
            let consequent = self.if_body()?;
            self.if_chain.push((start, test, consequent));
            if !self.at_kw(Keyword::Else) {
                break None;
            }
            self.bump()?;
            if !self.at_kw(Keyword::If) {
                break Some(self.if_body()?);
            }
        };
        let mut alternate = alternate;
        let mut node = None;
        while self.if_chain.len() > mark {
            let (start, test, consequent) = self.if_chain.pop().expect("longer than mark");
            let id = self.finish(
                start,
                Node::If {
                    test,
                    consequent,
                    alternate,
                },
            );
            alternate = Some(id);
            node = Some(id);
        }
        Ok(node.expect("an if statement has an if"))
    }

    /// An `if`'s body, which in sloppy code may be a plain function
    /// declaration (B.3.3).
    fn if_body(&mut self) -> R<NodeId> {
        if self.at_kw(Keyword::Function) && !self.ctx.strict {
            if self.peek().kind == Kind::Punct(Punct::Star) {
                return self.declaration_here();
            }
            let start = self.tok.span.start;
            return self.function(start, false, FunctionKind::Declaration, false);
        }
        self.in_body = true;
        self.statement()
    }

    fn loop_body(&mut self) -> R<NodeId> {
        let outer = self.ctx;
        self.ctx.loops += 1;
        self.ctx.breakable += 1;
        self.in_body = true;
        let body = self.statement();
        self.ctx = outer;
        body
    }

    /// `var`, `let` or `const` and its declarators. In a `for` head the
    /// initializers a declaration needs aren't checked: `in` or `of` may
    /// follow.
    pub(super) fn variable_declaration(
        &mut self,
        kind: VarKind,
        no_in: bool,
        for_head: bool,
    ) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mark = self.scratch.len();
        loop {
            let dstart = self.tok.span.start;
            let id = self.binding_target()?;
            if kind != VarKind::Var {
                self.check_not_let(id)?;
            }
            let init = if self.eat(Punct::Eq)? {
                Some(self.assign(no_in)?)
            } else {
                if !for_head {
                    self.check_initialized(kind, id)?;
                }
                None
            };
            let d = self.finish(dstart, Node::VariableDeclarator { id, init });
            self.scratch.push(d);
            if !self.eat(Punct::Comma)? {
                break;
            }
        }
        if kind != VarKind::Var {
            let mut names = std::mem::take(&mut self.names);
            names.clear();
            for &d in &self.scratch[mark..] {
                if let Node::VariableDeclarator { id, .. } = self.node(d) {
                    self.bound_names(id, &mut names);
                }
            }
            self.check_duplicate_names(names)?;
        }
        let declarations = self.take_list(mark);
        Ok(self.finish(start, Node::VariableDeclaration { kind, declarations }))
    }

    /// A `const`, or a declarator with a pattern, needs an initializer.
    fn check_initialized(&mut self, kind: VarKind, id: NodeId) -> R<()> {
        if kind == VarKind::Const {
            let span = self.span(id);
            return self.err("const-without-init", span, "a `const` needs an initializer");
        }
        if !matches!(self.node(id), Node::Identifier) {
            let span = self.span(id);
            return self.err(
                "pattern-without-init",
                span,
                "a destructuring declaration needs an initializer",
            );
        }
        Ok(())
    }

    /// `let` can't be a name a `let` or `const` declares.
    fn check_not_let(&mut self, id: NodeId) -> R<()> {
        let mut names = std::mem::take(&mut self.names);
        names.clear();
        self.bound_names(id, &mut names);
        let found = names.iter().copied().find(|&s| self.name_is(s, "let"));
        self.names = names;
        if let Some(span) = found {
            return self.err(
                "let-binding",
                span,
                "`let` can't be a name a `let` or `const` declares",
            );
        }
        Ok(())
    }

    fn for_statement(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let mut is_await = false;
        if self.at_kw(Keyword::Await) {
            if !self.ctx.await_ || self.ctx.static_block {
                let span = self.tok.span;
                return self.err(
                    "for-await-outside-async",
                    span,
                    "`for await` is only allowed in an async function or a module",
                );
            }
            self.bump()?;
            is_await = true;
        }
        let open = self.expect(Punct::LParen)?;
        let first = self.tok;
        let mut decl = None;
        let mut cover_mark = None;
        let init = if self.at(Punct::Semicolon) {
            None
        } else if self.at_kw(Keyword::Var) {
            decl = Some(VarKind::Var);
            Some(self.variable_declaration(VarKind::Var, true, true)?)
        } else if self.at_kw(Keyword::Const) {
            decl = Some(VarKind::Const);
            Some(self.variable_declaration(VarKind::Const, true, true)?)
        } else if self.at_kw(Keyword::Let) && self.let_declaration() {
            decl = Some(VarKind::Let);
            Some(self.variable_declaration(VarKind::Let, true, true)?)
        } else {
            cover_mark = Some(self.pending.len());
            Some(self.expression_cover(true)?)
        };
        let of = self.at_kw(Keyword::Of);
        if let Some(left) = init.filter(|_| of || self.at_kw(Keyword::In)) {
            if is_await && !of {
                return self.unexpected();
            }
            match decl {
                Some(kind) => {
                    let Node::VariableDeclaration { declarations, .. } = self.node(left) else {
                        unreachable!()
                    };
                    let declarators = self.ast.get(declarations);
                    if declarators.len() != 1 {
                        let span = self.span(left);
                        return self.err(
                            "for-in-of-declarations",
                            span,
                            "a for-in or for-of declares exactly one binding",
                        );
                    }
                    if let Node::VariableDeclarator { id, init: Some(_) } =
                        self.node(declarators[0])
                    {
                        // B.3.5: `for (var x = 1 in o)`, in sloppy code.
                        let annex_b = !of
                            && kind == VarKind::Var
                            && !self.ctx.strict
                            && matches!(self.node(id), Node::Identifier);
                        if !annex_b {
                            let span = self.span(declarators[0]);
                            return self.err(
                                "for-in-of-initializer",
                                span,
                                "a for-in or for-of binding can't have an initializer",
                            );
                        }
                    }
                }
                None => {
                    // `for (let of x)` and `for (async of x)` are kept
                    // for what they'd be read as otherwise.
                    let span = self.span(left);
                    let lets = first.kind == Kind::Keyword(Keyword::Let);
                    let async_of = first.kind == Kind::Keyword(Keyword::Async)
                        && span == first.span
                        && !is_await;
                    if of && (lets || async_of) {
                        return self.err(
                            "for-of-ambiguous",
                            span,
                            "a for-of can't start with `let`, or be `async of`",
                        );
                    }
                    match self.node(self.ast.unparenthesized(left)) {
                        Node::Assign { .. } => {
                            return self.err(
                                "for-in-of-initializer",
                                span,
                                "a for-in or for-of target can't have an initializer",
                            );
                        }
                        Node::Call { .. } => self.simple_target(left, true)?,
                        _ => self.reinterpret(left, false)?,
                    }
                    if let Some(mark) = cover_mark {
                        self.check_pending(mark)?;
                    }
                }
            }
            self.bump()?;
            let right = if of {
                self.assign(false)?
            } else {
                self.expression(false)?
            };
            self.expect_in(Punct::RParen, open)?;
            let body = self.loop_body()?;
            let node = if of {
                Node::ForOf {
                    left,
                    right,
                    body,
                    is_await,
                }
            } else {
                Node::ForIn { left, right, body }
            };
            return Ok(self.finish(start, node));
        }
        if is_await {
            return self.unexpected();
        }
        if let Some(mark) = cover_mark {
            self.check_pending(mark)?;
        }
        if let (Some(kind), Some(init)) = (decl, init) {
            let Node::VariableDeclaration { declarations, .. } = self.node(init) else {
                unreachable!()
            };
            for i in 0..self.ast.get(declarations).len() {
                let d = self.ast.get(declarations)[i];
                if let Node::VariableDeclarator { id, init: None } = self.node(d) {
                    self.check_initialized(kind, id)?;
                }
            }
        }
        self.expect(Punct::Semicolon)?;
        let test = if self.at(Punct::Semicolon) {
            None
        } else {
            Some(self.expression(false)?)
        };
        self.expect(Punct::Semicolon)?;
        let update = if self.at(Punct::RParen) {
            None
        } else {
            Some(self.expression(false)?)
        };
        self.expect_in(Punct::RParen, open)?;
        let body = self.loop_body()?;
        Ok(self.finish(
            start,
            Node::For {
                init,
                test,
                update,
                body,
            },
        ))
    }

    fn switch_statement(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let open = self.expect(Punct::LParen)?;
        let discriminant = self.expression(false)?;
        self.expect_in(Punct::RParen, open)?;
        self.expect(Punct::LBrace)?;
        self.ctx.breakable += 1;
        let mark = self.scratch.len();
        let mut default: Option<Span> = None;
        while !self.at(Punct::RBrace) {
            let cstart = self.tok.span.start;
            let test = if self.at_kw(Keyword::Case) {
                self.bump()?;
                Some(self.expression(false)?)
            } else if self.at_kw(Keyword::Default) {
                let span = self.tok.span;
                if let Some(first) = default {
                    return self.fail(
                        Diagnostic::error(
                            "duplicate-default",
                            span,
                            "a switch can have only one `default`",
                        )
                        .with_label(first, "the first `default`"),
                    );
                }
                default = Some(span);
                self.bump()?;
                None
            } else {
                return self.unexpected();
            };
            self.expect(Punct::Colon)?;
            let cmark = self.scratch.len();
            while !self.at(Punct::RBrace)
                && !self.at_kw(Keyword::Case)
                && !self.at_kw(Keyword::Default)
                && self.tok.kind != Kind::Eof
            {
                let item = self.statement_list_item()?;
                self.scratch.push(item);
            }
            let consequent = self.take_list(cmark);
            let case = self.finish(cstart, Node::SwitchCase { test, consequent });
            self.scratch.push(case);
        }
        self.bump()?;
        self.ctx.breakable -= 1;
        let cases = self.take_list(mark);
        Ok(self.finish(
            start,
            Node::Switch {
                discriminant,
                cases,
            },
        ))
    }

    fn try_statement(&mut self) -> R<NodeId> {
        let start = self.tok.span.start;
        self.bump()?;
        let block = self.block()?;
        let handler = if self.at_kw(Keyword::Catch) {
            let cstart = self.tok.span.start;
            self.bump()?;
            let param = if self.at(Punct::LParen) {
                let open = self.bump()?;
                let p = self.binding_target()?;
                self.check_duplicates(&[p])?;
                self.expect_in(Punct::RParen, open)?;
                Some(p)
            } else {
                None
            };
            let body = self.block()?;
            Some(self.finish(cstart, Node::CatchClause { param, body }))
        } else {
            None
        };
        let finalizer = if self.at_kw(Keyword::Finally) {
            self.bump()?;
            Some(self.block()?)
        } else {
            None
        };
        if handler.is_none() && finalizer.is_none() {
            return self.unexpected();
        }
        Ok(self.finish(
            start,
            Node::Try {
                block,
                handler,
                finalizer,
            },
        ))
    }
}

/// What [`Parser::list_item`] puts back after an error.
struct Saved {
    ctx: Ctx,
    depth: u32,
    scratch: usize,
    labels: usize,
    if_chain: usize,
    pending: usize,
    classes: usize,
    /// Where the statement started.
    start: u32,
}

/// The braces and template substitutions open, after a token of kind
/// `kind`. Parentheses and brackets aren't counted: one left open by a
/// typo would hide every statement after it.
fn track(open: &mut Vec<Kind>, kind: Kind) {
    match kind {
        Kind::Punct(Punct::LBrace) | Kind::TemplateHead => open.push(kind),
        Kind::Punct(Punct::RBrace) | Kind::TemplateTail => {
            open.pop();
        }
        _ => {}
    }
}

/// Keywords that start a statement, where skipping after an error stops
/// when one starts a line.
fn starts_statement(k: Keyword) -> bool {
    matches!(
        k,
        Keyword::Var
            | Keyword::Let
            | Keyword::Const
            | Keyword::Function
            | Keyword::Class
            | Keyword::If
            | Keyword::For
            | Keyword::While
            | Keyword::Do
            | Keyword::Return
            | Keyword::Try
            | Keyword::Throw
            | Keyword::Switch
            | Keyword::Break
            | Keyword::Continue
            | Keyword::Import
            | Keyword::Export
            | Keyword::Debugger
            | Keyword::With
    )
}

/// Whether a `/` after a token of this kind, met while skipping, starts
/// a regular expression: the guess the parser doesn't need to make
/// elsewhere.
fn regexp_after(kind: Kind) -> bool {
    match kind {
        Kind::Punct(p) => !matches!(p, Punct::RParen | Punct::RBracket | Punct::RBrace),
        Kind::Keyword(k) => {
            k.is_reserved()
                && !matches!(
                    k,
                    Keyword::This | Keyword::Super | Keyword::Null | Keyword::True | Keyword::False
                )
        }
        Kind::Eof | Kind::TemplateHead | Kind::TemplateMiddle => true,
        _ => false,
    }
}
