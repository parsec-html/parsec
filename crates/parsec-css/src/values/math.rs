// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Math functions, CSS Values and Units Level 4 §10: `calc()`, `min()`,
//! `max()`, `clamp()`, `round()`, `mod()`, `rem()`, the trigonometric and
//! exponential functions, `abs()` and `sign()`, with the constants `e`,
//! `pi`, `infinity`, `-infinity` and `NaN`.
//!
//! A function parses into a calculation tree, which is simplified as
//! §10.10 says (sums and products folded, absolute units made canonical,
//! functions of known values computed) and serialized as §10.12 says.
//! Whether it fits where it's used, a `<length>` say, is its type
//! (§10.9), checked by [`Math::fits`].

use std::fmt::{self, Write};

use super::units::{Kind, Unit, write_number};
use crate::parser::{Bracket, ComponentValue, Function};
use crate::tokenizer::Token;

/// How deep math functions and parentheses may nest. Browsers stop too;
/// past this a value is invalid rather than a risk to the stack.
const MAX_DEPTH: usize = 32;

/// A number, a percentage or a dimension: a calculation's leaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Leaf {
    Number(f64),
    Percentage(f64),
    Dimension(f64, Unit),
}

impl Leaf {
    pub fn value(self) -> f64 {
        match self {
            Leaf::Number(v) | Leaf::Percentage(v) | Leaf::Dimension(v, _) => v,
        }
    }

    fn with_value(self, v: f64) -> Leaf {
        match self {
            Leaf::Number(_) => Leaf::Number(v),
            Leaf::Percentage(_) => Leaf::Percentage(v),
            Leaf::Dimension(_, u) => Leaf::Dimension(v, u),
        }
    }

    /// Whether two leaves have the same unit, so they can be added.
    fn same_unit(self, other: Leaf) -> bool {
        match (self, other) {
            (Leaf::Number(_), Leaf::Number(_)) | (Leaf::Percentage(_), Leaf::Percentage(_)) => true,
            (Leaf::Dimension(_, a), Leaf::Dimension(_, b)) => a == b,
            _ => false,
        }
    }

    /// Whether this can be compared with another leaf of the same unit
    /// without knowing what it resolves against: numbers, and dimensions
    /// in a canonical unit.
    fn known(self) -> bool {
        match self {
            Leaf::Number(_) => true,
            Leaf::Percentage(_) => false,
            Leaf::Dimension(_, u) => u.canonical() == Some(u),
        }
    }

    /// The order §10.12 sorts a sum's or product's children in.
    fn rank(self) -> (u8, &'static str) {
        match self {
            Leaf::Number(_) => (0, ""),
            Leaf::Percentage(_) => (1, ""),
            Leaf::Dimension(_, u) => (2, u.as_str()),
        }
    }

    /// Writes the leaf as CSS: `1.5`, `50%`, `10px`.
    pub fn write(&self, out: &mut impl Write) -> fmt::Result {
        match *self {
            Leaf::Number(v) => write_number(out, v),
            Leaf::Percentage(v) => {
                write_number(out, v)?;
                out.write_char('%')
            }
            Leaf::Dimension(v, u) if !v.is_finite() => {
                // `infinity` has no unit of its own.
                write_number(out, v)?;
                write!(out, " * 1{u}")
            }
            Leaf::Dimension(v, u) => {
                write_number(out, v)?;
                out.write_str(u.as_str())
            }
        }
    }
}

impl fmt::Display for Leaf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write(f)
    }
}

/// `round()`'s rounding strategy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rounding {
    Nearest,
    Up,
    Down,
    ToZero,
}

/// A math function other than `calc()`, which only groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MathFunction {
    Min,
    Max,
    Clamp,
    Round(Rounding),
    Mod,
    Rem,
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Atan2,
    Pow,
    Sqrt,
    Hypot,
    Log,
    Exp,
    Abs,
    Sign,
    /// `sibling-index()` and `sibling-count()`, CSS Values 5 §8: numbers
    /// only the element's place in the tree can give.
    SiblingIndex,
    SiblingCount,
}

