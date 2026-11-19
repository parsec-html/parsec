// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `<color>`, CSS Color Level 4: hex colors, the named and system colors,
//! `currentcolor`, `transparent`, `rgb()`, `hsl()`, `hwb()`, `lab()`,
//! `lch()`, `oklab()`, `oklch()` and `color()` in every predefined space,
//! with `none` and math functions in any component; and from Level 5,
//! `device-cmyk()` and `light-dark()`.
//!
//! Values are kept as written, so a color serializes the way §15 says:
//! the legacy sRGB forms as `rgb(r, g, b)`, the others in their own
//! function, with `none` and unresolved math kept.

use std::fmt::{self, Write};

use super::math::{Expect, Leaf, Math};
use super::units::{Kind, write_number};
use crate::parser::{ComponentValue, Function};
use crate::tokenizer::Token;

/// One channel: a number, `none`, or a math function that couldn't be
/// worked out yet.
#[derive(Clone, Debug, PartialEq)]
pub enum Channel {
    None,
    Number(f64),
    Math(Math),
}

impl Channel {
    fn number(&self) -> Option<f64> {
        match self {
            Channel::Number(v) => Some(*v),
            _ => None,
        }
    }
}

/// A `color()` space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Predefined {
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
}

impl Predefined {
    const ALL: [(Predefined, &'static str); 9] = [
        (Predefined::Srgb, "srgb"),
        (Predefined::SrgbLinear, "srgb-linear"),
        (Predefined::DisplayP3, "display-p3"),
        (Predefined::DisplayP3Linear, "display-p3-linear"),
        (Predefined::A98Rgb, "a98-rgb"),
        (Predefined::ProphotoRgb, "prophoto-rgb"),
        (Predefined::Rec2020, "rec2020"),
        (Predefined::XyzD50, "xyz-d50"),
        (Predefined::XyzD65, "xyz-d65"),
    ];

    pub fn name(self) -> &'static str {
        Predefined::ALL.iter().find(|(p, _)| *p == self).unwrap().1
    }

    fn parse(name: &str) -> Option<Predefined> {
        if name.eq_ignore_ascii_case("xyz") {
            return Some(Predefined::XyzD65);
        }
        Predefined::ALL
            .iter()
            .find(|(_, n)| n.eq_ignore_ascii_case(name))
            .map(|(p, _)| *p)
    }
}

/// How a color's channels are to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Model {
    /// sRGB, 0 to 255: hex, named colors, `rgb()` and `rgba()`.
    Rgb,
    /// Hue in degrees, saturation and lightness in percent.
    Hsl,
    /// Hue, whiteness and blackness.
    Hwb,
    Lab,
    Lch,
    Oklab,
    Oklch,
    /// `color(space …)`.
    Color(Predefined),
    /// `device-cmyk()`, four channels from 0 to 1.
    DeviceCmyk,
}

/// A color with channels: everything but the keywords that stand for
/// one.
#[derive(Clone, Debug, PartialEq)]
pub struct Channels {
    pub model: Model,
    /// Three, or four for `device-cmyk()`.
    pub channels: Vec<Channel>,
    pub alpha: Channel,
}

