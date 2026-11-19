// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Back to CSS text. The spec asks only that parsing the output gives the
//! same values again, runs of whitespace aside; this writes each token in
//! its plainest form and puts `/**/` between two tokens wherever the
//! spec's table says they'd otherwise run together.

use std::fmt::{self, Write};

use crate::parser::{
    AtRule, BlockContents, ComponentValue, Declaration, Item, QualifiedRule, Rule, SimpleBlock,
    Stylesheet,
};
use crate::tokenizer::{Token, is_non_ascii_ident};

/// What the last token written was, as far as the comment table cares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Last {
    Other,
    Ident,
    /// The ident `--`, which before `>` would read back as `-->`.
    Dashes,
    AtKeyword,
    Hash,
    Dimension,
    Number,
    Delim(char),
}

/// What the next token is, as the table's columns see it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Next {
    Other,
    /// Ident, function, url or bad url: they start alike.
    IdentLike,
    Number,
    Cdc,
    OpenParen,
    Delim(char),
}

fn needs_comment(last: Last, next: Next) -> bool {
    use Next::{Cdc, IdentLike, Number as Num, OpenParen};
    match last {
        Last::Ident => matches!(next, IdentLike | Num | Cdc | OpenParen | Next::Delim('-')),
        // Not in the table either.
        Last::Dashes => needs_comment(Last::Ident, next) || next == Next::Delim('>'),
        Last::AtKeyword | Last::Hash | Last::Dimension | Last::Delim('#' | '-') => {
            matches!(next, IdentLike | Num | Cdc | Next::Delim('-'))
        }
        Last::Number => matches!(next, IdentLike | Num | Cdc | Next::Delim('%')),
        Last::Delim('@') => matches!(next, IdentLike | Cdc | Next::Delim('-')),
        Last::Delim('.' | '+') => next == Num,
        Last::Delim('/') => next == Next::Delim('*'),
        // Not in the table, but `<` `!` `--` would read back as `<!--`.
        Last::Delim('<') => next == Next::Delim('!'),
        _ => false,
    }
}

/// Writes tokens, remembering the last one.
struct Writer<'w, W> {
    out: &'w mut W,
    last: Last,
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_') || is_non_ascii_ident(c)
}

fn hex_escape(out: &mut impl Write, c: char) -> fmt::Result {
    write!(out, "\\{:x} ", c as u32)
}

/// CSSOM's "serialize an identifier", except that a non-ASCII code point
/// outside the ident set is escaped, as it must be since CSS Syntax
/// narrowed that set in 2024.
pub fn write_identifier(out: &mut impl Write, ident: &str) -> fmt::Result {
    if ident == "-" {
        return out.write_str("\\-");
    }
    for (i, c) in ident.chars().enumerate() {
        let digit_first = c.is_ascii_digit() && (i == 0 || (i == 1 && ident.starts_with('-')));
        match c {
            '\0' => out.write_char('\u{fffd}')?,
            '\x01'..='\x1f' | '\x7f' => hex_escape(out, c)?,
            _ if digit_first => hex_escape(out, c)?,
            _ if is_name_char(c) => out.write_char(c)?,
            _ => {
                out.write_char('\\')?;
                out.write_char(c)?;
            }
        }
    }
    Ok(())
}

/// Like [`write_identifier`] but for what follows `#` in a hash that isn't
/// an identifier, where a leading digit or `-` needs no escape.
fn write_name(out: &mut impl Write, name: &str) -> fmt::Result {
    for c in name.chars() {
        match c {
            '\0' => out.write_char('\u{fffd}')?,
            '\x01'..='\x1f' | '\x7f' => hex_escape(out, c)?,
            _ if is_name_char(c) => out.write_char(c)?,
            _ => {
                out.write_char('\\')?;
                out.write_char(c)?;
            }
        }
    }
    Ok(())
}

/// CSSOM's "serialize a string", in double quotes.
pub fn write_string(out: &mut impl Write, s: &str) -> fmt::Result {
    out.write_char('"')?;
    for c in s.chars() {
        match c {
            '\0' => out.write_char('\u{fffd}')?,
            '\x01'..='\x1f' | '\x7f' => hex_escape(out, c)?,
            '"' | '\\' => {
                out.write_char('\\')?;
                out.write_char(c)?;
            }
            _ => out.write_char(c)?,
        }
    }
    out.write_char('"')
}

/// An unquoted `url(…)`'s contents, escaped where the tokenizer would
/// otherwise stop or go bad.
fn write_url(out: &mut impl Write, s: &str) -> fmt::Result {
    for c in s.chars() {
        match c {
            '\0' => out.write_char('\u{fffd}')?,
            '\x01'..=' ' | '\x7f' | '(' | ')' | '"' | '\'' => hex_escape(out, c)?,
            '\\' => out.write_str("\\\\")?,
            _ => out.write_char(c)?,
        }
    }
    Ok(())
}

