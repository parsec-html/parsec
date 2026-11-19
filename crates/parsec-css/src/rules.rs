// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The at-rules besides `@media`, `@supports` and `@layer`: `@keyframes`,
//! `@counter-style` and `@property` parsed into what they say, and the
//! preludes of `@container`, `@scope` and `@import`, which the cascade
//! evaluates. Each is written back as CSSOM writes it.
//!
//! ```
//! use parsec_css::rules::{CounterStyle, PropertyRule};
//! use parsec_css::Rule;
//!
//! let css = "@property --gap { syntax: '<length>'; inherits: false; initial-value: 4px }
//!            @counter-style thumbs { system: cyclic; symbols: '👍'; suffix: ' ' }";
//! let sheet = parsec_css::parse_stylesheet_deep(css);
//! let Rule::At(p) = &sheet.rules[0] else { unreachable!() };
//! let p = PropertyRule::parse(p).unwrap();
//! assert_eq!((p.syntax.as_str(), p.inherits, p.initial_value.as_deref()), ("<length>", false, Some("4px")));
//! let Rule::At(c) = &sheet.rules[1] else { unreachable!() };
//! let c = CounterStyle::parse(c).unwrap();
//! assert_eq!(c.get("system"), Some("cyclic"));
//! ```

use std::collections::HashMap;
use std::fmt::{self, Write};
use std::sync::{Mutex, OnceLock};

use crate::media::{Condition, Environment, Feature, MediaQueryList};
use crate::parser::{AtRule, Block, Bracket, ComponentValue, Item};
use crate::serialize::{write_identifier, write_string};
use crate::tokenizer::Token;
use crate::values::{Component, Grammar, Specified, Unit};

fn trimmed<'v, 'a>(values: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
    let start = values
        .iter()
        .position(|v| !v.is_whitespace())
        .unwrap_or(values.len());
    let end = values
        .iter()
        .rposition(|v| !v.is_whitespace())
        .map_or(start, |i| i + 1);
    &values[start..end]
}

fn text(values: &[ComponentValue<'_>]) -> String {
    crate::to_css(trimmed(values))
}

fn written(c: Vec<Component<'_>>) -> String {
    Specified::Value(c).to_string()
}

const CSS_WIDE: &[&str] = &[
    "initial",
    "inherit",
    "unset",
    "revert",
    "revert-layer",
    "revert-rule",
    "default",
];

/// A name as it's written back: an identifier where it can be one.
fn write_name(out: &mut impl Write, name: &str) -> fmt::Result {
    write_identifier(out, name)
}

/// The declarations of an at-rule's block: name, value as written, and
/// whether it's `!important`.
fn declarations<'b>(
    block: &'b Block<'_>,
) -> impl Iterator<Item = (&'b str, &'b [ComponentValue<'b>], bool)> {
    block.items.iter().filter_map(|i| match i {
        Item::Declaration(d) => Some((&*d.name, &d.value[..], d.important)),
        _ => None,
    })
}

// ---- @keyframes --------------------------------------------------------------

/// `@keyframes`, CSS Animations 1 §3: a name and its keyframes, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframes {
    pub name: String,
    pub frames: Vec<Keyframe>,
}

/// One keyframe: where it is in the animation, and its declarations,
/// `!important` ones left out as §3.3 says.
#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub offsets: Vec<KeyframeOffset>,
    /// Name and value, the value as written.
    pub declarations: Vec<(String, String)>,
}

/// A keyframe selector: `from` is 0%, `to` 100%, and Scroll Animations 1
/// adds a range name before a percentage, `entry 50%`.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyframeOffset {
    Percent(f64),
    Range(String, f64),
}

const RANGE_NAMES: &[&str] = &[
    "cover",
    "contain",
    "entry",
    "exit",
    "entry-crossing",
    "exit-crossing",
    "scroll",
];

impl Keyframes {
    /// The rule, if it's a `@keyframes` with a valid name. Keyframes whose
    /// selectors don't parse are left out.
    pub fn parse(rule: &AtRule<'_, Block<'_>>) -> Option<Keyframes> {
        let name = rule.name.to_ascii_lowercase();
        if !matches!(name.as_str(), "keyframes" | "-webkit-keyframes") {
            return None;
        }
        let name = match trimmed(&rule.prelude) {
            [ComponentValue::Token(Token::Ident(n), _)]
                if !n.eq_ignore_ascii_case("none")
                    && !CSS_WIDE.iter().any(|w| n.eq_ignore_ascii_case(w)) =>
            {
                n.to_string()
            }
            [ComponentValue::Token(Token::String(s), _)] if !s.is_empty() => s.to_string(),
            _ => return None,
        };
        let mut frames = Vec::new();
        for item in &rule.block.as_ref()?.items {
            let Item::Rule(crate::Rule::Qualified(q)) = item else {
                continue;
            };
            let Some(offsets) = keyframe_offsets(&q.prelude) else {
                continue;
            };
            let declarations = declarations(&q.block)
                .filter(|(_, _, important)| !important)
                .map(|(n, v, _)| (n.to_string(), text(v)))
                .collect();
            frames.push(Keyframe {
                offsets,
                declarations,
            });
        }
        Some(Keyframes { name, frames })
    }
}

