// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The parser, CSS Syntax §5, with the entry points of the current
//! editor's draft.
//!
//! Each algorithm reads component values, not tokens: from the tokenizer,
//! which builds a block or function whole as soon as it starts, or from a
//! block already parsed. The spec allows either, and it means a rule's
//! `{}` block can be kept as it is and its contents parsed later, by
//! [`SimpleBlock::contents`], with the same result as parsing them at once.
//!
//! Nothing here recurses on the input's nesting, so `((((…` a million deep
//! parses, serializes and drops on any stack.

use std::borrow::Cow;

use crate::tokenizer::{Span, Token, Tokenizer};

/// Which bracket a [`SimpleBlock`] was written with.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bracket {
    /// `( )`
    Paren,
    /// `[ ]`
    Square,
    /// `{ }`
    Curly,
}

impl Bracket {
    pub fn open(self) -> char {
        match self {
            Bracket::Paren => '(',
            Bracket::Square => '[',
            Bracket::Curly => '{',
        }
    }

    pub fn close(self) -> char {
        match self {
            Bracket::Paren => ')',
            Bracket::Square => ']',
            Bracket::Curly => '}',
        }
    }
}

/// A token, a function or a block: what every value, prelude and block in
/// CSS is a list of.
#[derive(Clone, Debug, PartialEq)]
pub enum ComponentValue<'a> {
    /// Any token but the ones that open a function or a block. `)`, `]` and
    /// `}` with nothing to close are kept too, as the spec keeps them.
    Token(Token<'a>, Span),
    Function(Function<'a>),
    Block(SimpleBlock<'a>),
}

impl<'a> ComponentValue<'a> {
    pub fn span(&self) -> Span {
        match self {
            ComponentValue::Token(_, span) => *span,
            ComponentValue::Function(f) => f.span,
            ComponentValue::Block(b) => b.span,
        }
    }

    pub fn token(&self) -> Option<&Token<'a>> {
        match self {
            ComponentValue::Token(t, _) => Some(t),
            _ => None,
        }
    }

    pub fn is_whitespace(&self) -> bool {
        matches!(self, ComponentValue::Token(Token::Whitespace, _))
    }

    /// Whether this is `token`, which is one without a value, or a delim.
    fn is(&self, token: &Token<'_>) -> bool {
        match (self, token) {
            (ComponentValue::Token(Token::Delim(a), _), Token::Delim(b)) => a == b,
            (ComponentValue::Token(t, _), _) => {
                std::mem::discriminant(t) == std::mem::discriminant(token)
            }
            _ => false,
        }
    }

    fn is_curly(&self) -> bool {
        matches!(self, ComponentValue::Block(b) if b.bracket == Bracket::Curly)
    }
}

/// `name( … )`.
#[derive(Clone, Debug, PartialEq)]
pub struct Function<'a> {
    pub name: Cow<'a, str>,
    pub arguments: Vec<ComponentValue<'a>>,
    /// From the name to the `)`, or to the end of the input if it's missing.
    pub span: Span,
}

/// `( … )`, `[ … ]` or `{ … }`.
#[derive(Clone, Debug, PartialEq)]
pub struct SimpleBlock<'a> {
    pub bracket: Bracket,
    pub contents: Vec<ComponentValue<'a>>,
    pub span: Span,
}

/// Dropping a child at a time from a list, rather than letting each list
/// drop its own children, keeps a deep tree off the stack.
fn drop_flat(children: &mut Vec<ComponentValue<'_>>) {
    if children
        .iter()
        .all(|c| matches!(c, ComponentValue::Token(..)))
    {
        return;
    }
    let mut stack = std::mem::take(children);
    while let Some(child) = stack.pop() {
        match child {
            ComponentValue::Function(mut f) => stack.append(&mut f.arguments),
            ComponentValue::Block(mut b) => stack.append(&mut b.contents),
            ComponentValue::Token(..) => {}
        }
    }
}

impl Drop for Function<'_> {
    fn drop(&mut self) {
        drop_flat(&mut self.arguments);
    }
}

impl Drop for SimpleBlock<'_> {
    fn drop(&mut self) {
        drop_flat(&mut self.contents);
    }
}

