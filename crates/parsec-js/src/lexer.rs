// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The lexer: ECMA-262's lexical grammar (clause 12) and Annex B's
//! additions to it (B.1), one token per call.

use parsec_base::{Diagnostic, Limits, Severity, Span};

use crate::token::{Flags, Keyword, Kind, Punct, Token};
use crate::unicode;

/// Which of the lexical grammar's goal symbols to read the next token
/// with. The grammar is ambiguous without one: `/` divides after an
/// expression and starts a regular expression anywhere else, and `}`
/// closes a block unless it ends a template substitution. The parser
/// knows which; the lexer is told and never guesses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Goal {
    /// InputElementDiv: `/` and `/=` are punctuators, `}` is a punctuator.
    Div,
    /// InputElementRegExp: `/` starts a regular expression.
    RegExp,
    /// InputElementTemplateTail: `}` starts a TemplateMiddle or TemplateTail.
    TemplateTail,
    /// InputElementRegExpOrTemplateTail: both.
    RegExpOrTemplateTail,
}

impl Goal {
    const fn regexp(self) -> bool {
        matches!(self, Goal::RegExp | Goal::RegExpOrTemplateTail)
    }

    const fn template_tail(self) -> bool {
        matches!(self, Goal::TemplateTail | Goal::RegExpOrTemplateTail)
    }
}

/// Whether the source is a Script or a Module. Modules are strict, and
/// Annex B's HTML-like comments are a syntax error in them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SourceType {
    #[default]
    Script,
    Module,
}

