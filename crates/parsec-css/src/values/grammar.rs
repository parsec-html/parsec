// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The value definition syntax, CSS Values and Units Level 4 §2: the
//! language every property's grammar is written in, `<length> | auto`
//! and the like, parsed into a tree the matcher walks.

use std::sync::OnceLock;

/// A reference to a type, `<name>` or `<name [min,max]>`, with what it
/// names looked up once, the first time it's matched.
#[derive(Debug)]
pub struct TypeRef {
    pub name: Box<str>,
    pub range: Option<(f64, f64)>,
    pub(crate) target: OnceLock<super::matcher::Target>,
}

impl Clone for TypeRef {
    fn clone(&self) -> TypeRef {
        TypeRef {
            name: self.name.clone(),
            range: self.range,
            target: OnceLock::new(),
        }
    }
}

impl PartialEq for TypeRef {
    fn eq(&self, other: &TypeRef) -> bool {
        self.name == other.name && self.range == other.range
    }
}

/// A reference to a property's grammar, `<'name'>`, looked up once.
#[derive(Debug)]
pub struct PropertyRef {
    pub name: Box<str>,
    #[allow(clippy::type_complexity)]
    pub(crate) target: OnceLock<Option<(&'static super::PropertyData, &'static Grammar)>>,
}

impl Clone for PropertyRef {
    fn clone(&self) -> PropertyRef {
        PropertyRef {
            name: self.name.clone(),
            target: OnceLock::new(),
        }
    }
}

impl PartialEq for PropertyRef {
    fn eq(&self, other: &PropertyRef) -> bool {
        self.name == other.name
    }
}

/// A grammar, parsed.
#[derive(Clone, Debug, PartialEq)]
pub enum Grammar {
    /// A keyword, in lowercase, matched ignoring ASCII case.
    Keyword(Box<str>),
    /// A literal token: `,`, `/`, `+`, `:` and the like.
    Literal(char),
    /// `<name>` or `<name [min,max]>`.
    Type(TypeRef),
    /// `<'name'>`: whatever that property's value can be.
    Property(PropertyRef),
    /// `name( … )`.
    Function(Box<str>, Box<Grammar>),
    /// `[ … ]` written as a literal block in the value: `'[' … ']'`.
    SquareBlock(Box<Grammar>),
    /// `( … )`: a parenthesized block in the value.
    ParenBlock(Box<Grammar>),
    /// Juxtaposition: all, in order.
    Sequence(Vec<Grammar>),
    /// `&&`: all, in any order.
    All(Vec<Grammar>),
    /// `||`: one or more, in any order.
    Any(Vec<Grammar>),
    /// `|`: exactly one.
    One(Vec<Grammar>),
    /// A multiplier: `*`, `+`, `?`, `{a,b}`, and `#` when `commas`.
    Repeat {
        item: Box<Grammar>,
        min: u32,
        max: u32,
        commas: bool,
    },
    /// `[ … ]!`: the group must match at least one value.
    NonEmpty(Box<Grammar>),
    /// Matches nothing: for a definition that's prose, not grammar.
    Never,
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Word(String),
    Type(String, Option<(f64, f64)>),
    Property(String),
    FunctionType(String),
    Function(String),
    Literal(char),
    Open,
    Close,
    OpenParen,
    CloseParen,
    Bar,
    Bars,
    Ands,
    Star,
    Plus,
    Question,
    Hash,
    Bang,
    Range(u32, u32),
}

fn bound(s: &str) -> Option<f64> {
    match s.trim() {
        "∞" | "+∞" | "infinity" => Some(f64::INFINITY),
        "-∞" | "−∞" | "-infinity" => Some(f64::NEG_INFINITY),
        s => {
            // `0px`, `1deg`: the number is what bounds.
            let end = s
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e')))
                .unwrap_or(s.len());
            s[..end].parse().ok()
        }
    }
}

