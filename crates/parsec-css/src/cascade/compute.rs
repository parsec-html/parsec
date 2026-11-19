// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Computed values, CSS Cascade 5 §4.4 and each property's "Computed
//! value" line: the cascaded value, or the inherited or initial one, with
//! the CSS-wide keywords and `var()` dealt with, lengths made absolute,
//! colours worked out, and the keywords that compute to numbers
//! computed, then written as `getComputedStyle()` writes them.

use std::collections::HashMap;

use super::{Declared, Key, css_wide};
use crate::media::Environment;
use crate::parser::ComponentValue;
use crate::values::{
    Color, Component, CssWide, CustomProperties, Kind, Leaf, Resolved, Specified, Unit, expand,
    parse_value, property, substitute, write_components,
};

/// One element's computed values.
#[derive(Clone, Debug, Default)]
pub struct Computed {
    /// Every property that isn't at its initial value, written out.
    values: HashMap<&'static str, String>,
    /// Which declaration each value came from, to tell whether a
    /// shorthand's longhands all still come from it.
    keys: HashMap<&'static str, Key>,
    /// Custom properties, written out; `None` is guaranteed-invalid.
    custom: HashMap<String, Option<String>>,
    /// `font-size`, in pixels, which `em` here is relative to.
    pub font_size: f64,
    /// The root element's, for `rem`.
    root_font_size: f64,
    /// `color`, written out, which `currentcolor` here is.
    color: String,
    env: Environment,
}

impl Computed {
    /// A property's computed value, as `getComputedStyle()` writes it.
    pub fn get(&self, name: &str) -> Option<String> {
        if name.starts_with("--") {
            return self.custom.get(name).cloned().flatten();
        }
        let p = property(name)?;
        let p = p.alias_of.and_then(property).unwrap_or(p);
        if p.is_shorthand() {
            if let Some(v) = self.values.get(p.name)
                && let Some(k) = self.keys.get(p.name)
                && p.longhands
                    .iter()
                    .all(|l| self.keys.get(l).is_none_or(|lk| lk <= k))
            {
                return Some(v.clone());
            }
            // The longhands written in order and read back as the
            // shorthand, which writes itself at its shortest: CSSOM's
            // shorthand serialization, for the shorthands whose longhands
            // line up with their grammar.
            let parts: Vec<String> = p.longhands.iter().filter_map(|l| self.get(l)).collect();
            if let Some((first, rest)) = parts.split_first()
                && rest.iter().all(|r| r == first)
                && p.longhands.len() > 2
            {
                return Some(first.clone());
            }
            let text = parts.join(" ");
            let values = crate::parse_component_values(&text);
            return parse_value(p.name, &values).ok().map(|v| v.to_string());
        }
        if let Some(v) = self.values.get(p.name) {
            return Some(v.clone());
        }
        Some(self.initial(p.name))
    }

    /// The custom properties, as `var()` here sees them.
    pub fn custom_properties(&self) -> impl Iterator<Item = (&str, Option<&str>)> {
        self.custom.iter().map(|(k, v)| (k.as_str(), v.as_deref()))
    }

    fn cx(&self, name: &'static str, parent_font_size: f64) -> Cx<'_> {
        Cx {
            property: name,
            font_size: self.font_size,
            parent_font_size,
            root_font_size: self.root_font_size,
            color: &self.color,
            env: &self.env,
        }
    }

    fn initial(&self, name: &'static str) -> String {
        initial_of(name, &self.cx(name, self.font_size))
    }
}

pub(super) struct Input<'d, 's> {
    pub declared: &'d HashMap<&'static str, Vec<Declared<'s>>>,
    pub custom: &'d HashMap<String, Vec<(Key, Vec<ComponentValue<'s>>)>>,
    pub parent: Option<&'d Computed>,
    pub root: Option<&'d Computed>,
    pub is_root: bool,
    pub env: &'d Environment,
    /// The custom properties `@property` registers, with their syntax.
    pub registered: &'d HashMap<String, (crate::rules::PropertyRule, crate::rules::Syntax)>,
}

/// What computing one property needs to know.
struct Cx<'c> {
    property: &'static str,
    font_size: f64,
    parent_font_size: f64,
    root_font_size: f64,
    color: &'c str,
    env: &'c Environment,
}