/// A `<color>`.
///
/// ```
/// use parsec_css::values::Color;
///
/// let v = parsec_css::parse_component_value("hsl(120 30% 50%)").unwrap();
/// assert_eq!(Color::parse(&v).unwrap().to_string(), "rgb(89, 166, 89)");
/// let v = parsec_css::parse_component_value("lab(50% 50% -20 / 25%)").unwrap();
/// assert_eq!(Color::parse(&v).unwrap().to_string(), "lab(50 62.5 -20 / 0.25)");
/// ```
#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    CurrentColor,
    /// A named color or `transparent`, in lowercase, which a specified
    /// value keeps as its name.
    Named(&'static str),
    /// A system color, in lowercase: `canvastext`.
    System(&'static str),
    Channels(Channels),
    LightDark(Box<Color>, Box<Color>),
    /// CSS Color 5's relative color syntax: `rgb(from red r g calc(b / 2))`.
    Relative(Box<Relative>),
    /// `contrast-color(<color>)`.
    Contrast(Box<Color>),
    /// `color-mix()` or `color-layers()`, as matched by their grammars
    /// and written out, with what `color-mix()` mixes.
    Mix(Box<Mix>),
    /// A colour a computed value has worked out, written out.
    Text(String),
}

/// A relative color: an origin color and channels that may name the
/// origin's, kept as written until there's a value to work them out from.
#[derive(Clone, Debug, PartialEq)]
pub struct Relative {
    /// `rgb`, `hsl`, `hwb`, `lab`, `lch`, `oklab`, `oklch` or `color`.
    pub function: &'static str,
    /// For `color()`, its space.
    pub space: Option<Predefined>,
    pub origin: Color,
    pub channels: Vec<RelativeChannel>,
    pub alpha: Option<RelativeChannel>,
}

/// A `color-mix()`'s space, hue method, and colours with their
/// percentages.
pub type MixParts = (
    super::convert::Space,
    &'static str,
    Vec<(Color, Option<f64>)>,
);

/// `color-mix()` or `color-layers()`.
#[derive(Clone, Debug, PartialEq)]
pub struct Mix {
    /// The function written out, as a specified value writes it.
    pub text: String,
    /// For `color-mix()`: the space, the hue method, and each colour with
    /// its percentage if it has one.
    pub parts: Option<MixParts>,
}

/// One channel of a relative color, as written.
#[derive(Clone, Debug, PartialEq)]
pub enum RelativeChannel {
    Number(f64),
    Percentage(f64),
    Angle(f64, super::units::Unit),
    /// One of the origin's channels: `r`, `h`, `alpha` and so on.
    Keyword(&'static str),
    None,
    Math(Math),
}

include!("named_colors.rs");

const SYSTEM: &[&str] = &[
    "accentcolor",
    "accentcolortext",
    "activetext",
    "buttonborder",
    "buttonface",
    "buttontext",
    "canvas",
    "canvastext",
    "field",
    "fieldtext",
    "graytext",
    "highlight",
    "highlighttext",
    "linktext",
    "mark",
    "marktext",
    "selecteditem",
    "selecteditemtext",
    "visitedtext",
    // Deprecated, still valid.
    "activeborder",
    "activecaption",
    "appworkspace",
    "background",
    "buttonhighlight",
    "buttonshadow",
    "captiontext",
    "inactiveborder",
    "inactivecaption",
    "inactivecaptiontext",
    "infobackground",
    "infotext",
    "menu",
    "menutext",
    "scrollbar",
    "threeddarkshadow",
    "threedface",
    "threedhighlight",
    "threedlightshadow",
    "threedshadow",
    "window",
    "windowframe",
    "windowtext",
];

fn rgb(r: f64, g: f64, b: f64, a: f64) -> Color {
    Color::Channels(Channels {
        model: Model::Rgb,
        channels: vec![Channel::Number(r), Channel::Number(g), Channel::Number(b)],
        alpha: Channel::Number(a),
    })
}

/// What a channel may be, and what 100% of it is.
#[derive(Clone, Copy)]
enum Slot {
    /// A number or a percentage of this.
    Scaled(f64),
    Hue,
}

impl Color {
    /// The colour's channels in its own space, worked out: relative
    /// colours and mixes included. `None` for `currentcolor`, a system
    /// colour, or math that needs an element.
    pub fn absolute(&self) -> Option<super::convert::Absolute> {
        super::convert::absolute(self)
    }

    /// The color with named colors made `rgb()`, as a computed value
    /// has them; everything else as it is.
    pub fn computed(&self) -> Color {
        match self {
            Color::Named("transparent") => rgb(0.0, 0.0, 0.0, 0.0),
            Color::Named(name) => {
                let i = NAMED.binary_search_by(|(n, _)| n.cmp(name)).unwrap();
                let [r, g, b] = NAMED[i].1.map(f64::from);
                rgb(r, g, b, 1.0)
            }
            Color::LightDark(a, b) => {
                Color::LightDark(Box::new(a.computed()), Box::new(b.computed()))
            }
            c => c.clone(),
        }
    }

    /// The color `value` is, or `None` if it isn't one.
    pub fn parse(value: &ComponentValue<'_>) -> Option<Color> {
        match value {
            ComponentValue::Token(Token::Ident(word), _) => Color::keyword(word),
            ComponentValue::Token(Token::Hash { value, .. }, _) => hex(value),
            ComponentValue::Function(f)
                if f.name.eq_ignore_ascii_case("color-mix")
                    || f.name.eq_ignore_ascii_case("color-layers") =>
            {
                guarded(|| {
                    let mut buf = [0; 32];
                    let name = super::units::lowercase(&f.name, &mut buf)?;
                    let c = super::value::match_function(name, std::slice::from_ref(value))?;
                    let mut text = String::new();
                    super::value::write_components(&mut text, &c).ok()?;
                    let parts = super::convert::mix_parts(f);
                    Some(Color::Mix(Box::new(Mix { text, parts })))
                })
            }
            ComponentValue::Function(f) => function(f),
            _ => None,
        }
    }

    fn keyword(word: &str) -> Option<Color> {
        let mut buf = [0; 32];
        let lower = super::units::lowercase(word, &mut buf)?;
        if lower == "currentcolor" {
            return Some(Color::CurrentColor);
        }
        if lower == "transparent" {
            return Some(Color::Named("transparent"));
        }
        if let Ok(i) = NAMED.binary_search_by(|(n, _)| (*n).cmp(lower)) {
            return Some(Color::Named(NAMED[i].0));
        }
        SYSTEM
            .iter()
            .find(|s| **s == lower)
            .map(|s| Color::System(s))
    }
}

fn hex(digits: &str) -> Option<Color> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let d: Vec<f64> = digits
        .bytes()
        .map(|b| f64::from((b as char).to_digit(16).unwrap()))
        .collect();
    let (r, g, b, a) = match d.len() {
        3 => (d[0] * 17.0, d[1] * 17.0, d[2] * 17.0, 255.0),
        4 => (d[0] * 17.0, d[1] * 17.0, d[2] * 17.0, d[3] * 17.0),
        6 => (
            d[0] * 16.0 + d[1],
            d[2] * 16.0 + d[3],
            d[4] * 16.0 + d[5],
            255.0,
        ),
        8 => (
            d[0] * 16.0 + d[1],
            d[2] * 16.0 + d[3],
            d[4] * 16.0 + d[5],
            d[6] * 16.0 + d[7],
        ),
        _ => return None,
    };
    Some(rgb(r, g, b, a / 255.0))
}

/// The arguments of a color function: the channels, the alpha after a
/// `/` or the fourth comma, and whether commas separated them.
struct Args<'v, 'a> {
    channels: Vec<&'v ComponentValue<'a>>,
    alpha: Option<&'v ComponentValue<'a>>,
    commas: bool,
}

fn split_args<'v, 'a>(values: &'v [ComponentValue<'a>], count: usize) -> Option<Args<'v, 'a>> {
    let solid: Vec<&ComponentValue<'_>> = values.iter().filter(|v| !v.is_whitespace()).collect();
    let is =
        |v: &ComponentValue<'_>, t: &Token<'_>| matches!(v, ComponentValue::Token(x, _) if x == t);
    let commas = solid.iter().any(|v| is(v, &Token::Comma));
    if commas {
        // a, b, c[, d]: every other value a comma.
        let mut items = Vec::new();
        for (i, v) in solid.iter().enumerate() {
            if i % 2 == 1 {
                if !is(v, &Token::Comma) {
                    return None;
                }
            } else {
                items.push(*v);
            }
        }
        if solid.len().is_multiple_of(2) {
            return None;
        }
        let alpha = if items.len() == count + 1 {
            items.pop()
        } else {
            None
        };
        return Some(Args {
            channels: items,
            alpha,
            commas,
        });
    }
    let slash = solid.iter().position(|v| is(v, &Token::Delim('/')));
    let (channels, alpha) = match slash {
        Some(i) => {
            if solid.len() != i + 2 {
                return None;
            }
            (solid[..i].to_vec(), Some(solid[i + 1]))
        }
        None => (solid, None),
    };
    Some(Args {
        channels,
        alpha,
        commas,
    })
}