fn keyframe_offsets(prelude: &[ComponentValue<'_>]) -> Option<Vec<KeyframeOffset>> {
    let mut out = Vec::new();
    for part in prelude.split(|v| matches!(v, ComponentValue::Token(Token::Comma, _))) {
        let words: Vec<&ComponentValue<'_>> = part.iter().filter(|v| !v.is_whitespace()).collect();
        let percent = |v: &ComponentValue<'_>| match v {
            ComponentValue::Token(Token::Percentage(n), _) => Some(n.value),
            _ => None,
        };
        out.push(match words[..] {
            [ComponentValue::Token(Token::Ident(w), _)] if w.eq_ignore_ascii_case("from") => {
                KeyframeOffset::Percent(0.0)
            }
            [ComponentValue::Token(Token::Ident(w), _)] if w.eq_ignore_ascii_case("to") => {
                KeyframeOffset::Percent(100.0)
            }
            [v] => {
                let p = percent(v)?;
                if !(0.0..=100.0).contains(&p) {
                    return None;
                }
                KeyframeOffset::Percent(p)
            }
            [ComponentValue::Token(Token::Ident(w), _), v] => {
                let name = RANGE_NAMES.iter().find(|n| w.eq_ignore_ascii_case(n))?;
                KeyframeOffset::Range(name.to_string(), percent(v)?)
            }
            _ => return None,
        });
    }
    Some(out)
}

impl fmt::Display for KeyframeOffset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeyframeOffset::Percent(p) => {
                crate::values::write_number(f, *p)?;
                f.write_char('%')
            }
            KeyframeOffset::Range(n, p) => {
                write!(f, "{n} ")?;
                crate::values::write_number(f, *p)?;
                f.write_char('%')
            }
        }
    }
}

impl fmt::Display for Keyframes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("@keyframes ")?;
        write_name(f, &self.name)?;
        f.write_str(" {")?;
        for k in &self.frames {
            f.write_str(" ")?;
            for (i, o) in k.offsets.iter().enumerate() {
                if i > 0 {
                    f.write_str(", ")?;
                }
                write!(f, "{o}")?;
            }
            f.write_str(" {")?;
            for (n, v) in &k.declarations {
                write!(f, " {n}: {v};")?;
            }
            f.write_str(" }")?;
        }
        f.write_str(" }")
    }
}

// ---- descriptors ---------------------------------------------------------------

/// A descriptor's grammar, parsed once: they're the specs' and fixed.
fn grammar(src: &'static str) -> &'static Grammar {
    static ALL: OnceLock<Mutex<HashMap<&'static str, &'static Grammar>>> = OnceLock::new();
    let all = ALL.get_or_init(Default::default);
    let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
    // The descriptor grammars are a fixed handful, so leaking each once
    // is bounded.
    all.entry(src)
        .or_insert_with(|| Box::leak(Box::new(Grammar::parse(src).expect(src))))
}

/// A descriptor's value matched against `grammar`, written back.
fn descriptor(src: &'static str, value: &[ComponentValue<'_>]) -> Option<String> {
    crate::values::match_grammar(grammar(src), value).map(written)
}

// ---- @counter-style ------------------------------------------------------------

/// `@counter-style`, CSS Counter Styles 3 §3: a name and its valid
/// descriptors, each written back in its canonical form, in the order
/// CSSOM writes them. What a counter is for a number isn't worked out.
#[derive(Clone, Debug, PartialEq)]
pub struct CounterStyle {
    pub name: String,
    pub descriptors: Vec<(&'static str, String)>,
}

/// Each descriptor and its grammar, in CSSOM's order.
const COUNTER_DESCRIPTORS: &[(&str, &str)] = &[
    (
        "system",
        "cyclic | numeric | alphabetic | symbolic | additive | [ fixed <integer>? ] | [ extends <counter-style-name> ]",
    ),
    ("symbols", "[ <string> | <image> | <custom-ident> ]+"),
    (
        "additive-symbols",
        "[ <integer [0,∞]> && [ <string> | <image> | <custom-ident> ] ]#",
    ),
    (
        "negative",
        "[ <string> | <image> | <custom-ident> ] [ <string> | <image> | <custom-ident> ]?",
    ),
    ("prefix", "[ <string> | <image> | <custom-ident> ]"),
    ("suffix", "[ <string> | <image> | <custom-ident> ]"),
    ("range", "[ [ <integer> | infinite ]{2} ]# | auto"),
    (
        "pad",
        "<integer [0,∞]> && [ <string> | <image> | <custom-ident> ]",
    ),
    ("fallback", "<counter-style-name>"),
    (
        "speak-as",
        "auto | bullets | numbers | words | spell-out | <counter-style-name>",
    ),
];

/// The names a `@counter-style` can't define: CSS-wide keywords, `none`,
/// and the predefined styles §3.1 makes fixed.
const FIXED_STYLES: &[&str] = &[
    "none",
    "decimal",
    "disc",
    "square",
    "circle",
    "disclosure-open",
    "disclosure-closed",
];

impl CounterStyle {
    /// The rule, if it's a `@counter-style` with a valid name: its valid
    /// descriptors, whether or not they make a usable counter style.
    pub fn parse(rule: &AtRule<'_, Block<'_>>) -> Option<CounterStyle> {
        if !rule.name.eq_ignore_ascii_case("counter-style") {
            return None;
        }
        let [ComponentValue::Token(Token::Ident(name), _)] = trimmed(&rule.prelude) else {
            return None;
        };
        if CSS_WIDE
            .iter()
            .chain(FIXED_STYLES)
            .any(|w| name.eq_ignore_ascii_case(w))
        {
            return None;
        }
        let mut found: Vec<Option<String>> = vec![None; COUNTER_DESCRIPTORS.len()];
        for (n, value, _) in declarations(rule.block.as_ref()?) {
            let Some(i) = COUNTER_DESCRIPTORS
                .iter()
                .position(|(d, _)| n.eq_ignore_ascii_case(d))
            else {
                continue;
            };
            let (d, g) = COUNTER_DESCRIPTORS[i];
            let Some(v) = descriptor(g, value) else {
                continue;
            };
            if !counter_valid(d, value) {
                continue;
            }
            found[i] = Some(v);
        }
        let descriptors = COUNTER_DESCRIPTORS
            .iter()
            .zip(found)
            .filter_map(|((d, _), v)| Some((*d, v?)))
            .collect();
        Some(CounterStyle {
            name: name.to_string(),
            descriptors,
        })
    }

