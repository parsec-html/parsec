// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The shortest forms CSSOM writes specified values in, property by
//! property, type by type and function by function: what the grammar
//! alone can't say, as `circle(closest-side)` being `circle()`, `scale:
//! 100% 100%` being `1`, or `left` as a position being `left center`.
//!
//! Each works on values already matched and typed, so on values the
//! grammar says are valid; what isn't recognised is left as it is.

use super::color::{Color, Model};
use super::units::Unit;
use super::value::Component;

type C<'a> = Vec<Component<'a>>;

fn kw(c: &Component<'_>, k: &str) -> bool {
    matches!(c, Component::Keyword(w) if *w == k)
}

fn number(value: f64) -> Component<'static> {
    Component::Number {
        value,
        integer: value.fract() == 0.0,
    }
}

fn is_zero(c: &Component<'_>) -> bool {
    match c {
        Component::Dimension(v, _) | Component::Percentage(v) => *v == 0.0,
        Component::Number { value, .. } => *value == 0.0,
        _ => false,
    }
}

fn text(c: &[Component<'_>]) -> String {
    super::value::Specified::Value(c.to_vec()).to_string()
}

/// Applies `f` to each comma-separated part of `c`.
fn per_layer(c: &mut C<'_>, mut f: impl FnMut(&mut C<'_>)) {
    if !c.contains(&Component::Literal(',')) {
        f(c);
        return;
    }
    let mut out = Vec::with_capacity(c.len());
    let mut layer = Vec::new();
    for x in c.drain(..) {
        if x == Component::Literal(',') {
            f(&mut layer);
            out.append(&mut layer);
            out.push(x);
        } else {
            layer.push(x);
        }
    }
    f(&mut layer);
    out.append(&mut layer);
    *c = out;
}

/// The two halves of `a / b`, or all of it and nothing.
fn halves<'a>(c: &[Component<'a>]) -> (C<'a>, Option<C<'a>>) {
    match c.iter().position(|x| *x == Component::Literal('/')) {
        Some(i) => (c[..i].to_vec(), Some(c[i + 1..].to_vec())),
        None => (c.to_vec(), None),
    }
}

/// `1px 2px 1px 2px` as `1px 2px`, and so on down: four sides, or two.
pub(crate) fn sides(c: &mut C<'_>) {
    if c.len() == 4 && c[3] == c[1] {
        c.pop();
    }
    if c.len() == 3 && c[2] == c[0] {
        c.pop();
    }
    if c.len() == 2 && c[1] == c[0] {
        c.pop();
    }
}

/// Properties whose groups CSSOM writes in the order they were given.
pub(crate) const KEEP_ORDER: &[&str] = &["hanging-punctuation", "word-space-transform"];

/// The properties [`property`] has a shortest form for, sorted.
pub(crate) const PROPERTIES: &[&str] = &[
    "animation-name",
    "animation-range",
    "animation-range-end",
    "animation-range-start",
    "border-image-slice",
    "border-radius",
    "clip-path",
    "column-rule",
    "column-rule-inset",
    "counter-increment",
    "counter-reset",
    "counter-set",
    "display",
    "fill-opacity",
    "flex",
    "flood-opacity",
    "font-family",
    "font-feature-settings",
    "font-language-override",
    "font-size-adjust",
    "font-style",
    "grid",
    "grid-area",
    "grid-auto-flow",
    "grid-column",
    "grid-column-end",
    "grid-column-start",
    "grid-row",
    "grid-row-end",
    "grid-row-start",
    "grid-template",
    "grid-template-areas",
    "grid-template-columns",
    "grid-template-rows",
    "hyphenate-limit-chars",
    "line-clamp",
    "list-style",
    "margin-trim",
    "mask",
    "mask-border-slice",
    "object-fit",
    "offset",
    "offset-path",
    "offset-rotate",
    "opacity",
    "overflow-clip-margin",
    "place-content",
    "position-try",
    "rotate",
    "row-rule",
    "row-rule-inset",
    "rule",
    "rule-inset",
    "scale",
    "scroll-snap-type",
    "shape-image-threshold",
    "stop-opacity",
    "stroke-opacity",
    "text-decoration-inset",
    "text-emphasis-position",
    "text-spacing",
    "transform-origin",
    "transition",
    "translate",
    "white-space",
];

/// Whether [`property`] has anything to do for `name`.
pub(crate) fn has_form(name: &str) -> bool {
    PROPERTIES.binary_search(&name).is_ok()
}

/// A longhand's or shorthand's value in its shortest form; `name` is
/// the property's own, an alias's target for an alias.
pub(crate) fn property(name: &str, c: &mut C<'_>) {
    if has_form(name) {
        form(name, c);
    }
}