/// A channel's value, read for `slot`: percentages scaled, angles in
/// degrees, `none` only outside the legacy comma syntax; and how it was
/// written.
fn channel(v: &ComponentValue<'_>, slot: Slot, legacy: bool) -> Option<(Channel, Written)> {
    let scale = match slot {
        Slot::Scaled(s) => s,
        Slot::Hue => 1.0,
    };
    Some(match v {
        ComponentValue::Token(Token::Number(n), _) => (Channel::Number(n.value), Written::Number),
        ComponentValue::Token(Token::Percentage(n), _) if matches!(slot, Slot::Scaled(_)) => {
            (Channel::Number(n.value / 100.0 * scale), Written::Percent)
        }
        ComponentValue::Token(Token::Dimension { number, unit }, _)
            if matches!(slot, Slot::Hue) =>
        {
            let u = super::units::Unit::parse(unit)?;
            if u.kind() != Kind::Angle {
                return None;
            }
            (
                Channel::Number(number.value * u.to_canonical()?),
                Written::Number,
            )
        }
        ComponentValue::Token(Token::Ident(w), _) if !legacy && w.eq_ignore_ascii_case("none") => {
            (Channel::None, Written::None)
        }
        ComponentValue::Function(f) if Math::is_math_function(&f.name) => {
            let m = Math::parse(f)?;
            let (ok, unit) = match slot {
                Slot::Hue => (
                    m.fits(Expect::Number) || m.fits(Expect::Dimension(Kind::Angle)),
                    Written::Number,
                ),
                Slot::Scaled(_) if m.fits(Expect::Number) => (true, Written::Number),
                Slot::Scaled(_) => (m.fits(Expect::Percentage), Written::Percent),
            };
            if !ok {
                return None;
            }
            (Channel::Math(m), unit)
        }
        _ => return None,
    })
}

