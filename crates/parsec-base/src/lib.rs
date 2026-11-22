// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! What Parsec's parsers share, and nothing they don't: a [`Span`] of
//! input, a [`LineIndex`] that turns byte offsets into lines, columns and
//! UTF-16 offsets, a [`Diagnostic`] with a code and a span, and the
//! [`Limits`] and [`Budget`] that keep hostile input from costing more
//! than you allow.
//!
//! Each language keeps its own tokens, tree and error recovery; this
//! crate is only the plumbing under them, and it has no dependencies.
//!
//! ```
//! use parsec_base::{Diagnostic, LineIndex, Span};
//!
//! let src = "let a = 1;\nlet é = 'x\n";
//! let bad = Diagnostic::error("unterminated-string", Span::new(20, 22), "a string has no closing quote");
//! let lines = LineIndex::new(src);
//! assert_eq!(lines.line_col(bad.span.start), (2, 9).into());
//! assert_eq!(bad.span.text(src), "'x");
//! ```

use std::borrow::Cow;
use std::fmt;
use std::ops::Range;

/// A byte range in the input, `start..end`.
///
/// Offsets are `u32`, so a span is 8 bytes and a token that carries one
/// stays small; a parser that uses it takes at most [`Span::MAX_INPUT`]
/// bytes of input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    /// The longest input a `u32` offset can cover.
    pub const MAX_INPUT: usize = u32::MAX as usize;

    pub const fn new(start: u32, end: u32) -> Span {
        debug_assert!(start <= end);
        Span { start, end }
    }

    /// An empty span at `offset`, for something that's missing there.
    pub const fn at(offset: u32) -> Span {
        Span {
            start: offset,
            end: offset,
        }
    }

    pub const fn len(self) -> u32 {
        self.end - self.start
    }

    pub const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Whether `offset` is inside, the end excluded.
    pub const fn contains(self, offset: u32) -> bool {
        self.start <= offset && offset < self.end
    }

    /// The smallest span covering both.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub const fn range(self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// The text this span covers in `src`, or `""` if it isn't a range of
    /// whole characters there.
    pub fn text(self, src: &str) -> &str {
        src.get(self.range()).unwrap_or("")
    }
}

impl From<Span> for Range<usize> {
    fn from(s: Span) -> Range<usize> {
        s.range()
    }
}

/// A line and a column, both counted from 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LineCol {
    pub line: u32,
    pub column: u32,
}

impl From<(u32, u32)> for LineCol {
    fn from((line, column): (u32, u32)) -> LineCol {
        LineCol { line, column }
    }
}

impl fmt::Display for LineCol {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.column)
    }
}

/// Bytes between the checkpoints that keep a column lookup on a long line
/// (a minified script is one line) from counting the whole line.
const BLOCK: usize = 1024;

/// Where each line starts in a text, for turning byte offsets into lines
/// and columns, in characters or in UTF-16 code units as editors and the
/// language server protocol count them.
///
/// Building it is one pass over the text. A lookup is a binary search for
/// the line and a count of at most a thousand bytes for the column.
#[derive(Clone, Debug)]
pub struct LineIndex<'a> {
    src: &'a str,
    /// The byte offset each line starts at; the first is 0.
    starts: Vec<u32>,
    /// Characters, and UTF-16 code units, before each `BLOCK`-byte block.
    chars: Vec<u32>,
    utf16: Vec<u32>,
}