/// [`property`] where [`has_form`] is known to be true.
pub(crate) fn form(name: &str, c: &mut C<'_>) {
    match name {
        "display" => display(c),
        "white-space" => white_space(c),
        "flex" => flex(c),
        "opacity"
        | "shape-image-threshold"
        | "fill-opacity"
        | "stroke-opacity"
        | "flood-opacity"
        | "stop-opacity" => c.iter_mut().for_each(to_number),
        "scale" => scale(c),
        "translate" => translate(c),
        "rotate" => rotate(c),
        "border-radius" => border_radius(c),
        "border-image-slice" | "mask-border-slice" => {
            let fill = c.iter().position(|x| kw(x, "fill"));
            if let Some(i) = fill {
                c.remove(i);
            }
            sides(c);
            if fill.is_some() {
                c.push(Component::Keyword("fill"));
            }
        }
        "margin-trim" => {
            if c.len() == 2 && c.iter().all(|x| kw(x, "block-start") || kw(x, "block-end")) {
                *c = vec![Component::Keyword("block")];
            }
        }
        "font-family" => family_names(c),
        "font-feature-settings" => per_layer(c, feature),
        "font-language-override" => {
            if let [Component::String(s)] = &mut c[..] {
                let trimmed = s.trim_end_matches(' ').len();
                if trimmed < s.len() {
                    s.to_mut().truncate(trimmed);
                }
            }
        }
        "font-size-adjust" => {
            if c.len() == 2 && kw(&c[0], "ex-height") {
                c.remove(0);
            }
        }
        "font-style" => {
            if let [Component::Keyword("oblique"), Component::Dimension(v, _)] = &c[..]
                && *v == 0.0
            {
                *c = vec![Component::Keyword("normal")];
            }
        }
        "animation-name" => c.iter_mut().for_each(keyframes_name),
        "animation-range-start" => per_layer(c, |l| range_part(l, 0.0)),
        "animation-range-end" => per_layer(c, |l| range_part(l, 100.0)),
        "animation-range" => per_layer(c, animation_range),
        "position-try" | "offset" => {
            if c.len() > 1 && kw(&c[0], "normal") {
                c.remove(0);
            }
        }
        "offset-rotate" => {
            if c.len() == 2 && is_zero(&c[1]) {
                c.pop();
            }
        }
        "text-spacing" => {
            if c.len() == 2 && kw(&c[0], "space-all") && kw(&c[1], "no-autospace") {
                *c = vec![Component::Keyword("none")];
            }
        }
        "text-emphasis-position" => {
            if c.len() == 2 && kw(&c[1], "right") {
                c.pop();
            }
        }
        "overflow-clip-margin" => {
            c.retain(|x| {
                !kw(x, "padding-box") && !matches!(x, Component::Dimension(v, _) if *v == 0.0)
            });
            if c.is_empty() {
                c.push(Component::Dimension(0.0, Unit::Px));
            }
        }
        "scroll-snap-type" => {
            if c.len() == 2 && kw(&c[1], "proximity") {
                c.pop();
            }
        }
        "grid-auto-flow" => {
            if c.len() == 2 && kw(&c[0], "row") && kw(&c[1], "dense") {
                c.remove(0);
            }
        }
        "object-fit" => {
            if c.len() == 2 && kw(&c[0], "contain") && kw(&c[1], "scale-down") {
                c.remove(0);
            }
        }
        "hyphenate-limit-chars" => {
            if c.len() == 3 && c[2] == c[1] {
                c.pop();
            }
            if c.len() == 2 && kw(&c[1], "auto") {
                c.pop();
            }
        }
        "counter-increment" => counters(c, 1.0),
        "counter-reset" | "counter-set" => counters(c, 0.0),
        "list-style" => list_style(c),
        "grid-area" | "grid-row" | "grid-column" => {
            span_one(c);
            grid_lines(c)
        }
        "grid-row-start" | "grid-row-end" | "grid-column-start" | "grid-column-end" => span_one(c),
        "grid-template-areas" => c.iter_mut().for_each(area_row),
        "grid-template-rows" | "grid-template-columns" => line_names(c),
        "grid-template" | "grid" => grid_template(c),
        "transform-origin" => transform_origin(c),
        "transition" => per_layer(c, |l| {
            if l.len() > 1 && kw(&l[0], "all") {
                l.remove(0);
            }
        }),
        "clip-path" | "offset-path" => {
            if c.len() == 2 && kw(&c[1], "border-box") {
                c.pop();
            } else if c.len() == 2 && kw(&c[0], "border-box") {
                c.remove(0);
            }
        }
        "line-clamp" => line_clamp(c),
        "place-content" => {
            if let Some(last) = c.last()
                && kw(last, "baseline")
            {
                c.push(Component::Keyword("start"));
            }
        }
        "column-rule-inset" | "row-rule-inset" | "rule-inset" => rule_inset(c),
        "mask" => per_layer(c, mask_layer),
        "text-decoration-inset" => sides(c),
        "column-rule" | "row-rule" | "rule" => per_layer(c, |l| {
            if l.iter().any(|x| matches!(x, Component::Function { .. })) {
                return;
            }
            let before = l.len();
            l.retain(|x| {
                !kw(x, "medium")
                    && !kw(x, "none")
                    && !matches!(x, Component::Color(Color::CurrentColor))
            });
            if l.is_empty() && before > 0 {
                l.push(Component::Keyword("medium"));
            }
        }),
        _ => {}
    }
}

/// A type's value in its shortest form.
pub(crate) const TYPES: &[&str] = &[
    "position",
    "bg-position",
    "repeat-style",
    "bg-size",
    "baseline-position",
    "position-area",
    "easing-function",
    "try-tactic",
];

pub(crate) fn of_type(name: &str, c: &mut C<'_>) {
    match name {
        "position" | "bg-position" => position(c),
        "repeat-style" => {
            if c.len() == 2 {
                if c[0] == c[1] {
                    c.pop();
                } else if kw(&c[0], "repeat") && kw(&c[1], "no-repeat") {
                    *c = vec![Component::Keyword("repeat-x")];
                } else if kw(&c[0], "no-repeat") && kw(&c[1], "repeat") {
                    *c = vec![Component::Keyword("repeat-y")];
                }
            }
        }
        "bg-size" => {
            if c.len() == 2 && kw(&c[0], "auto") && kw(&c[1], "auto") {
                c.pop();
            } else if c.len() == 1 && !matches!(c[0], Component::Keyword(_)) {
                c.push(Component::Keyword("auto"));
            }
        }
        // `first baseline` is `baseline`, CSS Box Alignment §4.2.
        "baseline-position" => {
            if c.len() == 2 && kw(&c[0], "first") {
                c.remove(0);
            }
        }
        "position-area" => {
            if c.len() == 2 && c[0] == c[1] {
                c.pop();
            }
        }
        "easing-function" => {
            if let [Component::Keyword(k @ ("step-start" | "step-end"))] = c[..] {
                let mut arguments = vec![number(1.0)];
                if k == "step-start" {
                    arguments.extend([Component::Literal(','), Component::Keyword("start")]);
                }
                c[0] = Component::Function {
                    name: "steps",
                    arguments,
                };
            }
        }
        _ => {}
    }
}