/// The order properties are computed in: what others refer to first.
const FIRST: &[&str] = &["font-size", "color"];

fn inherited(name: &str) -> bool {
    property(name).and_then(|p| p.inherited) == Some(true)
}

pub(super) fn compute(input: Input<'_, '_>) -> Computed {
    let parent = input.parent;
    let parent_font_size = parent.map_or(16.0, |p| p.font_size);
    let root_font_size = input.root.map_or(16.0, |r| r.font_size);
    let mut out = Computed {
        values: HashMap::new(),
        keys: HashMap::new(),
        custom: HashMap::new(),
        font_size: parent_font_size,
        root_font_size,
        color: parent.map_or_else(|| "rgb(0, 0, 0)".to_string(), |p| p.color.clone()),
        env: input.env.clone(),
    };
    // Inherited values first: what isn't declared keeps them.
    if let Some(p) = parent {
        for (k, v) in &p.values {
            if inherited(k) {
                out.values.insert(k, v.clone());
            }
        }
        out.custom = p.custom.clone();
    }
    let custom = resolve_custom(&input, parent);
    for (name, value) in custom.iter() {
        out.custom
            .insert(name.to_string(), value.map(crate::to_css));
    }
    let mut order: Vec<&'static str> = input.declared.keys().copied().collect();
    order.sort_by_key(|n| FIRST.iter().position(|f| f == n).unwrap_or(FIRST.len()));
    for name in order {
        let list = &input.declared[name];
        let value = cascaded(
            name,
            list,
            list.len() - 1,
            &custom,
            parent,
            &out,
            parent_font_size,
            input.is_root,
        );
        out.keys.insert(name, list[list.len() - 1].key);
        match value {
            Some(v) => {
                out.values.insert(name, v);
            }
            None => {
                out.values.remove(name);
            }
        }
        if name == "font-size" {
            if let Some(v) = out.values.get("font-size") {
                out.font_size = px_of(v).unwrap_or(parent_font_size);
            }
            if input.is_root {
                out.root_font_size = out.font_size;
            }
        }
        if name == "color" {
            out.color = out
                .values
                .get("color")
                .cloned()
                .unwrap_or_else(|| out.initial("color"));
        }
    }
    if input.is_root {
        out.root_font_size = out.font_size;
    }
    registered(&input, &custom, parent, &mut out);
    // `currentcolor` and the border widths depend on others, worked out
    // after everything's in.
    fix_up(&mut out, input.is_root, parent);
    out
}

/// Registered custom properties, CSS Properties and Values API 1 §2: each
/// takes its declared value if it matches the syntax, computed (`1em` as
/// pixels, colours worked out); otherwise, or if nothing's declared, the
/// parent's if it inherits, and its initial value if not.
fn registered(
    input: &Input<'_, '_>,
    custom: &Resolved<'_>,
    parent: Option<&Computed>,
    out: &mut Computed,
) {
    for (name, (rule, syntax)) in input.registered {
        let initial = rule.initial_value.clone();
        let from_parent = || {
            parent
                .and_then(|p| p.custom.get(name).cloned().flatten())
                .or_else(|| initial.clone())
        };
        let unset = || {
            if rule.inherits {
                from_parent()
            } else {
                initial.clone()
            }
        };
        let declared = input
            .custom
            .get(name)
            .and_then(|l| l.last())
            .map(|(_, v)| v);
        let keyword = match declared.map(|v| &v[..]) {
            Some([ComponentValue::Token(crate::Token::Ident(w), _)]) => {
                Some(w.to_ascii_lowercase())
            }
            _ => None,
        };
        let value = match (declared, keyword.as_deref()) {
            (None, _) => unset(),
            (Some(_), Some("inherit")) => from_parent(),
            (Some(_), Some("initial")) => initial.clone(),
            (Some(_), Some("unset" | "revert" | "revert-layer")) => unset(),
            (Some(_), _) => match custom.get(name) {
                // Guaranteed-invalid: as `unset`.
                None => unset(),
                Some(tokens) if syntax.is_universal() => {
                    Some(crate::to_css(tokens).trim().to_string())
                }
                Some(tokens) => match syntax.parse_value(tokens) {
                    Some(c) => {
                        let cx = out.cx("--", out.font_size);
                        let c: Vec<Component<'_>> = components(&c, &cx, None)
                            .into_iter()
                            .map(|x| match x {
                                Component::Color(c) => Component::Color(c.computed()),
                                x => x,
                            })
                            .collect();
                        Some(Specified::Value(c).to_string())
                    }
                    None => unset(),
                },
            },
        };
        out.custom.insert(name.clone(), value);
    }
}

