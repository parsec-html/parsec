// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The tokenizer, CSS Syntax §4, run straight over the input's bytes.
//!
//! The spec first filters the input: CR, FF and CR LF become LF, and NUL
//! becomes U+FFFD. Copying a whole stylesheet to do that costs more than
//! the tokenizing, so it's done where it shows instead: every newline test
//! accepts all four forms, and NUL counts as the U+FFFD it stands for. A
//! value borrows the input unless an escape or a NUL changes its text.

use std::borrow::Cow;

/// Where something is in the input, in bytes: `start..end`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Span {
        Span { start, end }
    }

    /// The text the span covers in `src`, the string it was made from.
    pub fn text<'s>(&self, src: &'s str) -> &'s str {
        &src[self.start..self.end]
    }
}

/// A `<number-token>`'s value, and the text it was written as.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Number<'a> {
    pub value: f64,
    /// The type flag: "integer" (true) or "number".
    pub integer: bool,
    /// The digits as written, sign and exponent included: `+.5e3`.
    pub repr: &'a str,
}

impl Number<'_> {
    /// The sign character, if the number was written with one.
    pub fn sign(&self) -> Option<char> {
        match self.repr.as_bytes().first() {
            Some(b'+') => Some('+'),
            Some(b'-') => Some('-'),
            _ => None,
        }
    }

    /// The value as an integer, saturated to `i32`'s range. Meaningful
    /// when [`integer`](Number::integer) is set.
    pub fn int(&self) -> i32 {
        self.value as i32
    }
}