impl<'a> SimpleBlock<'a> {
    /// Parses a `{}` block's contents as declarations and rules, the way
    /// style rules and nested at-rules hold them. `src` is the text the
    /// block was parsed from.
    pub fn contents(mut self, src: &'a str) -> BlockContents<'a> {
        let values = std::mem::take(&mut self.contents);
        let mut p = Parser::new(src, values.into_iter());
        let items = p.block_contents();
        BlockContents {
            items,
            errors: p.errors,
        }
    }
}

/// `@name prelude;` or `@name prelude { block }`.
#[derive(Clone, Debug, PartialEq)]
pub struct AtRule<'a> {
    pub name: Cow<'a, str>,
    pub prelude: Vec<ComponentValue<'a>>,
    /// The `{}` block, unparsed: what it holds depends on the rule, so
    /// [`SimpleBlock::contents`] parses it when the rule calls for that.
    pub block: Option<SimpleBlock<'a>>,
    pub span: Span,
}

/// `prelude { block }`: a style rule when the prelude is a selector.
#[derive(Clone, Debug, PartialEq)]
pub struct QualifiedRule<'a> {
    pub prelude: Vec<ComponentValue<'a>>,
    /// The `{}` block, unparsed; [`SimpleBlock::contents`] gives its
    /// declarations and nested rules.
    pub block: SimpleBlock<'a>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Rule<'a> {
    At(AtRule<'a>),
    Qualified(QualifiedRule<'a>),
}

impl Rule<'_> {
    pub fn span(&self) -> Span {
        match self {
            Rule::At(r) => r.span,
            Rule::Qualified(r) => r.span,
        }
    }
}

/// `name: value !important`.
#[derive(Clone, Debug, PartialEq)]
pub struct Declaration<'a> {
    pub name: Cow<'a, str>,
    /// Without the whitespace at either end or the `!important`.
    pub value: Vec<ComponentValue<'a>>,
    pub important: bool,
    /// From the name to the end of the value or the `important`, not
    /// including the `;`.
    pub span: Span,
}

impl Declaration<'_> {
    /// The value as it was written, which a custom property keeps:
    /// comments and all, from `src`, the text it was parsed from.
    pub fn original_text<'s>(&self, src: &'s str) -> &'s str {
        match (self.value.first(), self.value.last()) {
            (Some(first), Some(last)) => &src[first.span().start..last.span().end],
            _ => "",
        }
    }
}

/// One thing in a block: style rules and the nesting-era at-rules mix
/// declarations and rules, in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Item<'a> {
    Declaration(Declaration<'a>),
    Rule(Rule<'a>),
}

/// What a parse found wrong, and where.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub kind: ErrorKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    /// Nothing but whitespace and comments where one thing was wanted.
    Empty,
    /// More after the one thing that was wanted.
    ExtraInput,
    /// Something that isn't a declaration where one was wanted, dropped.
    InvalidDeclaration,
    /// A rule that didn't parse (a prelude with no block, say), dropped.
    InvalidRule,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = match self.kind {
            ErrorKind::Empty => "nothing to parse",
            ErrorKind::ExtraInput => "unexpected input after the end",
            ErrorKind::InvalidDeclaration => "invalid declaration",
            ErrorKind::InvalidRule => "invalid rule",
        };
        write!(f, "{what} at bytes {}..{}", self.span.start, self.span.end)
    }
}

impl std::error::Error for ParseError {}

/// A stylesheet's rules, and what was dropped while parsing it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Stylesheet<'a> {
    pub rules: Vec<Rule<'a>>,
    pub errors: Vec<ParseError>,
}

/// A block's declarations and rules, and what was dropped while parsing it.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct BlockContents<'a> {
    pub items: Vec<Item<'a>>,
    pub errors: Vec<ParseError>,
}

/// Component values straight from the tokenizer. A function or block is
/// built whole, with a stack of open ones rather than recursion. The
/// values of every open one share one buffer, so each list is allocated
/// once, at its final size, when it closes.
struct Values<'a> {
    tokens: Tokenizer<'a>,
    open: Vec<Open<'a>>,
    buffer: Vec<ComponentValue<'a>>,
}

impl<'a> Values<'a> {
    fn new(tokens: Tokenizer<'a>) -> Values<'a> {
        Values {
            tokens,
            open: Vec::new(),
            buffer: Vec::new(),
        }
    }
}

struct Open<'a> {
    /// `None` for a function.
    bracket: Option<Bracket>,
    name: Cow<'a, str>,
    start: usize,
    /// Where its values start in the buffer.
    at: usize,
}