fn tokenize(src: &str) -> Option<Vec<Tok>> {
    let b: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            _ if c.is_whitespace() => i += 1,
            '<' => {
                let end = (i..b.len()).find(|&j| b[j] == '>')?;
                let inner: String = b[i + 1..end].iter().collect();
                let inner = inner.trim();
                if let Some(p) = inner.strip_prefix('\'') {
                    out.push(Tok::Property(p.trim_end_matches('\'').to_string()));
                } else if let Some(f) = inner.strip_suffix("()") {
                    out.push(Tok::FunctionType(f.to_string()));
                } else if let Some((name, range)) = inner.split_once('[') {
                    let range = range.trim_end_matches(']');
                    // A numeric range, or a generic type's argument, which
                    // this doesn't model: `<boolean-expr[ <if-test> ]>`.
                    let bounds = range
                        .split_once(',')
                        .and_then(|(lo, hi)| Some((bound(lo)?, bound(hi)?)));
                    out.push(Tok::Type(name.trim().to_string(), bounds));
                } else {
                    out.push(Tok::Type(inner.to_string(), None));
                }
                i = end + 1;
            }
            '\'' => {
                let end = (i + 1..b.len()).find(|&j| b[j] == '\'')?;
                let lit: String = b[i + 1..end].iter().collect();
                let mut chars = lit.chars();
                match (chars.next(), chars.next()) {
                    (Some('['), None) => out.push(Tok::Literal('[')),
                    (Some(']'), None) => out.push(Tok::Literal(']')),
                    (Some(c), None) => out.push(Tok::Literal(c)),
                    // A quoted word is a keyword that looks like syntax.
                    _ => out.push(Tok::Word(lit)),
                }
                i = end + 1;
            }
            '[' => {
                out.push(Tok::Open);
                i += 1;
            }
            ']' => {
                out.push(Tok::Close);
                i += 1;
            }
            '(' => {
                out.push(Tok::OpenParen);
                i += 1;
            }
            ')' => {
                out.push(Tok::CloseParen);
                i += 1;
            }
            '|' if b.get(i + 1) == Some(&'|') => {
                out.push(Tok::Bars);
                i += 2;
            }
            '|' => {
                out.push(Tok::Bar);
                i += 1;
            }
            '&' if b.get(i + 1) == Some(&'&') => {
                out.push(Tok::Ands);
                i += 2;
            }
            '*' => {
                out.push(Tok::Star);
                i += 1;
            }
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '?' => {
                out.push(Tok::Question);
                i += 1;
            }
            '#' => {
                out.push(Tok::Hash);
                i += 1;
            }
            '!' => {
                out.push(Tok::Bang);
                i += 1;
            }
            '{' => {
                let end = (i..b.len()).find(|&j| b[j] == '}')?;
                let inner: String = b[i + 1..end].iter().collect();
                let num = |s: &str| s.trim().parse::<u32>().ok();
                let range = match inner.split_once(',') {
                    Some((lo, hi)) if hi.trim().is_empty() => num(lo).map(|lo| (lo, u32::MAX)),
                    Some((lo, hi)) => num(lo).zip(num(hi)),
                    None => num(&inner).map(|n| (n, n)),
                };
                match range {
                    Some((lo, hi)) => {
                        out.push(Tok::Range(lo, hi));
                        i = end + 1;
                    }
                    // A literal `{`, as an at-rule's grammar has: never a
                    // token in a value, so it matches nothing.
                    None => {
                        out.push(Tok::Literal('{'));
                        i += 1;
                    }
                }
            }
            '}' => {
                out.push(Tok::Literal('}'));
                i += 1;
            }
            ',' | '/' | ':' | ';' | '=' => {
                out.push(Tok::Literal(c));
                i += 1;
            }
            _ => {
                let start = i;
                while i < b.len()
                    && (b[i].is_alphanumeric() || matches!(b[i], '-' | '_' | '@' | '.' | '%'))
                {
                    i += 1;
                }
                if i == start {
                    return None;
                }
                let word: String = b[start..i].iter().collect();
                if b.get(i) == Some(&'(') {
                    out.push(Tok::Function(word.to_ascii_lowercase()));
                    i += 1;
                } else {
                    out.push(Tok::Word(word));
                }
            }
        }
    }
    Some(out)
}

struct P {
    toks: Vec<Tok>,
    i: usize,
}