/// What a channel was written as, for the legacy forms' rule that the
/// three be all numbers or all percentages.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Written {
    Number,
    Percent,
    None,
}

/// Works out a math channel in a legacy form, where it's resolved.
fn resolve(c: Channel, scale: f64) -> Channel {
    match c {
        Channel::Math(m) => match m.leaf() {
            Some(Leaf::Number(v)) => Channel::Number(nan_zero(v)),
            Some(Leaf::Percentage(v)) => Channel::Number(nan_zero(v) / 100.0 * scale),
            Some(Leaf::Dimension(v, _)) => Channel::Number(nan_zero(v)),
            None => Channel::Math(m),
        },
        c => c,
    }
}

fn nan_zero(v: f64) -> f64 {
    if v.is_nan() { 0.0 } else { v }
}

/// How deep colors may nest inside colors, through `from` and the like.
const MAX_DEPTH: usize = 16;

fn function(f: &Function<'_>) -> Option<Color> {
    guarded(|| function_at(f))
}

/// Colors nest in colors (`light-dark()`, `rgb(from …)`); past a limit a
/// color is invalid rather than a risk to the stack.
fn guarded(f: impl FnOnce() -> Option<Color>) -> Option<Color> {
    let depth = DEPTH.with(|d| d.get());
    if depth >= MAX_DEPTH {
        return None;
    }
    DEPTH.with(|d| d.set(depth + 1));
    let result = f();
    DEPTH.with(|d| d.set(depth));
    result
}

fn function_at(f: &Function<'_>) -> Option<Color> {
    let mut buf = [0; 32];
    let name = super::units::lowercase(&f.name, &mut buf)?;
    let first = f.arguments.iter().find(|v| !v.is_whitespace());
    if matches!(first, Some(ComponentValue::Token(Token::Ident(w), _)) if w.eq_ignore_ascii_case("from"))
    {
        return relative(f, name);
    }
    if name == "contrast-color" {
        let solid: Vec<_> = f.arguments.iter().filter(|v| !v.is_whitespace()).collect();
        return match solid[..] {
            [v] => Some(Color::Contrast(Box::new(Color::parse(v)?))),
            _ => None,
        };
    }
    if name == "light-dark" {
        let parts: Vec<&[ComponentValue<'_>]> = f
            .arguments
            .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
            .collect();
        let [a, b] = parts[..] else { return None };
        let one = |vs: &[ComponentValue<'_>]| {
            let solid: Vec<_> = vs.iter().filter(|v| !v.is_whitespace()).collect();
            match solid[..] {
                [v] => Color::parse(v),
                _ => None,
            }
        };
        return Some(Color::LightDark(Box::new(one(a)?), Box::new(one(b)?)));
    }
    let mut values: &[ComponentValue<'_>] = &f.arguments;
    let model = match name {
        "rgb" | "rgba" => Model::Rgb,
        "hsl" | "hsla" => Model::Hsl,
        "hwb" => Model::Hwb,
        "lab" => Model::Lab,
        "lch" => Model::Lch,
        "oklab" => Model::Oklab,
        "oklch" => Model::Oklch,
        "device-cmyk" => Model::DeviceCmyk,
        "color" => {
            let start = values.iter().position(|v| !v.is_whitespace())?;
            let ComponentValue::Token(Token::Ident(space), _) = &values[start] else {
                return None;
            };
            values = &values[start + 1..];
            Model::Color(Predefined::parse(space)?)
        }
        _ => return None,
    };
    let count = if model == Model::DeviceCmyk { 4 } else { 3 };
    let args = split_args(values, count)?;
    let legacy = args.commas;
    if legacy && !matches!(model, Model::Rgb | Model::Hsl | Model::DeviceCmyk) {
        return None;
    }
    let slots: &[Slot] = match model {
        Model::Rgb => &[Slot::Scaled(255.0); 3],
        Model::Hsl | Model::Hwb => &[Slot::Hue, Slot::Scaled(100.0), Slot::Scaled(100.0)],
        Model::Lab => &[
            Slot::Scaled(100.0),
            Slot::Scaled(125.0),
            Slot::Scaled(125.0),
        ],
        Model::Lch => &[Slot::Scaled(100.0), Slot::Scaled(150.0), Slot::Hue],
        Model::Oklab => &[Slot::Scaled(1.0), Slot::Scaled(0.4), Slot::Scaled(0.4)],
        Model::Oklch => &[Slot::Scaled(1.0), Slot::Scaled(0.4), Slot::Hue],
        Model::Color(_) => &[Slot::Scaled(1.0); 3],
        Model::DeviceCmyk => &[Slot::Scaled(1.0); 4],
    };
    if args.channels.len() != slots.len() {
        return None;
    }
    let mut channels = Vec::with_capacity(slots.len());
    let mut units = Vec::with_capacity(slots.len());
    for (v, slot) in args.channels.iter().zip(slots) {
        let (c, u) = channel(v, *slot, legacy)?;
        channels.push(c);
        units.push(u);
    }
    if legacy {
        match model {
            // All numbers or all percentages.
            Model::Rgb if units.iter().any(|u| *u != units[0]) => return None,
            // Saturation and lightness are percentages.
            Model::Hsl if units[1..].iter().any(|u| *u != Written::Percent) => return None,
            Model::DeviceCmyk if units.iter().any(|u| *u != Written::Number) => return None,
            _ => {}
        }
    }
    let mut alpha = match args.alpha {
        Some(v) => channel(v, Slot::Scaled(1.0), legacy)?.0,
        None => Channel::Number(1.0),
    };
    // Parse-time clamping, of what was written as a number.
    let clamp = |c: &mut Channel, lo: f64, hi: f64| {
        if let Channel::Number(v) = c {
            *v = v.clamp(lo, hi);
        }
    };
    let legacy_model = matches!(model, Model::Rgb | Model::Hsl | Model::Hwb);
    if legacy_model {
        for (i, c) in channels.iter_mut().enumerate() {
            let scale = match slots[i] {
                Slot::Scaled(s) => s,
                Slot::Hue => 1.0,
            };
            *c = resolve(std::mem::replace(c, Channel::None), scale);
        }
        alpha = resolve(alpha, 1.0);
    }
    if let Channel::Number(a) = &mut alpha {
        *a = nan_zero(*a).clamp(0.0, 1.0);
    }
    match model {
        Model::Rgb => channels.iter_mut().for_each(|c| clamp(c, 0.0, 255.0)),
        Model::DeviceCmyk => channels.iter_mut().for_each(|c| clamp(c, 0.0, 1.0)),
        Model::Hsl => clamp(&mut channels[1], 0.0, f64::INFINITY),
        Model::Lab => clamp(&mut channels[0], 0.0, 100.0),
        Model::Lch => {
            clamp(&mut channels[0], 0.0, 100.0);
            clamp(&mut channels[1], 0.0, f64::INFINITY);
        }
        Model::Oklab => clamp(&mut channels[0], 0.0, 1.0),
        Model::Oklch => {
            clamp(&mut channels[0], 0.0, 1.0);
            clamp(&mut channels[1], 0.0, f64::INFINITY);
        }
        _ => {}
    }
    // Hues go round to [0, 360).
    for (c, slot) in channels.iter_mut().zip(slots) {
        if let (Slot::Hue, Channel::Number(h)) = (slot, &mut *c) {
            *h = if h.is_finite() {
                h.rem_euclid(360.0)
            } else {
                0.0
            };
        }
    }
    Some(Color::Channels(Channels {
        model,
        channels,
        alpha,
    }))
}

// ---- conversion and serialization ------------------------------------------

/// sRGB from 0 to 1, from HSL in degrees and percent.
fn hsl_to_rgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let (s, l) = (s / 100.0, l / 100.0);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn hwb_to_rgb(h: f64, w: f64, b: f64) -> [f64; 3] {
    let (w, b) = (w / 100.0, b / 100.0);
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray; 3];
    }
    hsl_to_rgb(h, 100.0, 50.0).map(|c| c * (1.0 - w - b) + w)
}