/// Moves `buf[at..]` into a list of its own size, by copying the bytes,
/// and leaves `buf` its room for next time.
fn take_tail<T>(buf: &mut Vec<T>, at: usize) -> Vec<T> {
    if at > 0 {
        return buf.split_off(at);
    }
    // `split_off(0)` would hand over the whole buffer, room and all.
    let mut tail = Vec::with_capacity(buf.len());
    tail.append(buf);
    tail
}

/// What a token opens, if anything.
#[inline]
fn opens<'a>(token: Token<'a>, start: usize, at: usize) -> Result<Open<'a>, Token<'a>> {
    let (bracket, name) = match token {
        Token::Function(name) => (None, name),
        Token::LeftParen => (Some(Bracket::Paren), Cow::Borrowed("")),
        Token::LeftBracket => (Some(Bracket::Square), Cow::Borrowed("")),
        Token::LeftBrace => (Some(Bracket::Curly), Cow::Borrowed("")),
        other => return Err(other),
    };
    Ok(Open {
        bracket,
        name,
        start,
        at,
    })
}

impl<'a> Open<'a> {
    fn closes(&self, token: &Token<'_>) -> bool {
        matches!(
            (self.bracket, token),
            (None | Some(Bracket::Paren), Token::RightParen)
                | (Some(Bracket::Square), Token::RightBracket)
                | (Some(Bracket::Curly), Token::RightBrace)
        )
    }

    fn finish(self, values: Vec<ComponentValue<'a>>, end: usize) -> ComponentValue<'a> {
        let span = Span::new(self.start, end);
        match self.bracket {
            None => ComponentValue::Function(Function {
                name: self.name,
                arguments: values,
                span,
            }),
            Some(bracket) => ComponentValue::Block(SimpleBlock {
                bracket,
                contents: values,
                span,
            }),
        }
    }
}

impl<'a> Iterator for Values<'a> {
    type Item = ComponentValue<'a>;

    fn next(&mut self) -> Option<ComponentValue<'a>> {
        let (token, span) = self.tokens.next_token()?;
        match opens(token, span.start, 0) {
            Ok(open) => self.open.push(open),
            Err(token) => return Some(ComponentValue::Token(token, span)),
        }
        loop {
            let Some((token, span)) = self.tokens.next_token() else {
                // Everything still open closes at the end of the input.
                let end = self.tokens.source().len();
                loop {
                    let open = self.open.pop().unwrap();
                    let values = take_tail(&mut self.buffer, open.at);
                    let done = open.finish(values, end);
                    if self.open.is_empty() {
                        return Some(done);
                    }
                    self.buffer.push(done);
                }
            };
            if self.open.last().unwrap().closes(&token) {
                let open = self.open.pop().unwrap();
                let values = take_tail(&mut self.buffer, open.at);
                let done = open.finish(values, span.end);
                if self.open.is_empty() {
                    return Some(done);
                }
                self.buffer.push(done);
                continue;
            }
            if !matches!(
                token,
                Token::Function(_) | Token::LeftParen | Token::LeftBracket | Token::LeftBrace
            ) {
                self.buffer.push(ComponentValue::Token(token, span));
                continue;
            }
            match opens(token, span.start, self.buffer.len()) {
                Ok(open) => self.open.push(open),
                Err(token) => self.buffer.push(ComponentValue::Token(token, span)),
            }
        }
    }
}

/// Where a parser's values come from. A parsed block's own list can be
/// looked along, so a declaration's values move out of it in one go.
trait Source<'a>: Iterator<Item = ComponentValue<'a>> {
    /// What's left, if it can be seen without taking it.
    fn rest(&self) -> Option<&[ComponentValue<'a>]> {
        None
    }
}

impl<'a> Source<'a> for Values<'a> {}

impl<'a> Source<'a> for std::vec::IntoIter<ComponentValue<'a>> {
    fn rest(&self) -> Option<&[ComponentValue<'a>]> {
        Some(self.as_slice())
    }
}