fn px_of(s: &str) -> Option<f64> {
    s.strip_suffix("px")?.parse().ok()
}

/// The custom properties: the cascade's winner for each, then resolved
/// against each other and the parent's.
fn resolve_custom<'r>(input: &Input<'_, 'r>, parent: Option<&'r Computed>) -> Resolved<'r> {
    let mut props = CustomProperties::new();
    for (name, list) in input.custom {
        let Some((_, value)) = list.last() else {
            continue;
        };
        // `inherit` and `unset` keep the parent's: not declaring it does.
        if let [ComponentValue::Token(crate::Token::Ident(w), _)] = &value[..]
            && ["inherit", "unset", "revert", "revert-layer"]
                .iter()
                .any(|k| w.eq_ignore_ascii_case(k))
        {
            continue;
        }
        props.declare(name, value.clone());
    }
    // The parent's, read back from their text.
    let parent_resolved = Resolved::from_values(parent.into_iter().flat_map(|p| {
        p.custom
            .iter()
            .map(|(k, v)| (k.clone(), v.as_deref().map(crate::parse_component_values)))
    }));
    props.resolve(Some(&parent_resolved))
}

/// The base origin of a cascade rank: user agent, user or author.
fn base_origin(rank: u8) -> u8 {
    if rank <= 2 { rank } else { 5 - rank }
}