impl P {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i)
    }

    /// `|` binds loosest, then `||`, then `&&`, then juxtaposition.
    fn one(&mut self) -> Option<Grammar> {
        let mut alts = vec![self.any()?];
        while self.peek() == Some(&Tok::Bar) {
            self.i += 1;
            alts.push(self.any()?);
        }
        Some(if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            Grammar::One(alts)
        })
    }

    fn any(&mut self) -> Option<Grammar> {
        let mut alts = vec![self.all()?];
        while self.peek() == Some(&Tok::Bars) {
            self.i += 1;
            alts.push(self.all()?);
        }
        Some(if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            Grammar::Any(alts)
        })
    }

    fn all(&mut self) -> Option<Grammar> {
        let mut alts = vec![self.sequence()?];
        while self.peek() == Some(&Tok::Ands) {
            self.i += 1;
            alts.push(self.sequence()?);
        }
        Some(if alts.len() == 1 {
            alts.pop().unwrap()
        } else {
            Grammar::All(alts)
        })
    }

    fn sequence(&mut self) -> Option<Grammar> {
        let mut items = Vec::new();
        while let Some(t) = self.peek() {
            if matches!(
                t,
                Tok::Bar | Tok::Bars | Tok::Ands | Tok::Close | Tok::CloseParen | Tok::Literal(']')
            ) {
                break;
            }
            items.push(self.multiplied()?);
        }
        match items.len() {
            0 => None,
            1 => items.pop(),
            _ => Some(Grammar::Sequence(items)),
        }
    }

    fn multiplied(&mut self) -> Option<Grammar> {
        let mut g = self.term()?;
        loop {
            let (min, max, commas) = match self.peek() {
                Some(Tok::Star) => (0, u32::MAX, false),
                Some(Tok::Plus) => (1, u32::MAX, false),
                Some(Tok::Question) => (0, 1, false),
                Some(Tok::Range(a, b)) => (*a, *b, false),
                Some(Tok::Hash) => {
                    self.i += 1;
                    if let Some(Tok::Range(a, b)) = self.peek() {
                        let (a, b) = (*a, *b);
                        self.i += 1;
                        g = Grammar::Repeat {
                            item: Box::new(g),
                            min: a,
                            max: b,
                            commas: true,
                        };
                    } else {
                        g = Grammar::Repeat {
                            item: Box::new(g),
                            min: 1,
                            max: u32::MAX,
                            commas: true,
                        };
                    }
                    continue;
                }
                Some(Tok::Bang) => {
                    self.i += 1;
                    g = Grammar::NonEmpty(Box::new(g));
                    continue;
                }
                _ => return Some(g),
            };
            self.i += 1;
            g = Grammar::Repeat {
                item: Box::new(g),
                min,
                max,
                commas,
            };
        }
    }

    fn term(&mut self) -> Option<Grammar> {
        let t = self.peek()?.clone();
        self.i += 1;
        Some(match t {
            Tok::Word(w) => Grammar::Keyword(w.to_ascii_lowercase().into()),
            Tok::Literal('[') => {
                // `'[' <custom-ident>* ']'`: a square block in the value.
                let inner = self.one()?;
                if self.peek() != Some(&Tok::Literal(']')) {
                    return None;
                }
                self.i += 1;
                Grammar::SquareBlock(Box::new(inner))
            }
            Tok::Literal(c) => Grammar::Literal(c),
            Tok::Type(name, range) => Grammar::Type(TypeRef {
                name: name.into(),
                range,
                target: OnceLock::new(),
            }),
            Tok::Property(p) => Grammar::Property(PropertyRef {
                name: p.into(),
                target: OnceLock::new(),
            }),
            Tok::FunctionType(f) => Grammar::Type(TypeRef {
                name: format!("{f}()").into(),
                range: None,
                target: OnceLock::new(),
            }),
            Tok::Function(name) => {
                let inner = if self.peek() == Some(&Tok::CloseParen) {
                    Grammar::Sequence(Vec::new())
                } else {
                    self.one()?
                };
                if self.peek() != Some(&Tok::CloseParen) {
                    return None;
                }
                self.i += 1;
                Grammar::Function(name.into(), Box::new(inner))
            }
            Tok::OpenParen => {
                let inner = self.one()?;
                if self.peek() != Some(&Tok::CloseParen) {
                    return None;
                }
                self.i += 1;
                Grammar::ParenBlock(Box::new(inner))
            }
            Tok::Open => {
                let inner = self.one()?;
                if self.peek() != Some(&Tok::Close) {
                    return None;
                }
                self.i += 1;
                inner
            }
            _ => return None,
        })
    }
}

impl Grammar {
    /// Parses a grammar written in the value definition syntax, or `None`
    /// if it isn't one.
    ///
    /// ```
    /// use parsec_css::values::Grammar;
    ///
    /// let g = Grammar::parse("<length [0,∞]>{1,4} | auto").unwrap();
    /// assert!(matches!(g, Grammar::One(alts) if alts.len() == 2));
    /// ```
    pub fn parse(src: &str) -> Option<Grammar> {
        let toks = tokenize(src)?;
        let mut p = P { toks, i: 0 };
        let g = p.one()?;
        (p.i == p.toks.len()).then_some(g)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_grammar_in_the_table() {
        let mut bad = Vec::new();
        for p in super::super::generated::PROPERTIES {
            if let Some(s) = p.syntax
                && Grammar::parse(s).is_none()
            {
                bad.push(format!("{}: {s}", p.name));
            }
        }
        // Written in tokens rather than values (`<function-token> … )`), or
        // generic (`<boolean-expr[ <if-test> ]>`): grammars for preludes
        // and conditions, which no property's value uses.
        let token_level = ["general-enclosed", "if-condition", "pseudo-class-selector"];
        for (name, s) in super::super::generated::TYPES
            .iter()
            .chain(super::super::generated::FUNCTIONS)
        {
            if Grammar::parse(s).is_none() && !token_level.contains(name) {
                bad.push(format!("<{name}>: {s}"));
            }
        }
        assert!(
            bad.is_empty(),
            "{} didn't parse:\n{}",
            bad.len(),
            bad.join("\n")
        );
    }

    #[test]
    fn precedence_and_multipliers() {
        let g = Grammar::parse("a b | c && d || e").unwrap();
        let Grammar::One(alts) = g else { panic!() };
        assert!(matches!(&alts[0], Grammar::Sequence(_)));
        assert!(matches!(&alts[1], Grammar::Any(_)));
        let g = Grammar::parse("<length>#{2,3}").unwrap();
        assert!(matches!(
            g,
            Grammar::Repeat {
                min: 2,
                max: 3,
                commas: true,
                ..
            }
        ));
    }
}