impl MathFunction {
    pub fn name(self) -> &'static str {
        use MathFunction::*;
        match self {
            Min => "min",
            Max => "max",
            Clamp => "clamp",
            Round(_) => "round",
            Mod => "mod",
            Rem => "rem",
            Sin => "sin",
            Cos => "cos",
            Tan => "tan",
            Asin => "asin",
            Acos => "acos",
            Atan => "atan",
            Atan2 => "atan2",
            Pow => "pow",
            Sqrt => "sqrt",
            Hypot => "hypot",
            Log => "log",
            Exp => "exp",
            Abs => "abs",
            Sign => "sign",
            SiblingIndex => "sibling-index",
            SiblingCount => "sibling-count",
        }
    }

    fn from_name(name: &str) -> Option<MathFunction> {
        use MathFunction::*;
        let all = [
            Min,
            Max,
            Clamp,
            Round(Rounding::Nearest),
            Mod,
            Rem,
            Sin,
            Cos,
            Tan,
            Asin,
            Acos,
            Atan,
            Atan2,
            Pow,
            Sqrt,
            Hypot,
            Log,
            Exp,
            Abs,
            Sign,
            SiblingIndex,
            SiblingCount,
        ];
        all.into_iter()
            .find(|f| f.name().eq_ignore_ascii_case(name))
    }

    /// How many arguments it takes, at least and at most.
    fn arity(self) -> (usize, usize) {
        use MathFunction::*;
        match self {
            Min | Max | Hypot => (1, usize::MAX),
            Clamp => (3, 3),
            Round(_) => (1, 2),
            Mod | Rem | Atan2 | Pow => (2, 2),
            Log => (1, 2),
            Sin | Cos | Tan | Asin | Acos | Atan | Sqrt | Exp | Abs | Sign => (1, 1),
            SiblingIndex | SiblingCount => (0, 0),
        }
    }
}

/// A calculation tree, CSS Values 4 §10.8.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Value(Leaf),
    Sum(Vec<Node>),
    Product(Vec<Node>),
    Negate(Box<Node>),
    Invert(Box<Node>),
    Function(MathFunction, Vec<Node>),
    /// A name that stands for a number not known yet: a relative color's
    /// channel, as `r` in `calc(r * 0.5)`.
    Keyword(&'static str),
}

type Keywords<'k> = &'k [&'static str];

/// A math function's value: its simplified calculation tree.
///
/// ```
/// use parsec_css::values::Math;
///
/// let values = parsec_css::parse_component_values("calc(1in + 2px - 10% * 2)");
/// let parsec_css::ComponentValue::Function(f) = &values[0] else { unreachable!() };
/// assert_eq!(Math::parse(f).unwrap().to_string(), "calc(-20% + 98px)");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Math {
    pub root: Node,
}

/// A math type, §10.9: the power of each kind of dimension, and of
/// percentages that resolve against nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Type([i8; 7]);

const PERCENT: usize = 6;

fn kind_index(k: Kind) -> usize {
    match k {
        Kind::Length => 0,
        Kind::Angle => 1,
        Kind::Time => 2,
        Kind::Frequency => 3,
        Kind::Resolution => 4,
        Kind::Flex => 5,
    }
}

impl Type {
    fn one(i: usize) -> Type {
        let mut t = Type::default();
        t.0[i] = 1;
        t
    }

    fn times(self, o: Type) -> Type {
        let mut t = self;
        for (a, b) in t.0.iter_mut().zip(o.0) {
            *a = a.saturating_add(b);
        }
        t
    }

    fn inverse(self) -> Type {
        Type(self.0.map(|e| e.saturating_neg()))
    }
}

/// What a value must be, for [`Math::fits`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    Number,
    Percentage,
    Dimension(Kind),
    /// A dimension, or a percentage of one: `<length-percentage>`.
    DimensionPercentage(Kind),
}

impl Math {
    /// Parses a math function, or `None` if it isn't one or doesn't parse.
    /// Whether its type suits where it's used is [`Math::fits`].
    pub fn parse(function: &Function<'_>) -> Option<Math> {
        Math::parse_with(function, &[])
    }

