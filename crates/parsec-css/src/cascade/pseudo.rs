// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Pseudo-elements, CSS Pseudo-Elements 4: which rules style them, and
//! their computed values, from their originating element.
//!
//! A rule whose selector ends in a pseudo-element is matched, minus that
//! pseudo-element, against the originating element while the document is
//! styled, and remembered with its place in the cascade. Its computed
//! values are only worked out when they're asked for, so a page whose
//! pseudo-elements nobody reads costs no more than matching their rules.

use std::collections::HashMap;
use std::fmt;

use parsec::NodeId;

use super::{Computed, Declared, Key, add, compute};
use crate::media::Environment;
use crate::parser::ComponentValue;
use crate::rules::{PropertyRule, Syntax};
use crate::tokenizer::{Token, Tokenizer};

/// A pseudo-element the cascade styles.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Pseudo {
    Before,
    After,
    Marker,
    Placeholder,
    FirstLine,
    FirstLetter,
    Selection,
    Backdrop,
    FileSelectorButton,
    /// `::highlight(name)`, a custom highlight.
    Highlight(String),
}

impl Pseudo {
    /// `::before`, or the one-colon `:before` CSS 2 allowed for its four,
    /// as `getComputedStyle()` takes them; ASCII case-insensitive.
    pub fn parse(text: &str) -> Option<Pseudo> {
        let text = text.trim();
        let (name, legacy) = match text.strip_prefix("::") {
            Some(n) => (n, false),
            None => (text.strip_prefix(':')?, true),
        };
        let p = Pseudo::named(name)?;
        let legacy_ok = matches!(
            p,
            Pseudo::Before | Pseudo::After | Pseudo::FirstLine | Pseudo::FirstLetter
        );
        (!legacy || legacy_ok).then_some(p)
    }

    fn named(name: &str) -> Option<Pseudo> {
        let lower = name.to_ascii_lowercase();
        if let Some(arg) = lower
            .strip_prefix("highlight(")
            .and_then(|a| a.strip_suffix(')'))
        {
            // The name keeps its case: it's a custom identifier.
            let start = name.len() - arg.len() - 1;
            let arg = name[start..name.len() - 1].trim();
            return (!arg.is_empty()).then(|| Pseudo::Highlight(arg.to_string()));
        }
        Some(match lower.as_str() {
            "before" => Pseudo::Before,
            "after" => Pseudo::After,
            "marker" => Pseudo::Marker,
            "placeholder" => Pseudo::Placeholder,
            "first-line" => Pseudo::FirstLine,
            "first-letter" => Pseudo::FirstLetter,
            "selection" => Pseudo::Selection,
            "backdrop" => Pseudo::Backdrop,
            "file-selector-button" => Pseudo::FileSelectorButton,
            _ => return None,
        })
    }

    /// A highlight pseudo-element, which inherits from its parent
    /// element's, CSS Pseudo 4 §3.5.
    fn is_highlight(&self) -> bool {
        matches!(self, Pseudo::Selection | Pseudo::Highlight(_))
    }
}

impl fmt::Display for Pseudo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Pseudo::Before => "before",
            Pseudo::After => "after",
            Pseudo::Marker => "marker",
            Pseudo::Placeholder => "placeholder",
            Pseudo::FirstLine => "first-line",
            Pseudo::FirstLetter => "first-letter",
            Pseudo::Selection => "selection",
            Pseudo::Backdrop => "backdrop",
            Pseudo::FileSelectorButton => "file-selector-button",
            Pseudo::Highlight(n) => return write!(f, "::highlight({n})"),
        };
        write!(f, "::{name}")
    }
}

/// What a complex selector's pseudo-element is.
#[derive(Debug, PartialEq)]
pub(super) enum Split {
    /// It has none: it styles elements.
    No,
    /// It styles this pseudo-element of what the text before it matches.
    Pseudo(String, Pseudo),
    /// A pseudo-element this doesn't style, or one followed by more (a
    /// state like `:hover`, or `::marker` on `::before`), which nothing
    /// in a static document matches.
    Unstyled,
}