/// How many values come before the next `;` (or, nested, `}`).
fn before_semicolon(values: &[ComponentValue<'_>], nested: bool) -> usize {
    values
        .iter()
        .position(|v| v.is(&Token::Semicolon) || (nested && v.is(&Token::RightBrace)))
        .unwrap_or(values.len())
}

/// A stream of component values with the spec's one item of lookahead,
/// and room to put back what a failed declaration took.
struct Parser<'a, I> {
    src: &'a str,
    input: I,
    back: Vec<ComponentValue<'a>>,
    /// Where a prelude collects, reused from rule to rule.
    scratch: Vec<ComponentValue<'a>>,
    errors: Vec<ParseError>,
}

fn text(src: &str) -> Parser<'_, Values<'_>> {
    Parser::new(src, Values::new(Tokenizer::new(src)))
}

impl<'a, I: Source<'a>> Parser<'a, I> {
    fn new(src: &'a str, input: I) -> Parser<'a, I> {
        Parser {
            src,
            input,
            back: Vec::new(),
            scratch: Vec::new(),
            errors: Vec::new(),
        }
    }

    fn peek(&mut self) -> Option<&ComponentValue<'a>> {
        if self.back.is_empty() {
            if self.input.rest().is_some() {
                return self.input.rest().and_then(<[_]>::first);
            }
            let next = self.input.next()?;
            self.back.push(next);
        }
        self.back.last()
    }

    fn next(&mut self) -> Option<ComponentValue<'a>> {
        self.back.pop().or_else(|| self.input.next())
    }

    fn put_back(&mut self, values: Vec<ComponentValue<'a>>) {
        self.back.extend(values.into_iter().rev());
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(ComponentValue::is_whitespace) {
            self.next();
        }
    }

    fn error(&mut self, kind: ErrorKind, start: usize, end: usize) {
        self.errors.push(ParseError {
            kind,
            span: Span::new(start, end.max(start)),
        });
    }

    /// Consume a stylesheet's contents. With `cdo_cdc` false this is the
    /// 2021 draft's "list of rules", where `<!--` and `-->` aren't skipped.
    fn rules(&mut self, cdo_cdc: bool) -> Vec<Rule<'a>> {
        let mut rules = Vec::new();
        while let Some(next) = self.peek() {
            match next {
                ComponentValue::Token(Token::Whitespace, _) => {
                    self.next();
                }
                ComponentValue::Token(Token::Cdo | Token::Cdc, _) if cdo_cdc => {
                    self.next();
                }
                ComponentValue::Token(Token::AtKeyword(_), _) => {
                    let rule = self.at_rule(false);
                    rules.push(Rule::At(rule));
                }
                _ => {
                    if let Some(rule) = self.qualified_rule(false, false) {
                        rules.push(Rule::Qualified(rule));
                    }
                }
            }
        }
        rules
    }

    /// Consume an at-rule; the next value is its at-keyword. Every at-rule
    /// is valid at this level, so one always comes back.
    fn at_rule(&mut self, nested: bool) -> AtRule<'a> {
        let Some(ComponentValue::Token(Token::AtKeyword(name), span)) = self.next() else {
            unreachable!("at_rule is called on an at-keyword");
        };
        let mut rule = AtRule {
            name,
            prelude: Vec::new(),
            block: None,
            span,
        };
        while let Some(next) = self.next() {
            match next {
                ComponentValue::Token(Token::Semicolon, _) => break,
                ComponentValue::Token(Token::RightBrace, _) if nested => {
                    self.back.push(next);
                    break;
                }
                ComponentValue::Block(block) if block.bracket == Bracket::Curly => {
                    rule.span.end = block.span.end;
                    rule.block = Some(block);
                    break;
                }
                other => {
                    rule.span.end = other.span().end;
                    self.scratch.push(other);
                }
            }
        }
        rule.prelude = take_tail(&mut self.scratch, 0);
        rule
    }

    /// Consume a qualified rule. `stop` is the nested form's `;`, which
    /// ends a rule that's really a broken declaration.
    fn qualified_rule(&mut self, stop: bool, nested: bool) -> Option<QualifiedRule<'a>> {
        let end_of_input = self.src.len();
        let start = self.peek().map_or(end_of_input, |v| v.span().start);
        // The prelude collects in `scratch`, to be allocated once at its size.
        self.scratch.clear();
        let mut end = start;
        loop {
            let Some(next) = self.next() else {
                self.scratch.clear();
                self.error(ErrorKind::InvalidRule, start, end);
                return None;
            };
            match next {
                ComponentValue::Token(Token::Semicolon, _) if stop => {
                    self.back.push(next);
                    self.scratch.clear();
                    self.error(ErrorKind::InvalidRule, start, end);
                    return None;
                }
                ComponentValue::Token(Token::RightBrace, _) if nested => {
                    self.back.push(next);
                    self.scratch.clear();
                    self.error(ErrorKind::InvalidRule, start, end);
                    return None;
                }
                ComponentValue::Block(block) if block.bracket == Bracket::Curly => {
                    let end = block.span.end;
                    let prelude = take_tail(&mut self.scratch, 0);
                    if looks_like_custom_property(&prelude) {
                        // `--x:hover {…}` is a custom property gone wrong,
                        // never a rule; nested, the property runs to its `;`.
                        if nested {
                            self.bad_declaration_remnants(true);
                        }
                        self.error(ErrorKind::InvalidRule, start, end);
                        return None;
                    }
                    return Some(QualifiedRule {
                        prelude,
                        block,
                        span: Span::new(start, end),
                    });
                }
                other => {
                    end = other.span().end;
                    self.scratch.push(other);
                }
            }
        }
    }

    fn bad_declaration_remnants(&mut self, nested: bool) {
        while let Some(next) = self.next() {
            match next {
                ComponentValue::Token(Token::Semicolon, _) => return,
                ComponentValue::Token(Token::RightBrace, _) if nested => {
                    self.back.push(next);
                    return;
                }
                _ => {}
            }
        }
    }

    /// Everything up to the next `;` (or the `}` ending a nested block), the
    /// most a declaration can be.
    fn up_to_semicolon(&mut self, nested: bool) -> Vec<ComponentValue<'a>> {
        let mut values = Vec::new();
        while let Some(next) = self.peek() {
            if next.is(&Token::Semicolon) || (nested && next.is(&Token::RightBrace)) {
                break;
            }
            values.push(self.next().unwrap());
        }
        values
    }

    /// Consume a declaration: takes what one could be, and gives it back if
    /// it isn't one, for the caller to try as a rule.
    fn declaration(&mut self, nested: bool) -> Result<Declaration<'a>, Vec<ComponentValue<'a>>> {
        // From a parsed block, look first and move only what's kept.
        if self.back.is_empty()
            && let Some(rest) = self.input.rest()
        {
            let n = before_semicolon(rest, nested);
            if let Some(plan) = plan(&rest[..n]) {
                let name = self.input.next().unwrap();
                let value = self
                    .input
                    .by_ref()
                    .skip(plan.value_start - 1)
                    .take(plan.value_end - plan.value_start)
                    .collect();
                // Past the `!important` and trailing whitespace.
                for _ in plan.value_end.max(plan.value_start)..n {
                    self.input.next();
                }
                return Ok(build(name, value, &plan, self.src));
            }
            return Err(self.input.by_ref().take(n).collect());
        }
        let values = self.up_to_semicolon(nested);
        declaration_from(values, self.src)
    }

    /// Consume a block's contents.
    fn block_contents(&mut self) -> Vec<Item<'a>> {
        let mut items = Vec::new();
        while let Some(next) = self.peek() {
            match next {
                ComponentValue::Token(Token::Whitespace | Token::Semicolon, _) => {
                    self.next();
                }
                // Only parsed text can have a `}` here, a block's own never
                // does: it's dropped, as a stray one is at the top level.
                ComponentValue::Token(Token::RightBrace, span) => {
                    let span = *span;
                    self.next();
                    self.error(ErrorKind::InvalidRule, span.start, span.end);
                }
                ComponentValue::Token(Token::AtKeyword(_), _) => {
                    let rule = self.at_rule(true);
                    items.push(Item::Rule(Rule::At(rule)));
                }
                // Only an ident can start a declaration.
                ComponentValue::Token(Token::Ident(_), _) => match self.declaration(true) {
                    Ok(decl) => items.push(Item::Declaration(decl)),
                    Err(values) => {
                        self.put_back(values);
                        if let Some(rule) = self.qualified_rule(true, true) {
                            items.push(Item::Rule(Rule::Qualified(rule)));
                        }
                    }
                },
                _ => {
                    if let Some(rule) = self.qualified_rule(true, true) {
                        items.push(Item::Rule(Rule::Qualified(rule)));
                    }
                }
            }
        }
        items
    }

    /// Parse a list of declarations, keeping the at-rules the spec has the
    /// caller drop, in case the caller wants them.
    fn declaration_list(&mut self) -> Vec<Item<'a>> {
        let mut items = Vec::new();
        while let Some(next) = self.peek() {
            match next {
                ComponentValue::Token(Token::Whitespace | Token::Semicolon, _) => {
                    self.next();
                }
                ComponentValue::Token(Token::AtKeyword(_), _) => {
                    let rule = self.at_rule(false);
                    items.push(Item::Rule(Rule::At(rule)));
                }
                _ => match self.declaration(false) {
                    Ok(decl) => items.push(Item::Declaration(decl)),
                    Err(values) => {
                        let start = values.first().map_or(0, |v| v.span().start);
                        let end = values.last().map_or(start, |v| v.span().end);
                        self.error(ErrorKind::InvalidDeclaration, start, end);
                    }
                },
            }
        }
        items
    }

    /// Fails with ExtraInput unless only whitespace is left.
    fn end<T>(&mut self, value: T) -> Result<T, ParseError> {
        self.skip_whitespace();
        match self.peek() {
            None => Ok(value),
            Some(v) => {
                let start = v.span().start;
                Err(ParseError {
                    kind: ErrorKind::ExtraInput,
                    span: Span::new(start, self.src.len()),
                })
            }
        }
    }

    fn start(&mut self) -> Result<(), ParseError> {
        self.skip_whitespace();
        match self.peek() {
            Some(_) => Ok(()),
            None => Err(ParseError {
                kind: ErrorKind::Empty,
                span: Span::new(0, self.src.len()),
            }),
        }
    }
}