/// A function's arguments in their shortest form, and the name to write
/// it with.
pub(crate) fn function(name: &'static str, args: &mut C<'_>) -> &'static str {
    match name {
        n if n.ends_with("gradient") => gradient(n, args),
        "color-mix" | "palette-mix" => mix(args),
        "color-layers" => {
            // `normal` is the default blend mode.
            if args.first().is_some_and(|a| kw(a, "normal"))
                && args.get(1) == Some(&Component::Literal(','))
            {
                args.drain(..2);
            }
        }
        "circle" | "ellipse" => {
            let end = args.iter().position(|a| kw(a, "at")).unwrap_or(args.len());
            if args[..end].iter().all(|a| kw(a, "closest-side")) {
                args.drain(..end);
            }
        }
        "polygon" | "path" | "shape" => {
            if args.first().is_some_and(|a| kw(a, "nonzero")) {
                args.remove(0);
                if args.first() == Some(&Component::Literal(',')) {
                    args.remove(0);
                }
            }
            if name == "polygon"
                && args.first().is_some_and(|a| kw(a, "round"))
                && args.get(1).is_some_and(is_zero)
            {
                args.drain(..2);
                if args.first() == Some(&Component::Literal(',')) {
                    args.remove(0);
                }
            }
            if name == "path"
                && let Some(Component::String(s)) = args.last_mut()
                && let Some(d) = super::path::normalize(s)
            {
                *s = d.into();
            }
            if name == "shape" {
                shape(args);
            }
        }
        "inset" | "xywh" | "rect" => {
            if let Some(i) = args.iter().position(|a| kw(a, "round"))
                && args[i + 1..].iter().all(is_zero)
            {
                args.truncate(i);
            }
        }
        "ray" => args.retain(|a| !kw(a, "closest-side")),
        "scale" | "scalex" | "scaley" | "scalez" | "scale3d" => args.iter_mut().for_each(to_number),
        "rotate" | "rotatex" | "rotatey" | "rotatez" | "skew" | "skewx" | "skewy"
        | "hue-rotate" => {
            for a in args.iter_mut() {
                if matches!(a, Component::Number { value: 0.0, .. }) {
                    *a = Component::Dimension(0.0, Unit::Deg);
                }
            }
        }
        "grayscale" | "invert" | "opacity" | "sepia" => {
            for a in args.iter_mut() {
                match a {
                    Component::Percentage(v) if *v > 100.0 => *v = 100.0,
                    Component::Number { value, .. } if *value > 1.0 => *a = number(1.0),
                    _ => {}
                }
            }
        }
        "steps" => {
            if args.len() == 3 && (kw(&args[2], "end") || kw(&args[2], "jump-end")) {
                args.truncate(1);
            }
        }
        "counter" | "counters" => {
            if let Some(Component::Ident(s)) = args.last()
                && s.eq_ignore_ascii_case("decimal")
            {
                args.pop();
                args.pop();
            }
        }
        "symbols" => {
            if args.first().is_some_and(|a| kw(a, "symbolic")) {
                args.remove(0);
            }
        }
        "repeat" => line_names(args),
        // CSS Images 4 §2.2: a string is a URL.
        "image-set" => {
            for a in args.iter_mut() {
                if let Component::String(s) = a {
                    *a = Component::Url(std::mem::take(s));
                }
            }
        }
        // CSS Transforms writes these as the spec does.
        "translatex" => return "translateX",
        "translatey" => return "translateY",
        "translatez" => return "translateZ",
        _ => {}
    }
    name
}

/// `shape()`'s arc commands without their defaults, `small`, `ccw` and
/// `rotate 0deg`.
fn shape(args: &mut C<'_>) {
    let mut i = 0;
    while i < args.len() {
        if kw(&args[i], "small") || kw(&args[i], "ccw") {
            args.remove(i);
        } else if kw(&args[i], "rotate") && args.get(i + 1).is_some_and(is_zero) {
            args.drain(i..i + 2);
        } else {
            i += 1;
        }
    }
}

fn to_number(c: &mut Component<'_>) {
    if let Component::Percentage(v) = *c {
        *c = Component::Number {
            value: v / 100.0,
            integer: false,
        };
    }
}

fn scale(c: &mut C<'_>) {
    c.iter_mut().for_each(to_number);
    if c.len() == 3 && matches!(c[2], Component::Number { value: 1.0, .. }) {
        c.pop();
    }
    if c.len() == 2 && text(&c[..1]) == text(&c[1..]) {
        c.pop();
    }
}

fn translate(c: &mut C<'_>) {
    if c.len() == 3 && is_zero(&c[2]) {
        c.pop();
    }
    if c.len() == 2 && matches!(c[1], Component::Dimension(v, _) if v == 0.0) {
        c.pop();
    }
}