    /// Whether it defines a counter style, as §3.1.1 says each system
    /// needs: symbols enough for it, or additive symbols, or for
    /// `extends`, neither. One that doesn't is kept as CSSOM keeps it, and
    /// counters fall back from it.
    pub fn usable(&self) -> bool {
        let system = self.get("system").unwrap_or("symbolic");
        let symbols = self.get("symbols").map_or(0, |s| {
            crate::parse_component_values(s)
                .iter()
                .filter(|v| !v.is_whitespace())
                .count()
        });
        let additive = self.get("additive-symbols").is_some();
        match system.split(' ').next().unwrap_or("") {
            "cyclic" | "fixed" | "symbolic" => symbols >= 1,
            "alphabetic" | "numeric" => symbols >= 2,
            "additive" => additive,
            "extends" => symbols == 0 && !additive,
            _ => false,
        }
    }

    /// A descriptor's value, written back, if the rule gives a valid one.
    pub fn get(&self, descriptor: &str) -> Option<&str> {
        self.descriptors
            .iter()
            .find(|(d, _)| *d == descriptor)
            .map(|(_, v)| v.as_str())
    }
}

/// What the prose adds to a descriptor's grammar.
fn counter_valid(descriptor: &str, value: &[ComponentValue<'_>]) -> bool {
    let numbers = || {
        value.iter().filter_map(|v| match v {
            ComponentValue::Token(Token::Number(n), _) => Some(n.value),
            ComponentValue::Token(Token::Ident(w), _) if w.eq_ignore_ascii_case("infinite") => {
                Some(f64::NAN)
            }
            _ => None,
        })
    };
    // §1.1: a counter style's name isn't `none`.
    let named_none = value.iter().any(|v| {
        matches!(v, ComponentValue::Token(Token::Ident(w), _) if w.eq_ignore_ascii_case("none"))
    });
    match descriptor {
        "system" | "fallback" => !named_none,
        // §3.5: each range's lower bound no more than its upper.
        "range" => {
            let n: Vec<f64> = numbers().collect();
            n.chunks(2).all(|r| match r {
                [a, b] => a.is_nan() || b.is_nan() || a <= b,
                _ => true,
            })
        }
        // §3.2: weights in descending order, with no repeats.
        "additive-symbols" => {
            let n: Vec<f64> = numbers().collect();
            n.windows(2).all(|w| w[0] > w[1])
        }
        _ => true,
    }
}

impl fmt::Display for CounterStyle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("@counter-style ")?;
        write_name(f, &self.name)?;
        f.write_str(" {")?;
        for (d, v) in &self.descriptors {
            write!(f, " {d}: {v};")?;
        }
        f.write_str(" }")
    }
}

// ---- @property -----------------------------------------------------------------

/// `@property`, CSS Properties and Values API 1 §3: a custom property
/// registered with a syntax, whether it inherits, and an initial value.
/// A descriptor that's missing or invalid takes its default, `*` and
/// `true`, as CSSOM has it; the rule registers the property only if its
/// syntax is `*` or it has an initial value ([`PropertyRule::registers`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PropertyRule {
    pub name: String,
    /// The syntax as written, `*` for any value.
    pub syntax: String,
    pub inherits: bool,
    /// As written.
    pub initial_value: Option<String>,
}

/// The type names a registered property's syntax can use, §5.1.
const SYNTAX_TYPES: &[&str] = &[
    "angle",
    "color",
    "custom-ident",
    "image",
    "integer",
    "length",
    "length-percentage",
    "number",
    "percentage",
    "resolution",
    "string",
    "time",
    "transform-function",
    "transform-list",
    "url",
];

/// A registered property's syntax: `None` for the universal `*`, or the
/// grammar it stands for.
#[derive(Clone, Copy, Debug)]
pub struct Syntax {
    grammar: Option<&'static Grammar>,
}

/// How many distinct syntaxes are kept matched: each is parsed into a
/// grammar once and kept for the program, so a hostile stylesheet can't
/// grow the table past this; beyond it, a new syntax is taken as `*`.
const MAX_SYNTAXES: usize = 4096;

impl Syntax {
    /// Parses a syntax string by §5.1, or `None` if it isn't one.
    pub fn parse(src: &str) -> Option<Syntax> {
        let s = src.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c'));
        if s == "*" {
            return Some(Syntax { grammar: None });
        }
        let mut parts = Vec::new();
        for part in s.split('|') {
            let p = part.trim_matches(|c: char| matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c'));
            let (body, multiplier) = match p.as_bytes().last() {
                Some(b'+') => (&p[..p.len() - 1], "+"),
                Some(b'#') => (&p[..p.len() - 1], "#"),
                _ => (p, ""),
            };
            if let Some(name) = body.strip_prefix('<').and_then(|b| b.strip_suffix('>')) {
                if !SYNTAX_TYPES.contains(&name) {
                    return None;
                }
                // A list of lists isn't one.
                if name == "transform-list" && !multiplier.is_empty() {
                    return None;
                }
                parts.push(format!("<{name}>{multiplier}"));
            } else {
                // An identifier, written plainly: a keyword of the syntax.
                let ok = !body.is_empty()
                    && body.chars().all(|c| {
                        c.is_ascii_alphanumeric() || c == '-' || c == '_' || !c.is_ascii()
                    })
                    && !body.starts_with(|c: char| c.is_ascii_digit())
                    && !CSS_WIDE.iter().any(|w| body.eq_ignore_ascii_case(w));
                if !ok {
                    return None;
                }
                parts.push(format!("{body}{multiplier}"));
            }
        }
        let text = parts.join(" | ");
        static ALL: OnceLock<Mutex<HashMap<String, &'static Grammar>>> = OnceLock::new();
        let all = ALL.get_or_init(Default::default);
        let mut all = all.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(g) = all.get(&text) {
            return Some(Syntax { grammar: Some(g) });
        }
        // shortcut: past MAX_SYNTAXES distinct syntaxes a new one acts as
        // `*`; a table that's evicted would let this be unbounded instead.
        if all.len() >= MAX_SYNTAXES {
            return Some(Syntax { grammar: None });
        }
        let g: &'static Grammar = Box::leak(Box::new(Grammar::parse(&text)?));
        all.insert(text, g);
        Some(Syntax { grammar: Some(g) })
    }

    /// Whether it's `*`.
    pub fn is_universal(&self) -> bool {
        self.grammar.is_none()
    }

    /// `values` typed by the syntax, or `None` if they don't match it.
    pub fn parse_value<'a>(&self, values: &[ComponentValue<'a>]) -> Option<Vec<Component<'a>>> {
        match self.grammar {
            Some(g) => crate::values::match_grammar(g, values),
            None => None,
        }
    }
}

