// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Media queries, Media Queries Level 5: a `<media-query-list>` parsed,
//! written back as CSSOM's `mediaText` writes it, and evaluated against
//! an [`Environment`] you give, with the three-valued logic §3.2 has, so
//! a feature nobody knows makes its query false rather than an error.

use std::fmt::{self, Write};

use crate::parser::{Bracket, ComponentValue};
use crate::tokenizer::Token;
use crate::values::{Expect, Kind, Leaf, Math, Unit, write_number};

/// What a media query is evaluated against: the viewport and the device.
/// [`Environment::default`] is a 1280×720 colour screen at 1dppx with a
/// fine pointer that can hover, light colour scheme, no preferences set.
#[derive(Clone, Debug, PartialEq)]
pub struct Environment {
    /// `screen` or `print`; every other media type matches nothing.
    pub media_type: MediaType,
    /// The viewport, in CSS pixels.
    pub width: f64,
    pub height: f64,
    /// The whole screen, in CSS pixels.
    pub device_width: f64,
    pub device_height: f64,
    /// Device pixels per CSS pixel.
    pub resolution: f64,
    /// Bits per colour component, 0 for a monochrome device.
    pub color: u32,
    pub color_index: u32,
    /// Bits per pixel of a monochrome device, 0 otherwise.
    pub monochrome: u32,
    /// The font size `em` in a media query is relative to, in pixels.
    pub font_size: f64,
    /// The value of each discrete feature, by name: `hover` to `hover`,
    /// `prefers-color-scheme` to `light`. A feature not here is unknown.
    pub keywords: Vec<(&'static str, &'static str)>,
}

impl Default for Environment {
    fn default() -> Environment {
        Environment {
            media_type: MediaType::Screen,
            width: 1280.0,
            height: 720.0,
            device_width: 1280.0,
            device_height: 720.0,
            resolution: 1.0,
            color: 8,
            color_index: 0,
            monochrome: 0,
            font_size: 16.0,
            keywords: vec![
                // A screen isn't scanned: `(scan)` is known and false.
                ("scan", "none"),
                ("grid", "0"),
                ("update", "fast"),
                ("overflow-block", "scroll"),
                ("overflow-inline", "scroll"),
                ("color-gamut", "srgb"),
                ("dynamic-range", "standard"),
                ("video-dynamic-range", "standard"),
                ("inverted-colors", "none"),
                ("pointer", "fine"),
                ("any-pointer", "fine"),
                ("hover", "hover"),
                ("any-hover", "hover"),
                ("scripting", "none"),
                ("prefers-reduced-motion", "no-preference"),
                ("prefers-reduced-transparency", "no-preference"),
                ("prefers-reduced-data", "no-preference"),
                ("prefers-contrast", "no-preference"),
                ("prefers-color-scheme", "light"),
                ("forced-colors", "none"),
                ("display-mode", "browser"),
                ("environment-blending", "opaque"),
            ],
        }
    }
}