/// `rotate: 0 1 0 90deg` as `y 90deg`, and `z 90deg` as `90deg`.
fn rotate(c: &mut C<'_>) {
    let n = |x: &Component<'_>| match x {
        Component::Number { value, .. } => Some(*value),
        _ => None,
    };
    if let [a, b, z, angle] = &c[..]
        && let (Some(x), Some(y), Some(zz)) = (n(a), n(b), n(z))
    {
        let axis = match (x != 0.0, y != 0.0, zz != 0.0) {
            (true, false, false) => Some(("x", x)),
            (false, true, false) => Some(("y", y)),
            (false, false, true) => Some(("z", zz)),
            _ => None,
        };
        if let Some((name, sign)) = axis {
            let mut angle = angle.clone();
            if sign < 0.0
                && let Component::Dimension(v, _) = &mut angle
            {
                *v = -*v;
            }
            *c = if name == "z" {
                vec![angle]
            } else {
                vec![Component::Keyword(name), angle]
            };
            return;
        }
    }
    if c.len() == 2 && kw(&c[0], "z") {
        c.remove(0);
    } else if c.len() == 2 && kw(&c[1], "z") {
        c.pop();
    }
}

fn border_radius(c: &mut C<'_>) {
    let (mut a, b) = halves(c);
    sides(&mut a);
    match b {
        Some(mut b) => {
            sides(&mut b);
            if a == b {
                *c = a;
            } else {
                a.push(Component::Literal('/'));
                a.extend(b);
                *c = a;
            }
        }
        None => *c = a,
    }
}

/// Family names given as strings, as identifiers where they can be:
/// `"New Century Schoolbook"` as `New Century Schoolbook`. A name of one
/// word stays a string, which a keyword would otherwise read as.
fn family_names(c: &mut C<'_>) {
    fn plain(w: &str) -> bool {
        let mut chars = w.chars();
        let start = match chars.next() {
            Some('-') => chars.next(),
            first => first,
        };
        start.is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || !c.is_ascii())
            && w.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || !c.is_ascii())
    }
    if !c
        .iter()
        .any(|x| matches!(x, Component::String(s) if s.contains(' ')))
    {
        return;
    }
    let mut out = Vec::with_capacity(c.len());
    for x in c.drain(..) {
        match x {
            Component::String(s) if s.split(' ').count() > 1 && s.split(' ').all(plain) => {
                let words: Vec<String> = s.split(' ').map(String::from).collect();
                out.extend(words.into_iter().map(|w| Component::Ident(w.into())));
            }
            x => out.push(x),
        }
    }
    *c = out;
}

/// `"liga" 1` as `"liga"`, `on` left out, `off` as `0`.
fn feature(l: &mut C<'_>) {
    match l.get(1) {
        Some(Component::Number { value: 1.0, .. }) => {
            l.pop();
        }
        Some(x) if kw(x, "on") => {
            l.pop();
        }
        Some(x) if kw(x, "off") => l[1] = number(0.0),
        _ => {}
    }
}

/// A keyframes name given as a string, as an identifier unless it's a
/// keyword one would read as.
fn keyframes_name(c: &mut Component<'_>) {
    let Component::String(s) = c else { return };
    let reserved = [
        "none",
        "initial",
        "inherit",
        "unset",
        "revert",
        "revert-layer",
        "default",
    ];
    if s.is_empty() || reserved.iter().any(|r| s.eq_ignore_ascii_case(r)) {
        return;
    }
    *c = Component::Ident(std::mem::take(s));
}

/// One of `animation-range`'s ends: `cover 0%` as `cover` for a start,
/// `cover 100%` for an end.
fn range_part(l: &mut C<'_>, default: f64) {
    if l.len() == 2
        && matches!(l[0], Component::Keyword(_))
        && matches!(l[1], Component::Percentage(v) if v == default)
    {
        l.pop();
    }
}

fn animation_range(l: &mut C<'_>) {
    // The start is a name and maybe an offset, or one value; the end the
    // same, after it.
    let split = match l.first() {
        Some(Component::Keyword(k)) if *k != "normal" => {
            if l.get(1)
                .is_some_and(|x| !matches!(x, Component::Keyword(_)))
            {
                2
            } else {
                1
            }
        }
        Some(_) => 1,
        None => return,
    };
    let mut start = l[..split.min(l.len())].to_vec();
    let mut end = l[split.min(l.len())..].to_vec();
    range_part(&mut start, 0.0);
    if !end.is_empty() {
        range_part(&mut end, 100.0);
        let omitted = match start.first() {
            Some(Component::Keyword(name)) if *name != "normal" => {
                end.len() == 1 && kw(&end[0], name)
            }
            _ => {
                end.len() == 1
                    && (kw(&end[0], "normal")
                        || matches!(end[0], Component::Percentage(v) if v == 100.0))
            }
        };
        if omitted {
            end.clear();
        }
    }
    start.extend(end);
    *l = start;
}

/// Counters without a number get the one the property gives them.
fn counters(c: &mut C<'_>, default: f64) {
    if c.len() == 1 && kw(&c[0], "none") {
        return;
    }
    let mut out = Vec::with_capacity(c.len() * 2);
    let mut it = std::mem::take(c).into_iter().peekable();
    while let Some(x) = it.next() {
        let named = matches!(x, Component::Ident(_));
        out.push(x);
        if named
            && !matches!(
                it.peek(),
                Some(Component::Number { .. } | Component::Math(_))
            )
        {
            out.push(number(default));
        }
    }
    *c = out;
}

