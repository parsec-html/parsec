// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! A whole stylesheet at once: each `{}` block's contents parsed straight
//! from the tokenizer as the parser reaches it, rather than kept as
//! component values and walked a second time. That's how the current
//! draft's "consume an at-rule" and "consume a qualified rule" read, with
//! every block parsed as declarations and rules.
//!
//! A declaration is tried first wherever an ident starts one, as the spec
//! says. When it isn't one (`a:hover { … }` nested is a rule), the
//! tokenizer goes back to the ident and the text is read again as a rule:
//! the spec's "put back", with a position instead of a list.
//!
//! The open blocks are a stack, as in [`Values`], so nesting doesn't
//! recurse, and they count against the same [`MAX_NESTING`](crate::MAX_NESTING) as the values
//! in them; their items share one buffer, so each block's list is
//! allocated once, at its size, when it closes.

use std::borrow::Cow;

use crate::parser::{
    AtRule, Block, ComponentValue, Declaration, ErrorKind, Item, ParseError, QualifiedRule, Rule,
    Seen, Stylesheet, Values, Watch, looks_like_custom_property, take_tail, unicode_range_tokens,
    value_plan,
};
use crate::tokenizer::{Span, Token, Tokenizer};

/// A rule whose block is open: what goes before the block.
enum Head<'a> {
    At {
        name: Cow<'a, str>,
        prelude: Vec<ComponentValue<'a>>,
        start: usize,
    },
    Qualified {
        prelude: Vec<ComponentValue<'a>>,
        start: usize,
    },
}

struct Open<'a> {
    head: Head<'a>,
    /// Where the `{` is.
    start: usize,
    /// Where its items start in the shared buffer.
    at: usize,
}

struct Deep<'a> {
    src: &'a str,
    values: Values<'a>,
    open: Vec<Open<'a>>,
    items: Vec<Item<'a, Block<'a>>>,
    rules: Vec<Rule<'a, Block<'a>>>,
    /// Where a prelude or a value collects, reused from one to the next.
    scratch: Vec<ComponentValue<'a>>,
    errors: Vec<ParseError>,
}

impl<'a> Deep<'a> {
    fn next_token(&mut self) -> Option<(Token<'a>, Span)> {
        self.values.tokens.next_token()
    }

    /// Goes back to `position`, to read what's there again.
    fn back_to(&mut self, position: usize) {
        self.values.tokens.reset(position);
    }

    fn error(&mut self, kind: ErrorKind, start: usize, end: usize) {
        self.errors.push(ParseError {
            kind,
            span: Span::new(start, end.max(start)),
        });
    }

    fn place(&mut self, rule: Rule<'a, Block<'a>>) {
        if self.open.is_empty() {
            self.rules.push(rule);
        } else {
            self.items.push(Item::Rule(rule));
        }
    }

    fn run(mut self) -> Stylesheet<'a, Block<'a>> {
        while let Some((token, span)) = self.next_token() {
            if self.open.is_empty() {
                // A stylesheet's contents, with `<!--` and `-->` skipped.
                match token {
                    Token::Whitespace | Token::Cdo | Token::Cdc => {}
                    Token::AtKeyword(name) => self.at_rule(name, span, false),
                    token => self.qualified_rule(token, span, false),
                }
                continue;
            }
            // A block's contents.
            match token {
                Token::Whitespace | Token::Semicolon => {}
                Token::RightBrace => self.close(span.end),
                Token::AtKeyword(name) => self.at_rule(name, span, true),
                Token::Ident(name) => {
                    if !self.declaration(name, span) {
                        self.back_to(span.start);
                        let (token, span) = self.next_token().unwrap();
                        self.qualified_rule(token, span, true);
                    }
                }
                token => self.qualified_rule(token, span, true),
            }
        }
        // Everything still open closes at the end of the input.
        while !self.open.is_empty() {
            self.close(self.src.len());
        }
        Stylesheet {
            rules: self.rules,
            errors: self.errors,
        }
    }