/// Whether a value has nothing in it that depends on the element:
/// relative lengths and `var()`, §3.3's computational independence.
fn independent(c: &[Component<'_>]) -> bool {
    fn unit_ok(u: Unit) -> bool {
        u.kind() != crate::values::Kind::Length || u.canonical().is_some()
    }
    fn node_ok(n: &crate::values::Node) -> bool {
        use crate::values::{Leaf, Node};
        match n {
            Node::Value(Leaf::Dimension(_, u)) => unit_ok(*u),
            Node::Value(_) | Node::Keyword(_) => true,
            Node::Sum(c) | Node::Product(c) | Node::Function(_, c) => c.iter().all(node_ok),
            Node::Negate(c) | Node::Invert(c) => node_ok(c),
        }
    }
    c.iter().all(|x| match x {
        Component::Dimension(_, u) => unit_ok(*u),
        Component::Math(m) => node_ok(&m.root),
        Component::Function { arguments, .. } => independent(arguments),
        Component::Block(b) | Component::Parens(b) => independent(b),
        _ => true,
    })
}

impl PropertyRule {
    /// The rule, if it's a valid `@property`: a custom property's name,
    /// and an initial value, if there is one, that parses for the syntax
    /// and doesn't depend on the element.
    pub fn parse(rule: &AtRule<'_, Block<'_>>) -> Option<PropertyRule> {
        if !rule.name.eq_ignore_ascii_case("property") {
            return None;
        }
        let [ComponentValue::Token(Token::Ident(name), _)] = trimmed(&rule.prelude) else {
            return None;
        };
        if !name.starts_with("--") || name.len() < 3 {
            return None;
        }
        let (mut syntax, mut inherits, mut initial) = (None, None, None);
        for (n, value, _) in declarations(rule.block.as_ref()?) {
            match n.to_ascii_lowercase().as_str() {
                "syntax" => {
                    if let [ComponentValue::Token(Token::String(s), _)] = trimmed(value)
                        && Syntax::parse(s).is_some()
                    {
                        syntax = Some(s.to_string());
                    }
                }
                "inherits" => match trimmed(value) {
                    [ComponentValue::Token(Token::Ident(w), _)]
                        if w.eq_ignore_ascii_case("true") =>
                    {
                        inherits = Some(true)
                    }
                    [ComponentValue::Token(Token::Ident(w), _)]
                        if w.eq_ignore_ascii_case("false") =>
                    {
                        inherits = Some(false)
                    }
                    _ => {}
                },
                "initial-value" => initial = Some(value),
                _ => {}
            }
        }
        let syntax = syntax.unwrap_or_else(|| "*".to_string());
        let inherits = inherits.unwrap_or(true);
        let parsed = Syntax::parse(&syntax)?;
        let initial_value = match initial {
            Some(v) if parsed.is_universal() => Some(text(v)),
            Some(v) => {
                let c = parsed.parse_value(v)?;
                if !independent(&c) || crate::values::has_substitution(v) {
                    return None;
                }
                Some(text(v))
            }
            None => None,
        };
        Some(PropertyRule {
            name: name.to_string(),
            syntax,
            inherits,
            initial_value,
        })
    }
}

impl PropertyRule {
    /// Whether the rule registers its property: a syntax other than `*`
    /// needs an initial value.
    pub fn registers(&self) -> bool {
        self.initial_value.is_some()
            || Syntax::parse(&self.syntax).is_some_and(|s| s.is_universal())
    }
}

impl fmt::Display for PropertyRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("@property ")?;
        write_name(f, &self.name)?;
        f.write_str(" { syntax: ")?;
        write_string(f, &self.syntax)?;
        write!(f, "; inherits: {};", self.inherits)?;
        if let Some(v) = &self.initial_value {
            write!(f, " initial-value: {v};")?;
        }
        f.write_str(" }")
    }
}

// ---- @container ----------------------------------------------------------------

/// One `@container` query, CSS Conditional 5 §6: an optional container
/// name and a condition on the container's size or its custom
/// properties' values.
#[derive(Clone, Debug, PartialEq)]
pub struct ContainerQuery {
    pub name: Option<String>,
    /// `None` for a name alone, which any container of that name matches.
    pub condition: Option<Condition>,
    /// Each `style()` in the condition, which [`Condition`] keeps as
    /// text, parsed.
    styles: Vec<(String, StyleQuery)>,
}