/// `list-style`: position, image, type, each left out at its initial
/// value, `none` once for both an image and a type of `none`, and
/// `outside` if that's all.
fn list_style(c: &mut C<'_>) {
    let mut position = None;
    let mut image = None;
    let mut kind = None;
    let mut nones = 0;
    for x in c.drain(..) {
        match &x {
            Component::Keyword("inside" | "outside") if position.is_none() => position = Some(x),
            Component::Keyword("none") => nones += 1,
            Component::Url(_) | Component::Function { .. }
                if image.is_none()
                    && !matches!(
                        x,
                        Component::Function {
                            name: "symbols",
                            ..
                        }
                    ) =>
            {
                image = Some(x)
            }
            _ => kind = Some(x),
        }
    }
    // `none` is the image's or the type's, whichever else isn't given, or
    // both's.
    if nones > 0 {
        match (&image, &kind) {
            (None, None) => {
                image = Some(Component::Keyword("none"));
                kind = Some(Component::Keyword("none"));
            }
            (Some(_), _) => kind = Some(Component::Keyword("none")),
            (None, Some(_)) => image = Some(Component::Keyword("none")),
        }
    }
    let ambiguous = kind.as_ref().is_some_and(|k| {
        matches!(k, Component::Ident(s) if s.eq_ignore_ascii_case("inside") || s.eq_ignore_ascii_case("outside"))
    });
    if position.as_ref().is_some_and(|p| kw(p, "outside")) && !ambiguous {
        position = None;
    }
    if image.as_ref().is_some_and(|i| kw(i, "none")) {
        image = None;
    }
    if kind.as_ref().is_some_and(|k| {
        kw(k, "disc") || matches!(k, Component::Ident(s) if s.eq_ignore_ascii_case("disc"))
    }) {
        kind = None;
    }
    c.extend(position);
    c.extend(image);
    c.extend(kind);
    if c.is_empty() {
        c.push(Component::Keyword("outside"));
    }
}

/// `grid-area: a / b / a / b` as `a / b`: each line left out that's
/// what leaving it out would give, the one it pairs with if that's a
/// name and `auto` if not.
/// `span 1 a` as `span a`: 1 is what `span` counts when left out.
pub(crate) fn span_one(c: &mut C<'_>) {
    let mut i = 0;
    while i + 2 < c.len() {
        let line_start = i == 0 || c[i - 1] == Component::Literal('/');
        if line_start
            && kw(&c[i], "span")
            && matches!(c[i + 1], Component::Number { value: 1.0, .. })
            && matches!(c[i + 2], Component::Ident(_))
        {
            c.remove(i + 1);
        }
        i += 1;
    }
}

fn grid_lines(c: &mut C<'_>) {
    if !c.contains(&Component::Literal('/')) {
        return;
    }
    // Where each line starts and ends in `c`.
    let mut parts: Vec<(usize, usize)> = Vec::with_capacity(4);
    let mut start = 0;
    for (i, x) in c.iter().enumerate() {
        if *x == Component::Literal('/') {
            parts.push((start, i));
            start = i + 1;
        }
    }
    parts.push((start, c.len()));
    let omitted = |line: &[Component<'_>], of: &[Component<'_>]| match of {
        [Component::Ident(_)] => line == of,
        _ => matches!(line, [Component::Keyword("auto")]),
    };
    while parts.len() > 1 {
        // The fourth pairs with the second; the third and second with the
        // first.
        let with = if parts.len() == 4 { 1 } else { 0 };
        let (a, b) = parts[parts.len() - 1];
        let (wa, wb) = parts[with];
        if omitted(&c[a..b], &c[wa..wb]) {
            parts.pop();
        } else {
            break;
        }
    }
    let end = parts.last().map_or(c.len(), |p| p.1);
    c.truncate(end);
}

/// A `grid-template-areas` row as CSS Grid writes it: its cells one space
/// apart, a run of `.` as one.
fn area_row(c: &mut Component<'_>) {
    let Component::String(s) = c else { return };
    let cells: Vec<&str> = s
        .split([' ', '\t', '\n', '\r', '\x0c'])
        .filter(|w| !w.is_empty())
        .flat_map(split_cells)
        .collect();
    let written = cells.join(" ");
    if written != **s {
        *s = written.into();
    }
}

/// `a..b` as `a`, `.`, and `b`: dots are null cells of their own.
fn split_cells(w: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = w;
    while !rest.is_empty() {
        if rest.starts_with('.') {
            let end = rest.find(|c| c != '.').unwrap_or(rest.len());
            out.push(".");
            rest = &rest[end..];
        } else {
            let end = rest.find('.').unwrap_or(rest.len());
            out.push(&rest[..end]);
            rest = &rest[end..];
        }
    }
    out
}

/// Line names with empty `[]` left out and neighbours joined: `[a] [b]`
/// as `[a b]`.
fn line_names(c: &mut C<'_>) {
    let mut out: C<'_> = Vec::with_capacity(c.len());
    for x in c.drain(..) {
        match x {
            Component::Block(names) if names.is_empty() => {}
            Component::Block(names) => match out.last_mut() {
                Some(Component::Block(prev)) => prev.extend(names),
                _ => out.push(Component::Block(names)),
            },
            x => out.push(x),
        }
    }
    *c = out;
}

fn grid_template(c: &mut C<'_>) {
    if c.len() == 3 && kw(&c[0], "none") && kw(&c[2], "none") {
        c.truncate(1);
        return;
    }
    line_names(c);
    // In the areas form, a row's `auto` size is the default.
    if c.iter().any(|x| matches!(x, Component::String(_))) {
        let slash = c
            .iter()
            .position(|x| *x == Component::Literal('/'))
            .unwrap_or(c.len());
        let mut i = 1;
        let mut end = slash;
        while i < end {
            if kw(&c[i], "auto") && matches!(c[i - 1], Component::String(_) | Component::Block(_)) {
                let after_string = c[..i]
                    .iter()
                    .rev()
                    .find(|x| !matches!(x, Component::Block(_)))
                    .is_some_and(|x| matches!(x, Component::String(_)));
                if after_string && matches!(c[i - 1], Component::String(_)) {
                    c.remove(i);
                    end -= 1;
                    continue;
                }
            }
            i += 1;
        }
    }
}