    /// Parses a math function in which `keywords` stand for numbers, as a
    /// relative color's channel names do: `calc(r * 0.5)`.
    pub fn parse_with(function: &Function<'_>, keywords: &[&'static str]) -> Option<Math> {
        let root = parse_function(function, 0, keywords)?;
        let root = simplify(root);
        // A type is needed for the tree to make sense at all.
        any_type(&root)?;
        Some(Math { root })
    }

    /// Whether `name` is a math function's name.
    pub fn is_math_function(name: &str) -> bool {
        name.eq_ignore_ascii_case("calc") || MathFunction::from_name(name).is_some()
    }

    /// Whether the function's type is what `expect` asks for.
    pub fn fits(&self, expect: Expect) -> bool {
        let (percent, want) = match expect {
            Expect::Number => (None, Type::default()),
            Expect::Percentage => (None, Type::one(PERCENT)),
            Expect::Dimension(k) => (None, Type::one(kind_index(k))),
            Expect::DimensionPercentage(k) => (Some(k), Type::one(kind_index(k))),
        };
        type_of(&self.root, percent, 0) == Some(want)
    }

    /// The function with each leaf replaced by what `f` makes of it, then
    /// simplified again: how a computed value turns `1em` into pixels and
    /// lets `calc(1em + 2px)` become `calc(18px)`.
    pub fn map_leaves(&self, f: &impl Fn(Leaf) -> Leaf) -> Math {
        fn map(n: &Node, f: &impl Fn(Leaf) -> Leaf) -> Node {
            match n {
                Node::Value(l) => Node::Value(f(*l)),
                Node::Keyword(k) => Node::Keyword(k),
                Node::Sum(c) => Node::Sum(c.iter().map(|x| map(x, f)).collect()),
                Node::Product(c) => Node::Product(c.iter().map(|x| map(x, f)).collect()),
                Node::Negate(c) => Node::Negate(Box::new(map(c, f))),
                Node::Invert(c) => Node::Invert(Box::new(map(c, f))),
                Node::Function(func, args) => {
                    Node::Function(*func, args.iter().map(|x| map(x, f)).collect())
                }
            }
        }
        Math {
            root: simplify(map(&self.root, f)),
        }
    }

    /// The function with its keywords (a relative colour's channel names)
    /// replaced by the numbers `f` gives, then simplified again.
    pub fn substitute_keywords(&self, f: &impl Fn(&str) -> Option<f64>) -> Math {
        fn map(n: &Node, f: &impl Fn(&str) -> Option<f64>) -> Node {
            match n {
                Node::Keyword(k) => f(k).map_or(Node::Keyword(k), |v| Node::Value(Leaf::Number(v))),
                Node::Value(l) => Node::Value(*l),
                Node::Sum(c) => Node::Sum(c.iter().map(|x| map(x, f)).collect()),
                Node::Product(c) => Node::Product(c.iter().map(|x| map(x, f)).collect()),
                Node::Negate(c) => Node::Negate(Box::new(map(c, f))),
                Node::Invert(c) => Node::Invert(Box::new(map(c, f))),
                Node::Function(func, args) => {
                    Node::Function(*func, args.iter().map(|x| map(x, f)).collect())
                }
            }
        }
        Math {
            root: simplify(map(&self.root, f)),
        }
    }

    /// The value, if simplifying left a single one.
    pub fn leaf(&self) -> Option<Leaf> {
        match self.root {
            Node::Value(l) => Some(l),
            _ => None,
        }
    }
}

/// A type that every node allows, trying percentages against each kind.
fn any_type(root: &Node) -> Option<Type> {
    type_of(root, None, 0).or_else(|| {
        [
            Kind::Length,
            Kind::Angle,
            Kind::Time,
            Kind::Frequency,
            Kind::Resolution,
        ]
        .into_iter()
        .find_map(|k| type_of(root, Some(k), 0))
    })
}