/// The computed value from the cascade's list for one property, the
/// winner at `at`; `None` for the initial value.
#[allow(clippy::too_many_arguments)]
fn cascaded(
    name: &'static str,
    list: &[Declared<'_>],
    at: usize,
    custom: &Resolved<'_>,
    parent: Option<&Computed>,
    out: &Computed,
    parent_font_size: f64,
    is_root: bool,
) -> Option<String> {
    let d = &list[at];
    let inherit = || parent.and_then(|p| p.get(name)).filter(|_| true);
    let unset = || if inherited(name) { inherit() } else { None };
    if let Some(k) = css_wide(&d.value) {
        return match k {
            CssWide::Inherit => inherit().or_else(|| Some(out.initial(name))),
            CssWide::Initial => None,
            CssWide::Unset => unset(),
            // Roll back to the origin before, or the layer before.
            CssWide::Revert | CssWide::RevertLayer => {
                let origin = base_origin(d.key.origin);
                let back = list[..at].iter().rposition(|e| {
                    if k == CssWide::Revert {
                        base_origin(e.key.origin) < origin
                    } else {
                        base_origin(e.key.origin) < origin
                            || (e.key.origin == d.key.origin
                                && e.key.layer < d.key.layer
                                && !e.key.attached)
                    }
                });
                match back {
                    Some(i) => cascaded(
                        name,
                        list,
                        i,
                        custom,
                        parent,
                        out,
                        parent_font_size,
                        is_root,
                    ),
                    None => unset(),
                }
            }
        };
    }
    let substituted;
    let value = match &d.value {
        Specified::Unparsed(v) => {
            // Invalid once substituted is `unset`.
            let Ok(values) = substitute(v, custom) else {
                return unset();
            };
            let parsed = match d.from_shorthand {
                Some(shorthand) => expand(shorthand, &values)
                    .ok()
                    .and_then(|l| l.into_iter().find(|(n, _)| *n == name).map(|(_, v)| v)),
                None => parse_value(name, &values).ok(),
            };
            match parsed {
                Some(v) => {
                    substituted = v;
                    if let Some(k) = css_wide(&substituted) {
                        return match k {
                            CssWide::Inherit => inherit(),
                            CssWide::Initial => None,
                            _ => unset(),
                        };
                    }
                    &substituted
                }
                None => return unset(),
            }
        }
        v => v,
    };
    let Specified::Value(components) = value else {
        return None;
    };
    let cx = out.cx(name, parent_font_size);
    Some(compute_value(name, components, &cx, is_root, parent))
}

fn initial_of(name: &'static str, cx: &Cx<'_>) -> String {
    let Some(text) = property(name).and_then(|p| p.initial) else {
        return String::new();
    };
    let values = crate::parse_component_values(text);
    match parse_value(name, &values) {
        Ok(Specified::Value(c)) => compute_value(name, &c, cx, false, None),
        _ => text.to_string(),
    }
}

// ---- computing values ------------------------------------------------------

/// Pixels in one of `u`, where the context says what the relative ones
/// are; `None` for a unit that isn't a length.
fn px(u: Unit, cx: &Cx<'_>) -> Option<f64> {
    // In `font-size`, `em` is the parent's.
    let em = if cx.property == "font-size" {
        cx.parent_font_size
    } else {
        cx.font_size
    };
    let (w, h) = (cx.env.width, cx.env.height);
    Some(match u {
        Unit::Em => em,
        Unit::Rem => cx.root_font_size,
        // No fonts here: the fallbacks the specs give.
        Unit::Ex | Unit::Ch | Unit::Cap => em * 0.5,
        Unit::Rex | Unit::Rch | Unit::Rcap => cx.root_font_size * 0.5,
        Unit::Ic => em,
        Unit::Ric => cx.root_font_size,
        Unit::Lh => em * 1.2,
        Unit::Rlh => cx.root_font_size * 1.2,
        // Without containers, container units are the small viewport's.
        Unit::Vw
        | Unit::Svw
        | Unit::Lvw
        | Unit::Dvw
        | Unit::Vi
        | Unit::Svi
        | Unit::Lvi
        | Unit::Dvi
        | Unit::Cqw
        | Unit::Cqi => w / 100.0,
        Unit::Vh
        | Unit::Svh
        | Unit::Lvh
        | Unit::Dvh
        | Unit::Vb
        | Unit::Svb
        | Unit::Lvb
        | Unit::Dvb
        | Unit::Cqh
        | Unit::Cqb => h / 100.0,
        Unit::Vmin | Unit::Svmin | Unit::Lvmin | Unit::Dvmin | Unit::Cqmin => w.min(h) / 100.0,
        Unit::Vmax | Unit::Svmax | Unit::Lvmax | Unit::Dvmax | Unit::Cqmax => w.max(h) / 100.0,
        u if u.kind() == Kind::Length => u.to_canonical()?,
        _ => return None,
    })
}

/// A leaf computed: lengths in pixels, other dimensions canonical.
fn leaf(l: Leaf, cx: &Cx<'_>, percent_of: Option<f64>) -> Leaf {
    match l {
        Leaf::Dimension(v, u) => match px(u, cx) {
            Some(f) => Leaf::Dimension(v * f, Unit::Px),
            // Angles in degrees, times in seconds, and the rest in their
            // canonical unit, as computed values have them.
            None => match (u.canonical(), u.to_canonical()) {
                (Some(c), Some(f)) => Leaf::Dimension(v * f, c),
                _ => l,
            },
        },
        Leaf::Percentage(p) => match percent_of {
            Some(base) => Leaf::Dimension(p / 100.0 * base, Unit::Px),
            None => l,
        },
        l => l,
    }
}

/// A value's components computed.
fn components<'a>(c: &[Component<'a>], cx: &Cx<'_>, percent_of: Option<f64>) -> Vec<Component<'a>> {
    c.iter()
        .map(|x| match x {
            Component::Dimension(v, u) => match leaf(Leaf::Dimension(*v, *u), cx, percent_of) {
                Leaf::Dimension(v, u) => Component::Dimension(v, u),
                _ => x.clone(),
            },
            Component::Percentage(p) => match percent_of {
                Some(base) => Component::Dimension(p / 100.0 * base, Unit::Px),
                None => x.clone(),
            },
            Component::Math(m) => {
                let m = m.map_leaves(&|l| leaf(l, cx, percent_of));
                match m.leaf() {
                    Some(Leaf::Dimension(v, u)) => Component::Dimension(v, u),
                    Some(Leaf::Percentage(v)) => Component::Percentage(v),
                    Some(Leaf::Number(v)) => Component::Number {
                        value: v,
                        integer: false,
                    },
                    None => Component::Math(m),
                }
            }
            Component::Color(c) => Component::Color(resolved_color(c, cx)),
            Component::Function { name, arguments } => Component::Function {
                name,
                arguments: components(arguments, cx, percent_of),
            },
            x => x.clone(),
        })
        .collect()
}