fn horizontal(c: &Component<'_>) -> bool {
    matches!(
        c,
        Component::Keyword("left" | "right" | "x-start" | "x-end")
    )
}

fn vertical(c: &Component<'_>) -> bool {
    matches!(
        c,
        Component::Keyword("top" | "bottom" | "y-start" | "y-end")
    )
}

/// A position as two parts or more, across first: `left` as `left
/// center`, `top` as `center top`, `top 1px left` as `left top 1px`.
fn position(c: &mut C<'_>) {
    let logical = c.iter().any(|x| {
        matches!(
            x,
            Component::Keyword(
                "block-start" | "block-end" | "inline-start" | "inline-end" | "start" | "end"
            )
        )
    });
    if logical {
        return;
    }
    match c.len() {
        1 => {
            if vertical(&c[0]) {
                c.insert(0, Component::Keyword("center"));
            } else {
                c.push(Component::Keyword("center"));
            }
        }
        2 => {
            if vertical(&c[0]) || horizontal(&c[1]) {
                c.swap(0, 1);
            }
        }
        3 | 4 => {
            // Keywords with their offsets, across first.
            let mut groups: Vec<C<'_>> = Vec::new();
            for x in c.drain(..) {
                match x {
                    Component::Keyword(_) => groups.push(vec![x]),
                    x => match groups.last_mut() {
                        Some(g) => g.push(x),
                        None => groups.push(vec![x]),
                    },
                }
            }
            if groups.len() == 2 && (vertical(&groups[0][0]) || horizontal(&groups[1][0])) {
                groups.swap(0, 1);
            }
            *c = groups.concat();
        }
        _ => {}
    }
}

fn transform_origin(c: &mut C<'_>) {
    match c.len() {
        1 => position(c),
        2 | 3 if vertical(&c[0]) || horizontal(&c[1]) => c.swap(0, 1),
        _ => {}
    }
}

/// `line-clamp`: its number, `auto`, what ends the last line unless it's
/// `ellipsis`, and `-webkit-legacy`; `auto` too where nothing else would
/// say the line count.
fn line_clamp(c: &mut C<'_>) {
    if c.len() == 1 && kw(&c[0], "none") {
        return;
    }
    let mut count = None;
    let mut auto = false;
    let mut ellipsis = None;
    let mut legacy = false;
    for x in c.drain(..) {
        match &x {
            Component::Number { .. } | Component::Math(_) => count = Some(x),
            Component::Keyword("auto") => auto = true,
            Component::Keyword("-webkit-legacy") => legacy = true,
            Component::Keyword("ellipsis") => {}
            _ => ellipsis = Some(x),
        }
    }
    let ends_in_string = matches!(ellipsis, None | Some(Component::String(_)));
    if count.is_none() && ends_in_string {
        auto = true;
    }
    c.extend(count);
    if auto {
        c.push(Component::Keyword("auto"));
    }
    c.extend(ellipsis);
    if legacy {
        c.push(Component::Keyword("-webkit-legacy"));
    }
}

/// `column-rule-inset`: a cap's two values, `/`, a junction's two, all
/// written out, and one value if all four are the same.
fn rule_inset<'a>(c: &mut C<'a>) {
    let (cap, junction) = halves(c);
    let pair = |mut v: C<'a>| -> Option<C<'a>> {
        match v.len() {
            1 => {
                v.push(v[0].clone());
                Some(v)
            }
            2 => Some(v),
            _ => None,
        }
    };
    let Some(cap) = pair(cap) else { return };
    let Some(junction) = pair(junction.unwrap_or_else(|| cap.clone())) else {
        return;
    };
    if cap.iter().chain(&junction).all(|x| *x == cap[0]) {
        *c = vec![cap[0].clone()];
        return;
    }
    let mut out = cap;
    out.push(Component::Literal('/'));
    out.extend(junction);
    *c = out;
}

/// One `mask` layer, each part left out at its initial value, and `none`
/// if that's all.
fn mask_layer(l: &mut C<'_>) {
    let boxes: Vec<usize> = (0..l.len())
        .filter(|&i| {
            matches!(
                l[i],
                Component::Keyword(
                    "border-box"
                        | "padding-box"
                        | "content-box"
                        | "fill-box"
                        | "stroke-box"
                        | "view-box"
                        | "no-clip"
                )
            )
        })
        .collect();
    // One box is both origin and clip; two are each.
    match boxes[..] {
        [o] if kw(&l[o], "border-box") => {
            l.remove(o);
        }
        [o, c] if kw(&l[c], "border-box") && kw(&l[o], "border-box") => {
            l.remove(c);
            l.remove(o);
        }
        [o, c] if l[o] == l[c] => {
            l.remove(c);
        }
        [o, c] if kw(&l[o], "border-box") && kw(&l[c], "no-clip") => {
            l.remove(o);
        }
        _ => {}
    }
    // The position and size: `0% 0%` and `auto`.
    if let Some(slash) = l.iter().position(|x| *x == Component::Literal('/')) {
        if l.get(slash + 1).is_some_and(|x| kw(x, "auto"))
            && !l
                .get(slash + 2)
                .is_some_and(|x| !matches!(x, Component::Keyword(_)) || kw(x, "auto"))
        {
            l.drain(slash..slash + 2);
        } else if l.get(slash + 1).is_some_and(|x| kw(x, "auto"))
            && l.get(slash + 2).is_some_and(|x| kw(x, "auto"))
        {
            l.drain(slash..slash + 3);
        }
    }
    let repetition = |x: Option<&Component<'_>>| {
        x.is_some_and(|x| {
            ["repeat", "space", "round", "no-repeat"]
                .iter()
                .any(|k| kw(x, k))
        })
    };
    let lone_repeat: Vec<usize> = (0..l.len())
        .filter(|&i| {
            kw(&l[i], "repeat")
                && !repetition(i.checked_sub(1).and_then(|j| l.get(j)))
                && !repetition(l.get(i + 1))
        })
        .collect();
    for i in lone_repeat.into_iter().rev() {
        l.remove(i);
    }
    l.retain(|x| !kw(x, "add") && !kw(x, "match-source"));
    if l.len() > 1 && kw(&l[0], "none") {
        l.remove(0);
    }
    if l.is_empty() {
        l.push(Component::Keyword("none"));
    }
}