/// Reads tokens from a source, one per [`Lexer::next_token`].
///
/// It never fails: a token it can't read whole comes back as far as it
/// goes (a string with no closing quote ends at the end of the line), or
/// as [`Kind::Invalid`], and a [`Diagnostic`] says what was wrong. Time is
/// linear in the input, and nothing is allocated per token; only a
/// diagnostic is.
///
/// ```
/// use parsec_js::{Goal, Kind, Lexer, Punct, SourceType};
///
/// let src = "a / b; /b/g";
/// let mut lexer = Lexer::new(src, SourceType::Script);
/// let kinds: Vec<Kind> = [Goal::RegExp, Goal::Div, Goal::RegExp, Goal::Div, Goal::RegExp]
///     .into_iter()
///     .map(|goal| lexer.next_token(goal).kind)
///     .collect();
/// assert_eq!(kinds[1], Kind::Punct(Punct::Slash));
/// assert_eq!(kinds[4], Kind::RegExp);
/// assert!(lexer.diagnostics().is_empty());
/// ```
#[derive(Clone, Debug)]
pub struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    source: SourceType,
    /// A line terminator since the last token that wasn't trivia.
    newline: bool,
    /// Only whitespace and comments without line terminators since the
    /// last line terminator: where Annex B lets `-->` start a comment.
    line_start: bool,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Lexer<'a> {
    pub fn new(src: &'a str, source: SourceType) -> Lexer<'a> {
        Lexer::with_limits(src, source, &Limits::default())
    }

    /// A lexer that refuses input longer than `limits.max_input`, or than
    /// spans can hold: it reports `input-too-large` and gives only
    /// [`Kind::Eof`].
    pub fn with_limits(src: &'a str, source: SourceType, limits: &Limits) -> Lexer<'a> {
        let mut diagnostics = Vec::new();
        let src = match limits.check_input(src.len()) {
            Ok(()) => src,
            Err(d) => {
                diagnostics.push(d);
                ""
            }
        };
        Lexer {
            src,
            pos: 0,
            source,
            newline: false,
            // InputElementHashbangOrRegExp, the goal at the start, has
            // HTMLCloseComment: `-->` can start the first line too.
            line_start: true,
            diagnostics,
        }
    }

    pub fn source(&self) -> &'a str {
        self.src
    }

    /// The byte offset the next token is read from.
    pub fn position(&self) -> u32 {
        self.pos as u32
    }

    /// What's been reported so far, in the order it was found.
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn take_diagnostics(&mut self) -> Vec<Diagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    /// The next token, read with `goal`. Comments and a hashbang come
    /// back as tokens too ([`Kind::is_trivia`]); whitespace and line
    /// terminators don't, but [`Token::newline_before`] says where a line
    /// terminator was.
    pub fn next_token(&mut self, goal: Goal) -> Token {
        if self.pos == 0 && self.src.starts_with("#!") {
            let start = self.pos;
            self.pos = self.line_end(2);
            return self.trivia(Kind::Hashbang, start, false);
        }
        self.skip_whitespace();
        let start = self.pos;
        let b = self.src.as_bytes();
        let Some(&c) = b.get(start) else {
            return self.token(Kind::Eof, start, Flags::empty());
        };
        let script = self.source == SourceType::Script;
        let kind = match c {
            b'/' => match b.get(start + 1) {
                Some(b'/') => {
                    self.pos = self.line_end(start + 2);
                    return self.trivia(Kind::LineComment, start, false);
                }
                Some(b'*') => return self.block_comment(start),
                _ if goal.regexp() => return self.regexp(start),
                Some(b'=') => self.punct(2, Punct::SlashEq),
                _ => self.punct(1, Punct::Slash),
            },
            // Annex B's HTML-like comments, in scripts only: in a module
            // `<!--` is `<` `!` `--` and `-->` is `--` `>`.
            b'<' if script && b[start..].starts_with(b"<!--") => {
                self.pos = self.line_end(start + 4);
                return self.trivia(Kind::HtmlOpenComment, start, false);
            }
            b'-' if script && self.line_start && b[start..].starts_with(b"-->") => {
                self.pos = self.line_end(start + 3);
                return self.trivia(Kind::HtmlCloseComment, start, false);
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\' => return self.ident(start),
            b'0'..=b'9' => return self.number(start),
            b'.' if b.get(start + 1).is_some_and(u8::is_ascii_digit) => return self.number(start),
            b'"' | b'\'' => return self.string(start, c),
            b'`' => return self.template(start, false),
            b'}' if goal.template_tail() => return self.template(start, true),
            b'#' => return self.private_name(start),
            0x80.. => {
                let ch = self.char_at(start);
                if is_id_start(ch) {
                    return self.ident(start);
                }
                self.pos += ch.len_utf8();
                self.error(
                    "unexpected-character",
                    start,
                    "this character can't start a token",
                );
                Kind::Invalid
            }
            _ => match self.punctuator(start) {
                Some(p) => Kind::Punct(p),
                None => {
                    self.pos += 1;
                    self.error(
                        "unexpected-character",
                        start,
                        "this character can't start a token",
                    );
                    Kind::Invalid
                }
            },
        };
        self.token(kind, start, Flags::empty())
    }

    // Helpers that end a token.

    fn token(&mut self, kind: Kind, start: usize, mut flags: Flags) -> Token {
        if self.newline {
            flags.insert(Flags::NEWLINE_BEFORE);
        }
        self.newline = false;
        self.line_start = false;
        Token {
            kind,
            flags,
            span: Span::new(start as u32, self.pos as u32),
        }
    }

    /// A comment, which leaves the newline flag for the token after it,
    /// and sets it if the comment held a line terminator.
    fn trivia(&mut self, kind: Kind, start: usize, holds_newline: bool) -> Token {
        let flags = if self.newline {
            Flags::NEWLINE_BEFORE
        } else {
            Flags::empty()
        };
        if holds_newline {
            self.newline = true;
            self.line_start = true;
        } else if kind != Kind::BlockComment {
            // A line comment runs to a line terminator, which sets
            // line_start again when it's skipped.
            self.line_start = false;
        }
        Token {
            kind,
            flags,
            span: Span::new(start as u32, self.pos as u32),
        }
    }

    fn punct(&mut self, len: usize, p: Punct) -> Kind {
        self.pos += len;
        Kind::Punct(p)
    }

    fn report(
        &mut self,
        code: &'static str,
        severity: Severity,
        start: usize,
        message: &'static str,
    ) {
        let span = Span::new(start as u32, self.pos.max(start) as u32);
        self.diagnostics
            .push(Diagnostic::new(code, severity, span, message));
    }

    fn error(&mut self, code: &'static str, start: usize, message: &'static str) {
        self.report(code, Severity::Error, start, message);
    }

    /// A legacy form: an error in a module, a warning in a script, whose
    /// strictness only the parser knows.
    fn legacy(&mut self, code: &'static str, start: usize, message: &'static str) {
        let severity = match self.source {
            SourceType::Module => Severity::Error,
            SourceType::Script => Severity::Warning,
        };
        self.report(code, severity, start, message);
    }

    // Characters.

    fn char_at(&self, at: usize) -> char {
        self.src[at..].chars().next().unwrap_or('\0')
    }

    /// The offset of the first line terminator at or after `from`, or the
    /// end.
    fn line_end(&self, from: usize) -> usize {
        let b = self.src.as_bytes();
        let mut i = from;
        while i < b.len() {
            match b[i] {
                b'\n' | b'\r' => return i,
                0xE2 if is_separator(&b[i..]) => return i,
                _ => i += 1,
            }
        }
        i
    }

    /// Whitespace and line terminators (12.2, 12.3).
    fn skip_whitespace(&mut self) {
        let b = self.src.as_bytes();
        while let Some(&c) = b.get(self.pos) {
            match c {
                b' ' | b'\t' | 0x0B | 0x0C => self.pos += 1,
                b'\n' | b'\r' => {
                    self.pos += 1;
                    self.newline = true;
                    self.line_start = true;
                }
                0x80.. => {
                    let ch = self.char_at(self.pos);
                    if matches!(ch, '\u{2028}' | '\u{2029}') {
                        self.newline = true;
                        self.line_start = true;
                    } else if !is_whitespace(ch) {
                        return;
                    }
                    self.pos += ch.len_utf8();
                }
                _ => return,
            }
        }
    }

    // Comments (12.4, B.1.1).

    fn block_comment(&mut self, start: usize) -> Token {
        let b = self.src.as_bytes();
        let mut i = start + 2;
        let mut newline = false;
        loop {
            match b.get(i) {
                None => {
                    self.pos = i;
                    self.error(
                        "unterminated-comment",
                        start,
                        "a /* comment has no closing */",
                    );
                    break;
                }
                Some(b'*') if b.get(i + 1) == Some(&b'/') => {
                    self.pos = i + 2;
                    break;
                }
                Some(b'\n' | b'\r') => newline = true,
                Some(0xE2) if is_separator(&b[i..]) => newline = true,
                _ => {}
            }
            i += 1;
        }
        self.trivia(Kind::BlockComment, start, newline)
    }

    // Identifiers (12.7) and private names.

    fn private_name(&mut self, start: usize) -> Token {
        let next = self.pos + 1;
        let starts_ident = match self.src.as_bytes().get(next) {
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\') => true,
            Some(0x80..) => is_id_start(self.char_at(next)),
            _ => false,
        };
        if !starts_ident {
            self.pos = next;
            self.error(
                "unexpected-character",
                start,
                "# starts a private name, like #x",
            );
            return self.token(Kind::Invalid, start, Flags::empty());
        }
        let ident = self.ident(next);
        // An escape that isn't one is reported already; the name is still
        // a name.
        Token {
            kind: Kind::PrivateName,
            span: Span::new(start as u32, ident.span.end),
            ..ident
        }
    }

    fn ident(&mut self, start: usize) -> Token {
        let b = self.src.as_bytes();
        let mut flags = Flags::empty();
        self.pos = start;
        let mut first = true;
        loop {
            match b.get(self.pos) {
                Some(&c) if c < 0x80 && ID_CONTINUE_ASCII[c as usize] => {
                    if first && c.is_ascii_digit() {
                        break;
                    }
                    self.pos += 1;
                }
                Some(b'\\') => {
                    let at = self.pos;
                    flags.insert(Flags::ESCAPED);
                    self.pos += 1;
                    let ok = if b.get(self.pos) == Some(&b'u') {
                        self.pos += 1;
                        self.unicode_escape()
                            .and_then(char::from_u32)
                            .is_some_and(|ch| {
                                if first {
                                    is_id_start(ch) || ch == '$' || ch == '_'
                                } else {
                                    is_id_continue(ch)
                                }
                            })
                    } else {
                        false
                    };
                    if !ok {
                        if first && self.pos == at + 1 {
                            // A lone `\`: no identifier at all.
                            self.error("invalid-escape", at, "\\ here has to start a \\u escape");
                            return self.token(Kind::Invalid, start, Flags::empty());
                        }
                        self.error(
                            "invalid-identifier-escape",
                            at,
                            "this escape isn't a character an identifier can have here",
                        );
                    }
                }
                Some(0x80..) => {
                    let ch = self.char_at(self.pos);
                    if !(if first {
                        is_id_start(ch)
                    } else {
                        is_id_continue(ch)
                    }) {
                        break;
                    }
                    self.pos += ch.len_utf8();
                }
                _ => break,
            }
            first = false;
        }
        let kind = match flags.contains(Flags::ESCAPED) {
            false => {
                Keyword::from_str(&self.src[start..self.pos]).map_or(Kind::Ident, Kind::Keyword)
            }
            true => Kind::Ident,
        };
        self.token(kind, start, flags)
    }

    /// After `\u`: four hex digits, or hex digits in braces up to
    /// 10FFFF. It consumes only hex digits and the braces, so a bad escape
    /// never swallows the quote or backtick after it.
    fn unicode_escape(&mut self) -> Option<u32> {
        let b = self.src.as_bytes();
        if b.get(self.pos) == Some(&b'{') {
            self.pos += 1;
            let mut value: u32 = 0;
            let mut digits = 0;
            while let Some(d) = b.get(self.pos).and_then(|&c| (c as char).to_digit(16)) {
                value = value.saturating_mul(16).saturating_add(d).min(0x11_0000);
                digits += 1;
                self.pos += 1;
            }
            if digits == 0 || b.get(self.pos) != Some(&b'}') {
                return None;
            }
            self.pos += 1;
            (value <= 0x10_FFFF).then_some(value)
        } else {
            self.hex_digits(4)
        }
    }

    /// Exactly `n` hex digits, consuming the ones there are up to `n`.
    fn hex_digits(&mut self, n: usize) -> Option<u32> {
        let b = self.src.as_bytes();
        let mut value = 0;
        for _ in 0..n {
            let d = b.get(self.pos).and_then(|&c| (c as char).to_digit(16))?;
            value = value * 16 + d;
            self.pos += 1;
        }
        Some(value)
    }

    // Numeric literals (12.9.3, B.1.1).

    fn number(&mut self, start: usize) -> Token {
        let b = self.src.as_bytes();
        let mut flags = Flags::empty();
        let mut kind = Kind::Number;
        self.pos = start;
        let radix = match (b[start], b.get(start + 1).map(u8::to_ascii_lowercase)) {
            (b'0', Some(b'x')) => 16,
            (b'0', Some(b'o')) => 8,
            (b'0', Some(b'b')) => 2,
            _ => 10,
        };
        if radix != 10 {
            self.pos += 2;
            if self.digits(radix) == 0 {
                self.error(
                    "missing-digits",
                    start,
                    "a number needs digits after its prefix",
                );
            }
            if b.get(self.pos) == Some(&b'n') {
                self.pos += 1;
                kind = Kind::BigInt;
            }
        } else if b[start] == b'0' && b.get(start + 1).is_some_and(u8::is_ascii_digit) {
            // A legacy octal literal (017), or a decimal one with a leading
            // zero (08, 09.5), which has a fraction and exponent like any.
            self.pos += 1;
            let mut octal = true;
            while let Some(&c) = b.get(self.pos).filter(|c| c.is_ascii_digit()) {
                octal &= c < b'8';
                self.pos += 1;
            }
            flags.insert(Flags::LEGACY_OCTAL);
            if octal {
                self.legacy(
                    "legacy-octal-literal",
                    start,
                    "legacy octal literals like 017 aren't allowed in strict code; write 0o17",
                );
            } else {
                self.legacy(
                    "leading-zero-literal",
                    start,
                    "decimal literals with a leading zero aren't allowed in strict code",
                );
                self.fraction_and_exponent();
            }
        } else {
            let mut integer = true;
            if b[start] == b'0' {
                self.pos += 1;
            } else if b[start] != b'.' {
                self.digits(10);
            }
            if b.get(self.pos) == Some(&b'.') {
                integer = false;
            }
            integer &= !self.fraction_and_exponent();
            if integer && b.get(self.pos) == Some(&b'n') {
                self.pos += 1;
                kind = Kind::BigInt;
            }
        }
        // The SourceCharacter after a NumericLiteral can't be an
        // IdentifierStart or a digit: `3in`, `1_`, `0b12` and `1.5n` are all
        // errors. The rest of the word goes in the token.
        let bad = match b.get(self.pos) {
            Some(c) if c.is_ascii_digit() => true,
            Some(b'a'..=b'z' | b'A'..=b'Z' | b'$' | b'_' | b'\\') => true,
            Some(0x80..) => is_id_start(self.char_at(self.pos)),
            _ => false,
        };
        if bad {
            let at = self.pos;
            while let Some(&c) = b.get(self.pos) {
                if c < 0x80 && ID_CONTINUE_ASCII[c as usize] {
                    self.pos += 1;
                } else if c >= 0x80 && is_id_continue(self.char_at(self.pos)) {
                    self.pos += self.char_at(self.pos).len_utf8();
                } else {
                    break;
                }
            }
            self.pos = self.pos.max(at + 1);
            self.error(
                "identifier-after-number",
                at,
                "a number can't be followed straight away by a letter or digit",
            );
        }
        self.token(kind, start, flags)
    }

    /// `.digits` and `e±digits`, if they're there. Whether either was.
    fn fraction_and_exponent(&mut self) -> bool {
        let b = self.src.as_bytes();
        let mut any = false;
        if b.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            self.digits(10);
            any = true;
        }
        if matches!(b.get(self.pos), Some(b'e' | b'E')) {
            let at = self.pos;
            self.pos += 1;
            if matches!(b.get(self.pos), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits(10) == 0 {
                self.error("missing-exponent", at, "an exponent needs digits");
            }
            any = true;
        }
        any
    }

    /// Digits in `radix`, with `_` separators between two of them. How
    /// many digits there were.
    fn digits(&mut self, radix: u32) -> usize {
        let b = self.src.as_bytes();
        let mut count = 0;
        loop {
            match b.get(self.pos) {
                Some(&c) if (c as char).is_digit(radix) => {
                    count += 1;
                    self.pos += 1;
                }
                Some(b'_') => {
                    let next_digit = b
                        .get(self.pos + 1)
                        .is_some_and(|&c| (c as char).is_digit(radix));
                    let prev_digit = count > 0 && b[self.pos - 1] != b'_';
                    if !(prev_digit && next_digit) {
                        let at = self.pos;
                        self.pos += 1;
                        self.error(
                            "invalid-numeric-separator",
                            at,
                            "a _ in a number has to be between two digits",
                        );
                        if !next_digit {
                            return count;
                        }
                    } else {
                        self.pos += 1;
                    }
                }
                _ => return count,
            }
        }
    }

    // String literals (12.9.4, B.1.2).

    fn string(&mut self, start: usize, quote: u8) -> Token {
        let b = self.src.as_bytes();
        let mut flags = Flags::empty();
        self.pos = start + 1;
        loop {
            match b.get(self.pos) {
                None | Some(b'\n' | b'\r') => {
                    self.error(
                        "unterminated-string",
                        start,
                        "a string has no closing quote on its line",
                    );
                    break;
                }
                Some(&c) if c == quote => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    flags.insert(Flags::ESCAPED);
                    self.pos += 1;
                    self.string_escape(&mut flags);
                }
                Some(_) => self.pos += 1,
            }
        }
        self.token(Kind::String, start, flags)
    }

    /// After a `\` in a string.
    fn string_escape(&mut self, flags: &mut Flags) {
        let b = self.src.as_bytes();
        let at = self.pos - 1;
        match b.get(self.pos) {
            None => {}
            Some(b'\r') => {
                self.pos += 1;
                if b.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
            }
            Some(b'x') => {
                self.pos += 1;
                if self.hex_digits(2).is_none() {
                    self.error("invalid-escape", at, "\\x needs two hex digits");
                }
            }
            Some(b'u') => {
                self.pos += 1;
                if self.unicode_escape().is_none() {
                    self.error(
                        "invalid-escape",
                        at,
                        "\\u needs four hex digits, or hex digits in braces up to 10FFFF",
                    );
                }
            }
            Some(b'0') if !b.get(self.pos + 1).is_some_and(u8::is_ascii_digit) => self.pos += 1,
            Some(&c @ b'0'..=b'7') => {
                // ZeroToThree OctalDigit OctalDigit at most, or FourToSeven
                // OctalDigit.
                let max = if c <= b'3' { 3 } else { 2 };
                let mut n = 0;
                while n < max && b.get(self.pos).is_some_and(|c| (b'0'..=b'7').contains(c)) {
                    self.pos += 1;
                    n += 1;
                }
                flags.insert(Flags::LEGACY_OCTAL);
                self.legacy(
                    "legacy-octal-escape",
                    at,
                    "octal escapes like \\01 aren't allowed in strict code",
                );
            }
            Some(b'8' | b'9') => {
                self.pos += 1;
                flags.insert(Flags::LEGACY_OCTAL);
                self.legacy(
                    "legacy-octal-escape",
                    at,
                    "\\8 and \\9 aren't allowed in strict code",
                );
            }
            Some(_) => self.pos += self.char_at(self.pos).len_utf8(),
        }
    }

    // Templates (12.9.6).

    /// From `` ` `` or, for a middle or tail, from `}`.
    fn template(&mut self, start: usize, continued: bool) -> Token {
        let b = self.src.as_bytes();
        let mut flags = Flags::empty();
        self.pos = start + 1;
        let kind = loop {
            match b.get(self.pos) {
                None => {
                    self.error(
                        "unterminated-template",
                        start,
                        "a template has no closing `",
                    );
                    break if continued {
                        Kind::TemplateTail
                    } else {
                        Kind::Template
                    };
                }
                Some(b'`') => {
                    self.pos += 1;
                    break if continued {
                        Kind::TemplateTail
                    } else {
                        Kind::Template
                    };
                }
                Some(b'$') if b.get(self.pos + 1) == Some(&b'{') => {
                    self.pos += 2;
                    break if continued {
                        Kind::TemplateMiddle
                    } else {
                        Kind::TemplateHead
                    };
                }
                Some(b'\\') => {
                    flags.insert(Flags::ESCAPED);
                    self.pos += 1;
                    if !self.template_escape() {
                        flags.insert(Flags::INVALID_ESCAPE);
                    }
                }
                Some(_) => self.pos += 1,
            }
        };
        self.token(kind, start, flags)
    }

    /// After a `\` in a template: whether it's an escape sequence and not
    /// a NotEscapeSequence.
    fn template_escape(&mut self) -> bool {
        let b = self.src.as_bytes();
        match b.get(self.pos) {
            None => true,
            Some(b'x') => {
                self.pos += 1;
                self.hex_digits(2).is_some()
            }
            Some(b'u') => {
                self.pos += 1;
                self.unicode_escape().is_some()
            }
            Some(b'0') if !b.get(self.pos + 1).is_some_and(u8::is_ascii_digit) => {
                self.pos += 1;
                true
            }
            // `\0` before a digit, and `\1` to `\9`: the digit is the
            // escape, and what follows is template text again.
            Some(b'0'..=b'9') => {
                self.pos += 1;
                false
            }
            // A line continuation, CRLF included, or any other character.
            Some(_) => {
                self.pos += self.char_at(self.pos).len_utf8();
                if b[self.pos - 1] == b'\r' && b.get(self.pos) == Some(&b'\n') {
                    self.pos += 1;
                }
                true
            }
        }
    }

    // Regular expression literals (12.9.5).

    fn regexp(&mut self, start: usize) -> Token {
        let b = self.src.as_bytes();
        self.pos = start + 1;
        let mut class = false;
        loop {
            match b.get(self.pos) {
                None | Some(b'\n' | b'\r') => {
                    self.error(
                        "unterminated-regexp",
                        start,
                        "a regular expression has no closing / on its line",
                    );
                    return self.token(Kind::RegExp, start, Flags::empty());
                }
                Some(0xE2) if is_separator(&b[self.pos..]) => {
                    self.error(
                        "unterminated-regexp",
                        start,
                        "a regular expression has no closing / on its line",
                    );
                    return self.token(Kind::RegExp, start, Flags::empty());
                }
                Some(b'\\') => {
                    self.pos += 1;
                    match b.get(self.pos) {
                        None | Some(b'\n' | b'\r') => {}
                        Some(0xE2) if is_separator(&b[self.pos..]) => {}
                        Some(_) => self.pos += self.char_at(self.pos).len_utf8(),
                    }
                }
                Some(b'[') => {
                    class = true;
                    self.pos += 1;
                }
                Some(b']') => {
                    class = false;
                    self.pos += 1;
                }
                Some(b'/') if !class => {
                    self.pos += 1;
                    break;
                }
                Some(_) => self.pos += 1,
            }
        }
        // The flags: IdentifierPartChars, of which only dgimsuyv mean
        // anything, each once, and not u with v.
        let flags_at = self.pos;
        let mut seen = 0u8;
        let mut bad = false;
        while let Some(&c) = b.get(self.pos) {
            if c == b'\\' {
                let at = self.pos;
                self.pos += 1;
                if b.get(self.pos) == Some(&b'u') {
                    self.pos += 1;
                    self.unicode_escape();
                }
                self.error(
                    "invalid-regexp-flags",
                    at,
                    "regular expression flags can't be escaped",
                );
                continue;
            }
            let ch = self.char_at(self.pos);
            if !(c < 0x80 && ID_CONTINUE_ASCII[c as usize] || c >= 0x80 && is_id_continue(ch)) {
                break;
            }
            let bit = "dgimsuyv".find(ch).map(|i| 1u8 << i);
            match bit {
                Some(bit) if seen & bit == 0 => seen |= bit,
                _ => bad = true,
            }
            self.pos += ch.len_utf8();
        }
        if bad || seen & 0b1010_0000 == 0b1010_0000 {
            self.error(
                "invalid-regexp-flags",
                flags_at,
                "regular expression flags are d, g, i, m, s, u, v and y, each at most once, and not both u and v",
            );
        }
        self.token(Kind::RegExp, start, Flags::empty())
    }

    // Punctuators (12.8).

    /// The longest punctuator at `start`, other than `/`, `/=` and `}`
    /// with a template goal, which the caller has handled.
    fn punctuator(&mut self, start: usize) -> Option<Punct> {
        let b = &self.src.as_bytes()[start..];
        let at = |i: usize| b.get(i).copied().unwrap_or(0);
        use Punct::*;
        let (len, p) = match (b[0], at(1), at(2), at(3)) {
            (b'{', ..) => (1, LBrace),
            (b'}', ..) => (1, RBrace),
            (b'(', ..) => (1, LParen),
            (b')', ..) => (1, RParen),
            (b'[', ..) => (1, LBracket),
            (b']', ..) => (1, RBracket),
            (b';', ..) => (1, Semicolon),
            (b',', ..) => (1, Comma),
            (b':', ..) => (1, Colon),
            (b'~', ..) => (1, Tilde),
            (b'.', b'.', b'.', _) => (3, Ellipsis),
            (b'.', ..) => (1, Dot),
            (b'<', b'<', b'=', _) => (3, ShlEq),
            (b'<', b'<', ..) => (2, Shl),
            (b'<', b'=', ..) => (2, LtEq),
            (b'<', ..) => (1, Lt),
            (b'>', b'>', b'>', b'=') => (4, UShrEq),
            (b'>', b'>', b'>', _) => (3, UShr),
            (b'>', b'>', b'=', _) => (3, ShrEq),
            (b'>', b'>', ..) => (2, Shr),
            (b'>', b'=', ..) => (2, GtEq),
            (b'>', ..) => (1, Gt),
            (b'=', b'=', b'=', _) => (3, EqEqEq),
            (b'=', b'=', ..) => (2, EqEq),
            (b'=', b'>', ..) => (2, Arrow),
            (b'=', ..) => (1, Eq),
            (b'!', b'=', b'=', _) => (3, NotEqEq),
            (b'!', b'=', ..) => (2, NotEq),
            (b'!', ..) => (1, Bang),
            (b'+', b'+', ..) => (2, PlusPlus),
            (b'+', b'=', ..) => (2, PlusEq),
            (b'+', ..) => (1, Plus),
            (b'-', b'-', ..) => (2, MinusMinus),
            (b'-', b'=', ..) => (2, MinusEq),
            (b'-', ..) => (1, Minus),
            (b'*', b'*', b'=', _) => (3, StarStarEq),
            (b'*', b'*', ..) => (2, StarStar),
            (b'*', b'=', ..) => (2, StarEq),
            (b'*', ..) => (1, Star),
            (b'%', b'=', ..) => (2, PercentEq),
            (b'%', ..) => (1, Percent),
            (b'&', b'&', b'=', _) => (3, AmpAmpEq),
            (b'&', b'&', ..) => (2, AmpAmp),
            (b'&', b'=', ..) => (2, AmpEq),
            (b'&', ..) => (1, Amp),
            (b'|', b'|', b'=', _) => (3, PipePipeEq),
            (b'|', b'|', ..) => (2, PipePipe),
            (b'|', b'=', ..) => (2, PipeEq),
            (b'|', ..) => (1, Pipe),
            (b'^', b'=', ..) => (2, CaretEq),
            (b'^', ..) => (1, Caret),
            (b'?', b'?', b'=', _) => (3, QuestionQuestionEq),
            (b'?', b'?', ..) => (2, QuestionQuestion),
            // `?.` isn't optional chaining before a digit: `a?.5:b`.
            (b'?', b'.', c, _) if !c.is_ascii_digit() => (2, QuestionDot),
            (b'?', ..) => (1, Question),
            _ => return None,
        };
        self.pos += len;
        Some(p)
    }
}