/// The used colour, which is what `getComputedStyle()` gives: named and
/// system colours as `rgb()`, `currentcolor` as the element's colour.
fn resolved_color(c: &Color, cx: &Cx<'_>) -> Color {
    match c {
        Color::CurrentColor => {
            // In `color` itself this is the parent's, which is what the
            // context holds until `color` is computed.
            let values = crate::parse_component_values(cx.color);
            values
                .first()
                .and_then(Color::parse)
                .map_or(Color::CurrentColor, |c| c.computed())
        }
        Color::System(name) => system_color(name).unwrap_or_else(|| c.clone()),
        Color::LightDark(a, b) => {
            let dark = cx
                .env
                .keywords
                .iter()
                .any(|(f, v)| *f == "prefers-color-scheme" && *v == "dark");
            resolved_color(if dark { b } else { a }, cx)
        }
        Color::Relative(_) | Color::Mix(_) => {
            // Worked out where it can be: with `currentcolor` inside, its
            // colour stands in first.
            let c = resolve_current(c, cx);
            match c.absolute() {
                Some(a) => Color::Text(a.write()),
                None => c,
            }
        }
        // A legacy colour's math, worked out with the element's lengths,
        // makes it a legacy colour again.
        Color::Channels(ch) if ch.model_is_legacy() && ch.has_math() => {
            let mut ch = ch.clone();
            ch.map_math(&|m| m.map_leaves(&|l| leaf(l, cx, None)));
            let text = Color::Channels(ch).to_string();
            let values = crate::parse_component_values(&text);
            values
                .first()
                .and_then(Color::parse)
                .unwrap_or_else(|| c.clone())
        }
        // Math in a channel, worked out with the element's lengths.
        Color::Channels(ch) if !ch.model_is_legacy() && ch.has_math() => {
            let mut ch = ch.clone();
            ch.map_math(&|m| m.map_leaves(&|l| leaf(l, cx, None)));
            let c = Color::Channels(ch);
            match c.absolute() {
                Some(a) => Color::Text(a.write()),
                None => c,
            }
        }
        c => c.computed(),
    }
}

/// `currentcolor` inside a relative colour's origin replaced by the
/// element's colour.
fn resolve_current(c: &Color, cx: &Cx<'_>) -> Color {
    match c {
        Color::Relative(r) if r.origin == Color::CurrentColor => {
            let mut r = (**r).clone();
            r.origin = resolved_color(&Color::CurrentColor, cx);
            Color::Relative(Box::new(r))
        }
        c => c.clone(),
    }
}

/// The light-scheme system colours, as browsers render them.
fn system_color(name: &str) -> Option<Color> {
    let [r, g, b] = match name {
        "canvas" | "field" | "highlighttext" | "selecteditemtext" | "accentcolortext"
        | "window" => [255, 255, 255],
        "canvastext" | "fieldtext" | "buttontext" | "marktext" | "windowtext" | "menutext"
        | "captiontext" | "infotext" => [0, 0, 0],
        "linktext" => [0, 0, 238],
        "visitedtext" => [85, 26, 139],
        "activetext" => [255, 0, 0],
        "buttonface" | "buttonhighlight" | "threedface" | "threedlightshadow" => [240, 240, 240],
        "buttonborder" | "activeborder" | "inactiveborder" | "threedshadow" => [118, 118, 118],
        "graytext" => [109, 109, 109],
        "mark" => [255, 255, 0],
        "highlight" | "selecteditem" | "accentcolor" => [0, 117, 255],
        _ => return None,
    };
    let css = format!("rgb({r}, {g}, {b})");
    Color::parse(crate::parse_component_values(&css).first()?)
}