/// Splits a complex selector at its pseudo-element: the first one outside
/// any brackets, read with the CSS tokenizer so strings and escapes can't
/// fool it. The originating part is `*` when nothing comes before.
pub(super) fn split_pseudo(selector: &str) -> Split {
    if !selector.contains(':') {
        return Split::No;
    }
    let tokens: Vec<(Token<'_>, crate::Span)> = Tokenizer::new(selector).collect();
    let mut depth = 0usize;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i].0 {
            Token::Function(_) | Token::LeftParen | Token::LeftBracket => depth += 1,
            Token::RightParen | Token::RightBracket => depth = depth.saturating_sub(1),
            Token::Colon if depth == 0 => {
                let double = matches!(tokens.get(i + 1), Some((Token::Colon, _)));
                let at = if double { i + 2 } else { i + 1 };
                let (name, end) = match tokens.get(at) {
                    Some((Token::Ident(n), s)) => (n.to_string(), s.end),
                    // `::highlight(name)`: up to its `)`.
                    Some((Token::Function(n), _)) if double => {
                        let close = tokens[at..]
                            .iter()
                            .position(|(t, _)| *t == Token::RightParen)
                            .map(|k| at + k);
                        let Some(close) = close else {
                            return Split::Unstyled;
                        };
                        let inner = &selector[tokens[at].1.end..tokens[close].1.start];
                        (format!("{n}({inner})"), tokens[close].1.end)
                    }
                    _ => {
                        i += 1;
                        continue;
                    }
                };
                let legacy = ["before", "after", "first-line", "first-letter"]
                    .iter()
                    .any(|l| name.eq_ignore_ascii_case(l));
                if !double && !legacy {
                    // A pseudo-class.
                    i = at + 1;
                    continue;
                }
                if !selector[end..].trim().is_empty() {
                    return Split::Unstyled;
                }
                let Some(p) = Pseudo::named(&name) else {
                    return Split::Unstyled;
                };
                let before = &selector[..tokens[i].1.start];
                let origin = before.trim_end();
                // With nothing in its compound but itself, `a > ::before`,
                // it's on `*`.
                let bare = origin.is_empty()
                    || before.ends_with(char::is_whitespace)
                    || origin.ends_with(['>', '+', '~']);
                return match bare {
                    true => Split::Pseudo(format!("{before}*").trim_start().to_string(), p),
                    false => Split::Pseudo(origin.to_string(), p),
                };
            }
            _ => {}
        }
        i += 1;
    }
    Split::No
}

/// One rule for pseudo-elements, its declarations kept as text, so the
/// styles can be worked out after the stylesheets have gone.
pub(super) struct PseudoRule {
    pub pseudo: Pseudo,
    /// Name, value and importance.
    pub declarations: Vec<(String, String, bool)>,
    /// Whether a `content` has `attr()` in it, which needs the
    /// originating element's attributes.
    pub needs_attrs: bool,
}

impl PseudoRule {
    /// A rule's declarations copied out of its stylesheet.
    pub fn of(e: &super::Entry<'_>) -> PseudoRule {
        PseudoRule {
            pseudo: e.pseudo.clone().unwrap_or(Pseudo::Before),
            declarations: e
                .declarations
                .iter()
                .map(|(n, v, imp)| (n.to_string(), crate::to_css(v), *imp))
                .collect(),
            needs_attrs: e
                .declarations
                .iter()
                .any(|(n, v, _)| n.eq_ignore_ascii_case("content") && has_attr(v)),
        }
    }
}

/// Everything working out a pseudo-element's style needs, kept with the
/// document's styles.
#[derive(Default)]
pub(super) struct PseudoStyles {
    pub rules: Vec<PseudoRule>,
    /// Rules that match every element: the originating part is `*`.
    pub always: Vec<(usize, Key)>,
    /// The other rules each element matched, by element.
    pub matched: HashMap<NodeId, Vec<(usize, Key)>>,
    /// Elements' attributes, for `attr()` in `content`.
    pub attrs: HashMap<NodeId, Vec<(String, String)>>,
    pub env: Environment,
    pub registered: HashMap<String, (PropertyRule, Syntax)>,
    pub root: Option<NodeId>,
}