/// A CSS token. The ones that open a block or a function never reach a
/// parser's output; they turn into [`ComponentValue`](crate::ComponentValue)s.
#[derive(Clone, Debug, PartialEq)]
pub enum Token<'a> {
    Ident(Cow<'a, str>),
    /// `name(`, with the name.
    Function(Cow<'a, str>),
    /// `@name`, with the name.
    AtKeyword(Cow<'a, str>),
    /// `#name`. `id` is the type flag: set when the name would also be an
    /// identifier, which an ID selector needs.
    Hash {
        value: Cow<'a, str>,
        id: bool,
    },
    String(Cow<'a, str>),
    BadString,
    /// An unquoted `url(…)`, with what's between the parentheses.
    Url(Cow<'a, str>),
    BadUrl,
    Delim(char),
    Number(Number<'a>),
    Percentage(Number<'a>),
    Dimension {
        number: Number<'a>,
        unit: Cow<'a, str>,
    },
    /// `U+0-7F`, made only where a `unicode-range` descriptor's value is
    /// tokenized; everywhere else that text is an ident and numbers.
    UnicodeRange {
        start: u32,
        end: u32,
    },
    Whitespace,
    Cdo,
    Cdc,
    Colon,
    Semicolon,
    Comma,
    LeftBracket,
    RightBracket,
    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
}

/// Turns CSS text into tokens, one at a time, skipping comments.
///
/// ```
/// use parsec_css::{Token, Tokenizer};
///
/// let mut tokens = Tokenizer::new("a{color:red}");
/// let (first, span) = tokens.next().unwrap();
/// assert_eq!((first, span.start, span.end), (Token::Ident("a".into()), 0, 1));
/// assert_eq!(tokens.count(), 5);
/// ```
#[derive(Clone, Debug)]
pub struct Tokenizer<'a> {
    src: &'a str,
    pos: usize,
    unicode_ranges: bool,
}

/// The ASCII ident code points.
const NAME: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 128 {
        t[b] = matches!(b as u8, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_');
        b += 1;
    }
    t
};

/// The newlines preprocessing would leave as LF.
fn is_newline(b: u8) -> bool {
    matches!(b, b'\n' | b'\r' | b'\x0c')
}

fn is_whitespace(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c')
}

fn is_non_printable(b: u8) -> bool {
    matches!(b, 0x01..=0x08 | 0x0b | 0x0e..=0x1f | 0x7f)
}

/// The non-ASCII ident code points, which since 2024 are the ones HTML
/// allows in custom element names rather than all of them.
pub(crate) fn is_non_ascii_ident(c: char) -> bool {
    matches!(c as u32,
        0xb7 | 0xc0..=0xd6 | 0xd8..=0xf6 | 0xf8..=0x37d | 0x37f..=0x1fff
        | 0x200c | 0x200d | 0x203f | 0x2040 | 0x2070..=0x218f
        | 0x2c00..=0x2fef | 0x3001..=0xd7ff | 0xf900..=0xfdcf
        | 0xfdf0..=0xfffd | 0x10000..)
}

impl<'a> Tokenizer<'a> {
    pub fn new(src: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            src,
            pos: 0,
            unicode_ranges: false,
        }
    }

    /// A tokenizer that makes `<unicode-range-token>`s, as for the value of
    /// `@font-face`'s `unicode-range` descriptor.
    pub fn with_unicode_ranges(src: &'a str) -> Tokenizer<'a> {
        Tokenizer {
            unicode_ranges: true,
            ..Tokenizer::new(src)
        }
    }

    /// The byte offset of the next token, or of the comments before it.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Moves back (or forward) to a position this tokenizer returned.
    pub fn reset(&mut self, position: usize) {
        self.pos = position;
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    #[inline]
    fn at(&self, i: usize) -> Option<u8> {
        self.src.as_bytes().get(i).copied()
    }

    fn char_at(&self, i: usize) -> Option<char> {
        self.src.get(i..)?.chars().next()
    }

    fn is_ident_start(&self, i: usize) -> bool {
        match self.at(i) {
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'_' | 0) => true,
            Some(0x80..) => self.char_at(i).is_some_and(is_non_ascii_ident),
            _ => false,
        }
    }

    fn is_ident_char(&self, i: usize) -> bool {
        matches!(self.at(i), Some(b'0'..=b'9' | b'-')) || self.is_ident_start(i)
    }

    /// Whether the bytes at `i` are a backslash and something it escapes.
    fn is_escape(&self, i: usize) -> bool {
        self.at(i) == Some(b'\\') && !self.at(i + 1).is_some_and(is_newline)
    }

    fn starts_ident(&self, i: usize) -> bool {
        match self.at(i) {
            Some(b'-') => {
                self.at(i + 1) == Some(b'-') || self.is_ident_start(i + 1) || self.is_escape(i + 1)
            }
            Some(b'\\') => self.is_escape(i),
            _ => self.is_ident_start(i),
        }
    }

    fn starts_number(&self, i: usize) -> bool {
        let digit = |j| self.at(j).is_some_and(|b: u8| b.is_ascii_digit());
        match self.at(i) {
            Some(b'+' | b'-') => digit(i + 1) || (self.at(i + 1) == Some(b'.') && digit(i + 2)),
            Some(b'.') => digit(i + 1),
            Some(b'0'..=b'9') => true,
            _ => false,
        }
    }

    fn starts_unicode_range(&self, i: usize) -> bool {
        matches!(self.at(i), Some(b'u' | b'U'))
            && self.at(i + 1) == Some(b'+')
            && self
                .at(i + 2)
                .is_some_and(|b| b == b'?' || b.is_ascii_hexdigit())
    }

    /// Steps over one newline, CR LF counting as one.
    fn skip_newline(&mut self) {
        if self.at(self.pos) == Some(b'\r') && self.at(self.pos + 1) == Some(b'\n') {
            self.pos += 2;
        } else {
            self.pos += 1;
        }
    }

    fn skip_whitespace(&mut self) {
        let bytes = self.src.as_bytes();
        while bytes.get(self.pos).is_some_and(|&b| is_whitespace(b)) {
            self.pos += 1;
        }
    }

    fn skip_comments(&mut self) {
        while self.src.as_bytes()[self.pos..].starts_with(b"/*") {
            self.pos = match self.src[self.pos + 2..].find("*/") {
                Some(end) => self.pos + 2 + end + 2,
                None => self.src.len(),
            };
        }
    }

    /// The code point after a backslash, which has been consumed.
    fn escape(&mut self) -> char {
        let Some(b) = self.at(self.pos) else {
            return '\u{fffd}';
        };
        if b.is_ascii_hexdigit() {
            let start = self.pos;
            while self.pos - start < 6 && self.at(self.pos).is_some_and(|b| b.is_ascii_hexdigit()) {
                self.pos += 1;
            }
            let n = u32::from_str_radix(&self.src[start..self.pos], 16).unwrap();
            if self.at(self.pos).is_some_and(is_whitespace) {
                self.skip_newline();
            }
            return match char::from_u32(n) {
                Some(c) if n != 0 => c,
                _ => '\u{fffd}',
            };
        }
        if b == 0 {
            self.pos += 1;
            return '\u{fffd}';
        }
        let c = self.char_at(self.pos).unwrap();
        self.pos += c.len_utf8();
        c
    }

    /// Consumes an ident sequence, borrowing it if no escape or NUL is in it.
    fn ident_sequence(&mut self) -> Cow<'a, str> {
        let start = self.pos;
        let bytes = self.src.as_bytes();
        loop {
            // ASCII names are nearly all of them, so they get a tight loop.
            while bytes.get(self.pos).is_some_and(|&b| NAME[b as usize]) {
                self.pos += 1;
            }
            match self.at(self.pos) {
                Some(0x80..) if self.is_ident_start(self.pos) => {
                    self.pos += self.char_at(self.pos).unwrap().len_utf8();
                }
                Some(b'\\' | 0) => break,
                _ => return Cow::Borrowed(&self.src[start..self.pos]),
            }
        }
        let mut out = self.src[start..self.pos].to_string();
        loop {
            if self.is_escape(self.pos) {
                self.pos += 1;
                out.push(self.escape());
            } else if self.at(self.pos) == Some(0) {
                self.pos += 1;
                out.push('\u{fffd}');
            } else if self.is_ident_char(self.pos) {
                let c = self.char_at(self.pos).unwrap();
                self.pos += c.len_utf8();
                out.push(c);
            } else {
                return Cow::Owned(out);
            }
        }
    }

    fn number(&mut self) -> Number<'a> {
        let start = self.pos;
        let digits = |t: &mut Self| {
            while t.at(t.pos).is_some_and(|b| b.is_ascii_digit()) {
                t.pos += 1;
            }
        };
        let digit = |t: &Self, i| t.at(i).is_some_and(|b: u8| b.is_ascii_digit());
        let mut integer = true;
        if matches!(self.at(self.pos), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        digits(self);
        if self.at(self.pos) == Some(b'.') && digit(self, self.pos + 1) {
            self.pos += 1;
            digits(self);
            integer = false;
        }
        if matches!(self.at(self.pos), Some(b'e' | b'E')) {
            let sign = matches!(self.at(self.pos + 1), Some(b'+' | b'-')) as usize;
            if digit(self, self.pos + 1 + sign) {
                self.pos += 1 + sign;
                digits(self);
                integer = false;
            }
        }
        let repr = &self.src[start..self.pos];
        Number {
            value: repr.parse().unwrap_or(0.0),
            integer,
            repr,
        }
    }

    fn numeric(&mut self) -> Token<'a> {
        let number = self.number();
        if self.starts_ident(self.pos) {
            let unit = self.ident_sequence();
            Token::Dimension { number, unit }
        } else if self.at(self.pos) == Some(b'%') {
            self.pos += 1;
            Token::Percentage(number)
        } else {
            Token::Number(number)
        }
    }

    fn ident_like(&mut self) -> Token<'a> {
        let name = self.ident_sequence();
        if self.at(self.pos) != Some(b'(') {
            return Token::Ident(name);
        }
        self.pos += 1;
        if !name.eq_ignore_ascii_case("url") {
            return Token::Function(name);
        }
        let open = self.pos;
        while self.at(self.pos).is_some_and(is_whitespace)
            && self.at(self.pos + 1).is_some_and(is_whitespace)
        {
            self.pos += 1;
        }
        let quote = |b: Option<u8>| matches!(b, Some(b'"' | b'\''));
        let next = self.at(self.pos);
        if quote(next) || (next.is_some_and(is_whitespace) && quote(self.at(self.pos + 1))) {
            // The spec has the function token take all but one of the
            // spaces; giving them all to the whitespace token after it
            // makes the same tokens, with `url(` as the function's text.
            self.pos = open;
            return Token::Function(name);
        }
        self.url()
    }

    /// An unquoted URL, after `url(`.
    fn url(&mut self) -> Token<'a> {
        self.skip_whitespace();
        let start = self.pos;
        let mut owned: Option<String> = None;
        while let Some(b) = self.at(self.pos) {
            match b {
                b')' => break,
                _ if is_whitespace(b) => {
                    let end = self.pos;
                    self.skip_whitespace();
                    if matches!(self.at(self.pos), None | Some(b')')) {
                        let value = finish(owned, &self.src[start..end]);
                        self.pos = (self.pos + 1).min(self.src.len());
                        return Token::Url(value);
                    }
                    self.bad_url_remnants();
                    return Token::BadUrl;
                }
                b'"' | b'\'' | b'(' => {
                    self.bad_url_remnants();
                    return Token::BadUrl;
                }
                _ if is_non_printable(b) => {
                    self.bad_url_remnants();
                    return Token::BadUrl;
                }
                b'\\' => {
                    if !self.is_escape(self.pos) {
                        self.bad_url_remnants();
                        return Token::BadUrl;
                    }
                    let s = owned.get_or_insert_with(|| self.src[start..self.pos].to_string());
                    self.pos += 1;
                    s.push(self.escape());
                }
                0 => {
                    owned
                        .get_or_insert_with(|| self.src[start..self.pos].to_string())
                        .push('\u{fffd}');
                    self.pos += 1;
                }
                _ => {
                    let len = self.char_at(self.pos).unwrap().len_utf8();
                    if let Some(s) = &mut owned {
                        s.push_str(&self.src[self.pos..self.pos + len]);
                    }
                    self.pos += len;
                }
            }
        }
        let value = finish(owned, &self.src[start..self.pos]);
        self.pos = (self.pos + 1).min(self.src.len());
        Token::Url(value)
    }

    fn bad_url_remnants(&mut self) {
        while let Some(b) = self.at(self.pos) {
            if b == b')' {
                self.pos += 1;
                return;
            }
            if self.is_escape(self.pos) {
                self.pos += 1;
                self.escape();
            } else {
                self.pos += self.char_at(self.pos).unwrap().len_utf8();
            }
        }
    }

    /// A string, after its opening quote.
    fn string(&mut self, quote: u8) -> Token<'a> {
        let start = self.pos;
        let mut owned: Option<String> = None;
        loop {
            // Everything but these is copied as it is.
            let run = self.pos;
            while self
                .at(self.pos)
                .is_some_and(|b| b != quote && !matches!(b, b'\\' | b'\n' | b'\r' | b'\x0c' | 0))
            {
                self.pos += 1;
            }
            if let Some(s) = &mut owned {
                s.push_str(&self.src[run..self.pos]);
            }
            match self.at(self.pos) {
                None => return Token::String(finish(owned, &self.src[start..self.pos])),
                Some(b) if b == quote => {
                    let value = finish(owned, &self.src[start..self.pos]);
                    self.pos += 1;
                    return Token::String(value);
                }
                Some(b'\n' | b'\r' | b'\x0c') => return Token::BadString,
                Some(0) => {
                    owned
                        .get_or_insert_with(|| self.src[start..self.pos].to_string())
                        .push('\u{fffd}');
                    self.pos += 1;
                }
                _ => {
                    let s = owned.get_or_insert_with(|| self.src[start..self.pos].to_string());
                    self.pos += 1;
                    match self.at(self.pos) {
                        None => {}
                        Some(b) if is_newline(b) => self.skip_newline(),
                        Some(_) => {
                            let c = self.escape();
                            s.push(c);
                        }
                    }
                }
            }
        }
    }

    /// After `u+`, which has been checked for but not consumed.
    fn unicode_range(&mut self) -> Token<'a> {
        self.pos += 2;
        let hex = |t: &mut Self, max: usize| {
            let start = t.pos;
            while t.pos - start < max && t.at(t.pos).is_some_and(|b| b.is_ascii_hexdigit()) {
                t.pos += 1;
            }
            start
        };
        let start = hex(self, 6);
        let digits = self.pos - start;
        while self.pos - start < 6 && self.at(self.pos) == Some(b'?') {
            self.pos += 1;
        }
        let first = &self.src[start..self.pos];
        if self.pos - start > digits {
            let low = u32::from_str_radix(&first.replace('?', "0"), 16).unwrap();
            let high = u32::from_str_radix(&first.replace('?', "F"), 16).unwrap();
            return Token::UnicodeRange {
                start: low,
                end: high,
            };
        }
        let low = u32::from_str_radix(first, 16).unwrap();
        if self.at(self.pos) == Some(b'-')
            && self.at(self.pos + 1).is_some_and(|b| b.is_ascii_hexdigit())
        {
            self.pos += 1;
            let s = hex(self, 6);
            let high = u32::from_str_radix(&self.src[s..self.pos], 16).unwrap();
            return Token::UnicodeRange {
                start: low,
                end: high,
            };
        }
        Token::UnicodeRange {
            start: low,
            end: low,
        }
    }

    /// The next token and its span, or `None` at the end of the input.
    pub fn next_token(&mut self) -> Option<(Token<'a>, Span)> {
        self.skip_comments();
        let start = self.pos;
        let b = self.at(start)?;
        let simple = |t: &mut Self, token| {
            t.pos += 1;
            token
        };
        let token = match b {
            // Most tokens are names; `u` waits for the unicode-range check.
            b'a'..=b't' | b'v'..=b'z' | b'A'..=b'T' | b'V'..=b'Z' | b'_' => self.ident_like(),
            b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' => {
                self.skip_whitespace();
                Token::Whitespace
            }
            b'"' | b'\'' => {
                self.pos += 1;
                self.string(b)
            }
            b'#' => {
                if self.is_ident_char(start + 1) || self.is_escape(start + 1) {
                    let id = self.starts_ident(start + 1);
                    self.pos += 1;
                    Token::Hash {
                        value: self.ident_sequence(),
                        id,
                    }
                } else {
                    simple(self, Token::Delim('#'))
                }
            }
            b'(' => simple(self, Token::LeftParen),
            b')' => simple(self, Token::RightParen),
            b',' => simple(self, Token::Comma),
            b':' => simple(self, Token::Colon),
            b';' => simple(self, Token::Semicolon),
            b'[' => simple(self, Token::LeftBracket),
            b']' => simple(self, Token::RightBracket),
            b'{' => simple(self, Token::LeftBrace),
            b'}' => simple(self, Token::RightBrace),
            b'+' | b'.' if self.starts_number(start) => self.numeric(),
            b'-' if self.starts_number(start) => self.numeric(),
            b'-' if self.src.as_bytes()[start + 1..].starts_with(b"->") => {
                self.pos += 3;
                Token::Cdc
            }
            b'-' if self.starts_ident(start) => self.ident_like(),
            b'0'..=b'9' => self.numeric(),
            b'<' if self.src.as_bytes()[start + 1..].starts_with(b"!--") => {
                self.pos += 4;
                Token::Cdo
            }
            b'@' if self.starts_ident(start + 1) => {
                self.pos += 1;
                Token::AtKeyword(self.ident_sequence())
            }
            b'\\' if self.is_escape(start) => self.ident_like(),
            b'u' | b'U' if self.unicode_ranges && self.starts_unicode_range(start) => {
                self.unicode_range()
            }
            _ if self.is_ident_start(start) => self.ident_like(),
            0..0x80 => simple(self, Token::Delim(b as char)),
            _ => {
                let c = self.char_at(start).unwrap();
                self.pos += c.len_utf8();
                Token::Delim(c)
            }
        };
        Some((token, Span::new(start, self.pos)))
    }
}