const FONT_SIZES: &[(&str, f64)] = &[
    ("xx-small", 9.0),
    ("x-small", 10.0),
    ("small", 13.0),
    ("medium", 16.0),
    ("large", 18.0),
    ("x-large", 24.0),
    ("xx-large", 32.0),
    ("xxx-large", 48.0),
];

fn number(v: f64) -> Component<'static> {
    Component::Number {
        value: v,
        integer: v.fract() == 0.0,
    }
}

fn compute_value(
    name: &'static str,
    c: &[Component<'_>],
    cx: &Cx<'_>,
    is_root: bool,
    parent: Option<&Computed>,
) -> String {
    let mut s = String::new();
    let computed: Vec<Component<'_>> = match (name, c) {
        ("font-size", [Component::Keyword(k)]) => {
            let base = cx.parent_font_size;
            let v = match *k {
                "larger" => base * 1.2,
                "smaller" => base / 1.2,
                k => FONT_SIZES
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map_or(base, |(_, v)| *v),
            };
            vec![Component::Dimension(v, Unit::Px)]
        }
        ("font-size", _) => components(c, cx, Some(cx.parent_font_size)),
        ("line-height", _) => components(c, cx, Some(cx.font_size)),
        ("font-weight", [Component::Keyword(k)]) => {
            let parent = parent
                .and_then(|p| p.get("font-weight"))
                .and_then(|w| w.parse::<f64>().ok())
                .unwrap_or(400.0);
            let v = match *k {
                "normal" => 400.0,
                "bold" => 700.0,
                "bolder" if parent < 350.0 => 400.0,
                "bolder" if parent < 550.0 => 700.0,
                "bolder" => parent.max(900.0),
                "lighter" if parent < 100.0 => parent,
                "lighter" if parent < 550.0 => 100.0,
                "lighter" if parent < 750.0 => 400.0,
                "lighter" => 700.0,
                _ => parent,
            };
            vec![number(v)]
        }
        ("display", _) => {
            let mut c = components(c, cx, None);
            let blockify = is_root
                || parent.and_then(|p| p.get("display")).is_some_and(|d| {
                    matches!(d.as_str(), "flex" | "inline-flex" | "grid" | "inline-grid")
                });
            if blockify {
                c = blockified(c);
            }
            c
        }
        ("opacity" | "fill-opacity" | "stroke-opacity" | "flood-opacity" | "stop-opacity", [_]) => {
            match &components(c, cx, None)[..] {
                [Component::Number { value, .. }] => vec![number(value.clamp(0.0, 1.0))],
                [Component::Percentage(p)] => vec![number((p / 100.0).clamp(0.0, 1.0))],
                other => other.to_vec(),
            }
        }
        ("object-position" | "background-position" | "mask-position" | "offset-position", _) => {
            let c = components(c, cx, None);
            let mut out = Vec::new();
            for (i, part) in c.split(|x| *x == Component::Literal(',')).enumerate() {
                if i > 0 {
                    out.push(Component::Literal(','));
                }
                match position(part) {
                    Some(p) => out.extend(p),
                    None => out.extend(part.iter().cloned()),
                }
            }
            out
        }
        ("background-image" | "mask-image" | "list-style-image" | "border-image-source", _) => {
            components(c, cx, None).into_iter().map(gradient).collect()
        }
        ("filter" | "backdrop-filter", _) => components(c, cx, None)
            .into_iter()
            .map(filter_function)
            .collect(),
        _ if width_keyword(name) => {
            let px = |k: &str| match k {
                "thin" => Some(1.0),
                "medium" => Some(3.0),
                "thick" => Some(5.0),
                _ => None,
            };
            match c {
                [Component::Keyword(k)] if px(k).is_some() => {
                    vec![Component::Dimension(px(k).unwrap(), Unit::Px)]
                }
                _ => components(c, cx, None),
            }
        }
        _ => components(c, cx, None),
    };
    write_components(&mut s, &computed).unwrap();
    s
}