/// A gradient's arguments as CSS Images 4 writes them: the default color
/// interpolation left out (sRGB when every stop is a legacy color, Oklab
/// otherwise), `shorter hue` left out as the default, `xyz` as `xyz-d65`;
/// for a radial gradient `ellipse` and `farthest-corner` left out as the
/// defaults, and `circle` before a length, which says it.
fn gradient(name: &str, args: &mut C<'_>) {
    let end = args
        .iter()
        .position(|c| *c == Component::Literal(','))
        .unwrap_or(args.len());
    let legacy = args[end..].iter().all(|c| match c {
        Component::Color(Color::Channels(ch)) => {
            matches!(ch.model, Model::Rgb | Model::Hsl | Model::Hwb)
        }
        _ => true,
    });
    let mut end = end;
    if let Some(i) = args[..end]
        .iter()
        .position(|c| *c == Component::Keyword("in"))
    {
        if args.get(i + 1) == Some(&Component::Keyword("xyz")) {
            args[i + 1] = Component::Keyword("xyz-d65");
        }
        if args.get(i + 2) == Some(&Component::Keyword("shorter"))
            && args.get(i + 3) == Some(&Component::Keyword("hue"))
        {
            args.drain(i + 2..i + 4);
            end -= 2;
        }
        let default = if legacy { "srgb" } else { "oklab" };
        let hue = matches!(args.get(i + 3), Some(Component::Keyword("hue")));
        if !hue && args.get(i + 1) == Some(&Component::Keyword(default)) {
            args.drain(i..i + 2);
            end -= 2;
        }
    }
    if name.contains("radial") {
        let region = args[..end].iter().position(|c| kw(c, "at")).unwrap_or(end);
        let sizes = args[..region]
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    Component::Percentage(_) | Component::Dimension(..) | Component::Math(_)
                )
            })
            .count();
        let circle = args[..region].iter().any(|c| kw(c, "circle"));
        let before = args.len();
        let mut i = 0;
        args.retain(|c| {
            let inside = i < region;
            i += 1;
            !(inside
                && (kw(c, "farthest-corner")
                    || (kw(c, "ellipse") && !circle)
                    || (kw(c, "circle") && sizes == 1)))
        });
        end -= before - args.len();
    }
    // Nothing left before the stops: no comma either.
    if end == 0 && args.first() == Some(&Component::Literal(',')) {
        args.remove(0);
    }
}

/// `color-mix()`'s and `palette-mix()`'s arguments as CSS Color 5 and
/// CSS Fonts write them: `in xyz` as `in xyz-d65`, `shorter hue` and `in
/// oklab` left out as the defaults; and the percentages filled in: those
/// left out share what the others leave of 100%, and two at 50%, or one
/// at 100%, are left out.
fn mix(args: &mut C<'_>) {
    if args.first() == Some(&Component::Keyword("in")) {
        if args.get(1) == Some(&Component::Keyword("xyz")) {
            args[1] = Component::Keyword("xyz-d65");
        }
        if args.get(2) == Some(&Component::Keyword("shorter"))
            && args.get(3) == Some(&Component::Keyword("hue"))
        {
            args.drain(2..4);
        }
        let hue = matches!(args.get(3), Some(Component::Keyword("hue")));
        if !hue && args.get(1) == Some(&Component::Keyword("oklab")) {
            args.drain(0..2);
            if args.first() == Some(&Component::Literal(',')) {
                args.remove(0);
            }
        }
    }
    let start = match args.first() {
        Some(Component::Keyword("in")) => {
            match args.iter().position(|c| *c == Component::Literal(',')) {
                Some(i) => i + 1,
                None => return,
            }
        }
        _ => 0,
    };
    let parts: Vec<Vec<Component<'_>>> = args[start..]
        .split(|c| *c == Component::Literal(','))
        .map(<[_]>::to_vec)
        .collect();
    let mut items = Vec::new();
    for p in parts {
        match &p[..] {
            [c, Component::Percentage(v)] => items.push((c.clone(), Some(*v))),
            [c] => items.push((c.clone(), None)),
            // Math or anything else: leave the whole thing as it is.
            _ => return,
        }
    }
    let given: f64 = items.iter().filter_map(|(_, p)| *p).sum();
    let missing = items.iter().filter(|(_, p)| p.is_none()).count();
    if missing == items.len() {
        return;
    }
    if missing > 0 {
        let share = ((100.0 - given) / missing as f64).max(0.0);
        for (_, p) in &mut items {
            p.get_or_insert(share);
        }
    }
    if items.len() == 1 && items[0].1 == Some(100.0) {
        items[0].1 = None;
    }
    if items.len() == 2 && items.iter().all(|(_, p)| *p == Some(50.0)) {
        for (_, p) in &mut items {
            *p = None;
        }
    }
    args.truncate(start);
    for (i, (c, p)) in items.into_iter().enumerate() {
        if i > 0 {
            args.push(Component::Literal(','));
        }
        args.push(c);
        args.extend(p.map(Component::Percentage));
    }
}