/// A legacy color's alpha: two decimals if that's still the same in
/// eight bits, otherwise three.
fn write_legacy_alpha(out: &mut impl Write, a: f64) -> fmt::Result {
    let bits = (a * 255.0).round();
    let two = (a * 100.0).round() / 100.0;
    let v = if (two * 255.0).round() == bits {
        two
    } else {
        (a * 1000.0).round() / 1000.0
    };
    write_number(out, v)
}

fn write_channel(out: &mut impl Write, c: &Channel, percent: bool) -> fmt::Result {
    match c {
        Channel::None => out.write_str("none"),
        Channel::Number(v) => {
            write_number(out, *v)?;
            if percent {
                out.write_char('%')?;
            }
            Ok(())
        }
        Channel::Math(m) => write!(out, "{m}"),
    }
}

fn write_alpha(out: &mut impl Write, a: &Channel) -> fmt::Result {
    if *a == Channel::Number(1.0) {
        return Ok(());
    }
    out.write_str(" / ")?;
    write_channel(out, a, false)
}

impl Channels {
    pub(crate) fn model_is_legacy(&self) -> bool {
        matches!(self.model, Model::Rgb | Model::Hsl | Model::Hwb)
    }

    pub(crate) fn has_math(&self) -> bool {
        self.channels
            .iter()
            .chain([&self.alpha])
            .any(|c| matches!(c, Channel::Math(_)))
    }