/// A `<position>` as computed values have it, CSS Values 4 §8.3: two
/// length-percentages, horizontal first, keywords as percentages and an
/// offset from the right or bottom as `calc(100% - offset)`.
fn position<'a>(c: &[Component<'a>]) -> Option<Vec<Component<'a>>> {
    let kw = |x: &Component<'_>| match x {
        Component::Keyword(k) => Some(*k),
        _ => None,
    };
    let is_len = |x: &Component<'_>| {
        matches!(
            x,
            Component::Dimension(..) | Component::Percentage(_) | Component::Math(_)
        )
    };
    let axis = |k: &str| match k {
        "left" | "right" => Some('x'),
        "top" | "bottom" => Some('y'),
        "center" => Some('c'),
        _ => None,
    };
    let pct = |k: &str| match k {
        "left" | "top" => 0.0,
        "right" | "bottom" => 100.0,
        _ => 50.0,
    };
    // One axis: a keyword, perhaps with an offset from its edge.
    let side = |k: &str, off: Option<&Component<'a>>| -> Component<'a> {
        match (k, off) {
            (_, None) => Component::Percentage(pct(k)),
            ("left" | "top", Some(o)) => o.clone(),
            (_, Some(Component::Percentage(p))) => Component::Percentage(100.0 - p),
            (_, Some(o)) => {
                let text = format!("calc(100% - {o})");
                let values = crate::parse_component_values(&text);
                match values.first() {
                    Some(crate::ComponentValue::Function(f)) => {
                        crate::values::Math::parse(f).map_or(o.clone(), Component::Math)
                    }
                    _ => o.clone(),
                }
            }
        }
    };
    let (x, y) = match c {
        [a] if is_len(a) => (a.clone(), Component::Percentage(50.0)),
        [a] => {
            let k = kw(a)?;
            match axis(k)? {
                'y' => (Component::Percentage(50.0), side(k, None)),
                _ => (side(k, None), Component::Percentage(50.0)),
            }
        }
        [a, b] => {
            let (ka, kb) = (kw(a), kw(b));
            match (ka.and_then(axis), kb.and_then(axis)) {
                (Some('y'), _) | (_, Some('x')) => {
                    // Written vertical first: swap.
                    let x = kb.map_or_else(|| b.clone(), |k| side(k, None));
                    let y = ka.map_or_else(|| a.clone(), |k| side(k, None));
                    (x, y)
                }
                _ => (
                    ka.map_or_else(|| a.clone(), |k| side(k, None)),
                    kb.map_or_else(|| b.clone(), |k| side(k, None)),
                ),
            }
        }
        [a, b, c2] => {
            // `left 10px center`, `center top 8px` and the like.
            let (ka, kb, kc) = (kw(a), kw(b), kw(c2));
            let (first, second) = if is_len(b) {
                ((ka?, Some(b)), (kc?, None))
            } else {
                ((ka?, None), (kb?, Some(c2)))
            };
            let (p1, p2) = (side(first.0, first.1), side(second.0, second.1));
            let vertical_first = axis(first.0) == Some('y') || axis(second.0) == Some('x');
            if vertical_first { (p2, p1) } else { (p1, p2) }
        }
        [a, b, c2, d] => {
            let (ka, kc) = (kw(a)?, kw(c2)?);
            let (p1, p2) = (side(ka, Some(b)), side(kc, Some(d)));
            if axis(ka) == Some('y') {
                (p2, p1)
            } else {
                (p1, p2)
            }
        }
        _ => return None,
    };
    Some(vec![x, y])
}