    /// The innermost open block ends at `end`, and its rule with it.
    fn close(&mut self, end: usize) {
        let open = self.open.pop().unwrap();
        self.values.limit += 1;
        let block = Block {
            items: take_tail(&mut self.items, open.at),
            span: Span::new(open.start, end),
        };
        let rule = match open.head {
            Head::At {
                name,
                prelude,
                start,
            } => Rule::At(AtRule {
                name,
                prelude,
                block: Some(block),
                span: Span::new(start, end),
            }),
            Head::Qualified { prelude, start } => Rule::Qualified(QualifiedRule {
                prelude,
                block,
                span: Span::new(start, end),
            }),
        };
        self.place(rule);
    }

    fn open_block(&mut self, head: Head<'a>, brace: Span) {
        if self.values.limit <= 1 {
            // Too deep: the rule goes, block and all.
            let start = match head {
                Head::At { start, .. } | Head::Qualified { start, .. } => start,
            };
            let end = self.values.whole(Token::LeftBrace, brace).span().end;
            return self.error(ErrorKind::InvalidRule, start, end);
        }
        self.values.limit -= 1;
        self.open.push(Open {
            head,
            start: brace.start,
            at: self.items.len(),
        });
    }

    /// An at-rule, after its at-keyword: to a `;`, or a block it opens.
    fn at_rule(&mut self, name: Cow<'a, str>, keyword: Span, nested: bool) {
        let mut end = keyword.end;
        self.scratch.clear();
        while let Some((token, span)) = self.next_token() {
            match token {
                Token::Semicolon => break,
                Token::RightBrace if nested => {
                    self.back_to(span.start);
                    break;
                }
                Token::LeftBrace => {
                    let prelude = take_tail(&mut self.scratch, 0);
                    let head = Head::At {
                        name,
                        prelude,
                        start: keyword.start,
                    };
                    return self.open_block(head, span);
                }
                token => {
                    let value = self.values.whole(token, span);
                    end = value.span().end;
                    self.scratch.push(value);
                }
            }
        }
        let rule = Rule::At(AtRule {
            name,
            prelude: take_tail(&mut self.scratch, 0),
            block: None,
            span: Span::new(keyword.start, end),
        });
        self.place(rule);
    }

    /// A qualified rule from its first token. Nested, a `;` ends it as a
    /// broken declaration, and a `}` ends it with the block it's in.
    fn qualified_rule(&mut self, token: Token<'a>, span: Span, nested: bool) {
        let start = span.start;
        let mut end = start;
        self.scratch.clear();
        let mut next = Some((token, span));
        loop {
            let Some((token, span)) = next.take().or_else(|| self.next_token()) else {
                self.scratch.clear();
                return self.error(ErrorKind::InvalidRule, start, end);
            };
            match token {
                Token::Semicolon if nested => {
                    self.scratch.clear();
                    return self.error(ErrorKind::InvalidRule, start, end);
                }
                Token::RightBrace if nested => {
                    self.back_to(span.start);
                    self.scratch.clear();
                    return self.error(ErrorKind::InvalidRule, start, end);
                }
                Token::LeftBrace => {
                    let prelude = take_tail(&mut self.scratch, 0);
                    if looks_like_custom_property(&prelude) {
                        // `--x:hover {…}` is a custom property gone wrong,
                        // never a rule; nested, the property runs to its `;`.
                        let end = self.values.whole(Token::LeftBrace, span).span().end;
                        if nested {
                            self.bad_declaration_remnants();
                        }
                        return self.error(ErrorKind::InvalidRule, start, end);
                    }
                    return self.open_block(Head::Qualified { prelude, start }, span);
                }
                token => {
                    let value = self.values.whole(token, span);
                    end = value.span().end;
                    self.scratch.push(value);
                }
            }
        }
    }

    /// Past the next `;`, or up to the `}` that ends the block.
    fn bad_declaration_remnants(&mut self) {
        while let Some((token, span)) = self.next_token() {
            match token {
                Token::Semicolon => return,
                Token::RightBrace => return self.back_to(span.start),
                token => drop(self.values.whole(token, span)),
            }
        }
    }