/// Whether a prelude starts like a custom property: `--name` then `:`.
fn looks_like_custom_property(prelude: &[ComponentValue<'_>]) -> bool {
    let mut it = prelude.iter().filter(|v| !v.is_whitespace());
    matches!(it.next(), Some(ComponentValue::Token(Token::Ident(n), _)) if n.starts_with("--"))
        && it.next().is_some_and(|v| v.is(&Token::Colon))
}

/// Where a declaration's parts are in the values `up_to_semicolon` took:
/// worked out before anything moves, so a failed one costs no moves.
struct Plan {
    value_start: usize,
    value_end: usize,
    important: bool,
    unicode_range: bool,
    span: Span,
}

/// The plan for `values` as a declaration, or `None` if they aren't one.
fn plan(values: &[ComponentValue<'_>]) -> Option<Plan> {
    let skip_ws = |mut i: usize| {
        while values.get(i).is_some_and(ComponentValue::is_whitespace) {
            i += 1;
        }
        i
    };
    let Some(ComponentValue::Token(Token::Ident(name), first)) = values.first() else {
        return None;
    };
    let colon = skip_ws(1);
    if !values.get(colon).is_some_and(|v| v.is(&Token::Colon)) {
        return None;
    }
    let value_start = skip_ws(colon + 1);
    let span_end = values
        .iter()
        .rev()
        .find(|v| !v.is_whitespace())
        .map_or(first.end, |v| v.span().end);

    // The last two non-whitespace values: `!` and `important`?
    let mut solid = (value_start..values.len())
        .rev()
        .filter(|&i| !values[i].is_whitespace());
    let last = solid.next();
    let before = solid.next();
    let important = match (before, last) {
        (Some(bang), Some(word))
            if values[bang].is(&Token::Delim('!'))
                && matches!(&values[word], ComponentValue::Token(Token::Ident(w), _)
                    if w.eq_ignore_ascii_case("important")) =>
        {
            Some(bang)
        }
        _ => None,
    };
    let mut value_end = important.unwrap_or(values.len());
    while value_end > value_start && values[value_end - 1].is_whitespace() {
        value_end -= 1;
    }
    if !name.starts_with("--") {
        // A `{}` block is allowed only as the whole of a property's value.
        let value = &values[value_start..value_end];
        if value.len() > 1 && value.iter().any(ComponentValue::is_curly) {
            return None;
        }
    }
    Some(Plan {
        value_start,
        value_end,
        important: important.is_some(),
        unicode_range: name.eq_ignore_ascii_case("unicode-range"),
        span: Span::new(first.start, span_end),
    })
}

/// Builds the planned declaration from its name and value.
fn build<'a>(
    name: ComponentValue<'a>,
    mut value: Vec<ComponentValue<'a>>,
    plan: &Plan,
    src: &'a str,
) -> Declaration<'a> {
    let ComponentValue::Token(Token::Ident(name), _) = name else {
        unreachable!("a plan starts with an ident")
    };
    if plan.unicode_range && !value.is_empty() {
        // The one place the spec retokenizes: `U+0-7F` is a range here.
        let from = value[0].span().start;
        let to = value[value.len() - 1].span().end;
        value = Values::new(unicode_range_tokens(src, from, to)).collect();
    }
    Declaration {
        name,
        value,
        important: plan.important,
        span: plan.span,
    }
}