/// A gradient's computed value: its `at` position computed, and left out
/// when it's the centre; `from 0deg` left out.
fn gradient(c: Component<'_>) -> Component<'_> {
    let Component::Function {
        name,
        mut arguments,
    } = c
    else {
        return c;
    };
    if !name.ends_with("gradient") {
        return Component::Function { name, arguments };
    }
    let end = arguments
        .iter()
        .position(|x| *x == Component::Literal(','))
        .unwrap_or(arguments.len());
    if let Some(at) = arguments[..end]
        .iter()
        .position(|x| *x == Component::Keyword("at"))
    {
        let stop = arguments[at + 1..end]
            .iter()
            .position(|x| *x == Component::Keyword("in"))
            .map_or(end, |i| at + 1 + i);
        if let Some(p) = position(&arguments[at + 1..stop]) {
            let center = p == [Component::Percentage(50.0), Component::Percentage(50.0)];
            if center {
                arguments.drain(at..stop);
            } else {
                arguments.splice(at + 1..stop, p);
            }
        }
    }
    if let (Some(Component::Keyword("from")), Some(Component::Dimension(v, _))) =
        (arguments.first(), arguments.get(1))
        && *v == 0.0
    {
        arguments.drain(0..2);
    }
    // The default ending shape and size.
    if let Some(i) = arguments
        .iter()
        .position(|x| *x == Component::Keyword("farthest-corner"))
    {
        arguments.remove(i);
    }
    while arguments.first() == Some(&Component::Literal(',')) {
        arguments.remove(0);
    }
    Component::Function { name, arguments }
}

/// A filter function's computed value, Filter Effects 1 §6: percentages
/// as numbers, the defaults written, and the amounts that can't pass 1
/// held to it.
fn filter_function(c: Component<'_>) -> Component<'_> {
    let Component::Function {
        name,
        mut arguments,
    } = c
    else {
        return c;
    };
    let amount = |args: &[Component<'_>], max: f64| -> Option<Component<'static>> {
        let v = match args {
            [] => 1.0,
            [Component::Number { value, .. }] => *value,
            [Component::Percentage(p)] => p / 100.0,
            _ => return None,
        };
        Some(number(v.clamp(0.0, max)))
    };
    match name {
        "grayscale" | "invert" | "opacity" | "sepia" => {
            if let Some(a) = amount(&arguments, 1.0) {
                arguments = vec![a];
            }
        }
        "brightness" | "contrast" | "saturate" => {
            if let Some(a) = amount(&arguments, f64::INFINITY) {
                arguments = vec![a];
            }
        }
        "blur" => {
            if let [Component::Dimension(v, u)] = &arguments[..] {
                arguments = vec![Component::Dimension(v.max(0.0), *u)];
            } else if arguments.is_empty() {
                arguments = vec![Component::Dimension(0.0, Unit::Px)];
            }
        }
        "hue-rotate" => {
            if matches!(&arguments[..], [] | [Component::Number { .. }]) {
                arguments = vec![Component::Dimension(0.0, Unit::Deg)];
            }
        }
        _ => {}
    }
    Component::Function { name, arguments }
}

/// `display` made a block-level box, CSS Display §2.7, for the root and
/// a flex or grid container's children.
fn blockified(c: Vec<Component<'_>>) -> Vec<Component<'_>> {
    let k = |k| vec![Component::Keyword(k)];
    match &c[..] {
        [Component::Keyword("inline")]
        | [Component::Keyword("inline-block")]
        | [Component::Keyword("run-in")] => k("block"),
        [Component::Keyword("inline-flex")] => k("flex"),
        [Component::Keyword("inline-grid")] => k("grid"),
        [Component::Keyword("inline-table")] => k("table"),
        [Component::Keyword("contents")] => c,
        [Component::Keyword(t)] if t.starts_with("table-") || t.starts_with("ruby") => k("block"),
        _ => c,
    }
}

fn width_keyword(name: &str) -> bool {
    matches!(
        name,
        "border-top-width"
            | "border-right-width"
            | "border-bottom-width"
            | "border-left-width"
            | "outline-width"
            | "column-rule-width"
    )
}

/// A border's width is 0 when its style draws nothing, CSS Backgrounds
/// §3.3; an outline's too. Done last, once styles are known.
fn fix_up(out: &mut Computed, _is_root: bool, _parent: Option<&Computed>) {
    for side in ["top", "right", "bottom", "left"] {
        let style = out.get(&format!("border-{side}-style")).unwrap_or_default();
        if style == "none" || style == "hidden" {
            let w: &'static str = match side {
                "top" => "border-top-width",
                "right" => "border-right-width",
                "bottom" => "border-bottom-width",
                _ => "border-left-width",
            };
            out.values.insert(w, "0px".into());
        }
    }
    if out.get("outline-style").as_deref() == Some("none") {
        out.values.insert("outline-width", "0px".into());
    }
    if matches!(
        out.get("column-rule-style").as_deref(),
        Some("none" | "hidden")
    ) {
        out.values.insert("column-rule-width", "0px".into());
    }
}