/// `flex` in full, as browsers write it: grow, shrink and basis, with
/// `1`, `1` and `0%` where they're left out, and `none` as `0 0 auto`.
fn flex(c: &mut C<'_>) {
    if *c == [Component::Keyword("none")] {
        *c = vec![
            Component::Number {
                value: 0.0,
                integer: true,
            },
            Component::Number {
                value: 0.0,
                integer: true,
            },
            Component::Keyword("auto"),
        ];
        return;
    }
    let mut numbers = Vec::new();
    let mut basis = None;
    for x in c.drain(..) {
        match x {
            Component::Number { .. } if numbers.len() < 2 => numbers.push(x),
            Component::Math(ref m) if numbers.len() < 2 && m.fits(super::math::Expect::Number) => {
                numbers.push(x)
            }
            other => basis = Some(other),
        }
    }
    let one = Component::Number {
        value: 1.0,
        integer: true,
    };
    let grow = numbers.first().cloned().unwrap_or(one.clone());
    let shrink = numbers.get(1).cloned().unwrap_or(one);
    let basis = basis.unwrap_or(Component::Percentage(0.0));
    *c = vec![grow, shrink, basis];
}

fn keywords(c: &[Component<'_>]) -> Option<Vec<&'static str>> {
    c.iter()
        .map(|c| match c {
            Component::Keyword(k) => Some(*k),
            _ => None,
        })
        .collect()
}

/// `display` in its shortest form, CSS Display 3 §2: `block flow` is
/// `block`, `inline flex` is `inline-flex`, `block flow list-item` is
/// `list-item`.
fn display(c: &mut C<'_>) {
    let Some(words) = keywords(c) else { return };
    if words == ["flow"] {
        *c = vec![Component::Keyword("block")];
        return;
    }
    if words.len() < 2 {
        return;
    }
    let outer = ["block", "inline", "run-in"];
    let out = words.iter().copied().find(|w| outer.contains(w));
    let list_item = words.contains(&"list-item");
    let inner = words
        .iter()
        .copied()
        .find(|w| !outer.contains(w) && *w != "list-item");
    let out_word = out.unwrap_or("block");
    let inner_word = inner.unwrap_or("flow");
    let written: Vec<&'static str> = if list_item {
        let mut v = Vec::new();
        if out_word != "block" {
            v.push(out_word);
        }
        if inner_word != "flow" {
            v.push(inner_word);
        }
        v.push("list-item");
        v
    } else {
        match (out_word, inner_word) {
            ("block", "flow") => vec!["block"],
            ("inline", "flow") => vec!["inline"],
            ("run-in", "flow") => vec!["run-in"],
            ("block", "flow-root") => vec!["flow-root"],
            ("inline", "flow-root") => vec!["inline-block"],
            ("block", "table") => vec!["table"],
            ("inline", "table") => vec!["inline-table"],
            ("block", "flex") => vec!["flex"],
            ("inline", "flex") => vec!["inline-flex"],
            ("block", "grid") => vec!["grid"],
            ("inline", "grid") => vec!["inline-grid"],
            ("inline", "ruby") => vec!["ruby"],
            (o, i) => vec![o, i],
        }
    };
    *c = written.into_iter().map(Component::Keyword).collect();
}

/// `white-space` as its legacy keywords where one says it, and otherwise
/// without what's at its initial value.
fn white_space(c: &mut C<'_>) {
    let Some(words) = keywords(c) else { return };
    let collapse = [
        "collapse",
        "preserve",
        "preserve-breaks",
        "preserve-spaces",
        "break-spaces",
    ];
    let wrap = ["wrap", "nowrap"];
    if words
        .iter()
        .any(|w| ["normal", "pre", "pre-wrap", "pre-line"].contains(w))
    {
        return;
    }
    let col = words
        .iter()
        .copied()
        .find(|w| collapse.contains(w))
        .unwrap_or("collapse");
    let wr = words
        .iter()
        .copied()
        .find(|w| wrap.contains(w))
        .unwrap_or("wrap");
    let trim: Vec<&'static str> = words
        .iter()
        .copied()
        .filter(|w| !collapse.contains(w) && !wrap.contains(w) && *w != "none")
        .collect();
    let mut written: Vec<&'static str> = Vec::new();
    if trim.is_empty() {
        let legacy = match (col, wr) {
            ("collapse", "wrap") => Some("normal"),
            ("preserve", "nowrap") => Some("pre"),
            ("preserve", "wrap") => Some("pre-wrap"),
            ("preserve-breaks", "wrap") => Some("pre-line"),
            ("collapse", "nowrap") => Some("nowrap"),
            ("break-spaces", "wrap") => Some("break-spaces"),
            _ => None,
        };
        if let Some(l) = legacy {
            *c = vec![Component::Keyword(l)];
            return;
        }
    }
    if col != "collapse" {
        written.push(col);
    }
    if wr != "wrap" {
        written.push(wr);
    }
    written.extend(trim);
    if written.is_empty() {
        written.push("normal");
    }
    *c = written.into_iter().map(Component::Keyword).collect();
}

#[cfg(test)]
mod tests {
    use super::PROPERTIES;

    #[test]
    fn sorted() {
        assert!(PROPERTIES.windows(2).all(|w| w[0] < w[1]));
    }
}