    /// A declaration in a block, after the ident that may be its name.
    /// False if it isn't one, with the tokenizer anywhere.
    fn declaration(&mut self, name: Cow<'a, str>, ident: Span) -> bool {
        let colon_end = loop {
            match self.next_token() {
                Some((Token::Whitespace, _)) => {}
                Some((Token::Colon, span)) => break span.end,
                _ => return false,
            }
        };
        self.scratch.clear();
        let mut watch = match name.starts_with("--") {
            true => Watch::Custom,
            false => Watch::Value(Seen::Nothing),
        };
        while let Some((token, span)) = self.next_token() {
            match token {
                Token::Semicolon => break,
                Token::RightBrace => {
                    self.back_to(span.start);
                    break;
                }
                token => {
                    let value = self.values.whole(token, span);
                    if !watch.see(&value) {
                        self.scratch.clear();
                        return false;
                    }
                    self.scratch.push(value);
                }
            }
        }
        let Some(plan) = value_plan(&name, &self.scratch) else {
            self.scratch.clear();
            return false;
        };
        let mut value: Vec<_> = self.scratch.drain(plan.start..plan.end).collect();
        self.scratch.clear();
        if name.eq_ignore_ascii_case("unicode-range") && !value.is_empty() {
            // The one place the spec retokenizes: `U+0-7F` is a range here.
            let from = value[0].span().start;
            let to = value[value.len() - 1].span().end;
            value = Values::new(unicode_range_tokens(self.src, from, to)).collect();
        }
        self.items.push(Item::Declaration(Declaration {
            name,
            value,
            important: plan.important,
            span: Span::new(ident.start, plan.last_end.unwrap_or(colon_end)),
        }));
        true
    }
}

/// Parse a stylesheet with every rule's `{}` block parsed too, as
/// declarations and nested rules, down to
/// [`MAX_NESTING`](crate::MAX_NESTING): what
/// [`parse_stylesheet`](crate::parse_stylesheet) and
/// [`SimpleBlock::contents`](crate::SimpleBlock::contents) on each block
/// make, in one pass and
/// about 70% of the time. The errors are all in the
/// stylesheet's list, in the order they were found; a rule dropped for
/// opening deeper than that is an [`ErrorKind::InvalidRule`].
///
/// ```
/// use parsec_css::{Item, Rule};
///
/// let sheet = parsec_css::parse_stylesheet_deep("a { color: red; &:hover { color: blue } }");
/// let Rule::Qualified(rule) = &sheet.rules[0] else { unreachable!() };
/// assert!(matches!(&rule.block.items[0], Item::Declaration(d) if d.name == "color"));
/// let Item::Rule(Rule::Qualified(hover)) = &rule.block.items[1] else { unreachable!() };
/// assert_eq!(parsec_css::to_css(&hover.prelude), "&:hover ");
/// ```
pub fn parse_stylesheet_deep(css: &str) -> Stylesheet<'_, Block<'_>> {
    Deep {
        src: css,
        values: Values::new(Tokenizer::new(css)),
        open: Vec::new(),
        items: Vec::new(),
        rules: Vec::new(),
        scratch: Vec::new(),
        errors: Vec::new(),
    }
    .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deep_nesting_parses_writes_and_drops() {
        let css = format!("{}color: red", "a{".repeat(300_000));
        let sheet = parse_stylesheet_deep(&css);
        assert_eq!(sheet.rules.len(), 1);
        // Rules past MAX_NESTING go, with what's in them.
        let mut depth = 0;
        let mut rule = &sheet.rules[0];
        while let Rule::Qualified(q) = rule {
            depth += 1;
            match q.block.items.first() {
                Some(Item::Rule(r)) => rule = r,
                _ => break,
            }
        }
        assert_eq!(depth, crate::MAX_NESTING - 1);
        assert_eq!(sheet.errors.len(), 1);
        let written = sheet.to_string();
        assert!(written.ends_with(&"}".repeat(crate::MAX_NESTING - 1)));
        assert!(!written.contains("color"));
        // Kept this shallow, a derived clone or comparison recurses safely.
        assert!(sheet.clone() == sheet);
        drop(sheet);
    }

    #[test]
    fn an_ident_that_starts_a_rule_is_read_again_as_one() {
        let css = "a { b:hover { c: d } e: f; --x:y { g } h }";
        let sheet = parse_stylesheet_deep(css);
        let Rule::Qualified(a) = &sheet.rules[0] else {
            panic!()
        };
        let kinds: Vec<_> = a
            .block
            .items
            .iter()
            .map(|i| match i {
                Item::Declaration(d) => d.span.text(css),
                Item::Rule(r) => r.span().text(css),
            })
            .collect();
        assert_eq!(kinds, ["b:hover { c: d }", "e: f", "--x:y { g } h"]);
    }
}