fn type_of(node: &Node, percent: Option<Kind>, depth: usize) -> Option<Type> {
    if depth > MAX_DEPTH * 4 {
        return None;
    }
    let same = |nodes: &[Node]| -> Option<Type> {
        let first = type_of(&nodes[0], percent, depth + 1)?;
        nodes[1..]
            .iter()
            .all(|n| type_of(n, percent, depth + 1) == Some(first))
            .then_some(first)
    };
    let number = Type::default();
    Some(match node {
        Node::Value(Leaf::Number(_)) | Node::Keyword(_) => number,
        Node::Value(Leaf::Percentage(_)) => match percent {
            Some(k) => Type::one(kind_index(k)),
            None => Type::one(PERCENT),
        },
        Node::Value(Leaf::Dimension(_, u)) => Type::one(kind_index(u.kind())),
        Node::Sum(c) => same(c)?,
        Node::Negate(c) => type_of(c, percent, depth + 1)?,
        Node::Invert(c) => type_of(c, percent, depth + 1)?.inverse(),
        Node::Product(c) => {
            let mut t = number;
            for n in c {
                t = t.times(type_of(n, percent, depth + 1)?);
            }
            t
        }
        Node::Function(f, args) => {
            use MathFunction::*;
            match f {
                Min | Max | Clamp | Hypot | Round(_) | Mod | Rem | Abs => same(args)?,
                Sign => {
                    type_of(&args[0], percent, depth + 1)?;
                    number
                }
                Sin | Cos | Tan => {
                    let t = type_of(&args[0], percent, depth + 1)?;
                    if t != number && t != Type::one(kind_index(Kind::Angle)) {
                        return None;
                    }
                    number
                }
                Asin | Acos | Atan => {
                    if type_of(&args[0], percent, depth + 1)? != number {
                        return None;
                    }
                    Type::one(kind_index(Kind::Angle))
                }
                Atan2 => {
                    same(args)?;
                    Type::one(kind_index(Kind::Angle))
                }
                SiblingIndex | SiblingCount => number,
                Pow | Sqrt | Log | Exp => {
                    if same(args)? != number {
                        return None;
                    }
                    number
                }
            }
        }
    })
}

// ---- parsing ---------------------------------------------------------------