/// Builds a declaration from what `up_to_semicolon` took, or hands it back.
fn declaration_from<'a>(
    mut values: Vec<ComponentValue<'a>>,
    src: &'a str,
) -> Result<Declaration<'a>, Vec<ComponentValue<'a>>> {
    let Some(plan) = plan(&values) else {
        return Err(values);
    };
    let blank = ComponentValue::Token(Token::Whitespace, Span::default());
    let name = std::mem::replace(&mut values[0], blank);
    // The value moves down in place: no new list.
    values.truncate(plan.value_end);
    values.drain(..plan.value_start);
    Ok(build(name, values, &plan, src))
}

/// A tokenizer over `src[from..to]` making unicode-range tokens, with
/// spans that still point into `src`.
fn unicode_range_tokens(src: &str, from: usize, to: usize) -> Tokenizer<'_> {
    let mut t = Tokenizer::with_unicode_ranges(&src[..to]);
    t.reset(from);
    t
}

/// Parse a stylesheet: its rules, with `<!--` and `-->` skipped between
/// them as the HTML `<style>` element's history needs.
///
/// ```
/// let sheet = parsec_css::parse_stylesheet("a { color: red } @media print { b {} }");
/// assert_eq!(sheet.rules.len(), 2);
/// assert!(sheet.errors.is_empty());
/// ```
pub fn parse_stylesheet(css: &str) -> Stylesheet<'_> {
    let mut p = text(css);
    let rules = p.rules(true);
    Stylesheet {
        rules,
        errors: p.errors,
    }
}

