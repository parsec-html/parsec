// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The parser: ECMA-262's syntactic grammar, clauses 13 to 16, with
//! Annex B's additions for scripts, by recursive descent.
//!
//! The parser asks the lexer for one token at a time, always with
//! [`Goal::Div`], and reads a `/` or `}` again with the goal it turns
//! out to need: a regular expression where an expression starts, a
//! template's next part after a substitution. Nothing else is ever read
//! twice.
//!
//! The cover grammars are parsed once, as what they cover: `(a, b)` is
//! parsed as an expression and turned into arrow parameters in place when
//! `=>` follows, `[a, b]` into a pattern when `=` does. Errors that only
//! the expression reading has, like `{a = 1}` outside a pattern, wait in
//! [`Parser::pending`] until it's known which reading it is.
//!
//! The first error ends the parse. Recursion is counted against a depth
//! limit, and tokens against the step budget, so no input overflows the
//! stack or runs past what it was allowed.

mod expr;
mod function;
mod pattern;
mod stmt;

use std::borrow::Cow;
use std::collections::HashMap;

use parsec_base::{Budget, Diagnostic, Limits, Severity, Span};

use crate::ast::{Ast, Comment, CommentKind, List, Node, NodeId};
use crate::lexer::{Goal, Lexer, SourceType};
use crate::text;
use crate::token::{Flags, Keyword, Kind, Punct, Token};

/// How deep the parser recurses when [`Limits::max_depth`] doesn't say.
/// A level is a statement, a function or class, an operand of a prefix
/// operator or `**`, or an expression or pattern nested in brackets.
/// Real code stays far below it (typescript.js, 9 MB, needs 53), and at
/// the limit the parser uses at most about 250 KB of stack in a release
/// build and 700 KB in a debug one, so it fits a 1 MB thread.
pub const DEFAULT_MAX_DEPTH: u32 = 256;

/// What to parse and how much it may cost.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// A module is strict and may `await` at the top; its `import` and
    /// `export` declarations aren't parsed yet
    /// ([#154](https://github.com/parsec-html/parsec/issues/154)).
    pub source_type: SourceType,
    /// `max_depth` is how deep the tree may nest, [`DEFAULT_MAX_DEPTH`]
    /// without one; `max_steps` bounds the tokens read; `max_input` the
    /// bytes.
    pub limits: Limits,
}

/// Parses `src` as a Script or a Module, as `options` says. The first
/// syntax error is the `Err`.
///
/// ```
/// use parsec_js::{Node, Options, parse};
///
/// let ast = parse("let x = a?.b ?? 1;", &Options::default()).unwrap();
/// let Node::Program { body, .. } = *ast.node(ast.root()) else { unreachable!() };
/// assert_eq!(ast.get(body).len(), 1);
///
/// let err = parse("let x = ;", &Options::default()).unwrap_err();
/// assert_eq!((err.code, err.span.start), ("unexpected-token", 8));
/// ```
pub fn parse(src: &str, options: &Options) -> Result<Ast, Diagnostic> {
    Parser::new(src, options).parse()
}

/// Parses `src` as a Script.
pub fn parse_script(src: &str) -> Result<Ast, Diagnostic> {
    parse(src, &Options::default())
}

/// The parse ended; the diagnostic is in [`Parser::error`].
#[derive(Debug)]
pub(crate) struct Stop;

pub(crate) type R<T> = Result<T, Stop>;

/// What the grammar's parameters and the enclosing function allow here.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Ctx {
    pub strict: bool,
    /// `[Yield]`: `yield` is an operator.
    pub yield_: bool,
    /// `[Await]`: `await` is an operator.
    pub await_: bool,
    /// `[Return]`.
    pub ret: bool,
    pub new_target: bool,
    pub super_prop: bool,
    pub super_call: bool,
    /// In a class field's initializer or a static block, where
    /// `arguments` is an error.
    pub no_arguments: bool,
    /// In a class static block, where `await` is reserved.
    pub static_block: bool,
    /// Enclosing loops, and loops and switches, in this function.
    pub loops: u32,
    pub breakable: u32,
    /// Where this function's labels start in [`Parser::labels`].
    pub labels: usize,
}

