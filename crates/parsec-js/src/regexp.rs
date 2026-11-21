// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Regular expression patterns, checked by their own grammar (ECMA-262
//! 22.2.1) and its early errors, with Annex B's additions (B.1.2) for
//! patterns without the `u` or `v` flag.
//!
//! The lexer only finds where a regular expression literal ends; what's
//! between the slashes is a language of its own. [`validate`] reads it
//! the way a `RegExp` would be built from it and says whether that would
//! throw, without building anything.
//!
//! ```
//! use parsec_js::regexp::validate;
//!
//! assert!(validate(r"(?<year>\d{4})-\k<year>", "u").is_ok());
//! assert!(validate(r"[\p{L}--\p{Lu}]", "v").is_ok());
//! assert_eq!(validate(r"a{1}{2}", "").unwrap_err().code, "regexp-nothing-to-repeat");
//!
//! let err = validate(r"[z-a]", "").unwrap_err();
//! assert_eq!((err.code, err.span.start, err.span.end), ("regexp-range-order", 1, 4));
//! ```
//!
//! Each error has a stable code:
//!
//! | Code | What's wrong |
//! |---|---|
//! | `regexp-flags` | a flag that isn't `dgimsuyv`, one twice, or `u` with `v` |
//! | `regexp-nothing-to-repeat` | a quantifier after nothing, an assertion or another quantifier |
//! | `regexp-quantifier-range` | `{n,m}` with `n` more than `m` |
//! | `regexp-lone-bracket` | in `u` or `v` mode, a `]`, `{` or `}` that isn't escaped; anywhere, a `)` with no `(` |
//! | `regexp-unterminated` | a group or class that isn't closed |
//! | `regexp-group` | `(?` followed by something that isn't a group |
//! | `regexp-modifiers` | an `(?ims-ims:` with a flag twice, an unknown one, or none |
//! | `regexp-group-name` | a group name that isn't an identifier |
//! | `regexp-duplicate-group-name` | two groups of one name that could both match |
//! | `regexp-unknown-group-name` | `\k<name>` with no group of that name |
//! | `regexp-backreference` | in `u` or `v` mode, `\n` with fewer than `n` groups |
//! | `regexp-escape` | an escape the mode doesn't have |
//! | `regexp-range-order` | a class range whose start is after its end |
//! | `regexp-range-class` | in `u` or `v` mode, a class range with `\d` or the like at an end |
//! | `regexp-property` | a `\p{...}` that ECMAScript doesn't list |
//! | `regexp-negated-strings` | `\P{...}` or `[^...]` of something that can match a string |
//! | `regexp-class-syntax` | in `v` mode, a class that mixes operators, or has a character only an escape can be |
//! | `too-deep` | groups and classes nested past [`DEFAULT_MAX_DEPTH`](crate::DEFAULT_MAX_DEPTH) |

mod names;

use std::collections::HashMap;

use parsec_base::{Diagnostic, Span};

use crate::lexer::{is_id_continue, is_id_start};

/// Checks `pattern` with `flags` as a regular expression would: the
/// error a `RegExp` built from them would throw, if any. Spans are byte
/// offsets in `pattern`.
pub fn validate(pattern: &str, flags: &str) -> Result<(), Diagnostic> {
    let mut seen = 0u8;
    for (i, c) in flags.char_indices() {
        let bit = "dgimsuyv".find(c).map(|b| 1u8 << b);
        match bit {
            Some(bit) if seen & bit == 0 => seen |= bit,
            _ => {
                return Err(Diagnostic::error(
                    "regexp-flags",
                    Span::new(i as u32, (i + c.len_utf8()) as u32),
                    "regular expression flags are d, g, i, m, s, u, v and y, each at most once",
                ));
            }
        }
    }
    let unicode = flags.contains('u');
    let sets = flags.contains('v');
    if unicode && sets {
        return Err(Diagnostic::error(
            "regexp-flags",
            Span::new(0, flags.len() as u32),
            "a regular expression can't have both the u and v flags",
        ));
    }
    // Without u or v, a pattern is UTF-16 code units; with either, code
    // points.
    let mut src = Vec::with_capacity(pattern.len());
    let mut at = Vec::with_capacity(pattern.len() + 1);
    for (i, c) in pattern.char_indices() {
        if unicode || sets || (c as u32) < 0x10000 {
            src.push(c as u32);
            at.push(i as u32);
        } else {
            let mut units = [0u16; 2];
            for &u in c.encode_utf16(&mut units).iter() {
                src.push(u as u32);
                at.push(i as u32);
            }
        }
    }
    at.push(pattern.len() as u32);
    let (groups, has_names) = count_groups(&src, sets);
    let mut p = Parser {
        src: &src,
        at: &at,
        pos: 0,
        unicode: unicode || sets,
        sets,
        named: unicode || sets || has_names,
        groups,
        names: HashMap::new(),
        references: Vec::new(),
        path: Vec::new(),
        disjunctions: 0,
        depth: 0,
    };
    p.disjunction()?;
    if p.pos < src.len() {
        // Only a `)` stops a disjunction early.
        return p.err(
            "regexp-lone-bracket",
            p.pos,
            p.pos + 1,
            "this `)` has no `(`",
        );
    }
    for &(start, end, ref name) in &p.references {
        if !p.names.contains_key(name) {
            let shown: String = name.iter().filter_map(|&c| char::from_u32(c)).collect();
            return p.err(
                "regexp-unknown-group-name",
                start,
                end,
                format!("there's no group named `{shown}`"),
            );
        }
    }
    Ok(())
}