impl<'a> LineIndex<'a> {
    /// Lines ending at LF, CR or CRLF, as HTML, CSS and XML count them.
    pub fn new(src: &'a str) -> LineIndex<'a> {
        LineIndex::build(src, false)
    }

    /// Lines ending at LF, CR, CRLF, U+2028 or U+2029: ECMAScript's
    /// LineTerminatorSequence.
    pub fn ecmascript(src: &'a str) -> LineIndex<'a> {
        LineIndex::build(src, true)
    }

    fn build(src: &'a str, separators: bool) -> LineIndex<'a> {
        assert!(
            src.len() <= Span::MAX_INPUT,
            "LineIndex takes at most 4 GiB"
        );
        let b = src.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\n' => starts.push(i as u32 + 1),
                b'\r' if b.get(i + 1) != Some(&b'\n') => starts.push(i as u32 + 1),
                // U+2028 and U+2029 are E2 80 A8 and E2 80 A9.
                0xE2 if separators
                    && b.get(i + 1) == Some(&0x80)
                    && matches!(b.get(i + 2), Some(0xA8 | 0xA9)) =>
                {
                    starts.push(i as u32 + 3);
                    i += 2;
                }
                _ => {}
            }
            i += 1;
        }
        let (mut chars, mut utf16) = (Vec::new(), Vec::new());
        let (mut c, mut u) = (0, 0);
        for block in b.chunks(BLOCK) {
            chars.push(c);
            utf16.push(u);
            let (bc, bu) = count(block);
            c += bc;
            u += bu;
        }
        chars.push(c);
        utf16.push(u);
        LineIndex {
            src,
            starts,
            chars,
            utf16,
        }
    }

    pub fn line_count(&self) -> u32 {
        self.starts.len() as u32
    }

    /// The byte offset line `line` (from 1) starts at.
    pub fn line_start(&self, line: u32) -> Option<u32> {
        self.starts.get(line.checked_sub(1)? as usize).copied()
    }

    /// The line `offset` is on, from 1. An offset past the end is on the
    /// last line, and a line terminator is on the line it ends.
    pub fn line(&self, offset: u32) -> u32 {
        self.starts.partition_point(|&s| s <= offset) as u32
    }

    /// The line and column of `offset`, the column in characters (Unicode
    /// scalar values). An offset inside a character counts as after it.
    pub fn line_col(&self, offset: u32) -> LineCol {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1];
        let column = 1 + self.before(offset).0 - self.before(start).0;
        LineCol { line, column }
    }

    /// The line and column of `offset`, the column in UTF-16 code units,
    /// as JavaScript strings and source maps count it.
    pub fn line_col_utf16(&self, offset: u32) -> LineCol {
        let line = self.line(offset);
        let start = self.starts[line as usize - 1];
        let column = 1 + self.before(offset).1 - self.before(start).1;
        LineCol { line, column }
    }

    /// `offset` as an offset in UTF-16 code units from the start of the
    /// text: where it is in a JavaScript string of the same text.
    pub fn utf16_offset(&self, offset: u32) -> u32 {
        self.before(offset).1
    }

    /// Characters and UTF-16 code units before `offset`.
    fn before(&self, offset: u32) -> (u32, u32) {
        let at = (offset as usize).min(self.src.len());
        let block = at / BLOCK;
        let (c, u) = count(&self.src.as_bytes()[block * BLOCK..at]);
        (self.chars[block] + c, self.utf16[block] + u)
    }
}

/// Characters and UTF-16 code units in some UTF-8: one of each per byte
/// that isn't a continuation byte, and a second code unit for a four-byte
/// character.
fn count(bytes: &[u8]) -> (u32, u32) {
    let (mut c, mut u) = (0, 0);
    for &b in bytes {
        if b & 0xC0 != 0x80 {
            c += 1;
            u += 1 + u32::from(b >= 0xF0);
        }
    }
    (c, u)
}

/// How bad a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Severity {
    /// The input breaks the grammar; what the parser gives back is its
    /// recovery.
    Error,
    /// The input is allowed but worth knowing about, like a legacy form
    /// strict code would reject.
    Warning,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
        })
    }
}

/// A second place a [`Diagnostic`] points at, with what it is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Label {
    pub span: Span,
    pub message: Cow<'static, str>,
}

/// Something a parser has to say about its input: a stable `code` to
/// match on (like `unterminated-string`), where, and a sentence for
/// people. With a `'static` message and no labels, making one allocates
/// nothing.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub span: Span,
    pub message: Cow<'static, str>,
    pub labels: Vec<Label>,
}

impl Diagnostic {
    pub fn new(
        code: &'static str,
        severity: Severity,
        span: Span,
        message: impl Into<Cow<'static, str>>,
    ) -> Diagnostic {
        Diagnostic {
            code,
            severity,
            span,
            message: message.into(),
            labels: Vec::new(),
        }
    }

    pub fn error(
        code: &'static str,
        span: Span,
        message: impl Into<Cow<'static, str>>,
    ) -> Diagnostic {
        Diagnostic::new(code, Severity::Error, span, message)
    }