    /// Each math channel replaced by what `f` makes of it.
    pub(crate) fn map_math(&mut self, f: &impl Fn(&Math) -> Math) {
        for c in self.channels.iter_mut().chain([&mut self.alpha]) {
            if let Channel::Math(m) = c {
                *m = f(m);
            }
        }
    }

    /// The color in sRGB, 0 to 1, if it's a legacy one with every channel
    /// known.
    fn legacy_srgb(&self) -> Option<[f64; 3]> {
        let n: Vec<f64> = self
            .channels
            .iter()
            .map(Channel::number)
            .collect::<Option<_>>()?;
        self.alpha.number()?;
        Some(match self.model {
            Model::Rgb => [n[0] / 255.0, n[1] / 255.0, n[2] / 255.0],
            Model::Hsl => hsl_to_rgb(n[0], n[1], n[2]),
            Model::Hwb => hwb_to_rgb(n[0], n[1], n[2]),
            _ => return None,
        })
    }

    fn write(&self, out: &mut impl Write) -> fmt::Result {
        if let Some(rgb) = self.legacy_srgb() {
            // Rounded after a millionth's rounding, so 127.4999… from the
            // arithmetic is the 127.5 it stands for.
            let [r, g, b] = rgb.map(|c| {
                ((nan_zero(c) * 255.0 * 1e6).round() / 1e6)
                    .clamp(0.0, 255.0)
                    .round()
            });
            let a = self.alpha.number().unwrap();
            if a == 1.0 {
                return write!(out, "rgb({r}, {g}, {b})");
            }
            write!(out, "rgba({r}, {g}, {b}, ")?;
            write_legacy_alpha(out, a)?;
            return out.write_char(')');
        }
        let (name, percents): (&str, [bool; 3]) = match self.model {
            Model::Rgb if self.channels.contains(&Channel::None) => {
                out.write_str("color(srgb")?;
                for c in &self.channels {
                    out.write_char(' ')?;
                    match c {
                        Channel::Number(v) => write_number(out, v / 255.0)?,
                        c => write_channel(out, c, false)?,
                    }
                }
                write_alpha(out, &self.alpha)?;
                return out.write_char(')');
            }
            Model::Rgb => ("rgb", [false; 3]),
            // With `none` the percentages are written; with math, as WPT
            // has it, they aren't.
            Model::Hsl | Model::Hwb => {
                let math = self
                    .channels
                    .iter()
                    .chain([&self.alpha])
                    .any(|c| matches!(c, Channel::Math(_)));
                let name = if self.model == Model::Hsl {
                    "hsl"
                } else {
                    "hwb"
                };
                (name, [false, !math, !math])
            }
            Model::Lab => ("lab", [false; 3]),
            Model::Lch => ("lch", [false; 3]),
            Model::Oklab => ("oklab", [false; 3]),
            Model::Oklch => ("oklch", [false; 3]),
            Model::Color(space) => {
                write!(out, "color({}", space.name())?;
                for c in &self.channels {
                    out.write_char(' ')?;
                    write_channel(out, c, false)?;
                }
                write_alpha(out, &self.alpha)?;
                return out.write_char(')');
            }
            Model::DeviceCmyk => {
                out.write_str("color(device-cmyk")?;
                for c in &self.channels {
                    out.write_char(' ')?;
                    write_channel(out, c, false)?;
                }
                write_alpha(out, &self.alpha)?;
                return out.write_char(')');
            }
        };
        write!(out, "{name}(")?;
        for (i, (c, p)) in self.channels.iter().zip(percents).enumerate() {
            if i > 0 {
                out.write_char(' ')?;
            }
            write_channel(out, c, p)?;
        }
        write_alpha(out, &self.alpha)?;
        out.write_char(')')
    }
}