/// A dimension's unit, escaping an `e` that would read as an exponent.
fn write_unit(out: &mut impl Write, unit: &str) -> fmt::Result {
    let mut chars = unit.chars();
    let exponent = matches!(chars.next(), Some('e' | 'E'))
        && match chars.next() {
            Some('0'..='9') => true,
            Some('+' | '-') => chars.next().is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        };
    if exponent {
        hex_escape(out, unit.chars().next().unwrap())?;
        return write_name(out, &unit[1..]);
    }
    write_identifier(out, unit)
}

impl<W: Write> Writer<'_, W> {
    fn start(&mut self, next: Next) -> fmt::Result {
        if needs_comment(self.last, next) {
            self.out.write_str("/**/")?;
        }
        Ok(())
    }

    fn token(&mut self, token: &Token<'_>) -> fmt::Result {
        let next = match token {
            Token::Ident(_) | Token::Function(_) | Token::Url(_) | Token::BadUrl => Next::IdentLike,
            Token::Number(_) | Token::Percentage(_) | Token::Dimension { .. } => Next::Number,
            Token::Cdc => Next::Cdc,
            Token::LeftParen => Next::OpenParen,
            Token::Delim(c) => Next::Delim(*c),
            _ => Next::Other,
        };
        self.start(next)?;
        let out = &mut *self.out;
        self.last = Last::Other;
        match token {
            Token::Ident(v) => {
                write_identifier(out, v)?;
                self.last = if v == "--" { Last::Dashes } else { Last::Ident };
            }
            Token::Function(v) => {
                write_identifier(out, v)?;
                out.write_char('(')?;
            }
            Token::AtKeyword(v) => {
                out.write_char('@')?;
                write_identifier(out, v)?;
                self.last = Last::AtKeyword;
            }
            Token::Hash { value, id } => {
                out.write_char('#')?;
                if *id {
                    write_identifier(out, value)?;
                } else {
                    write_name(out, value)?;
                }
                self.last = Last::Hash;
            }
            Token::String(v) => write_string(out, v)?,
            // What makes these bad is gone, so they're rebuilt from the
            // smallest text that is bad the same way.
            Token::BadString => out.write_str("\"\n")?,
            Token::Url(v) => {
                out.write_str("url(")?;
                write_url(out, v)?;
                out.write_char(')')?;
            }
            Token::BadUrl => out.write_str("url(()")?,
            Token::Delim(c) => {
                // A lone backslash reads back as one only before a newline.
                if *c == '\\' {
                    out.write_str("\\\n")?;
                } else {
                    out.write_char(*c)?;
                    self.last = Last::Delim(*c);
                }
            }
            Token::Number(n) => {
                out.write_str(n.repr)?;
                self.last = Last::Number;
            }
            Token::Percentage(n) => {
                out.write_str(n.repr)?;
                out.write_char('%')?;
            }
            Token::Dimension { number, unit } => {
                out.write_str(number.repr)?;
                write_unit(out, unit)?;
                self.last = Last::Dimension;
            }
            Token::UnicodeRange { start, end } if start == end => write!(out, "U+{start:X}")?,
            Token::UnicodeRange { start, end } => write!(out, "U+{start:X}-{end:X}")?,
            Token::Whitespace => out.write_char(' ')?,
            Token::Cdo => out.write_str("<!--")?,
            Token::Cdc => out.write_str("-->")?,
            Token::Colon => out.write_char(':')?,
            Token::Semicolon => out.write_char(';')?,
            Token::Comma => out.write_char(',')?,
            Token::LeftBracket => out.write_char('[')?,
            Token::RightBracket => out.write_char(']')?,
            Token::LeftParen => out.write_char('(')?,
            Token::RightParen => out.write_char(')')?,
            Token::LeftBrace => out.write_char('{')?,
            Token::RightBrace => out.write_char('}')?,
        }
        Ok(())
    }

    /// Writes a list, keeping a stack of the functions and blocks it's in
    /// rather than recursing into them.
    fn values(&mut self, values: &[ComponentValue<'_>]) -> fmt::Result {
        let mut stack: Vec<(std::slice::Iter<'_, ComponentValue<'_>>, char)> =
            vec![(values.iter(), '\0')];
        while let Some((iter, close)) = stack.last_mut() {
            let close = *close;
            let Some(value) = iter.next() else {
                stack.pop();
                if close != '\0' {
                    self.token(&close_token(close))?;
                }
                continue;
            };
            match value {
                ComponentValue::Token(t, _) => self.token(t)?,
                ComponentValue::Function(f) => {
                    self.token(&Token::Function(f.name.clone()))?;
                    stack.push((f.arguments.iter(), ')'));
                }
                ComponentValue::Block(b) => {
                    self.token(&open_token(b.bracket.open()))?;
                    stack.push((b.contents.iter(), b.bracket.close()));
                }
            }
        }
        Ok(())
    }

    fn block(&mut self, block: &SimpleBlock<'_>) -> fmt::Result {
        self.token(&open_token(block.bracket.open()))?;
        self.values(&block.contents)?;
        self.token(&close_token(block.bracket.close()))
    }

    fn rule(&mut self, rule: &Rule<'_>) -> fmt::Result {
        match rule {
            Rule::At(r) => {
                self.token(&Token::AtKeyword(r.name.clone()))?;
                self.values(&r.prelude)?;
                match &r.block {
                    Some(b) => self.block(b),
                    None => self.token(&Token::Semicolon),
                }
            }
            Rule::Qualified(r) => {
                self.values(&r.prelude)?;
                self.block(&r.block)
            }
        }
    }

    fn declaration(&mut self, d: &Declaration<'_>) -> fmt::Result {
        self.token(&Token::Ident(d.name.clone()))?;
        self.token(&Token::Colon)?;
        self.values(&d.value)?;
        if d.important {
            self.token(&Token::Delim('!'))?;
            self.token(&Token::Ident("important".into()))?;
        }
        Ok(())
    }

    fn items(&mut self, items: &[Item<'_>]) -> fmt::Result {
        for item in items {
            match item {
                Item::Declaration(d) => {
                    self.declaration(d)?;
                    self.token(&Token::Semicolon)?;
                }
                Item::Rule(r) => self.rule(r)?,
            }
        }
        Ok(())
    }
}

fn open_token(c: char) -> Token<'static> {
    match c {
        '(' => Token::LeftParen,
        '[' => Token::LeftBracket,
        _ => Token::LeftBrace,
    }
}