/// `style()`'s query, §6.1.3: a custom property's value, or that it has
/// one, and those joined with `not`, `and` and `or`.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleQuery {
    /// `--name: value`, the value written as CSS writes it back, or
    /// `--name` alone.
    Feature {
        name: String,
        value: Option<String>,
    },
    Not(Box<StyleQuery>),
    And(Vec<StyleQuery>),
    Or(Vec<StyleQuery>),
    /// Anything else: unknown.
    Unknown,
}

impl StyleQuery {
    fn parse(values: &[ComponentValue<'_>], depth: usize) -> StyleQuery {
        if depth > 32 {
            return StyleQuery::Unknown;
        }
        let v = crate::media::solid(values);
        let inner = |c: &ComponentValue<'_>| match c {
            ComponentValue::Block(b) if b.bracket == Bracket::Paren => {
                Some(StyleQuery::parse(&b.contents, depth + 1))
            }
            _ => None,
        };
        let word = |c: Option<&&ComponentValue<'_>>, w: &str| matches!(c, Some(ComponentValue::Token(Token::Ident(x), _)) if x.eq_ignore_ascii_case(w));
        if word(v.first(), "not") {
            return match (v.len(), v.get(1).and_then(|c| inner(c))) {
                (2, Some(q)) => StyleQuery::Not(Box::new(q)),
                _ => StyleQuery::Unknown,
            };
        }
        if let Some(first) = v.first().and_then(|c| inner(c)) {
            if v.len() == 1 {
                return first;
            }
            let joiner = if word(v.get(1), "and") { "and" } else { "or" };
            let mut items = vec![first];
            let mut i = 1;
            while i < v.len() {
                match (word(v.get(i), joiner), v.get(i + 1).and_then(|c| inner(c))) {
                    (true, Some(q)) => items.push(q),
                    _ => return StyleQuery::Unknown,
                }
                i += 2;
            }
            return if joiner == "and" {
                StyleQuery::And(items)
            } else {
                StyleQuery::Or(items)
            };
        }
        // A declaration, or a name alone.
        let colon = values
            .iter()
            .position(|c| matches!(c, ComponentValue::Token(Token::Colon, _)));
        let name_part = trimmed(&values[..colon.unwrap_or(values.len())]);
        let [ComponentValue::Token(Token::Ident(name), _)] = name_part else {
            return StyleQuery::Unknown;
        };
        let value = colon.map(|c| text(&values[c + 1..]));
        StyleQuery::Feature {
            name: name.to_string(),
            value,
        }
    }

    /// Whether every part is a feature on a property that exists.
    fn whole(&self) -> bool {
        match self {
            StyleQuery::Feature { name, .. } => {
                name.starts_with("--") || crate::values::property(name).is_some()
            }
            StyleQuery::Not(q) => q.whole(),
            StyleQuery::And(qs) | StyleQuery::Or(qs) => qs.iter().all(StyleQuery::whole),
            StyleQuery::Unknown => false,
        }
    }

    fn evaluate(&self, get: &dyn Fn(&str) -> Option<String>) -> Option<bool> {
        match self {
            StyleQuery::Feature { name, value } => {
                if !name.starts_with("--") {
                    return None;
                }
                let have = get(name).filter(|v| !v.is_empty());
                Some(match value {
                    None => have.is_some(),
                    Some(v) => {
                        // Both as CSS writes them back.
                        let v = text(&crate::parse_component_values(v));
                        have.is_some_and(|h| text(&crate::parse_component_values(&h)) == v)
                    }
                })
            }
            StyleQuery::Not(q) => q.evaluate(get).map(|b| !b),
            StyleQuery::And(qs) => three(qs.iter().map(|q| q.evaluate(get)), false),
            StyleQuery::Or(qs) => three(qs.iter().map(|q| q.evaluate(get)), true),
            StyleQuery::Unknown => None,
        }
    }
}

/// Three-valued `and` (`or` when `any`): the deciding value wins over
/// unknown.
fn three(values: impl Iterator<Item = Option<bool>>, any: bool) -> Option<bool> {
    let mut unknown = false;
    for v in values {
        match v {
            Some(b) if b == any => return Some(any),
            None => unknown = true,
            Some(_) => {}
        }
    }
    if unknown { None } else { Some(!any) }
}

/// What a container query asks of its container: its width, its height,
/// or both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Axes {
    pub inline: bool,
    pub block: bool,
}

const SIZE_FEATURES: &[&str] = &[
    "width",
    "height",
    "inline-size",
    "block-size",
    "aspect-ratio",
    "orientation",
];