impl Environment {
    fn keyword(&self, feature: &str) -> Option<&'static str> {
        self.keywords
            .iter()
            .find(|(f, _)| *f == feature)
            .map(|(_, v)| *v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaType {
    Screen,
    Print,
}

/// The discrete features and the values each can take. `none` (or `0`)
/// is the value that makes the boolean form false.
const DISCRETE: &[(&str, &[&str])] = &[
    ("orientation", &["portrait", "landscape"]),
    ("scan", &["interlace", "progressive"]),
    ("grid", &[]),
    ("update", &["none", "slow", "fast"]),
    ("overflow-block", &["none", "scroll", "paged"]),
    ("overflow-inline", &["none", "scroll"]),
    ("color-gamut", &["srgb", "p3", "rec2020"]),
    ("dynamic-range", &["standard", "high"]),
    ("video-dynamic-range", &["standard", "high"]),
    ("inverted-colors", &["none", "inverted"]),
    ("pointer", &["none", "coarse", "fine"]),
    ("any-pointer", &["none", "coarse", "fine"]),
    ("hover", &["none", "hover"]),
    ("any-hover", &["none", "hover"]),
    ("scripting", &["none", "initial-only", "enabled"]),
    ("prefers-reduced-motion", &["no-preference", "reduce"]),
    ("prefers-reduced-transparency", &["no-preference", "reduce"]),
    ("prefers-reduced-data", &["no-preference", "reduce"]),
    (
        "prefers-contrast",
        &["no-preference", "less", "more", "custom"],
    ),
    ("prefers-color-scheme", &["light", "dark"]),
    ("forced-colors", &["none", "active"]),
    (
        "display-mode",
        &[
            "fullscreen",
            "standalone",
            "minimal-ui",
            "browser",
            "picture-in-picture",
        ],
    ),
    (
        "environment-blending",
        &["opaque", "additive", "subtractive"],
    ),
];

/// The range features, and what their values are.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RangeType {
    Length,
    Ratio,
    Resolution,
    Integer,
}

const RANGE: &[(&str, RangeType)] = &[
    ("width", RangeType::Length),
    ("height", RangeType::Length),
    ("device-width", RangeType::Length),
    ("device-height", RangeType::Length),
    ("aspect-ratio", RangeType::Ratio),
    ("device-aspect-ratio", RangeType::Ratio),
    ("resolution", RangeType::Resolution),
    ("color", RangeType::Integer),
    ("color-index", RangeType::Integer),
    ("monochrome", RangeType::Integer),
];

/// A value in a media feature.
#[derive(Clone, Debug, PartialEq)]
pub enum FeatureValue {
    Number(f64),
    /// A number written with a fraction or an exponent, `1.0`: not an
    /// `<integer>`.
    Real(f64),
    /// A length or resolution, as written.
    Dimension(f64, Unit),
    Math(Math),
    /// `a / b`.
    Ratio(f64, f64),
    Ident(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Comparison {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

impl Comparison {
    fn as_str(self) -> &'static str {
        match self {
            Comparison::Lt => "<",
            Comparison::Le => "<=",
            Comparison::Gt => ">",
            Comparison::Ge => ">=",
            Comparison::Eq => "=",
        }
    }

    fn flip(self) -> Comparison {
        match self {
            Comparison::Lt => Comparison::Gt,
            Comparison::Le => Comparison::Ge,
            Comparison::Gt => Comparison::Lt,
            Comparison::Ge => Comparison::Le,
            Comparison::Eq => Comparison::Eq,
        }
    }

    fn test(self, a: f64, b: f64) -> bool {
        match self {
            Comparison::Lt => a < b,
            Comparison::Le => a <= b,
            Comparison::Gt => a > b,
            Comparison::Ge => a >= b,
            Comparison::Eq => a == b,
        }
    }
}

/// `( … )` in a query.
#[derive(Clone, Debug, PartialEq)]
pub enum Feature {
    /// `(name)`.
    Boolean(String),
    /// `(name: value)`, `min-` and `max-` names included.
    Plain(String, FeatureValue),
    /// `(name >= value)` or `(value < name < value)`, written in the
    /// order it was, the name on the side given.
    Range {
        name: String,
        /// `value op name`.
        before: Option<(FeatureValue, Comparison)>,
        /// `name op value`.
        after: Option<(Comparison, FeatureValue)>,
    },
}

/// A media condition.
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    Feature(Feature),
    Not(Box<Condition>),
    And(Vec<Condition>),
    Or(Vec<Condition>),
    /// `( … )` around a condition.
    Parens(Box<Condition>),
    /// Anything else in parentheses or a function: unknown, kept as text.
    Unknown(String),
}

/// One media query.
#[derive(Clone, Debug, PartialEq)]
pub enum MediaQuery {
    /// A query that didn't parse, which is `not all`.
    Invalid,
    /// `[not | only] type [and condition]`, or a bare condition.
    Query {
        qualifier: Option<Qualifier>,
        media_type: Option<String>,
        condition: Option<Condition>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qualifier {
    Not,
    Only,
}

/// A comma-separated list of media queries: what `@media` and the
/// `media` attribute hold.
///
/// ```
/// use parsec_css::media::{Environment, MediaQueryList};
///
/// let list = MediaQueryList::parse(&parsec_css::parse_component_values("screen AND (min-width:600px), PRINT"));
/// assert_eq!(list.to_string(), "screen and (min-width: 600px), print");
/// assert!(list.matches(&Environment::default()));
/// let narrow = Environment { width: 500.0, ..Environment::default() };
/// assert!(!list.matches(&narrow));
/// ```
#[derive(Clone, Debug, PartialEq, Default)]
pub struct MediaQueryList {
    pub queries: Vec<MediaQuery>,
}

// ---- parsing ---------------------------------------------------------------

/// How deep conditions may nest in parentheses.
const MAX_DEPTH: usize = 64;

fn solid<'v, 'a>(values: &'v [ComponentValue<'a>]) -> Vec<&'v ComponentValue<'a>> {
    values.iter().filter(|v| !v.is_whitespace()).collect()
}

fn ident<'v>(v: Option<&&'v ComponentValue<'_>>) -> Option<&'v str> {
    match v {
        Some(ComponentValue::Token(Token::Ident(s), _)) => Some(s),
        _ => None,
    }
}

fn is_word(v: Option<&&ComponentValue<'_>>, word: &str) -> bool {
    ident(v).is_some_and(|s| s.eq_ignore_ascii_case(word))
}

impl MediaQueryList {
    /// Parses a media query list; a query that doesn't parse becomes
    /// `not all`, as the spec says, and the rest stand.
    pub fn parse(values: &[ComponentValue<'_>]) -> MediaQueryList {
        if values.iter().all(ComponentValue::is_whitespace) {
            return MediaQueryList::default();
        }
        let queries = values
            .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
            .map(|q| parse_query(q).unwrap_or(MediaQuery::Invalid))
            .collect();
        MediaQueryList { queries }
    }

    /// [`MediaQueryList::parse`] from text.
    pub fn parse_str(css: &str) -> MediaQueryList {
        MediaQueryList::parse(&crate::parse_component_values(css))
    }

    /// Whether any query matches; an empty list matches everything.
    pub fn matches(&self, env: &Environment) -> bool {
        self.queries.is_empty() || self.queries.iter().any(|q| q.evaluate(env) == Some(true))
    }
}

fn parse_query(values: &[ComponentValue<'_>]) -> Option<MediaQuery> {
    let v = solid(values);
    if v.is_empty() {
        return None;
    }
    // A condition on its own.
    let bare_not = is_word(v.first(), "not")
        && !matches!(v.get(1), Some(ComponentValue::Token(Token::Ident(_), _)));
    if !matches!(v[0], ComponentValue::Token(Token::Ident(_), _)) || bare_not {
        let condition = parse_condition(&v, true, 0)?;
        return Some(MediaQuery::Query {
            qualifier: None,
            media_type: None,
            condition: Some(condition),
        });
    }
    let mut i = 0;
    let qualifier = if is_word(v.first(), "not") {
        i += 1;
        Some(Qualifier::Not)
    } else if is_word(v.first(), "only") {
        i += 1;
        Some(Qualifier::Only)
    } else {
        None
    };
    let media_type = ident(v.get(i))?;
    if ["only", "not", "and", "or", "layer"]
        .iter()
        .any(|w| media_type.eq_ignore_ascii_case(w))
    {
        return None;
    }
    i += 1;
    let condition = if i < v.len() {
        if !is_word(v.get(i), "and") {
            return None;
        }
        Some(parse_condition(&v[i + 1..], false, 0)?)
    } else {
        None
    };
    Some(MediaQuery::Query {
        qualifier,
        media_type: Some(media_type.to_ascii_lowercase()),
        condition,
    })
}

/// A condition: `not X`, or `X [and X]*`, or (if `or_ok`) `X [or X]*`.
fn parse_condition(v: &[&ComponentValue<'_>], or_ok: bool, depth: usize) -> Option<Condition> {
    if depth > MAX_DEPTH || v.is_empty() {
        return None;
    }
    if is_word(v.first(), "not") {
        if v.len() != 2 {
            return None;
        }
        return Some(Condition::Not(Box::new(in_parens(v[1], depth)?)));
    }
    let first = in_parens(v[0], depth)?;
    if v.len() == 1 {
        return Some(first);
    }
    let joiner = ident(v.get(1))?.to_ascii_lowercase();
    if joiner != "and" && !(joiner == "or" && or_ok) {
        return None;
    }
    let mut items = vec![first];
    let mut i = 1;
    while i < v.len() {
        if !is_word(v.get(i), &joiner) {
            return None;
        }
        items.push(in_parens(v.get(i + 1)?, depth)?);
        i += 2;
    }
    Some(if joiner == "and" {
        Condition::And(items)
    } else {
        Condition::Or(items)
    })
}

/// `( condition )`, `( feature )`, or anything else in parentheses or a
/// function, which is unknown.
fn in_parens(v: &ComponentValue<'_>, depth: usize) -> Option<Condition> {
    match v {
        ComponentValue::Block(b) if b.bracket == Bracket::Paren => {
            let inner = solid(&b.contents);
            if let Some(c) = parse_condition(&inner, true, depth + 1) {
                return Some(Condition::Parens(Box::new(c)));
            }
            if let Some(f) = parse_feature(&inner) {
                return Some(Condition::Feature(f));
            }
            Some(Condition::Unknown(crate::to_css(std::slice::from_ref(v))))
        }
        ComponentValue::Function(_) => {
            Some(Condition::Unknown(crate::to_css(std::slice::from_ref(v))))
        }
        _ => None,
    }
}

fn value(v: &[&ComponentValue<'_>]) -> Option<FeatureValue> {
    match v {
        [ComponentValue::Token(Token::Number(n), _)] if n.integer => {
            Some(FeatureValue::Number(n.value))
        }
        [ComponentValue::Token(Token::Number(n), _)] => Some(FeatureValue::Real(n.value)),
        [ComponentValue::Token(Token::Dimension { number, unit }, _)] => {
            Some(FeatureValue::Dimension(number.value, Unit::parse(unit)?))
        }
        [ComponentValue::Token(Token::Ident(s), _)] => {
            Some(FeatureValue::Ident(s.to_ascii_lowercase()))
        }
        [ComponentValue::Function(f)] if Math::is_math_function(&f.name) => {
            Some(FeatureValue::Math(Math::parse(f)?))
        }
        [a, ComponentValue::Token(Token::Delim('/'), _), b] => {
            let num = |v: &ComponentValue<'_>| match v {
                ComponentValue::Token(Token::Number(n), _) => Some(n.value),
                ComponentValue::Function(f) if Math::is_math_function(&f.name) => {
                    match Math::parse(f)?.leaf()? {
                        Leaf::Number(x) => Some(x),
                        _ => None,
                    }
                }
                _ => None,
            };
            Some(FeatureValue::Ratio(num(a)?, num(b)?))
        }
        _ => None,
    }
}

fn comparison(v: &[&ComponentValue<'_>]) -> Option<(Comparison, usize)> {
    let delim = |i: usize| match v.get(i) {
        Some(ComponentValue::Token(Token::Delim(c), _)) => Some(*c),
        _ => None,
    };
    let c = delim(0)?;
    // `>=` is two delims written together; `> =` isn't one.
    let together = match (v.first(), v.get(1)) {
        (Some(a), Some(b)) => a.span().end == b.span().start,
        _ => false,
    };
    Some(match (c, delim(1)) {
        ('<', Some('=')) if together => (Comparison::Le, 2),
        ('>', Some('=')) if together => (Comparison::Ge, 2),
        ('<', _) => (Comparison::Lt, 1),
        ('>', _) => (Comparison::Gt, 1),
        ('=', _) => (Comparison::Eq, 1),
        _ => return None,
    })
}

fn parse_feature(v: &[&ComponentValue<'_>]) -> Option<Feature> {
    // `(name)` and `(name: value)`.
    if let Some(name) = ident(v.first()) {
        if v.len() == 1 {
            return Some(Feature::Boolean(name.to_ascii_lowercase()));
        }
        if matches!(v.get(1), Some(ComponentValue::Token(Token::Colon, _))) {
            return Some(Feature::Plain(name.to_ascii_lowercase(), value(&v[2..])?));
        }
        // `name op value`.
        let (op, n) = comparison(&v[1..])?;
        return Some(Feature::Range {
            name: name.to_ascii_lowercase(),
            before: None,
            after: Some((op, value(&v[1 + n..])?)),
        });
    }
    // `value op name [op value]`: find the name, the one ident.
    let at = v
        .iter()
        .position(|x| matches!(x, ComponentValue::Token(Token::Ident(_), _)))?;
    let name = ident(v.get(at))?.to_ascii_lowercase();
    // The value before it ends at the comparison before the name.
    let mut op_start = at;
    while op_start > 0 && matches!(v[op_start - 1], ComponentValue::Token(Token::Delim(_), _)) {
        op_start -= 1;
    }
    let (op1, n1) = comparison(&v[op_start..at])?;
    if op_start + n1 != at {
        return None;
    }
    let first = value(&v[..op_start])?;
    let after = if at + 1 < v.len() {
        let (op2, n2) = comparison(&v[at + 1..])?;
        // Both `<`-ish or both `>`-ish, never `=`.
        let lt = |o: Comparison| matches!(o, Comparison::Lt | Comparison::Le);
        let gt = |o: Comparison| matches!(o, Comparison::Gt | Comparison::Ge);
        if !(lt(op1) && lt(op2) || gt(op1) && gt(op2)) {
            return None;
        }
        Some((op2, value(&v[at + 1 + n2..])?))
    } else {
        None
    };
    Some(Feature::Range {
        name,
        before: Some((first, op1)),
        after,
    })
}

// ---- evaluation ------------------------------------------------------------

/// A value in the feature's canonical unit: pixels, dppx, or a number.
fn resolve(v: &FeatureValue, t: RangeType, env: &Environment) -> Option<f64> {
    let length = |x: f64, u: Unit| -> Option<f64> {
        match u {
            Unit::Em | Unit::Rem => Some(x * env.font_size),
            Unit::Ex
            | Unit::Rex
            | Unit::Cap
            | Unit::Rcap
            | Unit::Ch
            | Unit::Rch
            | Unit::Ic
            | Unit::Ric => Some(x * env.font_size / 2.0),
            Unit::Lh | Unit::Rlh => Some(x * env.font_size * 1.2),
            Unit::Vw
            | Unit::Svw
            | Unit::Lvw
            | Unit::Dvw
            | Unit::Vi
            | Unit::Svi
            | Unit::Lvi
            | Unit::Dvi => Some(x * env.width / 100.0),
            Unit::Vh
            | Unit::Svh
            | Unit::Lvh
            | Unit::Dvh
            | Unit::Vb
            | Unit::Svb
            | Unit::Lvb
            | Unit::Dvb => Some(x * env.height / 100.0),
            Unit::Vmin | Unit::Svmin | Unit::Lvmin | Unit::Dvmin => {
                Some(x * env.width.min(env.height) / 100.0)
            }
            Unit::Vmax | Unit::Svmax | Unit::Lvmax | Unit::Dvmax => {
                Some(x * env.width.max(env.height) / 100.0)
            }
            u if u.kind() == Kind::Length => Some(x * u.to_canonical()?),
            _ => None,
        }
    };
    match (t, v) {
        (RangeType::Length, FeatureValue::Number(n) | FeatureValue::Real(n)) if *n == 0.0 => {
            Some(0.0)
        }
        (RangeType::Length, FeatureValue::Dimension(x, u)) => length(*x, *u),
        (RangeType::Length, FeatureValue::Math(m)) if m.fits(Expect::Dimension(Kind::Length)) => {
            match m.leaf()? {
                Leaf::Dimension(x, u) => length(x, u),
                _ => None,
            }
        }
        (RangeType::Resolution, FeatureValue::Dimension(x, u))
            if u.kind() == Kind::Resolution && *x >= 0.0 =>
        {
            Some(x * u.to_canonical()?)
        }
        (RangeType::Resolution, FeatureValue::Math(m)) => match m.leaf()? {
            Leaf::Dimension(x, u) if u.kind() == Kind::Resolution => Some(x * u.to_canonical()?),
            _ => None,
        },
        (RangeType::Integer, FeatureValue::Number(n)) if n.fract() == 0.0 => Some(*n),
        (RangeType::Integer, FeatureValue::Math(m)) => match m.leaf()? {
            Leaf::Number(x) => Some(x.round()),
            _ => None,
        },
        (RangeType::Ratio, FeatureValue::Number(n) | FeatureValue::Real(n)) if *n >= 0.0 => {
            Some(*n)
        }
        (RangeType::Ratio, FeatureValue::Ratio(a, b)) if *a >= 0.0 && *b >= 0.0 => {
            Some(if *b == 0.0 {
                if *a == 0.0 { f64::NAN } else { f64::INFINITY }
            } else {
                a / b
            })
        }
        _ => None,
    }
}

fn actual(name: &str, env: &Environment) -> f64 {
    match name {
        "width" => env.width,
        "height" => env.height,
        "device-width" => env.device_width,
        "device-height" => env.device_height,
        "aspect-ratio" => env.width / env.height,
        "device-aspect-ratio" => env.device_width / env.device_height,
        "resolution" => env.resolution,
        "color" => f64::from(env.color),
        "color-index" => f64::from(env.color_index),
        "monochrome" => f64::from(env.monochrome),
        _ => f64::NAN,
    }
}

/// Exact for ratios, which are compared as fractions: 59/80 is 413/560.
fn cmp(op: Comparison, a: f64, b: f64, t: RangeType) -> bool {
    if t == RangeType::Ratio && op == Comparison::Eq {
        return (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0);
    }
    op.test(a, b)
}

impl Feature {
    /// `Some(true)`, `Some(false)`, or `None` for unknown.
    pub fn evaluate(&self, env: &Environment) -> Option<bool> {
        match self {
            Feature::Boolean(name) => {
                if RANGE.iter().any(|(n, _)| n == name) {
                    let a = actual(name, env);
                    return Some(a != 0.0 && !a.is_nan());
                }
                if name == "orientation" {
                    return Some(true);
                }
                if name == "grid" {
                    return Some(env.keyword("grid")? != "0");
                }
                DISCRETE.iter().find(|(n, _)| n == name)?;
                // False only for the value that says there's nothing.
                Some(!matches!(env.keyword(name)?, "none" | "no-preference"))
            }
            Feature::Plain(name, v) => {
                let (prefix, base) = match name.strip_prefix("min-") {
                    Some(b) => (Some(Comparison::Ge), b),
                    None => match name.strip_prefix("max-") {
                        Some(b) => (Some(Comparison::Le), b),
                        None => (None, name.as_str()),
                    },
                };
                if let Some((_, t)) = RANGE.iter().find(|(n, _)| *n == base) {
                    let want = resolve(v, *t, env)?;
                    let op = prefix.unwrap_or(Comparison::Eq);
                    return Some(cmp(op, actual(base, env), want, *t));
                }
                if prefix.is_some() {
                    return None;
                }
                let FeatureValue::Ident(word) = v else {
                    if name == "grid"
                        && let FeatureValue::Number(n) = v
                        && (*n == 0.0 || *n == 1.0)
                    {
                        return Some((*n == 1.0) == (env.keyword("grid")? != "0"));
                    }
                    return None;
                };
                if name == "orientation" {
                    let portrait = env.height >= env.width;
                    return match word.as_str() {
                        "portrait" => Some(portrait),
                        "landscape" => Some(!portrait),
                        _ => None,
                    };
                }
                let (_, values) = DISCRETE.iter().find(|(n, _)| *n == name)?;
                if !values.contains(&word.as_str()) {
                    return None;
                }
                Some(env.keyword(name)? == word)
            }
            Feature::Range {
                name,
                before,
                after,
            } => {
                let (_, t) = RANGE.iter().find(|(n, _)| n == name)?;
                let a = actual(name, env);
                let mut ok = true;
                if let Some((v, op)) = before {
                    // `v op name` is `name flip(op) v`.
                    ok &= cmp(op.flip(), a, resolve(v, *t, env)?, *t);
                }
                if let Some((op, v)) = after {
                    ok &= cmp(*op, a, resolve(v, *t, env)?, *t);
                }
                Some(ok)
            }
        }
    }
}

impl Condition {
    pub fn evaluate(&self, env: &Environment) -> Option<bool> {
        match self {
            Condition::Feature(f) => f.evaluate(env),
            Condition::Parens(c) => c.evaluate(env),
            Condition::Not(c) => c.evaluate(env).map(|b| !b),
            Condition::Unknown(_) => None,
            // Three-valued: false beats unknown in `and`, true in `or`.
            Condition::And(cs) => {
                let mut unknown = false;
                for c in cs {
                    match c.evaluate(env) {
                        Some(false) => return Some(false),
                        None => unknown = true,
                        Some(true) => {}
                    }
                }
                if unknown { None } else { Some(true) }
            }
            Condition::Or(cs) => {
                let mut unknown = false;
                for c in cs {
                    match c.evaluate(env) {
                        Some(true) => return Some(true),
                        None => unknown = true,
                        Some(false) => {}
                    }
                }
                if unknown { None } else { Some(false) }
            }
        }
    }
}

impl MediaQuery {
    /// Whether the query matches; unknown counts as false at the top.
    pub fn evaluate(&self, env: &Environment) -> Option<bool> {
        let MediaQuery::Query {
            qualifier,
            media_type,
            condition,
        } = self
        else {
            return Some(false);
        };
        let type_ok = match media_type.as_deref() {
            None | Some("all") => true,
            Some("screen") => env.media_type == MediaType::Screen,
            Some("print") => env.media_type == MediaType::Print,
            Some(_) => false,
        };
        let result = match condition {
            Some(c) if type_ok => c.evaluate(env),
            Some(_) => Some(false),
            None => Some(type_ok),
        };
        // Unknown is false, `not` or no `not`: `not all and (foo: bar)`
        // matches nothing, which is how a feature is told to be unknown.
        match qualifier {
            Some(Qualifier::Not) => Some(result.map(|b| !b).unwrap_or(false)),
            _ => Some(result.unwrap_or(false)),
        }
    }
}

// ---- serialization ---------------------------------------------------------

impl fmt::Display for FeatureValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeatureValue::Number(n) | FeatureValue::Real(n) => write_number(f, *n),
            FeatureValue::Dimension(v, u) => {
                write_number(f, *v)?;
                f.write_str(u.as_str())
            }
            FeatureValue::Math(m) => write!(f, "{m}"),
            FeatureValue::Ratio(a, b) => {
                write_number(f, *a)?;
                f.write_str(" / ")?;
                write_number(f, *b)
            }
            FeatureValue::Ident(s) => crate::write_identifier(f, s),
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_char('(')?;
        match self {
            Feature::Boolean(n) => crate::write_identifier(f, n)?,
            Feature::Plain(n, v) => {
                crate::write_identifier(f, n)?;
                write!(f, ": {v}")?;
            }
            Feature::Range {
                name,
                before,
                after,
            } => {
                if let Some((v, op)) = before {
                    write!(f, "{v} {} ", op.as_str())?;
                }
                crate::write_identifier(f, name)?;
                if let Some((op, v)) = after {
                    write!(f, " {} {v}", op.as_str())?;
                }
            }
        }
        f.write_char(')')
    }
}

impl fmt::Display for Condition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Condition::Feature(x) => write!(f, "{x}"),
            Condition::Parens(c) => write!(f, "({c})"),
            Condition::Not(c) => write!(f, "not {c}"),
            Condition::Unknown(s) => f.write_str(s),
            Condition::And(cs) | Condition::Or(cs) => {
                let join = if matches!(self, Condition::And(_)) {
                    " and "
                } else {
                    " or "
                };
                for (i, c) in cs.iter().enumerate() {
                    if i > 0 {
                        f.write_str(join)?;
                    }
                    write!(f, "{c}")?;
                }
                Ok(())
            }
        }
    }
}

impl fmt::Display for MediaQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let MediaQuery::Query {
            qualifier,
            media_type,
            condition,
        } = self
        else {
            return f.write_str("not all");
        };
        match qualifier {
            Some(Qualifier::Not) => f.write_str("not ")?,
            Some(Qualifier::Only) => f.write_str("only ")?,
            None => {}
        }
        match (media_type, condition) {
            // `all and X` is written `X`, unless a qualifier needs the type.
            (Some(t), Some(c)) if t == "all" && qualifier.is_none() => write!(f, "{c}"),
            (Some(t), Some(c)) => {
                crate::write_identifier(f, t)?;
                write!(f, " and {c}")
            }
            (Some(t), None) => crate::write_identifier(f, t),
            (None, Some(c)) => write!(f, "{c}"),
            (None, None) => Ok(()),
        }
    }
}

impl fmt::Display for MediaQueryList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, q) in self.queries.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{q}")?;
        }
        Ok(())
    }
}