/// Parse a list of rules as the 2021 Candidate Recommendation did, where
/// `<!--` and `-->` are ordinary tokens. The current draft's "parse a
/// stylesheet's contents" is [`parse_stylesheet`].
pub fn parse_rule_list(css: &str) -> Stylesheet<'_> {
    let mut p = text(css);
    let rules = p.rules(false);
    Stylesheet {
        rules,
        errors: p.errors,
    }
}

/// Parse a block's contents: declarations, at-rules and nested style rules,
/// in order, the way a style rule's `{}` block holds them since CSS
/// Nesting. [`SimpleBlock::contents`] does the same for a parsed block.
///
/// ```
/// use parsec_css::Item;
///
/// let block = parsec_css::parse_block_contents("color: red; &:hover { color: blue }");
/// assert!(matches!(&block.items[0], Item::Declaration(d) if d.name == "color"));
/// assert!(matches!(&block.items[1], Item::Rule(_)));
/// ```
pub fn parse_block_contents(css: &str) -> BlockContents<'_> {
    let mut p = text(css);
    let items = p.block_contents();
    BlockContents {
        items,
        errors: p.errors,
    }
}

/// Parse a list of declarations, as for a `style` attribute. The spec has
/// at-rules here parsed and thrown away; they're kept, as
/// [`Item::Rule`]s, for you to throw away. Nothing else becomes a rule.
pub fn parse_declaration_list(css: &str) -> BlockContents<'_> {
    let mut p = text(css);
    let items = p.declaration_list();
    BlockContents {
        items,
        errors: p.errors,
    }
}

/// Parse a rule: exactly one, with only whitespace around it.
pub fn parse_rule(css: &str) -> Result<Rule<'_>, ParseError> {
    let mut p = text(css);
    p.start()?;
    let rule = if matches!(
        p.peek(),
        Some(ComponentValue::Token(Token::AtKeyword(_), _))
    ) {
        Rule::At(p.at_rule(false))
    } else {
        match p.qualified_rule(false, false) {
            Some(rule) => Rule::Qualified(rule),
            None => return Err(p.errors.pop().unwrap()),
        }
    };
    p.end(rule)
}