/// How many capturing groups a pattern has, and whether any is named,
/// counted before parsing, since `\2` may come before the second group.
fn count_groups(src: &[u32], sets: bool) -> (u32, bool) {
    let (mut count, mut named) = (0u32, false);
    let mut class = 0u32;
    let mut i = 0;
    while i < src.len() {
        match char::from_u32(src[i]) {
            Some('\\') => i += 1,
            Some('[') if class == 0 || sets => class += 1,
            Some(']') if class > 0 => class -= 1,
            Some('(') if class == 0 => {
                if src.get(i + 1) != Some(&('?' as u32)) {
                    count = count.saturating_add(1);
                } else if src.get(i + 2) == Some(&('<' as u32))
                    && !matches!(src.get(i + 3), Some(&c) if c == '=' as u32 || c == '!' as u32)
                {
                    count = count.saturating_add(1);
                    named = true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    (count, named)
}

/// What a class atom or operand stands for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Atom {
    /// One character.
    Char(u32),
    /// A set of characters, like `\d`.
    Set,
    /// A set that may hold strings, like `\p{RGI_Emoji}`.
    Strings,
}

struct Parser<'a> {
    src: &'a [u32],
    at: &'a [u32],
    pos: usize,
    /// `[UnicodeMode]`: the u or v flag.
    unicode: bool,
    /// `[UnicodeSetsMode]`: the v flag.
    sets: bool,
    /// `[NamedCaptureGroups]`: u or v, or a named group anywhere.
    named: bool,
    groups: u32,
    /// Each group name, and how many groups of it each alternative and
    /// disjunction holds.
    names: HashMap<Vec<u32>, Counts>,
    /// `\k<name>`s, checked once every group is known.
    references: Vec<(usize, usize, Vec<u32>)>,
    /// The disjunctions around the parse, and which alternative of each.
    path: Vec<(u32, u32)>,
    disjunctions: u32,
    depth: u32,
}

const SYNTAX: &str = "^$\\.*+?()[]{}|";

impl Parser<'_> {
    fn err<T>(
        &self,
        code: &'static str,
        start: usize,
        end: usize,
        message: impl Into<std::borrow::Cow<'static, str>>,
    ) -> Result<T, Diagnostic> {
        let end = end.min(self.src.len()).max(start);
        Err(Diagnostic::error(
            code,
            Span::new(self.at[start], self.at[end]),
            message,
        ))
    }

    fn peek(&self) -> Option<u32> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u32> {
        self.src.get(self.pos + n).copied()
    }

    fn at(&self, c: char) -> bool {
        self.peek() == Some(c as u32)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.at(c) {
            self.pos += 1;
            return true;
        }
        false
    }

    fn enter(&mut self) -> Result<(), Diagnostic> {
        self.depth += 1;
        if self.depth > crate::DEFAULT_MAX_DEPTH {
            return self.err(
                "too-deep",
                self.pos,
                self.pos + 1,
                format!(
                    "this pattern nests deeper than the limit of {}",
                    crate::DEFAULT_MAX_DEPTH
                ),
            );
        }
        Ok(())
    }

    // Only `disjunction`, `term` and `group_body` are on the stack once a
    // level, and each is kept small, so 256 levels fit a 1 MB thread in a
    // debug build.

    fn disjunction(&mut self) -> Result<(), Diagnostic> {
        let id = self.disjunctions;
        self.disjunctions += 1;
        let mut alternative = 0;
        self.path.push((id, alternative));
        while let Some(c) = self.peek() {
            if c == ')' as u32 {
                break;
            }
            if c == '|' as u32 {
                self.pos += 1;
                alternative += 1;
                *self.path.last_mut().expect("pushed above") = (id, alternative);
                continue;
            }
            if let Err(d) = self.term() {
                self.path.pop();
                return Err(d);
            }
        }
        self.path.pop();
        Ok(())
    }

    fn term(&mut self) -> Result<(), Diagnostic> {
        let quantifiable = if self.at('(') {
            let start = self.pos;
            let quantifiable = self.group_open()?;
            self.group_body(start)?;
            quantifiable
        } else {
            self.simple_term()?
        };
        self.after_term(quantifiable)
    }

    /// A group's opening, up to its disjunction: whether the group may be
    /// quantified.
    fn group_open(&mut self) -> Result<bool, Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        if !self.eat('?') {
            return Ok(true);
        }
        let c = self.peek().and_then(char::from_u32);
        let next = self.peek_at(1).and_then(char::from_u32);
        match (c, next) {
            (Some('=' | '!'), _) => {
                self.pos += 1;
                // B.1.2: without u, a lookahead may be quantified.
                Ok(!self.unicode)
            }
            (Some('<'), Some('=' | '!')) => {
                self.pos += 2;
                Ok(false)
            }
            (Some(':'), _) => {
                self.pos += 1;
                Ok(true)
            }
            (Some('<'), _) => {
                self.pos += 1;
                let name_start = self.pos;
                let name = self.group_name()?;
                self.declare(name, name_start)?;
                Ok(true)
            }
            _ => {
                self.modifiers(start)?;
                Ok(true)
            }
        }
    }

    /// A term that isn't a group: whether it may be quantified.
    fn simple_term(&mut self) -> Result<bool, Diagnostic> {
        let start = self.pos;
        let c = self.peek().unwrap_or(0);
        match char::from_u32(c) {
            Some('^' | '$') => {
                self.pos += 1;
                Ok(false)
            }
            Some('\\') if matches!(self.peek_at(1), Some(b) if b == 'b' as u32 || b == 'B' as u32) =>
            {
                self.pos += 2;
                Ok(false)
            }
            Some('*' | '+' | '?') => self.err(
                "regexp-nothing-to-repeat",
                start,
                start + 1,
                "this quantifier has nothing to repeat",
            ),
            Some('{') => {
                if self.unicode {
                    return self.err(
                        "regexp-lone-bracket",
                        start,
                        start + 1,
                        "with the u or v flag, a `{` has to be escaped",
                    );
                }
                if self.braced_quantifier().is_some() {
                    return self.err(
                        "regexp-nothing-to-repeat",
                        start,
                        self.pos,
                        "this quantifier has nothing to repeat",
                    );
                }
                self.pos = start + 1;
                Ok(true)
            }
            _ => {
                self.atom()?;
                Ok(true)
            }
        }
    }

    /// The quantifier after a term, if any.
    fn after_term(&mut self, quantifiable: bool) -> Result<(), Diagnostic> {
        let qstart = self.pos;
        if self.quantifier()? {
            if !quantifiable {
                return self.err(
                    "regexp-nothing-to-repeat",
                    qstart,
                    self.pos,
                    "an assertion can't be repeated",
                );
            }
            self.eat('?');
            if let Some(c) = self.peek()
                && matches!(char::from_u32(c), Some('*' | '+' | '?' | '{'))
            {
                let s = self.pos;
                if c != '{' as u32 || self.unicode || self.braced_quantifier().is_some() {
                    return self.err(
                        "regexp-nothing-to-repeat",
                        s,
                        s + 1,
                        "a quantifier can't repeat another",
                    );
                }
                self.pos = s;
            }
        }
        Ok(())
    }

    /// A quantifier, if one is here.
    fn quantifier(&mut self) -> Result<bool, Diagnostic> {
        match self.peek().and_then(char::from_u32) {
            Some('*' | '+' | '?') => {
                self.pos += 1;
                Ok(true)
            }
            Some('{') => {
                let start = self.pos;
                match self.braced_quantifier() {
                    Some((min, Some(max)))
                        if bigger(&self.src[min.0..min.1], &self.src[max.0..max.1]) =>
                    {
                        self.err(
                            "regexp-quantifier-range",
                            start,
                            self.pos,
                            "this quantifier's minimum is more than its maximum",
                        )
                    }
                    Some(_) => Ok(true),
                    None if self.unicode => self.err(
                        "regexp-lone-bracket",
                        start,
                        start + 1,
                        "with the u or v flag, a `{` has to be escaped",
                    ),
                    None => {
                        self.pos = start;
                        Ok(false)
                    }
                }
            }
            _ => Ok(false),
        }
    }

    /// `{n}`, `{n,}` or `{n,m}`: the digits' ranges. Leaves `pos` after
    /// it if it's one.
    #[allow(clippy::type_complexity)]
    fn braced_quantifier(&mut self) -> Option<((usize, usize), Option<(usize, usize)>)> {
        let start = self.pos;
        self.pos += 1;
        let min = self.digits();
        let result = match min {
            None => None,
            Some(min) if self.eat('}') => Some((min, Some(min))),
            Some(min) if self.eat(',') => {
                if self.eat('}') {
                    Some((min, None))
                } else {
                    match self.digits() {
                        Some(max) if self.eat('}') => Some((min, Some(max))),
                        _ => None,
                    }
                }
            }
            Some(_) => None,
        };
        if result.is_none() {
            self.pos = start;
        }
        result
    }

    fn digits(&mut self) -> Option<(usize, usize)> {
        let start = self.pos;
        while self.peek().is_some_and(is_digit) {
            self.pos += 1;
        }
        (self.pos > start).then_some((start, self.pos))
    }

    fn atom(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        let c = self.peek().unwrap_or(0);
        match char::from_u32(c) {
            Some('.') => self.pos += 1,
            Some('[') => {
                if self.sets {
                    self.class_v()?;
                } else {
                    self.class()?;
                }
            }
            Some('\\') => self.atom_escape()?,
            Some(']' | '}') if self.unicode => {
                return self.err(
                    "regexp-lone-bracket",
                    start,
                    start + 1,
                    "with the u or v flag, this has to be escaped",
                );
            }
            _ => self.pos += 1,
        }
        Ok(())
    }

    /// A group's disjunction and its `)`, from after its opening.
    fn group_body(&mut self, start: usize) -> Result<(), Diagnostic> {
        self.enter()?;
        self.disjunction()?;
        self.depth -= 1;
        if !self.eat(')') {
            return self.err(
                "regexp-unterminated",
                start,
                self.pos,
                "this group isn't closed",
            );
        }
        Ok(())
    }

    /// `(?ims-ims:`, from after the `?`.
    fn modifiers(&mut self, start: usize) -> Result<(), Diagnostic> {
        let mut seen = 0u8;
        let mut count = 0;
        let mut minus = false;
        loop {
            let c = self.peek().and_then(char::from_u32);
            match c {
                Some(f @ ('i' | 'm' | 's')) => {
                    let bit = 1u8 << "ims".find(f).unwrap_or(0);
                    if seen & bit != 0 {
                        self.pos += 1;
                        return self.err(
                            "regexp-modifiers",
                            start,
                            self.pos,
                            "a group's modifiers can't name a flag twice",
                        );
                    }
                    seen |= bit;
                    count += 1;
                    self.pos += 1;
                }
                Some('-') if !minus => {
                    minus = true;
                    self.pos += 1;
                }
                Some(':') if count > 0 => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(':') => {
                    return self.err(
                        "regexp-modifiers",
                        start,
                        self.pos + 1,
                        "`(?-:` needs a flag to add or remove",
                    );
                }
                _ if count > 0 || minus => {
                    return self.err(
                        "regexp-modifiers",
                        start,
                        self.pos + 1,
                        "a group's modifiers are i, m and s, then `:`",
                    );
                }
                _ => {
                    return self.err(
                        "regexp-group",
                        start,
                        self.pos + 1,
                        "`(?` starts no group here",
                    );
                }
            }
        }
    }

    /// Records a group name, which may repeat only in another
    /// alternative.
    ///
    /// Two groups could both take part in a match unless the innermost
    /// node around both is a disjunction, with them in different
    /// alternatives. So a group clashes with an earlier one of its name
    /// in some alternative around it, unless that one is in the same
    /// disjunction inside the alternative as this one: which counts of
    /// each alternative and disjunction tell, in a step per level.
    fn declare(&mut self, name: Vec<u32>, start: usize) -> Result<(), Diagnostic> {
        let counts = self.names.entry(name).or_default();
        let path = &self.path;
        for (k, alt) in path.iter().enumerate() {
            let around = counts.alternatives.get(alt).copied().unwrap_or(0);
            let inner = path
                .get(k + 1)
                .and_then(|next| counts.disjunctions.get(&next.0))
                .copied()
                .unwrap_or(0);
            if around > inner {
                return self.err(
                    "regexp-duplicate-group-name",
                    start,
                    self.pos - 1,
                    "two groups of this name could both match",
                );
            }
        }
        for &(disjunction, alternative) in path {
            *counts
                .alternatives
                .entry((disjunction, alternative))
                .or_default() += 1;
            *counts.disjunctions.entry(disjunction).or_default() += 1;
        }
        Ok(())
    }

    /// A group name and its `>`, from after the `<`.
    fn group_name(&mut self) -> Result<Vec<u32>, Diagnostic> {
        let start = self.pos;
        let mut name = Vec::new();
        loop {
            if self.at('>') {
                if name.is_empty() {
                    return self.err(
                        "regexp-group-name",
                        start,
                        self.pos + 1,
                        "a group name can't be empty",
                    );
                }
                self.pos += 1;
                return Ok(name);
            }
            let at = self.pos;
            let c = if self.eat('\\') {
                if !self.eat('u') {
                    return self.err(
                        "regexp-group-name",
                        start,
                        self.pos,
                        "this isn't a group name",
                    );
                }
                match self.unicode_escape(true) {
                    Some(c) => c,
                    None => {
                        return self.err(
                            "regexp-group-name",
                            start,
                            self.pos,
                            "this isn't a group name",
                        );
                    }
                }
            } else {
                match self.peek() {
                    Some(c) => {
                        self.pos += 1;
                        self.pair(c)
                    }
                    None => {
                        return self.err(
                            "regexp-group-name",
                            start,
                            self.pos,
                            "this group name isn't closed",
                        );
                    }
                }
            };
            let ok = char::from_u32(c).is_some_and(|ch| {
                if name.is_empty() {
                    is_id_start(ch)
                } else {
                    is_id_continue(ch)
                }
            });
            if !ok {
                return self.err(
                    "regexp-group-name",
                    at,
                    self.pos,
                    "this can't be in a group name",
                );
            }
            name.push(c);
        }
    }

    /// `c`, or with the trail surrogate after it in non-u code units,
    /// the code point they make.
    fn pair(&mut self, c: u32) -> u32 {
        if (0xD800..0xDC00).contains(&c)
            && let Some(t) = self.peek()
            && (0xDC00..0xE000).contains(&t)
        {
            self.pos += 1;
            return 0x10000 + ((c - 0xD800) << 10) + (t - 0xDC00);
        }
        c
    }

    /// After `\u`: `XXXX` (a surrogate pair of two of them combined in
    /// `unicode` mode) or, in `unicode` mode, `{X...}`. `None` if it isn't
    /// one; `pos` is then past what was read.
    fn unicode_escape(&mut self, unicode: bool) -> Option<u32> {
        if unicode && self.eat('{') {
            let mut value: u32 = 0;
            let mut any = false;
            while let Some(d) = self.peek().and_then(hex) {
                value = value.saturating_mul(16).saturating_add(d);
                any = true;
                self.pos += 1;
            }
            if !any || value > 0x10FFFF || !self.eat('}') {
                return None;
            }
            return Some(value);
        }
        let lead = self.hex4()?;
        if unicode
            && (0xD800..0xDC00).contains(&lead)
            && self.peek() == Some('\\' as u32)
            && self.peek_at(1) == Some('u' as u32)
        {
            let save = self.pos;
            self.pos += 2;
            match self.hex4() {
                Some(t) if (0xDC00..0xE000).contains(&t) => {
                    return Some(0x10000 + ((lead - 0xD800) << 10) + (t - 0xDC00));
                }
                _ => self.pos = save,
            }
        }
        Some(lead)
    }

    fn hex4(&mut self) -> Option<u32> {
        let mut value = 0;
        for i in 0..4 {
            value = value * 16 + self.peek_at(i).and_then(hex)?;
        }
        self.pos += 4;
        Some(value)
    }

    /// `\` and an AtomEscape, outside a class.
    fn atom_escape(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        let Some(c) = self.peek() else {
            return self.err(
                "regexp-escape",
                start,
                self.pos,
                "a pattern can't end in `\\`",
            );
        };
        if is_digit(c) && c != '0' as u32 {
            let digits_start = self.pos;
            let mut value: u32 = 0;
            while let Some(d) = self.peek().filter(|&d| is_digit(d)) {
                value = value.saturating_mul(10).saturating_add(d - '0' as u32);
                self.pos += 1;
            }
            if value <= self.groups {
                return Ok(());
            }
            if self.unicode {
                return self.err(
                    "regexp-backreference",
                    start,
                    self.pos,
                    format!("there's no group {value} to refer back to"),
                );
            }
            // B.1.2: a legacy octal escape, or `\8` and `\9` as themselves.
            self.pos = digits_start;
            self.legacy_octal();
            return Ok(());
        }
        if c == 'k' as u32 && self.named {
            self.pos += 1;
            if !self.eat('<') {
                return self.err(
                    "regexp-escape",
                    start,
                    self.pos,
                    "`\\k` needs a group name, `\\k<name>`",
                );
            }
            let name = self.group_name()?;
            self.references.push((start, self.pos, name));
            return Ok(());
        }
        self.escape(false).map(|_| ())
    }

    /// B.1.2's LegacyOctalEscapeSequence, or one digit as itself.
    fn legacy_octal(&mut self) {
        let first = self.peek().unwrap_or(0);
        self.pos += 1;
        if !(('0' as u32)..=('7' as u32)).contains(&first) {
            return;
        }
        let octal = |p: &Self, n| {
            p.peek_at(n)
                .is_some_and(|c| (('0' as u32)..=('7' as u32)).contains(&c))
        };
        if octal(self, 0) {
            self.pos += 1;
            if first <= '3' as u32 && octal(self, 0) {
                self.pos += 1;
            }
        }
    }

    /// An escape after `\` that's a CharacterClassEscape or a
    /// CharacterEscape, in a class or out: what it stands for. `pos` is at
    /// the character after `\`.
    fn escape(&mut self, in_class: bool) -> Result<Atom, Diagnostic> {
        let start = self.pos - 1;
        let c = self.peek().unwrap_or(0);
        self.pos += 1;
        let value = match char::from_u32(c) {
            Some('d' | 'D' | 's' | 'S' | 'w' | 'W') => return Ok(Atom::Set),
            Some(p @ ('p' | 'P')) if self.unicode => return self.property(start, p == 'P'),
            Some('f') => 0x0C,
            Some('n') => 0x0A,
            Some('r') => 0x0D,
            Some('t') => 0x09,
            Some('v') => 0x0B,
            Some('c') => match self.peek() {
                Some(l) if is_ascii_letter(l) => {
                    self.pos += 1;
                    l % 32
                }
                // B.1.2: in a class, `\c` and a digit or `_`.
                Some(l) if in_class && !self.unicode && (is_digit(l) || l == '_' as u32) => {
                    self.pos += 1;
                    l % 32
                }
                _ if self.unicode => {
                    return self.err(
                        "regexp-escape",
                        start,
                        self.pos,
                        "`\\c` needs a letter after it",
                    );
                }
                // B.1.2: `\` is itself, and `c` comes next.
                _ => {
                    self.pos -= 1;
                    '\\' as u32
                }
            },
            Some('0') if !self.peek().is_some_and(is_digit) => 0,
            Some('0'..='7') if !self.unicode => {
                self.pos -= 1;
                let from = self.pos;
                self.legacy_octal();
                let mut v = 0;
                for &d in &self.src[from..self.pos] {
                    v = v * 8 + (d - '0' as u32);
                }
                v
            }
            Some('x') => match (self.peek().and_then(hex), self.peek_at(1).and_then(hex)) {
                (Some(a), Some(b)) => {
                    self.pos += 2;
                    a * 16 + b
                }
                _ if self.unicode => {
                    return self.err(
                        "regexp-escape",
                        start,
                        self.pos,
                        "`\\x` needs two hex digits",
                    );
                }
                _ => 'x' as u32,
            },
            Some('u') => {
                let save = self.pos;
                match self.unicode_escape(self.unicode) {
                    Some(v) => v,
                    None if self.unicode => {
                        return self.err(
                            "regexp-escape",
                            start,
                            self.pos,
                            "this isn't a Unicode escape",
                        );
                    }
                    None => {
                        self.pos = save;
                        'u' as u32
                    }
                }
            }
            Some('b') if in_class => 0x08,
            Some('-') if in_class && self.unicode => '-' as u32,
            _ if self.unicode => {
                let syntax = char::from_u32(c).is_some_and(|ch| SYNTAX.contains(ch) || ch == '/');
                // v's classes also let a ClassSetReservedPunctuator be
                // escaped.
                let reserved = self.sets
                    && in_class
                    && char::from_u32(c).is_some_and(|ch| "&-!#%,:;<=>@`~".contains(ch));
                if !syntax && !reserved {
                    self.pos = start + 1;
                    let end = start + 2;
                    return self.err(
                        "regexp-escape",
                        start,
                        end,
                        "with the u or v flag, this can't be escaped",
                    );
                }
                c
            }
            // B.1.2: anything else is itself, but `\k` with named groups.
            Some('k') if self.named => {
                return self.err(
                    "regexp-escape",
                    start,
                    self.pos,
                    "`\\k` needs a group name, `\\k<name>`",
                );
            }
            _ => c,
        };
        Ok(Atom::Char(value))
    }

    /// `{name}` or `{name=value}` after `\p` or `\P`.
    fn property(&mut self, start: usize, negated: bool) -> Result<Atom, Diagnostic> {
        if !self.eat('{') {
            return self.err(
                "regexp-property",
                start,
                self.pos,
                "`\\p` needs a property, `\\p{...}`",
            );
        }
        let name_start = self.pos;
        while self.peek().is_some_and(|c| c != '}' as u32) {
            self.pos += 1;
        }
        if !self.at('}') {
            return self.err(
                "regexp-property",
                start,
                self.pos,
                "this property escape isn't closed",
            );
        }
        let text: String = self.src[name_start..self.pos]
            .iter()
            .filter_map(|&c| char::from_u32(c))
            .collect();
        self.pos += 1;
        let known = |list: &[&str], s: &str| list.binary_search(&s).is_ok();
        let found = match text.split_once('=') {
            Some(("General_Category" | "gc", v)) => {
                known(names::GENERAL_CATEGORY, v).then_some(Atom::Set)
            }
            Some(("Script" | "sc" | "Script_Extensions" | "scx", v)) => {
                known(names::SCRIPT, v).then_some(Atom::Set)
            }
            Some(_) => None,
            None if known(names::GENERAL_CATEGORY, &text) || known(names::BINARY, &text) => {
                Some(Atom::Set)
            }
            None if self.sets && known(names::STRINGS, &text) => Some(Atom::Strings),
            None => None,
        };
        match found {
            None => self.err(
                "regexp-property",
                start,
                self.pos,
                format!("`{text}` isn't a property ECMAScript knows"),
            ),
            Some(Atom::Strings) if negated => self.err(
                "regexp-negated-strings",
                start,
                self.pos,
                "\\P can't negate a property of strings",
            ),
            Some(atom) => Ok(atom),
        }
    }

    /// A class without the v flag, from `[`.
    fn class(&mut self) -> Result<(), Diagnostic> {
        let start = self.pos;
        self.pos += 1;
        self.eat('^');
        loop {
            let Some(c) = self.peek() else {
                return self.err(
                    "regexp-unterminated",
                    start,
                    self.pos,
                    "this class isn't closed",
                );
            };
            if c == ']' as u32 {
                self.pos += 1;
                return Ok(());
            }
            let first_start = self.pos;
            let first = self.class_atom()?;
            if self.at('-') && self.peek_at(1).is_some_and(|c| c != ']' as u32) {
                self.pos += 1;
                let second = self.class_atom()?;
                match (first, second) {
                    (Atom::Char(a), Atom::Char(b)) if a > b => {
                        return self.err(
                            "regexp-range-order",
                            first_start,
                            self.pos,
                            "this range's start is after its end",
                        );
                    }
                    (Atom::Char(_), Atom::Char(_)) => {}
                    // B.1.2: without u, `[\d-a]` is \d, `-` and `a`.
                    _ if !self.unicode => {}
                    _ => {
                        return self.err(
                            "regexp-range-class",
                            first_start,
                            self.pos,
                            "a range can't start or end with a class like \\d",
                        );
                    }
                }
            }
        }
    }

    fn class_atom(&mut self) -> Result<Atom, Diagnostic> {
        let c = self.peek().unwrap_or(0);
        self.pos += 1;
        if c != '\\' as u32 {
            return Ok(Atom::Char(c));
        }
        if self.pos >= self.src.len() {
            return self.err(
                "regexp-unterminated",
                self.pos - 1,
                self.pos,
                "this class isn't closed",
            );
        }
        // B.1.2: a class may hold `\1` as an octal escape, or `\8` as 8.
        if !self.unicode && self.peek().is_some_and(is_digit) {
            let from = self.pos;
            self.legacy_octal();
            let mut v = 0u32;
            for &d in &self.src[from..self.pos] {
                v = v.saturating_mul(8).saturating_add(d - '0' as u32);
            }
            if self.src[from] >= '8' as u32 {
                v = self.src[from];
            }
            return Ok(Atom::Char(v));
        }
        if self.unicode && self.peek().is_some_and(|d| is_digit(d) && d != '0' as u32) {
            return self.err(
                "regexp-escape",
                self.pos - 1,
                self.pos + 1,
                "a class can't refer back to a group",
            );
        }
        self.escape(true)
    }

    // The v flag's classes.

    /// A class with the v flag, from `[`: whether it may hold strings.
    fn class_v(&mut self) -> Result<bool, Diagnostic> {
        self.enter()?;
        let start = self.pos;
        self.pos += 1;
        let negated = self.eat('^');
        let strings = self.class_set_expression(start)?;
        self.depth -= 1;
        if negated && strings {
            return self.err(
                "regexp-negated-strings",
                start,
                self.pos,
                "a negated class can't hold anything that matches a string",
            );
        }
        Ok(strings && !negated)
    }

    /// ClassSetExpression and the `]`: whether it may hold strings.
    fn class_set_expression(&mut self, start: usize) -> Result<bool, Diagnostic> {
        if self.eat(']') {
            return Ok(false);
        }
        let (first, range) = self.class_set_item()?;
        let op = if self.double('&') {
            Some('&')
        } else if self.double('-') {
            Some('-')
        } else {
            None
        };
        let Some(op) = op else {
            // A union.
            let mut strings = first;
            loop {
                if self.eat(']') {
                    return Ok(strings);
                }
                if self.peek().is_none() {
                    return self.err(
                        "regexp-unterminated",
                        start,
                        self.pos,
                        "this class isn't closed",
                    );
                }
                if self.double('&') || self.double('-') {
                    return self.err(
                        "regexp-class-syntax",
                        self.pos - 2,
                        self.pos,
                        "a class can't mix a union with && or --",
                    );
                }
                strings |= self.class_set_item()?.0;
            }
        };
        if range {
            return self.err(
                "regexp-class-syntax",
                self.pos - 2,
                self.pos,
                "a range can't be an operand of && or --; put it in its own class",
            );
        }
        let mut strings = first;
        loop {
            if op == '&' && self.at('&') {
                return self.err(
                    "regexp-class-syntax",
                    self.pos,
                    self.pos + 1,
                    "&&& isn't an operator",
                );
            }
            let (operand, range) = self.class_set_item()?;
            if range {
                return self.err(
                    "regexp-class-syntax",
                    self.pos - 1,
                    self.pos,
                    "a range can't be an operand of && or --; put it in its own class",
                );
            }
            if op == '&' {
                strings &= operand;
            }
            if self.eat(']') {
                return Ok(strings);
            }
            if self.peek().is_none() {
                return self.err(
                    "regexp-unterminated",
                    start,
                    self.pos,
                    "this class isn't closed",
                );
            }
            if !self.double(op) {
                return self.err(
                    "regexp-class-syntax",
                    self.pos,
                    self.pos + 1,
                    "a class can't mix && and -- or a union; nest one in its own class",
                );
            }
        }
    }

    /// `cc` here, eaten.
    fn double(&mut self, c: char) -> bool {
        if self.at(c) && self.peek_at(1) == Some(c as u32) {
            self.pos += 2;
            return true;
        }
        false
    }

    /// A ClassSetOperand or ClassSetRange: whether it may hold strings,
    /// and whether it was a range.
    fn class_set_item(&mut self) -> Result<(bool, bool), Diagnostic> {
        let start = self.pos;
        match self.class_set_operand()? {
            Atom::Char(a) => {
                if self.at('-') && self.peek_at(1) != Some('-' as u32) {
                    self.pos += 1;
                    let Atom::Char(b) = self.class_set_operand()? else {
                        return self.err(
                            "regexp-range-class",
                            start,
                            self.pos,
                            "a range's ends have to be single characters",
                        );
                    };
                    if a > b {
                        return self.err(
                            "regexp-range-order",
                            start,
                            self.pos,
                            "this range's start is after its end",
                        );
                    }
                    return Ok((false, true));
                }
                Ok((false, false))
            }
            Atom::Set => Ok((false, false)),
            Atom::Strings => Ok((true, false)),
        }
    }

    /// A nested class, `\q{...}`, an escape, or a ClassSetCharacter.
    fn class_set_operand(&mut self) -> Result<Atom, Diagnostic> {
        let start = self.pos;
        let Some(c) = self.peek() else {
            return self.err(
                "regexp-unterminated",
                start,
                self.pos,
                "this class isn't closed",
            );
        };
        match char::from_u32(c) {
            Some('[') => {
                return Ok(if self.class_v()? {
                    Atom::Strings
                } else {
                    Atom::Set
                });
            }
            Some('\\') => {
                self.pos += 1;
                if self.at('q') && self.peek_at(1) == Some('{' as u32) {
                    self.pos += 2;
                    return self.class_strings(start);
                }
                if self.peek().is_some_and(|d| is_digit(d) && d != '0' as u32) {
                    return self.err(
                        "regexp-escape",
                        start,
                        self.pos + 1,
                        "a class can't refer back to a group",
                    );
                }
                if self.peek().is_none() {
                    return self.err(
                        "regexp-unterminated",
                        start,
                        self.pos,
                        "this class isn't closed",
                    );
                }
                return self.escape(true);
            }
            _ => {}
        }
        self.class_set_character(start)
    }

    /// A ClassSetCharacter that isn't escaped.
    fn class_set_character(&mut self, start: usize) -> Result<Atom, Diagnostic> {
        let c = self.peek().unwrap_or(0);
        if let Some(ch) = char::from_u32(c) {
            if "()[]{}/-\\|".contains(ch) {
                return self.err(
                    "regexp-class-syntax",
                    start,
                    start + 1,
                    "with the v flag, this has to be escaped in a class",
                );
            }
            if "&!#$%*+,.:;<=>?@^`~".contains(ch) && self.peek_at(1) == Some(c) {
                return self.err(
                    "regexp-class-syntax",
                    start,
                    start + 2,
                    "with the v flag, a class can't have this pair unescaped",
                );
            }
        }
        self.pos += 1;
        Ok(Atom::Char(c))
    }

    /// `\q{a|bc|}`'s strings, from after `{`: strings, unless each is one
    /// character.
    fn class_strings(&mut self, start: usize) -> Result<Atom, Diagnostic> {
        let mut strings = false;
        let mut len = 0;
        loop {
            match self.peek().and_then(char::from_u32) {
                None => {
                    return self.err(
                        "regexp-unterminated",
                        start,
                        self.pos,
                        "this \\q{ isn't closed",
                    );
                }
                Some('}') => {
                    self.pos += 1;
                    strings |= len != 1;
                    return Ok(if strings { Atom::Strings } else { Atom::Set });
                }
                Some('|') => {
                    self.pos += 1;
                    strings |= len != 1;
                    len = 0;
                }
                Some('\\') => {
                    self.pos += 1;
                    if self.peek().is_none() {
                        return self.err(
                            "regexp-unterminated",
                            start,
                            self.pos,
                            "this \\q{ isn't closed",
                        );
                    }
                    let at = self.pos;
                    match self.escape(true)? {
                        Atom::Char(_) => len += 1,
                        _ => {
                            return self.err(
                                "regexp-escape",
                                at - 1,
                                self.pos,
                                "\\q{} holds characters, not classes",
                            );
                        }
                    }
                }
                Some(_) => {
                    let at = self.pos;
                    self.class_set_character(at)?;
                    len += 1;
                }
            }
        }
    }
}

/// How many groups of one name each alternative, by its disjunction
/// and index, and each disjunction holds, at any depth.
#[derive(Default)]
struct Counts {
    alternatives: HashMap<(u32, u32), u32>,
    disjunctions: HashMap<u32, u32>,
}

/// Whether the decimal digits `a` are a bigger number than `b`.
fn bigger(a: &[u32], b: &[u32]) -> bool {
    let strip = |d: &[u32]| -> Vec<u32> {
        let i = d.iter().position(|&c| c != '0' as u32).unwrap_or(d.len());
        d[i..].to_vec()
    };
    let (a, b) = (strip(a), strip(b));
    a.len() > b.len() || a.len() == b.len() && a > b
}

fn is_digit(c: u32) -> bool {
    (('0' as u32)..=('9' as u32)).contains(&c)
}

fn is_ascii_letter(c: u32) -> bool {
    char::from_u32(c).is_some_and(|c| c.is_ascii_alphabetic())
}

fn hex(c: u32) -> Option<u32> {
    char::from_u32(c)?.to_digit(16)
}