impl ContainerQuery {
    /// A `@container` prelude: one or more queries, comma-separated.
    pub fn parse_list(prelude: &[ComponentValue<'_>]) -> Option<Vec<ContainerQuery>> {
        let mut out = Vec::new();
        for part in prelude.split(|v| matches!(v, ComponentValue::Token(Token::Comma, _))) {
            out.push(ContainerQuery::parse(part)?);
        }
        (!out.is_empty()).then_some(out)
    }

    /// One query.
    pub fn parse(values: &[ComponentValue<'_>]) -> Option<ContainerQuery> {
        let v = crate::media::solid(values);
        let (name, rest) = match v.first() {
            Some(ComponentValue::Token(Token::Ident(n), _))
                if !["not", "and", "or", "none"]
                    .iter()
                    .chain(CSS_WIDE)
                    .any(|w| n.eq_ignore_ascii_case(w)) =>
            {
                (Some(n.to_string()), &v[1..])
            }
            _ => (None, &v[..]),
        };
        if rest.is_empty() {
            return name.map(|name| ContainerQuery {
                name: Some(name),
                condition: None,
                styles: Vec::new(),
            });
        }
        let condition = crate::media::parse_condition(rest, true, 0)?;
        // A media type has no place here: `screen`, `not print`.
        if matches!(rest.first(), Some(ComponentValue::Token(Token::Ident(w), _)) if !w.eq_ignore_ascii_case("not"))
        {
            return None;
        }
        let mut styles = Vec::new();
        fn collect(c: &Condition, out: &mut Vec<(String, StyleQuery)>) {
            match c {
                Condition::Unknown(t) => {
                    let v = crate::parse_component_values(t);
                    if let [ComponentValue::Function(f)] = trimmed(&v)
                        && f.name.eq_ignore_ascii_case("style")
                    {
                        let q = StyleQuery::parse(&f.arguments, 0);
                        // Any part that isn't a style feature makes all of
                        // it `<general-enclosed>`, unknown.
                        let q = if q.whole() { q } else { StyleQuery::Unknown };
                        out.push((t.clone(), q));
                    }
                }
                Condition::Not(c) | Condition::Parens(c) => collect(c, out),
                Condition::And(cs) | Condition::Or(cs) => cs.iter().for_each(|c| collect(c, out)),
                Condition::Feature(_) => {}
            }
        }
        collect(&condition, &mut styles);
        Some(ContainerQuery {
            name,
            condition: Some(condition),
            styles,
        })
    }

    /// The axes the condition's size features need of a container.
    pub fn axes(&self) -> Axes {
        fn walk(c: &Condition, a: &mut Axes) {
            match c {
                Condition::Feature(f) => {
                    let name = match f {
                        Feature::Boolean(n) | Feature::Plain(n, _) => n.as_str(),
                        Feature::Range { name, .. } => name.as_str(),
                    };
                    let base = name
                        .strip_prefix("min-")
                        .or_else(|| name.strip_prefix("max-"))
                        .unwrap_or(name);
                    match base {
                        "width" | "inline-size" => a.inline = true,
                        "height" | "block-size" => a.block = true,
                        "aspect-ratio" | "orientation" => {
                            a.inline = true;
                            a.block = true;
                        }
                        _ => {}
                    }
                }
                Condition::Not(c) | Condition::Parens(c) => walk(c, a),
                Condition::And(cs) | Condition::Or(cs) => cs.iter().for_each(|c| walk(c, a)),
                Condition::Unknown(_) => {}
            }
        }
        let mut a = Axes::default();
        if let Some(c) = &self.condition {
            walk(c, &mut a);
        }
        a
    }

    /// The condition against a container `size` CSS pixels wide and high
    /// (horizontal writing), with `font_size` for `em` and `style` giving
    /// its custom properties' computed values: `Some(true)`, `Some(false)`,
    /// or `None` for unknown, which `@container` treats as false.
    pub fn evaluate(
        &self,
        size: Option<(f64, f64)>,
        font_size: f64,
        style: &dyn Fn(&str) -> Option<String>,
    ) -> Option<bool> {
        let Some(c) = &self.condition else {
            return Some(true);
        };
        let env = size.map(|(width, height)| Environment {
            width,
            height,
            font_size,
            keywords: Vec::new(),
            ..Environment::default()
        });
        evaluate(c, env.as_ref(), &self.styles, style)
    }

    /// Whether the condition has a `style()` in it.
    pub fn has_style(&self) -> bool {
        !self.styles.is_empty()
    }
}

/// A container condition: media's three-valued logic over the size
/// features alone, `inline-size` as `width` and `block-size` as `height`.
fn evaluate(
    c: &Condition,
    env: Option<&Environment>,
    styles: &[(String, StyleQuery)],
    style: &dyn Fn(&str) -> Option<String>,
) -> Option<bool> {
    let rename = |n: &str| -> Option<String> {
        let (prefix, base) = match n.strip_prefix("min-") {
            Some(b) => ("min-", b),
            None => match n.strip_prefix("max-") {
                Some(b) => ("max-", b),
                None => ("", n),
            },
        };
        if !SIZE_FEATURES.contains(&base) {
            return None;
        }
        let base = match base {
            "inline-size" => "width",
            "block-size" => "height",
            b => b,
        };
        Some(format!("{prefix}{base}"))
    };
    match c {
        Condition::Feature(f) => {
            let f = match f {
                Feature::Boolean(n) => Feature::Boolean(rename(n)?),
                Feature::Plain(n, v) => Feature::Plain(rename(n)?, v.clone()),
                Feature::Range {
                    name,
                    before,
                    after,
                } => Feature::Range {
                    name: rename(name)?,
                    before: before.clone(),
                    after: after.clone(),
                },
            };
            f.evaluate(env?)
        }
        Condition::Parens(c) => evaluate(c, env, styles, style),
        Condition::Not(c) => evaluate(c, env, styles, style).map(|b| !b),
        Condition::Unknown(t) => styles
            .iter()
            .find(|(text, _)| text == t)
            .and_then(|(_, q)| q.evaluate(style)),
        Condition::And(cs) => three(cs.iter().map(|c| evaluate(c, env, styles, style)), false),
        Condition::Or(cs) => three(cs.iter().map(|c| evaluate(c, env, styles, style)), true),
    }
}

impl fmt::Display for ContainerQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(n) = &self.name {
            write_name(f, n)?;
            if self.condition.is_some() {
                f.write_char(' ')?;
            }
        }
        if let Some(c) = &self.condition {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

// ---- @scope --------------------------------------------------------------------

/// `@scope`'s prelude, CSS Cascade 6 §3.5: the selectors for the scoping
/// roots and limits, as written; `None` for a root is the parent of the
/// `<style>` the rule is in, or the rule's parent style rule.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ScopeBounds {
    pub start: Option<String>,
    pub end: Option<String>,
}

impl ScopeBounds {
    /// `[ ( <scope-start> ) ]? [ to ( <scope-end> ) ]?`.
    pub fn parse(prelude: &[ComponentValue<'_>]) -> Option<ScopeBounds> {
        let v = crate::media::solid(prelude);
        // A scoping root or limit is an element, so a selector that ends in
        // a pseudo-element makes the rule invalid. Parsec's selector parser
        // accepts one, as querySelector does, so it's ruled out here.
        let parens = |c: &ComponentValue<'_>| match c {
            ComponentValue::Block(b)
                if b.bracket == Bracket::Paren && !has_pseudo_element(&b.contents) =>
            {
                Some(text(&b.contents))
            }
            _ => None,
        };
        let mut i = 0;
        let mut out = ScopeBounds::default();
        if let Some(c) = v.first()
            && let Some(s) = parens(c)
        {
            out.start = Some(s);
            i = 1;
        }
        match &v[i..] {
            [] => {}
            [ComponentValue::Token(Token::Ident(w), _), c] if w.eq_ignore_ascii_case("to") => {
                out.end = Some(parens(c)?);
            }
            _ => return None,
        }
        Some(out)
    }
}

/// Whether a selector's top level names a pseudo-element: `::name`, or one
/// of the four the single colon still reaches. Inside a functional
/// pseudo-class one is either an error already or dropped.
fn has_pseudo_element(selector: &[ComponentValue<'_>]) -> bool {
    selector.windows(2).any(|w| match w {
        [
            ComponentValue::Token(Token::Colon, _),
            ComponentValue::Token(Token::Colon, _),
        ] => true,
        [
            ComponentValue::Token(Token::Colon, _),
            ComponentValue::Token(Token::Ident(name), _),
        ] => ["before", "after", "first-line", "first-letter"]
            .iter()
            .any(|legacy| name.eq_ignore_ascii_case(legacy)),
        _ => false,
    })
}

impl fmt::Display for ScopeBounds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(s) = &self.start {
            write!(f, "({s})")?;
        }
        if let Some(e) = &self.end {
            if self.start.is_some() {
                f.write_char(' ')?;
            }
            write!(f, "to ({e})")?;
        }
        Ok(())
    }
}

// ---- @import -------------------------------------------------------------------

/// `@import`, CSS Cascade 5 §2: the URL as written, the layer it puts the
/// sheet in, and the conditions on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Import {
    pub url: String,
    /// `None` for no layer, `Some(None)` for `layer` alone, an anonymous
    /// one, `Some(Some(name))` for `layer(name)`.
    pub layer: Option<Option<String>>,
    /// The `supports()` condition, as written, and whether it holds.
    pub supports: Option<(String, bool)>,
    pub media: MediaQueryList,
    /// `scope(…)`, CSS Cascade 6 §2.1: the imported sheet's rules as if in
    /// an `@scope` with these bounds.
    pub scope: Option<ScopeBounds>,
}