thread_local! {
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// `rgb(from <color> …)` and the rest, CSS Color 5 §4.
fn relative(f: &Function<'_>, name: &str) -> Option<Color> {
    relative_at(f, name)
}

fn relative_at(f: &Function<'_>, name: &str) -> Option<Color> {
    let solid: Vec<&ComponentValue<'_>> =
        f.arguments.iter().filter(|v| !v.is_whitespace()).collect();
    let origin = Color::parse(solid.get(1)?)?;
    let mut rest = &solid[2..];
    let (function, space, keywords, slots): (&'static str, _, &[&'static str], [Slot; 3]) =
        match name {
            "rgb" | "rgba" => (
                "rgb",
                None,
                &["r", "g", "b", "alpha"],
                [Slot::Scaled(1.0); 3],
            ),
            "hsl" | "hsla" => (
                "hsl",
                None,
                &["h", "s", "l", "alpha"],
                [Slot::Hue, Slot::Scaled(1.0), Slot::Scaled(1.0)],
            ),
            "hwb" => (
                "hwb",
                None,
                &["h", "w", "b", "alpha"],
                [Slot::Hue, Slot::Scaled(1.0), Slot::Scaled(1.0)],
            ),
            "lab" => (
                "lab",
                None,
                &["l", "a", "b", "alpha"],
                [Slot::Scaled(1.0); 3],
            ),
            "oklab" => (
                "oklab",
                None,
                &["l", "a", "b", "alpha"],
                [Slot::Scaled(1.0); 3],
            ),
            "lch" => (
                "lch",
                None,
                &["l", "c", "h", "alpha"],
                [Slot::Scaled(1.0), Slot::Scaled(1.0), Slot::Hue],
            ),
            "oklch" => (
                "oklch",
                None,
                &["l", "c", "h", "alpha"],
                [Slot::Scaled(1.0), Slot::Scaled(1.0), Slot::Hue],
            ),
            "alpha" => ("alpha", None, &["alpha"], [Slot::Scaled(1.0); 3]),
            "color" => {
                let ComponentValue::Token(Token::Ident(s), _) = rest.first()? else {
                    return None;
                };
                let space = Predefined::parse(s)?;
                rest = &rest[1..];
                let keywords: &[&'static str] = match space {
                    Predefined::XyzD50 | Predefined::XyzD65 => &["x", "y", "z", "alpha"],
                    _ => &["r", "g", "b", "alpha"],
                };
                ("color", Some(space), keywords, [Slot::Scaled(1.0); 3])
            }
            _ => return None,
        };
    let (channels, alpha) = match rest
        .iter()
        .position(|v| matches!(v, ComponentValue::Token(Token::Delim('/'), _)))
    {
        Some(i) if rest.len() == i + 2 => (&rest[..i], Some(rest[i + 1])),
        Some(_) => return None,
        None => (rest, None),
    };
    // `alpha(from <color> / …)` has only the alpha.
    let count = if function == "alpha" { 0 } else { 3 };
    if channels.len() != count || (count == 0 && alpha.is_none()) {
        return None;
    }
    let read = |v: &ComponentValue<'_>, slot: Slot| -> Option<RelativeChannel> {
        Some(match v {
            ComponentValue::Token(Token::Number(n), _) => RelativeChannel::Number(n.value),
            ComponentValue::Token(Token::Percentage(n), _) if matches!(slot, Slot::Scaled(_)) => {
                RelativeChannel::Percentage(n.value)
            }
            ComponentValue::Token(Token::Dimension { number, unit }, _)
                if matches!(slot, Slot::Hue) =>
            {
                let u = super::units::Unit::parse(unit)?;
                if u.kind() != Kind::Angle {
                    return None;
                }
                RelativeChannel::Angle(number.value, u)
            }
            ComponentValue::Token(Token::Ident(w), _) if w.eq_ignore_ascii_case("none") => {
                RelativeChannel::None
            }
            ComponentValue::Token(Token::Ident(w), _) => {
                RelativeChannel::Keyword(keywords.iter().find(|k| w.eq_ignore_ascii_case(k))?)
            }
            ComponentValue::Function(f) if Math::is_math_function(&f.name) => {
                let m = Math::parse_with(f, keywords)?;
                let ok = match slot {
                    Slot::Hue => m.fits(Expect::Number) || m.fits(Expect::Dimension(Kind::Angle)),
                    Slot::Scaled(_) => m.fits(Expect::Number) || m.fits(Expect::Percentage),
                };
                if !ok {
                    return None;
                }
                RelativeChannel::Math(m)
            }
            _ => return None,
        })
    };
    let channels = channels
        .iter()
        .zip(slots)
        .map(|(v, s)| read(v, s))
        .collect::<Option<Vec<_>>>()?;
    let alpha = match alpha {
        Some(v) => Some(read(v, Slot::Scaled(1.0))?),
        None => None,
    };
    Some(Color::Relative(Box::new(Relative {
        function,
        space,
        origin,
        channels,
        alpha,
    })))
}

impl fmt::Display for RelativeChannel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RelativeChannel::Number(v) => write_number(f, *v),
            RelativeChannel::Percentage(v) => {
                write_number(f, *v)?;
                f.write_char('%')
            }
            RelativeChannel::Angle(v, u) => {
                write_number(f, *v)?;
                f.write_str(u.as_str())
            }
            RelativeChannel::Keyword(k) => f.write_str(k),
            RelativeChannel::None => f.write_str("none"),
            RelativeChannel::Math(m) => write!(f, "{m}"),
        }
    }
}