/// A label in scope, and whether it labels a loop.
struct Label {
    name: String,
    is_loop: bool,
}

/// What kind of class element a private name names.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Private {
    Field,
    Method,
    Get,
    Set,
    GetSet,
}

/// A class body being parsed: its private names, and the ones used in
/// it that might be declared later in it or in a class around it.
#[derive(Default)]
struct ClassScope {
    declared: HashMap<String, (Private, bool)>,
    used: Vec<(String, Span)>,
}

pub(crate) struct Parser<'a> {
    src: &'a str,
    lexer: Lexer<'a>,
    /// The token being looked at.
    tok: Token,
    /// The end of the last token consumed.
    prev_end: u32,
    ast: Ast,
    /// Children of lists being built, innermost at the end.
    scratch: Vec<NodeId>,
    error: Option<Diagnostic>,
    /// How many of the lexer's diagnostics have been looked at.
    seen: usize,
    module: bool,
    ctx: Ctx,
    depth: u32,
    max_depth: u32,
    budget: Budget,
    labels: Vec<Label>,
    /// Where the labels directly on the statement about to be parsed
    /// start, so a loop can mark them all as labelling a loop.
    label_chain: Option<usize>,
    /// Where the expression statement being parsed starts, whose first
    /// identifier may be a label rather than a reference.
    label_candidate: Option<u32>,
    /// The statement about to be parsed is the body of an `if`, a loop
    /// or a `with`, where a labelled function declaration is an error.
    in_body: bool,
    /// Object literals with something only a pattern may have (`{a = 1}`,
    /// a second `__proto__`), and where. An entry stands until the object
    /// turns into a pattern; one still an object when its expression ends
    /// is an error.
    pending: Vec<(NodeId, Span, &'static str)>,
    classes: Vec<ClassScope>,
    /// Where the last `yield` and `await` expressions, and the last
    /// `await` read as an identifier, started, for the parameter lists
    /// that may hold none of them.
    last_yield: Option<u32>,
    last_await: Option<u32>,
    last_await_ident: Option<u32>,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(src: &'a str, options: &Options) -> Parser<'a> {
        let lexer = Lexer::with_limits(src, options.source_type, &options.limits);
        // About one node per eight bytes of minified code; the vectors
        // grow from there.
        let ast = Ast::new(lexer.source().len() / 8);
        let module = options.source_type == SourceType::Module;
        Parser {
            src: lexer.source(),
            lexer,
            tok: Token {
                kind: Kind::Eof,
                flags: Flags::empty(),
                span: Span::at(0),
            },
            prev_end: 0,
            ast,
            scratch: Vec::new(),
            error: None,
            seen: 0,
            module,
            ctx: Ctx {
                strict: module,
                await_: module,
                ..Ctx::default()
            },
            depth: 0,
            max_depth: options.limits.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
            budget: options.limits.budget(),
            labels: Vec::new(),
            label_chain: None,
            label_candidate: None,
            in_body: false,
            pending: Vec::new(),
            classes: Vec::new(),
            last_yield: None,
            last_await: None,
            last_await_ident: None,
        }
    }

    pub(crate) fn parse(mut self) -> Result<Ast, Diagnostic> {
        let result = self.program();
        if let Err(Stop) = result {
            return Err(self.error.take().expect("a stop has a diagnostic"));
        }
        let mut ast = self.ast;
        ast.warnings = self
            .lexer
            .take_diagnostics()
            .into_iter()
            .filter(|d| d.severity == Severity::Warning)
            .collect();
        Ok(ast)
    }

    fn program(&mut self) -> R<()> {
        // A lexer that refused the input has said why.
        if let Some(d) = self.lexer.diagnostics().first() {
            return self.fail(d.clone());
        }
        self.tok = self.next_token()?;
        let mark = self.scratch.len();
        self.statement_list(true)?;
        if self.tok.kind != Kind::Eof {
            return self.unexpected();
        }
        let body = self.take_list(mark);
        let source_type = if self.module {
            SourceType::Module
        } else {
            SourceType::Script
        };
        let root = self.ast.push(
            Span::new(0, self.src.len() as u32),
            Node::Program { body, source_type },
        );
        self.ast.root = Some(root);
        Ok(())
    }

    // Errors.

    fn fail<T>(&mut self, d: Diagnostic) -> R<T> {
        if self.error.is_none() {
            self.error = Some(d);
        }
        Err(Stop)
    }

    fn err<T>(
        &mut self,
        code: &'static str,
        span: Span,
        message: impl Into<Cow<'static, str>>,
    ) -> R<T> {
        self.fail(Diagnostic::error(code, span, message))
    }

    /// An error at the token being looked at, which nothing here expects.
    fn unexpected<T>(&mut self) -> R<T> {
        let t = self.tok;
        if t.kind == Kind::Eof {
            return self.err(
                "unexpected-end",
                t.span,
                "the input ends in the middle of something",
            );
        }
        let text = t.text(self.src);
        let shown: String = text.chars().take(40).collect();
        self.err(
            "unexpected-token",
            t.span,
            format!("`{shown}` can't go here"),
        )
    }

    // Tokens.

    /// The next token that isn't a comment, read with [`Goal::Div`].
    fn next_token(&mut self) -> R<Token> {
        loop {
            let t = self.lexer.next_token(Goal::Div);
            self.check_lexer()?;
            let kind = match t.kind {
                Kind::LineComment => CommentKind::Line,
                Kind::BlockComment => CommentKind::Block,
                Kind::Hashbang => CommentKind::Hashbang,
                Kind::HtmlOpenComment => CommentKind::HtmlOpen,
                Kind::HtmlCloseComment => CommentKind::HtmlClose,
                _ => return Ok(t),
            };
            self.ast.comments.push(Comment { kind, span: t.span });
        }
    }

    /// Stops at the first error the lexer has reported since last asked.
    fn check_lexer(&mut self) -> R<()> {
        let diagnostics = self.lexer.diagnostics();
        if diagnostics.len() > self.seen {
            let new = &diagnostics[self.seen..];
            self.seen = diagnostics.len();
            if let Some(d) = new.iter().find(|d| d.is_error()) {
                let d = d.clone();
                return self.fail(d);
            }
        }
        Ok(())
    }

    /// Consumes the token being looked at and returns it.
    fn bump(&mut self) -> R<Token> {
        let t = self.tok;
        if self.budget.spend(1).is_err() {
            return self.err(
                "step-budget",
                t.span,
                "the parse used up its step budget here",
            );
        }
        if t.flags.contains(Flags::LEGACY_OCTAL) && self.ctx.strict {
            return self.err(
                "legacy-octal-in-strict",
                t.span,
                "legacy octal literals and escapes aren't allowed in strict code",
            );
        }
        self.prev_end = t.span.end;
        self.tok = self.next_token()?;
        Ok(t)
    }

    /// Reads the token being looked at, a `/` or `}`, again with `goal`.
    fn rescan(&mut self, goal: Goal) -> R<()> {
        self.tok = self.lexer.rescan(self.tok, goal);
        self.check_lexer()
    }

    /// The token after the one being looked at.
    fn peek(&self) -> Token {
        let mut lexer = self.lexer.fork();
        loop {
            let t = lexer.next_token(Goal::Div);
            if !t.is_trivia() {
                return t;
            }
        }
    }

    fn at(&self, p: Punct) -> bool {
        self.tok.kind == Kind::Punct(p)
    }

    /// The token is the keyword `k`, spelled without escapes.
    fn at_kw(&self, k: Keyword) -> bool {
        self.tok.kind == Kind::Keyword(k)
    }

    fn eat(&mut self, p: Punct) -> R<bool> {
        if self.at(p) {
            self.bump()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn expect(&mut self, p: Punct) -> R<Token> {
        if self.at(p) {
            return self.bump();
        }
        self.unexpected()
    }

    fn expect_kw(&mut self, k: Keyword) -> R<Token> {
        if self.at_kw(k) {
            return self.bump();
        }
        self.unexpected()
    }

    /// A `;`, or where automatic semicolon insertion puts one: before a
    /// line break, a `}` or the end.
    fn semicolon(&mut self) -> R<()> {
        if self.eat(Punct::Semicolon)? {
            return Ok(());
        }
        if self.can_insert_semicolon() {
            return Ok(());
        }
        self.unexpected()
    }

    fn can_insert_semicolon(&self) -> bool {
        self.tok.newline_before() || self.at(Punct::RBrace) || self.tok.kind == Kind::Eof
    }

    // Nodes and lists.

    fn finish(&mut self, start: u32, node: Node) -> NodeId {
        self.ast
            .push(Span::new(start, self.prev_end.max(start)), node)
    }

    fn take_list(&mut self, mark: usize) -> List {
        self.ast.list(self.scratch.drain(mark..))
    }

    fn node(&self, id: NodeId) -> Node {
        *self.ast.node(id)
    }

    fn span(&self, id: NodeId) -> Span {
        self.ast.span(id)
    }

    // Limits.

    /// One level deeper, or an error past the limit.
    fn enter(&mut self) -> R<()> {
        self.depth += 1;
        if self.depth > self.max_depth {
            let span = self.tok.span;
            return self.err(
                "too-deep",
                span,
                format!("this nests deeper than the limit of {}", self.max_depth),
            );
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    // Identifiers.

    /// An identifier's name, escapes decoded.
    fn name(&self, span: Span) -> Cow<'a, str> {
        text::ident_name(span.text(self.src))
    }

    /// The token is an IdentifierName: an identifier or any keyword.
    fn at_ident_name(&self) -> bool {
        matches!(self.tok.kind, Kind::Ident | Kind::Keyword(_))
    }

    /// The token could be an identifier reference here, though it may
    /// still be an error as one (a reserved word escaped, `yield` in
    /// strict code).
    fn at_identifier(&self) -> bool {
        match self.tok.kind {
            Kind::Ident => true,
            Kind::Keyword(k) => !k.is_reserved() || matches!(k, Keyword::Yield | Keyword::Await),
            _ => false,
        }
    }

    /// The word an identifier token spells, if it's one of [`Keyword`]'s,
    /// escapes decoded.
    fn word(&self, t: Token) -> Option<Keyword> {
        match t.kind {
            Kind::Keyword(k) => Some(k),
            Kind::Ident if t.flags.contains(Flags::ESCAPED) => {
                Keyword::from_str(&self.name(t.span))
            }
            _ => None,
        }
    }

    /// Checks `t` as an IdentifierReference, a BindingIdentifier when
    /// `binding`, or a LabelIdentifier, in this context.
    fn check_identifier(&mut self, t: Token, binding: bool) -> R<()> {
        let word = self.word(t);
        let reserved = match word {
            None => false,
            Some(Keyword::Await) => {
                self.last_await_ident = Some(t.span.start);
                self.ctx.await_ || self.module || self.ctx.static_block
            }
            Some(Keyword::Yield) => self.ctx.yield_ || self.ctx.strict,
            Some(k) if k.is_strict_reserved() => self.ctx.strict,
            Some(k) => k.is_reserved(),
        };
        if reserved {
            let name = self.name(t.span).into_owned();
            return self.err(
                "reserved-word",
                t.span,
                format!("`{name}` is reserved here, so it can't be an identifier"),
            );
        }
        if word.is_none() {
            let text = t.text(self.src);
            let name = if t.flags.contains(Flags::ESCAPED) {
                self.name(t.span)
            } else {
                Cow::Borrowed(text)
            };
            if binding && self.ctx.strict && (name == "eval" || name == "arguments") {
                return self.err(
                    "strict-binding",
                    t.span,
                    format!("strict code can't bind or assign `{name}`"),
                );
            }
        }
        Ok(())
    }

    /// `arguments` can't be referred to in a class field's initializer or
    /// a static block.
    fn check_arguments(&mut self, span: Span) -> R<()> {
        if self.ctx.no_arguments && self.name(span) == "arguments" {
            return self.err(
                "arguments-in-class-init",
                span,
                "`arguments` isn't allowed in a class field or static block",
            );
        }
        Ok(())
    }

    /// The token as an identifier reference or binding, checked, as a node.
    fn identifier(&mut self, binding: bool) -> R<NodeId> {
        // A reserved word spelled plainly is no identifier, and saying
        // it's reserved would only confuse `if (a) else`.
        if !self.at_identifier() {
            return self.unexpected();
        }
        let t = self.tok;
        self.check_identifier(t, binding)?;
        self.bump()?;
        Ok(self.ast.push(t.span, Node::Identifier))
    }

    /// The token as an IdentifierName, any word at all, as a node.
    fn identifier_name(&mut self) -> R<NodeId> {
        if !self.at_ident_name() {
            return self.unexpected();
        }
        let t = self.bump()?;
        Ok(self.ast.push(t.span, Node::Identifier))
    }

    // Private names.

    fn declare_private(&mut self, span: Span, kind: Private, is_static: bool) -> R<()> {
        let name = self.name(Span::new(span.start + 1, span.end)).into_owned();
        if name == "constructor" {
            return self.err(
                "private-constructor",
                span,
                "a private name can't be #constructor",
            );
        }
        let Some(scope) = self.classes.last_mut() else {
            return self.err(
                "private-outside-class",
                span,
                "private names are only declared in classes",
            );
        };
        match scope.declared.get_mut(&name) {
            None => {
                scope.declared.insert(name, (kind, is_static));
                Ok(())
            }
            Some((had, was_static)) if *was_static == is_static => match (*had, kind) {
                (Private::Get, Private::Set) | (Private::Set, Private::Get) => {
                    *had = Private::GetSet;
                    Ok(())
                }
                _ => self.err(
                    "duplicate-private-name",
                    span,
                    format!("#{name} is declared twice"),
                ),
            },
            Some(_) => self.err(
                "duplicate-private-name",
                span,
                format!("#{name} is declared twice"),
            ),
        }
    }

    fn use_private(&mut self, span: Span) -> R<()> {
        let name = self.name(Span::new(span.start + 1, span.end)).into_owned();
        match self.classes.last_mut() {
            Some(scope) => {
                scope.used.push((name, span));
                Ok(())
            }
            None => self.err(
                "undeclared-private-name",
                span,
                format!("#{name} isn't declared in a class around it"),
            ),
        }
    }

    /// Ends a class body: each private name it used is declared in it, or
    /// passed out to the class around it.
    fn end_class_scope(&mut self) -> R<()> {
        let scope = self.classes.pop().unwrap_or_default();
        for (name, span) in scope.used {
            if scope.declared.contains_key(&name) {
                continue;
            }
            match self.classes.last_mut() {
                Some(outer) => outer.used.push((name, span)),
                None => {
                    return self.err(
                        "undeclared-private-name",
                        span,
                        format!("#{name} isn't declared in a class around it"),
                    );
                }
            }
        }
        Ok(())
    }

    // Cover grammar.

    /// Fails on the first object literal since `mark` that has something
    /// only a pattern can have and didn't turn into a pattern.
    fn check_pending(&mut self, mark: usize) -> R<()> {
        if self.pending.len() <= mark {
            return Ok(());
        }
        let found = self.pending[mark..]
            .iter()
            .find(|(id, ..)| matches!(self.ast.node(*id), Node::Object { .. }))
            .copied();
        self.pending.truncate(mark);
        match found {
            Some((_, span, "shorthand-init")) => self.err(
                "shorthand-init",
                span,
                "`{a = 1}` is only allowed as a pattern, like `({a = 1} = b)`",
            ),
            Some((_, span, _)) => self.err(
                "duplicate-proto",
                span,
                "an object literal can set __proto__ only once",
            ),
            None => Ok(()),
        }
    }
}

/// Whether a token of this kind can start an expression, for `yield`,
/// whose operand is optional.
fn starts_expression(kind: Kind) -> bool {
    match kind {
        Kind::Eof => false,
        Kind::Keyword(Keyword::In | Keyword::Instanceof) => false,
        Kind::Punct(p) => matches!(
            p,
            Punct::LParen
                | Punct::LBracket
                | Punct::LBrace
                | Punct::Plus
                | Punct::Minus
                | Punct::Bang
                | Punct::Tilde
                | Punct::PlusPlus
                | Punct::MinusMinus
                | Punct::Slash
                | Punct::SlashEq
        ),
        _ => true,
    }
}