impl Import {
    /// The rule, if it's a valid `@import`.
    pub fn parse<B>(rule: &AtRule<'_, B>) -> Option<Import> {
        if !rule.name.eq_ignore_ascii_case("import") || rule.block.is_some() {
            return None;
        }
        let v: Vec<&ComponentValue<'_>> =
            rule.prelude.iter().filter(|v| !v.is_whitespace()).collect();
        let url = match v.first()? {
            ComponentValue::Token(Token::String(s) | Token::Url(s), _) => s.to_string(),
            ComponentValue::Function(f) if f.name.eq_ignore_ascii_case("url") => {
                match trimmed(&f.arguments) {
                    [ComponentValue::Token(Token::String(s), _)] => s.to_string(),
                    _ => return None,
                }
            }
            _ => return None,
        };
        let mut i = 1;
        let mut layer = None;
        match v.get(i) {
            Some(ComponentValue::Token(Token::Ident(w), _)) if w.eq_ignore_ascii_case("layer") => {
                layer = Some(None);
                i += 1;
            }
            Some(ComponentValue::Function(f)) if f.name.eq_ignore_ascii_case("layer") => {
                let name = text(&f.arguments);
                let parts: Vec<&str> = name.split('.').map(str::trim).collect();
                let ok = !parts.is_empty()
                    && parts.iter().all(|p| {
                        let c = crate::parse_component_values(p);
                        matches!(&c[..], [ComponentValue::Token(Token::Ident(_), _)])
                    });
                if !ok {
                    return None;
                }
                layer = Some(Some(parts.join(".")));
                i += 1;
            }
            _ => {}
        }
        let mut scope = None;
        if let Some(ComponentValue::Function(f)) = v.get(i)
            && f.name.eq_ignore_ascii_case("scope")
        {
            // `scope((.a) to (.b))`, or a root's selector alone.
            let bounds = ScopeBounds::parse(&f.arguments).or_else(|| {
                let start = text(&f.arguments);
                (!start.is_empty()).then_some(ScopeBounds {
                    start: Some(start),
                    end: None,
                })
            })?;
            scope = Some(bounds);
            i += 1;
        }
        let mut supports = None;
        if let Some(ComponentValue::Function(f)) = v.get(i)
            && f.name.eq_ignore_ascii_case("supports")
        {
            // A declaration alone, or a condition.
            let inner = trimmed(&f.arguments);
            let held = match crate::supports::supports(inner) {
                Some(b) => b,
                None => {
                    let wrapped = format!("({})", crate::to_css(inner));
                    crate::supports::supports(&crate::parse_component_values(&wrapped))
                        .unwrap_or(false)
                }
            };
            supports = Some((crate::to_css(inner), held));
            i += 1;
        }
        // What's left is the media query list.
        let first = v.get(i).map(|x| {
            rule.prelude
                .iter()
                .position(|p| std::ptr::eq(p, *x))
                .unwrap_or(rule.prelude.len())
        });
        let media = match first {
            Some(at) => MediaQueryList::parse(&rule.prelude[at..]),
            None => MediaQueryList::default(),
        };
        Some(Import {
            url,
            layer,
            supports,
            media,
            scope,
        })
    }