fn finish<'a>(owned: Option<String>, borrowed: &'a str) -> Cow<'a, str> {
    match owned {
        Some(s) => Cow::Owned(s),
        None => Cow::Borrowed(borrowed),
    }
}

impl<'a> Iterator for Tokenizer<'a> {
    type Item = (Token<'a>, Span);

    fn next(&mut self) -> Option<(Token<'a>, Span)> {
        self.next_token()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(css: &str) -> Vec<Token<'_>> {
        Tokenizer::new(css).map(|(t, _)| t).collect()
    }

    #[test]
    fn values_borrow_unless_an_escape_changes_them() {
        let t = tokens("abc a\\62 c 'x\\'y' url(q\\)r)");
        assert!(matches!(&t[0], Token::Ident(Cow::Borrowed("abc"))));
        assert!(matches!(&t[2], Token::Ident(Cow::Owned(s)) if s == "abc"));
        assert!(matches!(&t[4], Token::String(Cow::Owned(s)) if s == "x'y"));
        assert!(matches!(&t[6], Token::Url(Cow::Owned(s)) if s == "q)r"));
    }

    #[test]
    fn crlf_counts_as_one_newline() {
        // The escape eats one whitespace, which CR LF is.
        assert_eq!(tokens("\\41\r\nb"), [Token::Ident("Ab".into())]);
        assert_eq!(tokens("'a\\\r\nb'"), [Token::String("ab".into())]);
        assert_eq!(
            tokens("'a\rb"),
            [
                Token::BadString,
                Token::Whitespace,
                Token::Ident("b".into())
            ]
        );
    }

    #[test]
    fn nul_is_u_fffd() {
        assert_eq!(tokens("\0a"), [Token::Ident("\u{fffd}a".into())]);
        assert_eq!(tokens("'\0'"), [Token::String("\u{fffd}".into())]);
    }

    #[test]
    fn unicode_ranges_only_when_asked() {
        assert_eq!(tokens("u+1f")[0], Token::Ident("u".into()));
        let t: Vec<_> = Tokenizer::with_unicode_ranges("u+1f-2F U+4??")
            .map(|t| t.0)
            .collect();
        assert_eq!(
            t,
            [
                Token::UnicodeRange {
                    start: 0x1f,
                    end: 0x2f
                },
                Token::Whitespace,
                Token::UnicodeRange {
                    start: 0x400,
                    end: 0x4ff
                }
            ]
        );
    }
}