    pub fn warning(
        code: &'static str,
        span: Span,
        message: impl Into<Cow<'static, str>>,
    ) -> Diagnostic {
        Diagnostic::new(code, Severity::Warning, span, message)
    }

    /// The same, pointing at `span` too.
    pub fn with_label(mut self, span: Span, message: impl Into<Cow<'static, str>>) -> Diagnostic {
        self.labels.push(Label {
            span,
            message: message.into(),
        });
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    /// `error[code] at 12..14: message`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}[{}] at {}..{}: {}",
            self.severity, self.code, self.span.start, self.span.end, self.message
        )
    }
}

impl std::error::Error for Diagnostic {}

/// The most a parse may cost. `None` is no limit.
///
/// ```
/// use parsec_base::Limits;
///
/// let limits = Limits { max_input: Some(1 << 20), ..Limits::default() };
/// assert!(limits.check_input(2 << 20).is_err());
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Limits {
    /// The longest input in bytes. Parsers using [`Span`] never take more
    /// than [`Span::MAX_INPUT`], whatever this says.
    pub max_input: Option<usize>,
    /// How deep the parser's tree may nest.
    pub max_depth: Option<u32>,
    /// The work a parse may do, in the steps its parser counts.
    pub max_steps: Option<u64>,
}

impl Limits {
    /// An `input-too-large` error if `len` bytes are over the limit, or
    /// over [`Span::MAX_INPUT`].
    pub fn check_input(&self, len: usize) -> Result<(), Diagnostic> {
        let max = self.max_input.unwrap_or(usize::MAX).min(Span::MAX_INPUT);
        if len <= max {
            return Ok(());
        }
        Err(Diagnostic::error(
            "input-too-large",
            Span::at(0),
            format!("the input is {len} bytes, and the limit is {max}"),
        ))
    }

    /// Whether depth `depth` is past the limit.
    pub fn too_deep(&self, depth: u32) -> bool {
        self.max_depth.is_some_and(|max| depth > max)
    }

    /// A budget of [`Limits::max_steps`].
    pub fn budget(&self) -> Budget {
        Budget {
            left: self.max_steps,
        }
    }
}

/// Steps left to spend on a parse. Parsers spend from it on the work
/// whose cost the input controls and stop when it runs out, so the cost
/// of a hostile input is one you chose.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Budget {
    left: Option<u64>,
}

impl Budget {
    pub const fn new(steps: u64) -> Budget {
        Budget { left: Some(steps) }
    }

    pub const fn unlimited() -> Budget {
        Budget { left: None }
    }

    /// Spends `steps`, or fails, spending nothing, if fewer are left.
    pub fn spend(&mut self, steps: u64) -> Result<(), Exhausted> {
        match &mut self.left {
            None => Ok(()),
            Some(left) if *left >= steps => {
                *left -= steps;
                Ok(())
            }
            Some(_) => Err(Exhausted),
        }
    }

    /// The steps left, or `None` without a limit.
    pub const fn left(&self) -> Option<u64> {
        self.left
    }
}

/// A [`Budget`] ran out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Exhausted;

impl fmt::Display for Exhausted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the step budget ran out")
    }
}

impl std::error::Error for Exhausted {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn span_basics() {
        let s = Span::new(2, 5);
        assert_eq!((s.len(), s.is_empty()), (3, false));
        assert!(s.contains(2) && s.contains(4) && !s.contains(5));
        assert!(Span::at(7).is_empty());
        assert_eq!(s.to(Span::new(8, 9)), Span::new(2, 9));
        assert_eq!(Span::new(8, 9).to(s), Span::new(2, 9));
        assert_eq!(Range::<usize>::from(s), 2..5);
        assert_eq!(s.text("abcdefg"), "cde");
        // Not on character boundaries, or past the end: nothing.
        assert_eq!(Span::new(1, 2).text("é"), "");
        assert_eq!(Span::new(1, 20).text("abc"), "");
        assert_eq!(std::mem::size_of::<Span>(), 8);
    }