    /// Whether its conditions hold in `env`.
    pub fn applies(&self, env: &Environment) -> bool {
        self.supports.as_ref().is_none_or(|(_, held)| *held)
            && (self.media.queries.is_empty() || self.media.matches(env))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rule;

    fn at(css: &str) -> AtRule<'_, Block<'_>> {
        let mut sheet = crate::parse_stylesheet_deep(css);
        match sheet.rules.remove(0) {
            Rule::At(a) => a,
            _ => panic!("{css}"),
        }
    }

    #[test]
    fn keyframes() {
        let k = Keyframes::parse(&at(
            "@keyframes spin { from { rotate: 0deg } 50%, to { rotate: 1turn; color: red !important } 120% { x: y } }",
        ))
        .unwrap();
        assert_eq!(k.name, "spin");
        assert_eq!(k.frames.len(), 2);
        assert_eq!(
            k.to_string(),
            "@keyframes spin { 0% { rotate: 0deg; } 50%, 100% { rotate: 1turn; } }"
        );
        assert!(Keyframes::parse(&at("@keyframes none {}")).is_none());
        assert!(Keyframes::parse(&at("@keyframes \"a b\" {}")).is_some());
    }

    #[test]
    fn counter_styles() {
        let c = CounterStyle::parse(&at(
            "@counter-style x { system: fixed 3; symbols: a 'b'; range: 5 2; pad: 'z' 2; suffix: ' ' }",
        ))
        .unwrap();
        assert_eq!(c.get("system"), Some("fixed 3"));
        assert_eq!(c.get("symbols"), Some("a \"b\""));
        assert_eq!(c.get("range"), None);
        assert_eq!(c.get("pad"), Some("2 \"z\""));
        assert!(
            CounterStyle::parse(&at("@counter-style decimal { system: cyclic; symbols: a }"))
                .is_none()
        );
        let one = CounterStyle::parse(&at("@counter-style x { system: alphabetic; symbols: a }"))
            .unwrap();
        assert!(!one.usable() && c.usable());
        let none = CounterStyle::parse(&at("@counter-style x { system: extends none }")).unwrap();
        assert_eq!(none.get("system"), None);
    }

    #[test]
    fn properties() {
        let p = |css: &str| PropertyRule::parse(&at(css));
        assert!(
            p("@property --a { syntax: '<length>'; inherits: false; initial-value: 1em }")
                .is_none()
        );
        let no_inherits = p("@property --a { syntax: '<length>'; initial-value: 1px }").unwrap();
        assert!(no_inherits.inherits && no_inherits.registers());
        assert!(
            !p("@property --a { syntax: '<length>' }")
                .unwrap()
                .registers()
        );
        assert!(p("@property --a { syntax: '*'; inherits: true }").is_some());
        assert!(p("@property --a { syntax: '<color># | foo'; inherits: true; initial-value: red, blue }").is_some());
        assert_eq!(
            p("@property --a { syntax: '<colour>'; inherits: true }")
                .unwrap()
                .syntax,
            "*"
        );
        assert!(p("@property a { syntax: '*'; inherits: true }").is_none());
    }

    #[test]
    fn containers() {
        let q = ContainerQuery::parse_list(&crate::parse_component_values(
            "card (min-width: 400px) and (inline-size < 50em)",
        ))
        .unwrap();
        assert_eq!(q[0].name.as_deref(), Some("card"));
        let none = |_: &str| None;
        assert_eq!(q[0].evaluate(Some((500.0, 10.0)), 16.0, &none), Some(true));
        assert_eq!(q[0].evaluate(Some((300.0, 10.0)), 16.0, &none), Some(false));
        assert_eq!(
            q[0].axes(),
            Axes {
                inline: true,
                block: false
            }
        );
        let q = ContainerQuery::parse(&crate::parse_component_values("(color)")).unwrap();
        assert_eq!(q.evaluate(Some((500.0, 10.0)), 16.0, &none), None);
        let q = ContainerQuery::parse(&crate::parse_component_values(
            "style((--a: 1) and (not (--b)))",
        ))
        .unwrap();
        let get = |n: &str| (n == "--a").then(|| " 1".to_string());
        assert_eq!(q.evaluate(None, 16.0, &get), Some(true));
        assert!(ContainerQuery::parse(&crate::parse_component_values("name screen")).is_none());
    }

    #[test]
    fn a_scope_bound_cant_be_a_pseudo_element() {
        let bounds = |s: &str| ScopeBounds::parse(&crate::parse_component_values(s));
        assert!(bounds("(div::before)").is_none());
        assert!(bounds("(.a) to (div:after)").is_none());
        assert!(bounds("(div:hover) to (.b)").is_some());
        assert!(bounds("(a:is(.x, .y))").is_some());
    }

    #[test]
    fn scopes_and_imports() {
        let s = ScopeBounds::parse(&crate::parse_component_values("(.a) to (.b > c)")).unwrap();
        assert_eq!(
            (s.start.as_deref(), s.end.as_deref()),
            (Some(".a"), Some(".b > c"))
        );
        assert_eq!(s.to_string(), "(.a) to (.b > c)");
        assert!(ScopeBounds::parse(&crate::parse_component_values("to")).is_none());
        let i = Import::parse(&at(
            "@import url(a.css) layer(x.y) supports(display: grid) screen;",
        ))
        .unwrap();
        assert_eq!(i.url, "a.css");
        assert_eq!(i.layer, Some(Some("x.y".into())));
        assert_eq!(i.supports.as_ref().map(|s| s.1), Some(true));
        assert!(i.applies(&Environment::default()));
        let i = Import::parse(&at("@import 'b.css' supports(nonsense: 1);")).unwrap();
        assert!(!i.applies(&Environment::default()));
    }
}