/// U+2028 or U+2029 at the start of `b`.
fn is_separator(b: &[u8]) -> bool {
    b.len() >= 3 && b[0] == 0xE2 && b[1] == 0x80 && (b[2] == 0xA8 || b[2] == 0xA9)
}

/// WhiteSpace above ASCII: ZWNBSP and the Space_Separator characters.
fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}' | '\u{FEFF}'
    )
}

/// IdentifierPart in ASCII: letters, digits, `$` and `_`.
static ID_CONTINUE_ASCII: [bool; 128] = {
    let mut t = [false; 128];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        t[c] = b.is_ascii_alphanumeric() || b == b'$' || b == b'_';
        c += 1;
    }
    t
};

fn in_table(table: &[u32], c: u32) -> bool {
    // The last range starting at or before `c`.
    let pairs = table.len() / 2;
    let i = {
        let (mut lo, mut hi) = (0, pairs);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if table[2 * mid] <= c {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    };
    i > 0 && c <= table[2 * (i - 1) + 1]
}

/// IdentifierStartChar without `\`: ID_Start, `$` or `_`.
pub(crate) fn is_id_start(c: char) -> bool {
    match c {
        'a'..='z' | 'A'..='Z' | '$' | '_' => true,
        '\0'..='\x7F' => false,
        _ => in_table(unicode::ID_START, c as u32),
    }
}

/// IdentifierPartChar without `\`: ID_Continue, `$`, ZWNJ or ZWJ.
pub(crate) fn is_id_continue(c: char) -> bool {
    match c {
        '\0'..='\x7F' => ID_CONTINUE_ASCII[c as usize],
        '\u{200C}' | '\u{200D}' => true,
        _ => in_table(unicode::ID_CONTINUE, c as u32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_tables() {
        for c in [
            'é',
            'Ω',
            'ж',
            '中',
            'ㄱ',
            '\u{2118}',
            '\u{1E900}',
            '\u{30000}',
        ] {
            assert!(is_id_start(c), "{c:?}");
            assert!(is_id_continue(c), "{c:?}");
        }
        // Continue only: combining marks and digits of other scripts.
        for c in ['\u{300}', '\u{660}', '\u{200C}', '\u{200D}', '\u{B7}'] {
            assert!(!is_id_start(c) && is_id_continue(c), "{c:?}");
        }
        for c in ['\u{A0}', '€', '😀', '\u{2028}', '\u{FEFF}', '\u{10FFFF}'] {
            assert!(!is_id_start(c) && !is_id_continue(c), "{c:?}");
        }
        // The tables are sorted ranges.
        for t in [unicode::ID_START, unicode::ID_CONTINUE] {
            assert!(t.len() % 2 == 0);
            assert!(t.windows(2).all(|w| w[0] <= w[1]));
        }
    }
}