    /// Line and column by counting from the start, the slow way.
    fn slow(src: &str, separators: bool, offset: usize) -> (u32, u32, u32, u32) {
        let (mut line, mut col, mut col16, mut off16) = (1, 1, 1, 0);
        let mut chars = src.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if i >= offset {
                break;
            }
            off16 += c.len_utf16() as u32;
            let crlf = c == '\r' && chars.peek().is_some_and(|&(_, n)| n == '\n');
            if (c == '\n'
                || c == '\r' && !crlf
                || separators && matches!(c, '\u{2028}' | '\u{2029}'))
                && i + c.len_utf8() <= offset
            {
                line += 1;
                col = 1;
                col16 = 1;
            } else {
                col += 1;
                col16 += c.len_utf16() as u32;
            }
        }
        (line, col, col16, off16)
    }

    #[test]
    fn line_col_agrees_with_counting() {
        // Every line end, characters of every width, and a line longer
        // than a block, so checkpoints and line starts fall apart.
        let src = "a\r\nb\rc\nd\u{2028}e\u{2029}é€😀\n".repeat(40) + &"😀x".repeat(700) + "\r\nend";
        for separators in [false, true] {
            let index = if separators {
                LineIndex::ecmascript(&src)
            } else {
                LineIndex::new(&src)
            };
            for (offset, _) in src.char_indices().chain([(src.len(), ' ')]) {
                let (line, col, col16, off16) = slow(&src, separators, offset);
                let o = offset as u32;
                assert_eq!(
                    index.line_col(o),
                    (line, col).into(),
                    "{offset} {separators}"
                );
                assert_eq!(index.line_col_utf16(o), (line, col16).into(), "{offset}");
                assert_eq!(index.utf16_offset(o), off16, "{offset}");
            }
        }
    }

    #[test]
    fn line_starts() {
        let index = LineIndex::new("ab\r\ncd\ref\n");
        assert_eq!(index.line_count(), 4);
        assert_eq!(index.line_start(1), Some(0));
        assert_eq!(index.line_start(2), Some(4));
        assert_eq!(index.line_start(3), Some(7));
        assert_eq!(index.line_start(4), Some(10));
        assert_eq!((index.line_start(0), index.line_start(5)), (None, None));
        // The CR and the LF of a CRLF are both on the line they end.
        assert_eq!((index.line(2), index.line(3), index.line(4)), (1, 1, 2));
        // Past the end is the last line.
        assert_eq!(index.line(1000), 4);
        // Without separators, U+2028 is an ordinary character.
        assert_eq!(LineIndex::new("a\u{2028}b").line_count(), 1);
        assert_eq!(LineIndex::ecmascript("a\u{2028}b").line_count(), 2);
        assert_eq!(LineIndex::new("").line_col(0), (1, 1).into());
    }

    #[test]
    fn diagnostics() {
        let d = Diagnostic::error("bad-thing", Span::new(3, 4), "a bad thing")
            .with_label(Span::new(0, 1), "started here");
        assert!(d.is_error());
        assert_eq!(d.labels[0].message, "started here");
        assert_eq!(d.to_string(), "error[bad-thing] at 3..4: a bad thing");
        let w = Diagnostic::warning("odd", Span::at(0), String::from("odd"));
        assert_eq!(
            (w.is_error(), w.severity.to_string()),
            (false, "warning".into())
        );
        assert!(Severity::Error < Severity::Warning);
    }

    #[test]
    fn limits_and_budget() {
        let none = Limits::default();
        assert!(none.check_input(1 << 30).is_ok());
        assert!(none.check_input(Span::MAX_INPUT + 1).is_err());
        assert!(!none.too_deep(u32::MAX));
        let l = Limits {
            max_input: Some(10),
            max_depth: Some(2),
            max_steps: Some(5),
        };
        assert!(l.check_input(10).is_ok());
        assert_eq!(l.check_input(11).unwrap_err().code, "input-too-large");
        assert!(!l.too_deep(2) && l.too_deep(3));
        let mut b = l.budget();
        assert_eq!(b.spend(3), Ok(()));
        assert_eq!(b.spend(3), Err(Exhausted));
        assert_eq!(b.left(), Some(2));
        assert_eq!(b.spend(2), Ok(()));
        assert_eq!(b.spend(1), Err(Exhausted));
        let mut u = Budget::unlimited();
        assert_eq!((u.spend(u64::MAX), u.left()), (Ok(()), None));
        assert_eq!(Budget::new(1).left(), Some(1));
    }
}