/// Parse a declaration, as `@supports (…)` does: exactly one.
pub fn parse_declaration(css: &str) -> Result<Declaration<'_>, ParseError> {
    let mut p = text(css);
    p.start()?;
    match p.declaration(false) {
        Ok(decl) => p.end(decl),
        Err(values) => {
            let start = values.first().map_or(0, |v| v.span().start);
            Err(ParseError {
                kind: ErrorKind::InvalidDeclaration,
                span: Span::new(start, css.len()),
            })
        }
    }
}

/// Parse a component value: exactly one, with only whitespace around it.
pub fn parse_component_value(css: &str) -> Result<ComponentValue<'_>, ParseError> {
    let mut p = text(css);
    p.start()?;
    let value = p.next().unwrap();
    p.end(value)
}

/// Parse a list of component values: the whole input, as a selector or a
/// media query list is read.
pub fn parse_component_values(css: &str) -> Vec<ComponentValue<'_>> {
    Values::new(Tokenizer::new(css)).collect()
}

/// Parse a comma-separated list of component values, split at the commas
/// outside any block or function.
pub fn parse_comma_separated(css: &str) -> Vec<Vec<ComponentValue<'_>>> {
    let mut values = Values::new(Tokenizer::new(css)).peekable();
    let mut groups = Vec::new();
    while values.peek().is_some() {
        let mut group = Vec::new();
        for v in values.by_ref() {
            if v.is(&Token::Comma) {
                break;
            }
            group.push(v);
        }
        groups.push(group);
    }
    groups
}

/// The value of a `unicode-range` descriptor, tokenized with unicode-range
/// tokens: the one place CSS makes them.
pub fn parse_unicode_range_value(css: &str) -> Vec<ComponentValue<'_>> {
    Values::new(Tokenizer::with_unicode_ranges(css)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_nesting_parses_and_drops() {
        let css = "(".repeat(1_000_000);
        let values = parse_component_values(&css);
        assert_eq!(values.len(), 1);
        drop(values);
        let css = format!("a{{b:{}}}", "[".repeat(200_000));
        let sheet = parse_stylesheet(&css);
        let Rule::Qualified(rule) = sheet.rules.into_iter().next().unwrap() else {
            panic!()
        };
        assert_eq!(rule.block.contents(&css).items.len(), 1);
    }

    #[test]
    fn spans_cover_rules_and_declarations() {
        let css = "a { color: red !important; } @x y;";
        let sheet = parse_stylesheet(css);
        assert_eq!(
            sheet.rules[0].span().text(css),
            "a { color: red !important; }"
        );
        assert_eq!(
            sheet.rules[1].span().text(css),
            "@x y;".trim_end_matches(';')
        );
        let Rule::Qualified(rule) = sheet.rules.into_iter().next().unwrap() else {
            panic!()
        };
        let Item::Declaration(d) = &rule.block.contents(css).items[0] else {
            panic!()
        };
        assert_eq!(d.span.text(css), "color: red !important");
        assert_eq!(d.original_text(css), "red");
        assert!(d.important);
    }

    #[test]
    fn block_contents_parsed_later_match_parsing_at_once() {
        let css = "color: red; &:hover { b: c } --x: {a} b; d:e{f}; @media x { g: h }";
        let at_once = parse_block_contents(css);
        let wrapped = format!("{{{css}}}");
        let Ok(ComponentValue::Block(block)) = parse_component_value(&wrapped) else {
            panic!()
        };
        let later = block.contents(&wrapped);
        assert_eq!(at_once.items.len(), later.items.len());
        assert_eq!(at_once.errors.len(), later.errors.len());
    }

    #[test]
    fn unicode_range_descriptor_gets_range_tokens() {
        let d = parse_declaration("unicode-range: U+0-7F, u+4??").unwrap();
        let ranges: Vec<_> = d
            .value
            .iter()
            .filter_map(|v| match v.token() {
                Some(Token::UnicodeRange { start, end }) => Some((*start, *end)),
                _ => None,
            })
            .collect();
        assert_eq!(ranges, [(0, 0x7f), (0x400, 0x4ff)]);
    }
}