/// Whether `value` has an `attr()` in it, at any depth.
pub(super) fn has_attr(values: &[ComponentValue<'_>]) -> bool {
    values.iter().any(|v| match v {
        ComponentValue::Function(f) => {
            f.name.eq_ignore_ascii_case("attr") || has_attr(&f.arguments)
        }
        ComponentValue::Block(b) => has_attr(&b.contents),
        _ => false,
    })
}

/// `content`'s `attr()`s replaced by the attribute's value as a string,
/// as CSS Values 5 substitutes them at computed-value time; a missing
/// attribute is its fallback, or the empty string. A typed `attr()`
/// stays as written.
fn substitute_attrs(value: &str, attrs: &[(String, String)]) -> String {
    let values = crate::parse_component_values(value);
    let mut out = String::new();
    for v in &values {
        if let ComponentValue::Function(f) = v
            && f.name.eq_ignore_ascii_case("attr")
        {
            let args = crate::to_css(&f.arguments);
            let (name, fallback) = match args.split_once(',') {
                Some((n, fb)) => (n.trim(), Some(fb.trim())),
                None => (args.trim(), None),
            };
            let name = name.strip_suffix("raw-string").map_or(name, str::trim_end);
            if !name.contains(|c: char| c.is_whitespace() || c == '|') && !name.is_empty() {
                match attrs.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
                    Some((_, v)) => crate::write_string(&mut out, v).unwrap(),
                    None => match fallback {
                        Some(fb) if !fb.is_empty() => out.push_str(fb),
                        _ => out.push_str("\"\""),
                    },
                }
                continue;
            }
        }
        out.push_str(&crate::to_css(std::slice::from_ref(v)));
    }
    out
}

/// Whether a property applies to a pseudo-element, so its declarations
/// there count. What doesn't apply takes its value as if undeclared.
fn applies(p: &Pseudo, name: &str) -> bool {
    let font = name.starts_with("font");
    let background = name.starts_with("background");
    let decoration = name.starts_with("text-decoration")
        || name.starts_with("text-underline")
        || name.starts_with("text-emphasis")
        || name == "text-shadow";
    // Typesetting that applies to inline boxes and doesn't change where
    // lines break, which would change what the first line is.
    let typesetting = matches!(
        name,
        "letter-spacing" | "word-spacing" | "text-transform" | "text-justify"
    );
    let inline_layout = matches!(
        name,
        "line-height"
            | "vertical-align"
            | "baseline-shift"
            | "baseline-source"
            | "alignment-baseline"
            | "dominant-baseline"
    );
    // What browsers apply as well, which CSS Pseudo 4 allows.
    let animation = name.starts_with("animation");
    let first_line = font
        || background
        || decoration
        || typesetting
        || animation
        || matches!(name, "color" | "opacity" | "ruby-position");
    match p {
        Pseudo::Before | Pseudo::After | Pseudo::Backdrop | Pseudo::FileSelectorButton => true,
        Pseudo::FirstLine => first_line || inline_layout,
        Pseudo::Placeholder => first_line,
        Pseudo::FirstLetter => {
            first_line
                || inline_layout
                || name.starts_with("margin")
                || name.starts_with("padding")
                || (name.starts_with("border") && !name.starts_with("border-collapse"))
                || matches!(name, "box-shadow" | "float")
        }
        // CSS Lists 3 §3.1.1, and the text properties a marker's text
        // takes, as browsers have them.
        Pseudo::Marker => {
            font || animation
                || name.starts_with("transition")
                || name.starts_with("white-space")
                || name.starts_with("text-emphasis")
                || matches!(
                    name,
                    "color"
                        | "content"
                        | "quotes"
                        | "direction"
                        | "unicode-bidi"
                        | "text-combine-upright"
                        | "line-height"
                        | "letter-spacing"
                        | "word-spacing"
                        | "tab-size"
                        | "text-transform"
                        | "text-indent"
                        | "hyphens"
                        | "line-break"
                        | "overflow-wrap"
                        | "word-break"
                        | "text-shadow"
                        | "text-decoration-skip-ink"
                        | "cursor"
                )
        }
        // CSS Pseudo 4 §3.2.
        Pseudo::Selection | Pseudo::Highlight(_) => highlight_applies(name),
    }
}

pub(super) fn highlight_applies(name: &str) -> bool {
    name.starts_with("text-decoration")
        || name.starts_with("text-underline")
        || matches!(
            name,
            "color"
                | "background-color"
                | "text-shadow"
                | "stroke-color"
                | "fill-color"
                | "stroke-width"
        )
}