fn parse_function(f: &Function<'_>, depth: usize, kw: Keywords) -> Option<Node> {
    if depth > MAX_DEPTH {
        return None;
    }
    if f.name.eq_ignore_ascii_case("calc") {
        return parse_sum(&f.arguments, depth + 1, kw);
    }
    let mut kind = MathFunction::from_name(&f.name)?;
    let mut groups: Vec<&[ComponentValue<'_>]> = f
        .arguments
        .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
        .collect();
    if let MathFunction::Round(_) = kind {
        // An optional rounding strategy first.
        let words: Vec<_> = groups[0].iter().filter(|v| !v.is_whitespace()).collect();
        if let [ComponentValue::Token(Token::Ident(w), _)] = words[..] {
            let r = match w.to_ascii_lowercase().as_str() {
                "nearest" => Some(Rounding::Nearest),
                "up" => Some(Rounding::Up),
                "down" => Some(Rounding::Down),
                "to-zero" => Some(Rounding::ToZero),
                _ => None,
            };
            if let Some(r) = r {
                kind = MathFunction::Round(r);
                groups.remove(0);
            }
        }
    }
    if groups.len() == 1 && groups[0].iter().all(ComponentValue::is_whitespace) {
        groups.clear();
    }
    let (min, max) = kind.arity();
    if groups.len() < min || groups.len() > max {
        return None;
    }
    let args = groups
        .into_iter()
        .map(|g| parse_sum(g, depth + 1, kw))
        .collect::<Option<Vec<_>>>()?;
    Some(Node::Function(kind, args))
}

/// `<calc-sum>`: products joined by `+` and `-` with whitespace around.
fn parse_sum(values: &[ComponentValue<'_>], depth: usize, kw: Keywords) -> Option<Node> {
    if depth > MAX_DEPTH {
        return None;
    }
    let mut terms = Vec::new();
    let mut start = 0;
    let mut negate = false;
    let mut i = 0;
    while i < values.len() {
        let op = match &values[i] {
            ComponentValue::Token(Token::Delim(c @ ('+' | '-')), _) => *c,
            _ => {
                i += 1;
                continue;
            }
        };
        let spaced = i > 0
            && values[i - 1].is_whitespace()
            && values.get(i + 1).is_some_and(ComponentValue::is_whitespace);
        if !spaced {
            return None;
        }
        let term = parse_product(&values[start..i], depth, kw)?;
        terms.push(if negate {
            Node::Negate(Box::new(term))
        } else {
            term
        });
        negate = op == '-';
        start = i + 1;
        i += 1;
    }
    let term = parse_product(&values[start..], depth, kw)?;
    terms.push(if negate {
        Node::Negate(Box::new(term))
    } else {
        term
    });
    Some(if terms.len() == 1 {
        terms.pop().unwrap()
    } else {
        Node::Sum(terms)
    })
}

/// `<calc-product>`: values joined by `*` and `/`.
fn parse_product(values: &[ComponentValue<'_>], depth: usize, kw: Keywords) -> Option<Node> {
    let mut factors = Vec::new();
    let mut invert = false;
    let mut expect_value = true;
    for v in values.iter().filter(|v| !v.is_whitespace()) {
        if expect_value {
            let node = parse_value(v, depth, kw)?;
            factors.push(if invert {
                Node::Invert(Box::new(node))
            } else {
                node
            });
            expect_value = false;
            continue;
        }
        match v {
            ComponentValue::Token(Token::Delim('*'), _) => invert = false,
            ComponentValue::Token(Token::Delim('/'), _) => invert = true,
            _ => return None,
        }
        expect_value = true;
    }
    if expect_value {
        return None;
    }
    Some(if factors.len() == 1 {
        factors.pop().unwrap()
    } else {
        Node::Product(factors)
    })
}

fn parse_value(v: &ComponentValue<'_>, depth: usize, kw: Keywords) -> Option<Node> {
    let leaf = match v {
        ComponentValue::Token(Token::Number(n), _) => Leaf::Number(n.value),
        ComponentValue::Token(Token::Percentage(n), _) => Leaf::Percentage(n.value),
        ComponentValue::Token(Token::Dimension { number, unit }, _) => {
            Leaf::Dimension(number.value, Unit::parse(unit)?)
        }
        ComponentValue::Token(Token::Ident(word), _) => {
            if let Some(k) = kw.iter().find(|k| word.eq_ignore_ascii_case(k)) {
                return Some(Node::Keyword(k));
            }
            Leaf::Number(match word.to_ascii_lowercase().as_str() {
                "e" => std::f64::consts::E,
                "pi" => std::f64::consts::PI,
                "infinity" => f64::INFINITY,
                "-infinity" => f64::NEG_INFINITY,
                "nan" => f64::NAN,
                _ => return None,
            })
        }
        ComponentValue::Block(b) if b.bracket == Bracket::Paren => {
            return parse_sum(&b.contents, depth + 1, kw);
        }
        ComponentValue::Function(f) => return parse_function(f, depth + 1, kw),
        _ => return None,
    };
    Some(Node::Value(leaf))
}

// ---- simplification, §10.10 ----------------------------------------------

/// A leaf in its canonical unit, if it has one.
fn canonical(leaf: Leaf) -> Leaf {
    match leaf {
        Leaf::Dimension(v, u) => match (u.canonical(), u.to_canonical()) {
            (Some(c), Some(f)) => Leaf::Dimension(v * f, c),
            _ => leaf,
        },
        l => l,
    }
}

fn simplify(node: Node) -> Node {
    match node {
        Node::Value(l) => Node::Value(canonical(l)),
        Node::Keyword(k) => Node::Keyword(k),
        Node::Negate(c) => match simplify(*c) {
            Node::Value(l) => Node::Value(l.with_value(-l.value())),
            Node::Negate(g) => *g,
            c => Node::Negate(Box::new(c)),
        },
        Node::Invert(c) => match simplify(*c) {
            Node::Value(Leaf::Number(v)) => Node::Value(Leaf::Number(1.0 / v)),
            Node::Invert(g) => *g,
            c => Node::Invert(Box::new(c)),
        },
        Node::Sum(children) => simplify_sum(children),
        Node::Product(children) => simplify_product(children),
        Node::Function(f, args) => {
            let args: Vec<Node> = args.into_iter().map(simplify).collect();
            compute(f, &args).map_or(Node::Function(f, args), Node::Value)
        }
    }
}

fn simplify_sum(children: Vec<Node>) -> Node {
    let mut flat = Vec::new();
    for c in children {
        match simplify(c) {
            Node::Sum(inner) => flat.extend(inner),
            c => flat.push(c),
        }
    }
    // Numeric children with the same unit add up into one.
    let mut out: Vec<Node> = Vec::new();
    for c in flat {
        if let Node::Value(l) = c
            && let Some(Node::Value(prev)) = out
                .iter_mut()
                .find(|o| matches!(o, Node::Value(p) if p.same_unit(l)))
        {
            *prev = prev.with_value(prev.value() + l.value());
            continue;
        }
        out.push(c);
    }
    if out.len() == 1 {
        out.pop().unwrap()
    } else {
        Node::Sum(out)
    }
}

fn simplify_product(children: Vec<Node>) -> Node {
    let mut flat = Vec::new();
    for c in children {
        match simplify(c) {
            Node::Product(inner) => flat.extend(inner),
            c => flat.push(c),
        }
    }
    // The numbers multiply into one.
    let mut number: Option<f64> = None;
    let mut rest = Vec::new();
    for c in flat {
        match c {
            Node::Value(Leaf::Number(v)) => *number.get_or_insert(1.0) *= v,
            c => rest.push(c),
        }
    }
    let Some(n) = number else {
        return product_of(rest);
    };
    match rest.as_mut_slice() {
        [] => return Node::Value(Leaf::Number(n)),
        // A number times one value, or times a sum of values, scales them.
        [Node::Value(l)] => return Node::Value(l.with_value(l.value() * n)),
        [Node::Sum(terms)] if terms.iter().all(|t| matches!(t, Node::Value(_))) => {
            for t in terms.iter_mut() {
                if let Node::Value(l) = t {
                    *l = l.with_value(l.value() * n);
                }
            }
            return rest.pop().unwrap();
        }
        _ => {}
    }
    rest.insert(0, Node::Value(Leaf::Number(n)));
    product_of(rest)
}

fn product_of(mut nodes: Vec<Node>) -> Node {
    if nodes.len() == 1 {
        nodes.pop().unwrap()
    } else {
        Node::Product(nodes)
    }
}

/// A function of values known well enough to work out, worked out.
fn compute(f: MathFunction, args: &[Node]) -> Option<Leaf> {
    use MathFunction::*;
    if matches!(f, SiblingIndex | SiblingCount) {
        return None;
    }
    let leaves: Vec<Leaf> = args
        .iter()
        .map(|a| match a {
            Node::Value(l) => Some(*l),
            _ => None,
        })
        .collect::<Option<_>>()?;
    let first = leaves[0];
    // Percentages of one thing compare as their numbers do.
    let alike = leaves
        .iter()
        .all(|l| l.same_unit(first) && (l.known() || matches!(l, Leaf::Percentage(_))));
    let v: Vec<f64> = leaves.iter().map(|l| l.value()).collect();
    let number = |x: f64| Some(Leaf::Number(x));
    let angle = |x: f64| Some(Leaf::Dimension(x.to_degrees(), Unit::Deg));
    let radians = |l: Leaf| match l {
        Leaf::Number(x) => Some(x),
        Leaf::Dimension(x, Unit::Deg) => Some(x.to_radians()),
        _ => None,
    };
    match f {
        SiblingIndex | SiblingCount => None,
        Min | Max | Clamp | Hypot | Round(_) | Mod | Rem | Abs if !alike => None,
        Min => Some(first.with_value(v.iter().copied().fold(f64::INFINITY, nan_min))),
        Max => Some(first.with_value(v.iter().copied().fold(f64::NEG_INFINITY, nan_max))),
        Clamp => Some(first.with_value(nan_max(v[0], nan_min(v[1], v[2])))),
        Hypot => Some(first.with_value(v.iter().map(|x| x * x).sum::<f64>().sqrt())),
        Abs => Some(first.with_value(v[0].abs())),
        Round(r) => {
            let step = v.get(1).copied().unwrap_or(1.0);
            Some(first.with_value(round(r, v[0], step)))
        }
        Mod | Rem => {
            let (a, b) = (v[0], v[1]);
            let mut m = a % b;
            if f == Mod && m != 0.0 && (m < 0.0) != (b < 0.0) {
                m += b;
            }
            Some(first.with_value(m))
        }
        Sign if first.known() => number(if v[0] == 0.0 || v[0].is_nan() {
            v[0]
        } else {
            v[0].signum()
        }),
        Sign => None,
        Sin => number(radians(first)?.sin()),
        Cos => number(radians(first)?.cos()),
        Tan => number(radians(first)?.tan()),
        Asin => angle(radians(first)?.asin()),
        Acos => angle(radians(first)?.acos()),
        Atan => angle(radians(first)?.atan()),
        Atan2 if alike => angle(v[0].atan2(v[1])),
        Atan2 => None,
        Pow => number(v[0].powf(v[1])),
        Sqrt => number(v[0].sqrt()),
        Exp => number(v[0].exp()),
        Log => number(match v.get(1) {
            Some(base) => v[0].ln() / base.ln(),
            None => v[0].ln(),
        }),
    }
}

/// NaN wins, as the spec has `min()` and `max()` propagate it.
fn nan_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

fn nan_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

fn round(r: Rounding, a: f64, b: f64) -> f64 {
    if b == 0.0 {
        return f64::NAN;
    }
    if b.is_infinite() {
        return if a.is_infinite() {
            f64::NAN
        } else {
            a.signum() * 0.0
        };
    }
    let q = a / b;
    let n = match r {
        Rounding::Nearest => (q + 0.5).floor(),
        Rounding::Up => q.ceil(),
        Rounding::Down => q.floor(),
        Rounding::ToZero => q.trunc(),
    };
    n * b
}

// ---- serialization, §10.12 ----------------------------------------------

fn rank(node: &Node) -> (u8, &'static str) {
    match node {
        Node::Value(l) => l.rank(),
        _ => (3, ""),
    }
}

fn sorted(nodes: &[Node]) -> Vec<&Node> {
    let mut v: Vec<&Node> = nodes.iter().collect();
    // Stable, so the rest keep their order.
    v.sort_by(|a, b| rank(a).cmp(&rank(b)));
    v
}

fn write_node(out: &mut String, node: &Node) {
    match node {
        Node::Value(l) => l.write(out).unwrap(),
        Node::Keyword(k) => out.push_str(k),
        Node::Function(f, args) => write_function(out, *f, args),
        Node::Negate(c) => {
            out.push_str("(-1 * ");
            write_node(out, c);
            out.push(')');
        }
        Node::Invert(c) => {
            out.push_str("(1 / ");
            write_node(out, c);
            out.push(')');
        }
        Node::Sum(children) => {
            out.push('(');
            for (i, c) in sorted(children).into_iter().enumerate() {
                if i == 0 {
                    write_node(out, c);
                    continue;
                }
                match c {
                    Node::Negate(inner) => {
                        out.push_str(" - ");
                        write_node(out, inner);
                    }
                    Node::Value(l) if l.value() < 0.0 => {
                        out.push_str(" - ");
                        l.with_value(-l.value()).write(out).unwrap();
                    }
                    c => {
                        out.push_str(" + ");
                        write_node(out, c);
                    }
                }
            }
            out.push(')');
        }
        Node::Product(children) => {
            out.push('(');
            for (i, c) in sorted(children).into_iter().enumerate() {
                match c {
                    Node::Invert(inner) if i > 0 => {
                        out.push_str(" / ");
                        write_node(out, inner);
                    }
                    c => {
                        if i > 0 {
                            out.push_str(" * ");
                        }
                        write_node(out, c);
                    }
                }
            }
            out.push(')');
        }
    }
}

/// Writes `node` without the parentheses around it, if it has them.
fn write_bare(out: &mut String, node: &Node) {
    let mut s = String::new();
    write_node(&mut s, node);
    let bare = match (s.strip_prefix('('), s.ends_with(')')) {
        (Some(inner), true) if matches!(node, Node::Sum(_) | Node::Product(_)) => {
            &inner[..inner.len() - 1]
        }
        _ => &s,
    };
    out.push_str(bare);
}

fn write_function(out: &mut String, f: MathFunction, args: &[Node]) {
    out.push_str(f.name());
    out.push('(');
    if let MathFunction::Round(r) = f {
        let strategy = match r {
            Rounding::Nearest => "",
            Rounding::Up => "up, ",
            Rounding::Down => "down, ",
            Rounding::ToZero => "to-zero, ",
        };
        out.push_str(strategy);
    }
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        write_bare(out, a);
    }
    out.push(')');
}

impl fmt::Display for Math {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = String::new();
        match &self.root {
            Node::Function(func, args) => write_function(&mut s, *func, args),
            root => {
                s.push_str("calc(");
                write_bare(&mut s, root);
                s.push(')');
            }
        }
        f.write_str(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calc(css: &str) -> Option<String> {
        let values = crate::parse_component_values(css);
        let ComponentValue::Function(f) = &values[0] else {
            return None;
        };
        Math::parse(f).map(|m| m.to_string())
    }

    #[test]
    fn simplifies_and_serializes() {
        let cases = [
            ("calc(1px + 2px)", "calc(3px)"),
            ("calc(1in)", "calc(96px)"),
            ("calc(10px + 5%)", "calc(5% + 10px)"),
            ("calc(1em - 2px)", "calc(1em - 2px)"),
            ("calc(2 * (1em + 2px))", "calc(2em + 4px)"),
            (
                "calc(50% + (sign(1em - 10px) * 10%))",
                "calc(50% + (10% * sign(1em - 10px)))",
            ),
            ("min(1px, 2px)", "calc(1px)"),
            ("min(1em, 2px)", "min(1em, 2px)"),
            ("calc(min(1em, 2px))", "min(1em, 2px)"),
            ("clamp(1px, 5px, 3px)", "calc(3px)"),
            ("calc(infinity)", "calc(infinity)"),
            ("calc(0 / 0)", "calc(NaN)"),
            ("round(up, 2.5, 1)", "calc(3)"),
            ("mod(-5, 3)", "calc(1)"),
            ("rem(-5, 3)", "calc(-2)"),
            ("calc(1px * 2 / 4)", "calc(0.5px)"),
            ("sin(90deg)", "calc(1)"),
            ("atan2(1px, 1px)", "calc(45deg)"),
        ];
        for (css, want) in cases {
            assert_eq!(calc(css).as_deref(), Some(want), "{css}");
        }
    }

    #[test]
    fn rejects_bad_syntax_and_types() {
        for css in [
            "calc(1px+2px)",
            "calc(1px + 2)",
            "calc()",
            "calc(1px 2px)",
            "calc(1foo)",
            "sin(1px)",
        ] {
            assert_eq!(calc(css), None, "{css}");
        }
        let values = crate::parse_component_values("calc(10% + 1px)");
        let ComponentValue::Function(f) = &values[0] else {
            unreachable!()
        };
        let m = Math::parse(f).unwrap();
        assert!(m.fits(Expect::DimensionPercentage(Kind::Length)));
        assert!(!m.fits(Expect::Dimension(Kind::Length)));
    }

    #[test]
    fn deep_nesting_is_invalid_not_a_crash() {
        let css = format!("calc({}1{})", "(".repeat(10_000), ")".repeat(10_000));
        assert_eq!(calc(&css), None);
    }
}