impl fmt::Display for Relative {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(from ", self.function)?;
        match &self.origin {
            // An origin with no alpha, as with no channel, is written in
            // `color()`, as WPT has it: the legacy form has no `none`.
            Color::Channels(c) if c.model == Model::Rgb && c.alpha == Channel::None => {
                f.write_str("color(srgb")?;
                for ch in &c.channels {
                    f.write_char(' ')?;
                    match ch {
                        Channel::Number(v) => write_number(f, v / 255.0)?,
                        ch => write_channel(f, ch, false)?,
                    }
                }
                f.write_str(" / none)")?;
            }
            origin => write!(f, "{origin}")?,
        }
        if let Some(space) = self.space {
            write!(f, " {}", space.name())?;
        }
        for c in &self.channels {
            write!(f, " {c}")?;
        }
        if let Some(a) = &self.alpha {
            write!(f, " / {a}")?;
        }
        f.write_char(')')
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Relative(r) => write!(f, "{r}"),
            Color::Contrast(c) => write!(f, "contrast-color({c})"),
            Color::Mix(m) => f.write_str(&m.text),
            Color::Text(s) => f.write_str(s),
            Color::CurrentColor => f.write_str("currentcolor"),
            Color::Named(name) | Color::System(name) => f.write_str(name),
            Color::Channels(c) => c.write(f),
            Color::LightDark(a, b) => write!(f, "light-dark({a}, {b})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn color(css: &str) -> Option<String> {
        let v = crate::parse_component_value(css).ok()?;
        Color::parse(&v).map(|c| c.to_string())
    }

    #[test]
    fn serializes_as_css_color_4_says() {
        let cases = [
            ("#234", "rgb(34, 51, 68)"),
            ("#f6f8", "rgba(255, 102, 255, 0.533)"),
            ("transparent", "transparent"),
            ("RebeccaPurple", "rebeccapurple"),
            ("CanvasText", "canvastext"),
            ("currentColor", "currentcolor"),
            ("rgb(-2 3 4)", "rgb(0, 3, 4)"),
            ("rgba(2, 3, 4, 50%)", "rgba(2, 3, 4, 0.5)"),
            ("rgb(128 none none)", "color(srgb 0.501961 none none)"),
            ("hsl(120 30% 50%)", "rgb(89, 166, 89)"),
            ("hsl(120 30 none / 0.5)", "hsl(120 30% none / 0.5)"),
            ("hwb(120 0% 50%)", "rgb(0, 128, 0)"),
            ("lab(400 0 10/50%)", "lab(100 0 10 / 0.5)"),
            ("lch(0 150 -10)", "lch(0 150 350)"),
            ("oklch(1 0.4 700 / 0)", "oklch(1 0.4 340 / 0)"),
            ("color(xyz 10% 10% 10%)", "color(xyz-d65 0.1 0.1 0.1)"),
            ("rgb(calc(infinity), 0, 0)", "rgb(255, 0, 0)"),
            ("lab(calc(50 * 3) 0 0)", "lab(calc(150) 0 0)"),
            ("light-dark(black, white)", "light-dark(black, white)"),
        ];
        for (css, want) in cases {
            assert_eq!(color(css).as_deref(), Some(want), "{css}");
        }
    }

    #[test]
    fn relative_colors_keep_their_channels() {
        let cases = [
            (
                "rgba(from rgb(20%, 40%, 60%, 80%) r g 25 / 25%)",
                "rgb(from rgba(51, 102, 153, 0.8) r g 25 / 25%)",
            ),
            (
                "rgb(from rebeccapurple r calc(g * .5 + g * .5) 10)",
                "rgb(from rebeccapurple r calc((0.5 * g) + (0.5 * g)) 10)",
            ),
            (
                "hsl(from red 0deg s l / alpha)",
                "hsl(from red 0deg s l / alpha)",
            ),
            (
                "color(from color(srgb 0.7 0.5 0.3) srgb r g b)",
                "color(from color(srgb 0.7 0.5 0.3) srgb r g b)",
            ),
        ];
        for (css, want) in cases {
            assert_eq!(color(css).as_deref(), Some(want), "{css}");
        }
        for css in [
            "rgb(from red r 10deg 10)",
            "rgb(from red l g b)",
            "hsl(from red 10% s l)",
            "rgb(from red calc(r + 1%) g b)",
        ] {
            assert_eq!(color(css), None, "{css}");
        }
    }

    #[test]
    fn deep_nesting_is_invalid_not_a_crash() {
        let css = format!("{}red{}", "light-dark(".repeat(5000), ", red)".repeat(5000));
        assert_eq!(color(&css), None);
    }

    #[test]
    fn rejects_what_isnt_a_color() {
        for css in [
            "#12",
            "rgb(1, 2%, 3)",
            "hsl(1, 2, 3)",
            "rgb(none, 0, 0)",
            "lab(1, 2, 3)",
            "color(foo 1 2 3)",
            "rgb(1 2)",
            "blurple",
        ] {
            assert_eq!(color(css), None, "{css}");
        }
    }
}