impl PseudoStyles {
    /// The cascade's declarations for `p` on `el`, put in order.
    fn style(
        &self,
        el: NodeId,
        p: &Pseudo,
        parent: &Computed,
        originating: &Computed,
        root: Option<&Computed>,
        parent_highlight: Option<&Computed>,
    ) -> Computed {
        let none = Vec::new();
        let matched = self.matched.get(&el).unwrap_or(&none);
        let attrs = self.attrs.get(&el).map_or(&[][..], |a| &a[..]);
        // Values as text first, `attr()` substituted, then parsed: what
        // the declarations borrow has to outlive them.
        let mut texts: Vec<(Key, &str, std::borrow::Cow<'_, str>, bool)> = Vec::new();
        for &(r, key) in self.always.iter().chain(matched) {
            let rule = &self.rules[r];
            if rule.pseudo != *p {
                continue;
            }
            for (name, value, important) in &rule.declarations {
                let value = if rule.needs_attrs && name.eq_ignore_ascii_case("content") {
                    std::borrow::Cow::Owned(substitute_attrs(value, attrs))
                } else {
                    std::borrow::Cow::Borrowed(value.as_str())
                };
                texts.push((key, name, value, *important));
            }
        }
        let parsed: Vec<Vec<ComponentValue<'_>>> = texts
            .iter()
            .map(|(_, _, v, _)| crate::parse_component_values(v))
            .collect();
        let mut declared: HashMap<&'static str, Vec<Declared<'_>>> = HashMap::new();
        let mut custom: HashMap<String, Vec<(Key, Vec<ComponentValue<'_>>)>> = HashMap::new();
        for ((key, name, _, important), values) in texts.iter().zip(&parsed) {
            add(&mut declared, &mut custom, name, values, *important, *key);
        }
        declared.retain(|name, _| applies(p, name));
        for list in declared.values_mut() {
            list.sort_by_key(|d| d.key);
        }
        for list in custom.values_mut() {
            list.sort_by_key(|d| d.0);
        }
        let mut out = compute::compute(compute::Input {
            declared: &declared,
            custom: &custom,
            parent: Some(parent),
            root,
            is_root: false,
            env: &self.env,
            cq: parent.cq_children,
            registered: &self.registered,
        });
        compute::pseudo_fix_up(&mut out, p, originating, parent_highlight);
        out
    }
}

impl super::Styled {
    /// `property`'s value on `el`'s pseudo-element `p`, as
    /// `getComputedStyle(el, "::before")` writes it; `None` for a node
    /// that isn't an element or a property that isn't.
    ///
    /// ```
    /// use parsec_css::cascade::{Pseudo, Styler};
    ///
    /// let doc = parsec::parse("<style>p::before { content: 'x'; color: green }</style><p>", false);
    /// let styled = Styler::new(Default::default()).with_document_styles(&doc).style(&doc);
    /// let p = parsec::css::SelectorList::parse("p").unwrap().select_first(&doc, doc.root()).unwrap();
    /// assert_eq!(styled.get_pseudo(p, &Pseudo::Before, "color").as_deref(), Some("rgb(0, 128, 0)"));
    /// assert_eq!(styled.get_pseudo(p, &Pseudo::After, "content").as_deref(), Some("none"));
    /// assert_eq!(styled.pseudo_exists(p, &Pseudo::Before), Some(true));
    /// ```
    pub fn get_pseudo(&self, el: NodeId, p: &Pseudo, property: &str) -> Option<String> {
        self.computed_pseudo(el, p)?.get(property)
    }

    /// Everything about the style of `el`'s pseudo-element `p`, worked out
    /// now: it inherits from `el`, `::first-letter` from `::first-line`,
    /// and a highlight from its parent element's highlight.
    pub fn computed_pseudo(&self, el: NodeId, p: &Pseudo) -> Option<Computed> {
        let own = self.styles.get(&el)?;
        let ps = &self.pseudo;
        let root = ps.root.and_then(|r| self.styles.get(&r));
        if p.is_highlight() {
            // Down from the outermost element styled, each highlight
            // inheriting from the one before.
            let mut chain = vec![el];
            let mut at = own.parent;
            while let Some(a) = at {
                let Some(c) = self.styles.get(&a) else { break };
                chain.push(a);
                at = c.parent;
            }
            let mut highlight: Option<Computed> = None;
            for &n in chain.iter().rev() {
                let originating = &self.styles[&n];
                let h = ps.style(n, p, originating, originating, root, highlight.as_ref());
                highlight = Some(h);
            }
            return highlight;
        }
        let first_line;
        let parent = match p {
            Pseudo::FirstLetter => {
                first_line = self.computed_pseudo(el, &Pseudo::FirstLine)?;
                &first_line
            }
            _ => own,
        };
        Some(ps.style(el, p, parent, own, root, None))
    }

    /// Whether `el` has the pseudo-element `p` at all: `::before` and
    /// `::after` when their `content` is something, `::marker` on a list
    /// item with a marker to show, none of them under `display: none`.
    /// `None` for the others, whose being there depends on text, layout or
    /// state (the first line, a selection, the top layer), and for a node
    /// that isn't an element.
    pub fn pseudo_exists(&self, el: NodeId, p: &Pseudo) -> Option<bool> {
        let own = self.styles.get(&el)?;
        if !matches!(p, Pseudo::Before | Pseudo::After | Pseudo::Marker) {
            return None;
        }
        if own.get("display").as_deref() == Some("none") {
            return Some(false);
        }
        let c = self.computed_pseudo(el, p)?;
        let content = c.get("content").unwrap_or_default();
        Some(match p {
            Pseudo::Marker => {
                let list_item = own
                    .get("display")
                    .is_some_and(|d| d.split_ascii_whitespace().any(|w| w == "list-item"));
                let shown = content != "normal"
                    || own.get("list-style-type").as_deref() != Some("none")
                    || own.get("list-style-image").as_deref() != Some("none");
                list_item && content != "none" && shown
            }
            _ => {
                !matches!(content.as_str(), "none" | "normal")
                    && c.get("display").as_deref() != Some("none")
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cascade::Styler;
    use parsec::css::SelectorList;

    /// Each `(selector, pseudo, property)`'s value on `html` styled.
    fn styled(html: &str) -> (parsec::Document, super::super::Styled) {
        let doc = parsec::parse(html, false);
        let styled = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .style(&doc);
        (doc, styled)
    }

    fn at(doc: &parsec::Document, sel: &str) -> NodeId {
        SelectorList::parse(sel)
            .unwrap()
            .select_first(doc, doc.root())
            .unwrap()
    }

    fn get(html: &str, sel: &str, p: &str, property: &str) -> String {
        let (doc, s) = styled(html);
        let p = Pseudo::parse(p).unwrap();
        s.get_pseudo(at(&doc, sel), &p, property)
            .unwrap_or_default()
    }

    #[test]
    fn before_and_after() {
        let html = "<style>p { color: blue; margin-top: 3px } #a::before { content: 'x' counter(c) open-quote attr(title); margin-left: 2em } \
                    p:before { color: green } p::after { content: normal }</style><p id=a title='hi' style='font-size: 10px'>";
        assert_eq!(get(html, "p", "::before", "color"), "rgb(0, 128, 0)");
        // Inherited from the paragraph, `em` its font size; not inherited
        // isn't.
        assert_eq!(get(html, "p", "::before", "margin-left"), "20px");
        assert_eq!(get(html, "p", "::before", "margin-top"), "0px");
        assert_eq!(get(html, "p", "::after", "color"), "rgb(0, 0, 255)");
        assert_eq!(
            get(html, "p", "::before", "content"),
            "\"x\" counter(c) open-quote \"hi\""
        );
        assert_eq!(get(html, "p", "::after", "content"), "none");
        let (doc, s) = styled(html);
        let p = at(&doc, "p");
        assert_eq!(s.pseudo_exists(p, &Pseudo::Before), Some(true));
        assert_eq!(s.pseudo_exists(p, &Pseudo::After), Some(false));
        assert_eq!(s.pseudo_exists(p, &Pseudo::FirstLine), None);
        // The UA's quotes.
        assert_eq!(get("<q>", "q", "::before", "content"), "open-quote");
        let (doc, s) = styled("<style>q { display: none }</style><q>");
        assert_eq!(s.pseudo_exists(at(&doc, "q"), &Pseudo::Before), Some(false));
    }

    #[test]
    fn cascade_order() {
        // `p::before` is (0,0,2), and beats `*::before` and `::before`.
        let html = "<style>p::before { color: green } ::before { color: red } *::before { color: red }</style><p>";
        assert_eq!(get(html, "p", "::before", "color"), "rgb(0, 128, 0)");
        let html = "<style>@layer a { #x::before { color: green !important } } #x::before { color: red !important }</style><p id=x>";
        assert_eq!(get(html, "p", "::before", "color"), "rgb(0, 128, 0)");
        // `attr()` from a rule for every element.
        let html = "<style>*::before { content: attr(data-n) }</style><p data-n=3><i>";
        assert_eq!(get(html, "p", "::before", "content"), "\"3\"");
        assert_eq!(get(html, "i", "::before", "content"), "\"\"");
        // Scoped, and in a container that's the originating element.
        let html = "<style>@scope (.s) { p::before { color: green } } .c { container-type: inline-size } \
                    @container (width > 100px) { .c::after { color: green } }</style><div class=s><p></p></div><p class=c>";
        assert_eq!(get(html, "div p", "::before", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "body > p", "::before", "color"), "rgb(0, 0, 0)");
        let doc = parsec::parse(html, false);
        let s = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .container_sizes(|_, _| Some((200.0, 10.0)))
            .style(&doc);
        let c = at(&doc, ".c");
        assert_eq!(
            s.get_pseudo(c, &Pseudo::After, "color").as_deref(),
            Some("rgb(0, 128, 0)")
        );
        // A pseudo-element after a state never matches here.
        let html = "<style>p::before:hover { color: red }</style><p>";
        assert_eq!(get(html, "p", "::before", "color"), "rgb(0, 0, 0)");
    }

    #[test]
    fn the_wiki_example() {
        let html = "<style>li { color: blue } li::marker { color: green; position: absolute } \
                    .n::before { content: '#' attr(data-n) ' ' }</style><ol><li class=n data-n=1>One</ol>";
        assert_eq!(get(html, "li", "::marker", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "li", "::marker", "position"), "static");
        assert_eq!(get(html, "li", "::before", "content"), "\"#\" \"1\" \" \"");
    }

    #[test]
    fn markers() {
        let html = "<style>li { text-indent: 4px; text-transform: uppercase } .in { list-style-position: inside } \
                    li::marker { color: green; position: absolute; list-style-type: square; display: none } .none { list-style-type: none }</style>\
                    <ul><li>a<li class=in>b<li class=none>c</ul><p>";
        assert_eq!(get(html, "li", "::marker", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "li", "::marker", "unicode-bidi"), "isolate");
        assert_eq!(
            get(html, "li", "::marker", "font-variant-numeric"),
            "tabular-nums"
        );
        assert_eq!(get(html, "li", "::marker", "text-transform"), "none");
        assert_eq!(get(html, "li", "::marker", "text-indent"), "0px");
        // What doesn't apply to a marker doesn't count.
        assert_eq!(get(html, "li", "::marker", "position"), "static");
        assert_eq!(get(html, "li", "::marker", "list-style-type"), "disc");
        assert_eq!(get(html, "li", "::marker", "display"), "block");
        assert_eq!(get(html, ".in", "::marker", "display"), "inline");
        assert_eq!(get(html, "li", "::marker", "content"), "normal");
        let (doc, s) = styled(html);
        assert_eq!(s.pseudo_exists(at(&doc, "li"), &Pseudo::Marker), Some(true));
        assert_eq!(
            s.pseudo_exists(at(&doc, ".none"), &Pseudo::Marker),
            Some(false)
        );
        assert_eq!(s.pseudo_exists(at(&doc, "p"), &Pseudo::Marker), Some(false));
    }

    #[test]
    fn first_line_and_letter() {
        let html = "<style>p { color: blue } p::first-line { color: green; margin-left: 9px; font-size: 20px } \
                    p::first-letter { margin-left: 3px; float: left; position: absolute }</style><p>";
        assert_eq!(get(html, "p", "::first-line", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "p", "::first-line", "margin-left"), "0px");
        // The first letter is in the first line.
        assert_eq!(get(html, "p", "::first-letter", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "p", "::first-letter", "font-size"), "20px");
        assert_eq!(get(html, "p", "::first-letter", "margin-left"), "3px");
        assert_eq!(get(html, "p", "::first-letter", "float"), "left");
        assert_eq!(get(html, "p", "::first-letter", "position"), "static");
        let html = "<style>input::placeholder { color: green; line-height: 9px; background-color: inherit } input { background-color: red }</style><input>";
        assert_eq!(
            get(html, "input", "::placeholder", "color"),
            "rgb(0, 128, 0)"
        );
        assert_eq!(
            get(html, "input", "::placeholder", "background-color"),
            "rgb(255, 0, 0)"
        );
        assert_eq!(get(html, "input", "::placeholder", "line-height"), "normal");
    }

    #[test]
    fn highlights() {
        let html = "<style>body { color: blue; margin-top: 2px } div::selection { background-color: green; color: red; margin-top: 7px } \
                    p::highlight(x) { color: green }</style><div><p></p></div>";
        // The child's ::selection inherits the parent's, `background-color`
        // too, though it's not an inherited property.
        assert_eq!(
            get(html, "p", "::selection", "background-color"),
            "rgb(0, 128, 0)"
        );
        assert_eq!(get(html, "p", "::selection", "color"), "rgb(255, 0, 0)");
        // What doesn't apply is the originating element's.
        assert_eq!(get(html, "div", "::selection", "margin-top"), "0px");
        assert_eq!(get(html, "body", "::selection", "margin-top"), "2px");
        // From the root's highlight, whose colour is the root's: not the
        // body's own.
        assert_eq!(get(html, "body", "::selection", "color"), "rgb(0, 0, 0)");
        assert_eq!(get(html, "p", "::highlight(x)", "color"), "rgb(0, 128, 0)");
        assert_eq!(get(html, "p", "::highlight(y)", "color"), "rgb(0, 0, 0)");
        let html = "<style>:root { color: blue }</style><p>";
        assert_eq!(get(html, "p", "::selection", "color"), "rgb(0, 0, 255)");
    }

    #[test]
    fn splits() {
        use Split::*;
        assert_eq!(split_pseudo("p.x"), No);
        assert_eq!(split_pseudo("p:hover"), No);
        assert_eq!(
            split_pseudo("#t::before"),
            Pseudo("#t".into(), super::Pseudo::Before)
        );
        assert_eq!(
            split_pseudo("li:BEFORE"),
            Pseudo("li".into(), super::Pseudo::Before)
        );
        assert_eq!(
            split_pseudo("::marker"),
            Pseudo("*".into(), super::Pseudo::Marker)
        );
        assert_eq!(
            split_pseudo("a > b:not(.c)::first-line"),
            Pseudo("a > b:not(.c)".into(), super::Pseudo::FirstLine)
        );
        assert_eq!(
            split_pseudo("#t::highlight(Foo)"),
            Pseudo("#t".into(), super::Pseudo::Highlight("Foo".into()))
        );
        // Inside `:is()`, or in an attribute value, it isn't one.
        assert_eq!(split_pseudo(":is(a::before)"), No);
        assert_eq!(split_pseudo("[title='::before']"), No);
        assert_eq!(split_pseudo("a::before:hover"), Unstyled);
        assert_eq!(split_pseudo("a::before::marker"), Unstyled);
        assert_eq!(split_pseudo("a::part(x)"), Unstyled);
        assert_eq!(
            split_pseudo("a ::before"),
            Pseudo("a *".into(), super::Pseudo::Before)
        );
        assert_eq!(
            split_pseudo("a>::after"),
            Pseudo("a>*".into(), super::Pseudo::After)
        );
        // `:marker` was never one with one colon.
        assert_eq!(split_pseudo("a:marker"), No);
    }

    #[test]
    fn parses_names() {
        assert_eq!(Pseudo::parse("::BEFORE"), Some(Pseudo::Before));
        assert_eq!(Pseudo::parse(":first-line"), Some(Pseudo::FirstLine));
        assert_eq!(Pseudo::parse(":marker"), None);
        assert_eq!(
            Pseudo::parse("::highlight(foo)"),
            Some(Pseudo::Highlight("foo".into()))
        );
        assert_eq!(Pseudo::parse("::nope"), None);
        assert_eq!(
            Pseudo::FileSelectorButton.to_string(),
            "::file-selector-button"
        );
    }

    #[test]
    fn attr_in_content() {
        let attrs = vec![("title".to_string(), "a\"b".to_string())];
        assert_eq!(substitute_attrs("attr(title)", &attrs), "\"a\\\"b\"");
        assert_eq!(
            substitute_attrs("\"[\" attr(nope) \"]\"", &attrs),
            "\"[\" \"\" \"]\""
        );
        assert_eq!(substitute_attrs("attr(nope, 'x')", &attrs), "\"x\"");
    }
}