fn close_token(c: char) -> Token<'static> {
    match c {
        ')' => Token::RightParen,
        ']' => Token::RightBracket,
        _ => Token::RightBrace,
    }
}

/// Writes component values as CSS text.
///
/// ```
/// let values = parsec_css::parse_component_values("a  b(c) d e");
/// assert_eq!(parsec_css::to_css(&values), "a b(c) d e");
/// ```
pub fn write_values(out: &mut impl Write, values: &[ComponentValue<'_>]) -> fmt::Result {
    writer(out).values(values)
}

/// [`write_values`] into a new string.
pub fn to_css(values: &[ComponentValue<'_>]) -> String {
    let mut out = String::new();
    write_values(&mut out, values).unwrap();
    out
}

impl<W: Write> Writer<'_, W> {
    fn rules(&mut self, rules: &[Rule<'_>]) -> fmt::Result {
        rules.iter().try_for_each(|r| self.rule(r))
    }
}

fn writer<W: Write>(out: &mut W) -> Writer<'_, W> {
    Writer {
        out,
        last: Last::Other,
    }
}

impl fmt::Display for ComponentValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).values(std::slice::from_ref(self))
    }
}

impl fmt::Display for SimpleBlock<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).block(self)
    }
}

impl fmt::Display for Declaration<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).declaration(self)
    }
}

impl fmt::Display for Rule<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).rule(self)
    }
}

impl fmt::Display for AtRule<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut w = writer(f);
        w.token(&Token::AtKeyword(self.name.clone()))?;
        w.values(&self.prelude)?;
        match &self.block {
            Some(b) => w.block(b),
            None => w.token(&Token::Semicolon),
        }
    }
}

impl fmt::Display for QualifiedRule<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut w = writer(f);
        w.values(&self.prelude)?;
        w.block(&self.block)
    }
}

impl fmt::Display for Stylesheet<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).rules(&self.rules)
    }
}

impl fmt::Display for BlockContents<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writer(f).items(&self.items)
    }
}

#[cfg(test)]
mod tests {
    use crate::*;

    #[test]
    fn tokens_that_would_merge_get_a_comment() {
        let v = parse_component_values("a/**/b 1/**/px #x/**/-y a/**/(b) @a(b) -/**/-");
        assert_eq!(to_css(&v), "a/**/b 1/**/px #x/**/-y a/**/(b) @a(b) -/**/-");
    }

    #[test]
    fn escapes_come_back_as_the_same_values() {
        let css = r#"\31 a "q\"\\" url(a\)b) 1\65 3 \- \0 #\31 x"#;
        let first = parse_component_values(css);
        let again = to_css(&first);
        let second = parse_component_values(&again);
        let tokens = |v: &[ComponentValue<'_>]| -> Vec<Token<'static>> {
            v.iter()
                .filter_map(|v| v.token().map(|t| owned(t.clone())))
                .collect()
        };
        assert_eq!(tokens(&first), tokens(&second), "{again}");
    }

    fn owned(t: Token<'_>) -> Token<'static> {
        // Spans and borrowing aside, the tokens must match; Debug text is
        // the simplest owned form of one.
        Token::String(format!("{t:?}").into())
    }

    #[test]
    fn rules_and_declarations() {
        let css = "a>b{c:d!important;e:f}@m x{y{}}@i 'u';";
        let sheet = parse_stylesheet(css);
        assert_eq!(sheet.to_string(), css.replace('\'', "\""));
        let block = parse_block_contents("c: d !important; e{f:g}");
        assert_eq!(block.to_string(), "c:d!important;e{f:g}");
    }
}
