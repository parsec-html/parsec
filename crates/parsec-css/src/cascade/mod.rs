// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The cascade, CSS Cascading and Inheritance Level 5, over a Parsec
//! document: stylesheets from the user agent, the user and the author,
//! `@media` and `@supports` evaluated against an [`Environment`],
//! `@layer`, nesting, specificity, `!important` and `style=""`, then
//! every element's computed values with inheritance, the CSS-wide
//! keywords (`revert` and `revert-layer` included), custom properties and
//! `var()`.
//!
//! ```
//! use parsec_css::cascade::Styler;
//!
//! let doc = parsec::parse(
//!     "<style>@media (min-width: 600px) { p { font-size: 2em } } .x { color: red !important }</style>\
//!      <p class=x style='color: blue'>hi</p>",
//!     false,
//! );
//! let styled = Styler::new(Default::default()).with_document_styles(&doc).style(&doc);
//! let p = parsec::css::SelectorList::parse("p").unwrap().select_first(&doc, doc.root()).unwrap();
//! assert_eq!(styled.get(p, "color").as_deref(), Some("rgb(255, 0, 0)"));
//! assert_eq!(styled.get(p, "font-size").as_deref(), Some("32px"));
//! assert_eq!(styled.get(p, "display").as_deref(), Some("block"));
//! ```
//!
//! What it doesn't do is layout: a value that needs boxes to work out,
//! `width: 50%` as pixels say, stays as its computed value, and
//! `@container` and `@scope` rules don't apply. `@import` isn't fetched.

mod compute;

use std::collections::HashMap;

use parsec::css::SelectorList;
use parsec::{Document, Kind, NodeId};

use crate::media::{Environment, MediaQueryList};
use crate::parser::{Block, ComponentValue, Item, Rule};
use crate::tokenizer::Token;
use crate::values::{CssWide, Specified, expand, property};

pub use compute::Computed;

/// Where a stylesheet comes from, which decides what wins between them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Origin {
    UserAgent,
    User,
    Author,
}

/// The HTML Standard's default styles, from its rendering section
/// (§15.3), the parts that don't need layout or form controls.
pub const HTML_UA_STYLESHEET: &str = include_str!("html.css");

struct Sheet {
    css: String,
    origin: Origin,
    media: Option<MediaQueryList>,
}

/// Collects stylesheets and styles documents with them.
#[derive(Default)]
pub struct Styler {
    env: Environment,
    sheets: Vec<Sheet>,
    /// Whether `style=""` attributes count, as they do in a browser.
    style_attributes: bool,
}

/// A declaration where the cascade put it.
#[derive(Clone)]
struct Declared<'s> {
    value: Specified<'s>,
    /// For a longhand set from a shorthand with `var()` in it: the
    /// shorthand, whose value is split once it's substituted.
    from_shorthand: Option<&'static str>,
    key: Key,
}

/// What orders declarations, CSS Cascade 5 §6: origin and importance,
/// then `style=""`, then layer, then specificity, then order.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Key {
    origin: u8,
    attached: bool,
    layer: u32,
    specificity: (u32, u32, u32),
    order: u32,
}

/// One style rule's selector, one complex selector of its list each, and
/// its declarations.
#[derive(Clone)]
struct Entry<'s> {
    selector: SelectorList,
    /// What an element must have for the selector to have a chance.
    bucket: Bucket,
    specificity: (u32, u32, u32),
    origin: Origin,
    layer: usize,
    order: u32,
    declarations: Vec<(&'s str, &'s [ComponentValue<'s>], bool)>,
}

/// Every element's computed values.
pub struct Styled {
    styles: HashMap<NodeId, Computed>,
}

impl Styled {
    /// `property`'s value on `el`, as `getComputedStyle()` writes it, or
    /// `None` for a node that isn't an element or a property that isn't.
    pub fn get(&self, el: NodeId, property: &str) -> Option<String> {
        self.styles.get(&el)?.get(property)
    }

    /// Everything about one element's style.
    pub fn computed(&self, el: NodeId) -> Option<&Computed> {
        self.styles.get(&el)
    }
}

impl Styler {
    /// A styler with the HTML user agent stylesheet, `style=""` counted.
    pub fn new(env: Environment) -> Styler {
        let mut s = Styler {
            env,
            sheets: Vec::new(),
            style_attributes: true,
        };
        s.add(HTML_UA_STYLESHEET, Origin::UserAgent);
        s
    }

    /// A styler with no stylesheet at all, not even the user agent's.
    pub fn bare(env: Environment) -> Styler {
        Styler {
            env,
            sheets: Vec::new(),
            style_attributes: true,
        }
    }

    pub fn environment(&self) -> &Environment {
        &self.env
    }

    /// Adds a stylesheet.
    pub fn add(&mut self, css: &str, origin: Origin) -> &mut Styler {
        self.sheets.push(Sheet {
            css: css.to_string(),
            origin,
            media: None,
        });
        self
    }

    /// Adds a stylesheet that applies only where `media` matches, as a
    /// `<style media>` or `<link media>` does.
    pub fn add_with_media(&mut self, css: &str, origin: Origin, media: &str) -> &mut Styler {
        self.sheets.push(Sheet {
            css: css.to_string(),
            origin,
            media: Some(MediaQueryList::parse_str(media)),
        });
        self
    }

    /// Adds the document's own `<style>` elements, with their `media`, as
    /// author stylesheets.
    pub fn with_document_styles(mut self, doc: &Document) -> Styler {
        let style = SelectorList::parse("style").unwrap();
        for el in style.select(doc, doc.root()) {
            let media = doc
                .attrs(el)
                .find(|(_, n, _)| *n == "media")
                .map(|(_, _, v)| v.to_string());
            let css = doc.text(el);
            match media {
                Some(m) => self.add_with_media(&css, Origin::Author, &m),
                None => self.add(&css, Origin::Author),
            };
        }
        self
    }

    /// Whether `style=""` attributes count; they do unless this says not.
    pub fn style_attributes(mut self, on: bool) -> Styler {
        self.style_attributes = on;
        self
    }

    /// Styles every element of `doc`.
    pub fn style(&self, doc: &Document) -> Styled {
        self.style_some(doc, None)
    }

    /// Styles one element, and the ancestors it inherits from, but no
    /// other: the cheap way to one element's computed values.
    pub fn style_element(&self, doc: &Document, el: NodeId) -> Option<Computed> {
        let mut chain = vec![el];
        let mut at = el;
        while let Some(p) = doc.parent(at) {
            if matches!(doc.kind(p), Kind::Element { .. }) {
                chain.push(p);
            }
            at = p;
        }
        chain.reverse();
        let mut styled = self.style_some(doc, Some(&chain));
        styled.styles.remove(&el)
    }

    fn style_some(&self, doc: &Document, only: Option<&[NodeId]>) -> Styled {
        let parsed: Vec<_> = self
            .sheets
            .iter()
            .filter(|s| !(s.origin == Origin::UserAgent && s.css == HTML_UA_STYLESHEET))
            .filter(|s| s.media.as_ref().is_none_or(|m| m.matches(&self.env)))
            .map(|s| (s, crate::parse_stylesheet_deep(&s.css)))
            .collect();
        let mut layers = Layers::default();
        let mut entries = Vec::new();
        let mut order = 0;
        // The HTML user agent stylesheet is the same every time: its rules
        // are collected once, for every styler.
        if self
            .sheets
            .iter()
            .any(|s| s.origin == Origin::UserAgent && s.css == HTML_UA_STYLESHEET)
        {
            let (ua, n) = ua_entries();
            entries.extend(ua.iter().cloned());
            order = *n;
        }
        for (sheet, parsed) in &parsed {
            let mut cx = Collect {
                env: &self.env,
                origin: sheet.origin,
                layers: &mut layers,
                entries: &mut entries,
                order: &mut order,
            };
            cx.rules(&parsed.rules, &[], &[]);
        }
        let ranks = layers.ranks();
        let mut index = Index::default();
        for (i, e) in entries.iter().enumerate() {
            match &e.bucket {
                Bucket::Id(x) => index.ids.entry(x.clone()).or_default().push(i),
                Bucket::Class(x) => index.classes.entry(x.clone()).or_default().push(i),
                Bucket::Tag(x) => index.tags.entry(x.clone()).or_default().push(i),
                Bucket::Attr(x) => index.attrs.entry(x.clone()).or_default().push(i),
                Bucket::Universal => index.universal.push(i),
            }
        }
        // Style attributes, parsed once each.
        let mut inline = HashMap::new();
        let attrs: Vec<(NodeId, String)> = if self.style_attributes {
            elements(doc)
                .filter_map(|el| {
                    doc.attrs(el)
                        .find(|(_, n, _)| *n == "style")
                        .map(|(_, _, v)| (el, v.to_string()))
                })
                .collect()
        } else {
            Vec::new()
        };
        let parsed_attrs: Vec<(NodeId, crate::BlockContents<'_>)> = attrs
            .iter()
            .map(|(el, css)| (*el, crate::parse_declaration_list(css)))
            .collect();
        for (el, block) in &parsed_attrs {
            inline.insert(*el, block);
        }
        let mut styles = HashMap::new();
        let root = doc_element(doc);
        // Parents before children: document order.
        let order: Vec<NodeId> = match only {
            Some(chain) => chain.to_vec(),
            None => elements(doc).collect(),
        };
        for el in order {
            let mut declared: HashMap<&'static str, Vec<Declared<'_>>> = HashMap::new();
            let mut custom: HashMap<String, Vec<(Key, Vec<ComponentValue<'_>>)>> = HashMap::new();
            // Only the rules whose rightmost id, class or type this element
            // has are tried.
            let mut candidates: Vec<usize> = index.universal.clone();
            if let Some(name) = doc.lower_name(el) {
                candidates.extend(index.tags.get(name).into_iter().flatten());
            }
            for (_, n, v) in doc.attrs(el) {
                candidates.extend(index.attrs.get(n).into_iter().flatten());
                if n == "id" {
                    candidates.extend(index.ids.get(v).into_iter().flatten());
                } else if n == "class" {
                    for c in v.split_ascii_whitespace() {
                        candidates.extend(index.classes.get(c).into_iter().flatten());
                    }
                }
            }
            candidates.sort_unstable();
            candidates.dedup();
            for &i in &candidates {
                let e = &entries[i];
                if !e.selector.matches(doc, el, None) {
                    continue;
                }
                let key = Key {
                    origin: e.origin as u8,
                    attached: false,
                    layer: ranks[e.layer],
                    specificity: e.specificity,
                    order: e.order,
                };
                for (name, value, important) in &e.declarations {
                    add(&mut declared, &mut custom, name, value, *important, key);
                }
            }
            if let Some(block) = inline.get(&el) {
                for item in &block.items {
                    if let Item::Declaration(d) = item {
                        let key = Key {
                            origin: Origin::Author as u8,
                            attached: true,
                            layer: ranks[0],
                            specificity: (0, 0, 0),
                            order: u32::MAX,
                        };
                        add(
                            &mut declared,
                            &mut custom,
                            &d.name,
                            &d.value,
                            d.important,
                            key,
                        );
                    }
                }
            }
            for list in declared.values_mut() {
                list.sort_by_key(|d| d.key);
            }
            for list in custom.values_mut() {
                list.sort_by_key(|d| d.0);
            }
            let parent = doc.parent(el).and_then(|p| styles.get(&p));
            let root_style = if Some(el) == root {
                None
            } else {
                root.and_then(|r| styles.get(&r))
            };
            let computed = compute::compute(compute::Input {
                declared: &declared,
                custom: &custom,
                parent,
                root: root_style,
                is_root: Some(el) == root,
                env: &self.env,
            });
            styles.insert(el, computed);
        }
        Styled { styles }
    }
}

/// Puts a declaration where the cascade sorts it: shorthands split, custom
/// properties apart.
fn add<'x>(
    declared: &mut HashMap<&'static str, Vec<Declared<'x>>>,
    custom: &mut HashMap<String, Vec<(Key, Vec<ComponentValue<'x>>)>>,
    name: &str,
    value: &'x [ComponentValue<'x>],
    important: bool,
    key: Key,
) {
    let mut key = key;
    key.origin = rank(key.origin, important);
    if important {
        key.layer = u32::MAX - key.layer;
    }
    if name.starts_with("--") {
        custom
            .entry(name.to_string())
            .or_default()
            .push((key, trim(value).to_vec()));
        return;
    }
    let Some(p) = property(name) else { return };
    let p = p.alias_of.and_then(property).unwrap_or(p);
    // `all`: a CSS-wide keyword for every property but `direction`,
    // `unicode-bidi` and the custom ones.
    if p.name == "all" {
        if let Ok(v @ Specified::CssWide(_)) = crate::values::parse_value("all", value) {
            for q in crate::values::properties() {
                if q.is_shorthand()
                    || q.alias_of.is_some()
                    || matches!(q.name, "all" | "direction" | "unicode-bidi")
                {
                    continue;
                }
                declared.entry(q.name).or_default().push(Declared {
                    value: v.clone(),
                    from_shorthand: None,
                    key,
                });
            }
        }
        return;
    }
    if !p.is_shorthand() {
        if let Ok(v) = crate::values::parse_value(p.name, value) {
            declared.entry(p.name).or_default().push(Declared {
                value: v,
                from_shorthand: None,
                key,
            });
        }
        return;
    }
    // The shorthand too, so its computed value can be written as it was
    // given when none of its longhands comes from anything later.
    if let Ok(v) = crate::values::parse_value(p.name, value) {
        declared.entry(p.name).or_default().push(Declared {
            value: v,
            from_shorthand: None,
            key,
        });
    }
    let Ok(longhands) = expand(p.name, value) else {
        return;
    };
    let pending = matches!(longhands.first(), Some((_, Specified::Unparsed(_))));
    for (l, v) in longhands {
        declared.entry(l).or_default().push(Declared {
            value: v,
            from_shorthand: pending.then_some(p.name),
            key,
        });
    }
}

/// The HTML user agent stylesheet's rules, collected once. Nothing in it
/// depends on the environment.
fn ua_entries() -> &'static (Vec<Entry<'static>>, u32) {
    static SHEET: std::sync::OnceLock<crate::Stylesheet<'static, Block<'static>>> =
        std::sync::OnceLock::new();
    static ENTRIES: std::sync::OnceLock<(Vec<Entry<'static>>, u32)> = std::sync::OnceLock::new();
    ENTRIES.get_or_init(|| {
        let sheet = SHEET.get_or_init(|| crate::parse_stylesheet_deep(HTML_UA_STYLESHEET));
        let env = Environment::default();
        let mut layers = Layers::default();
        let mut entries = Vec::new();
        let mut order = 0;
        let mut cx = Collect {
            env: &env,
            origin: Origin::UserAgent,
            layers: &mut layers,
            entries: &mut entries,
            order: &mut order,
        };
        cx.rules(&sheet.rules, &[], &[]);
        (entries, order)
    })
}

/// Origin and importance as one rank: normal user agent, user, author,
/// then important author, user, user agent.
fn rank(origin: u8, important: bool) -> u8 {
    if important { 5 - origin } else { origin }
}

fn trim<'v, 'a>(values: &'v [ComponentValue<'a>]) -> &'v [ComponentValue<'a>] {
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

fn elements(doc: &Document) -> impl Iterator<Item = NodeId> + '_ {
    // Every node in document order, elements only.
    let mut stack = vec![doc.root()];
    std::iter::from_fn(move || {
        while let Some(n) = stack.pop() {
            let children: Vec<NodeId> = doc.children(n).collect();
            stack.extend(children.into_iter().rev());
            if matches!(doc.kind(n), Kind::Element { .. }) {
                return Some(n);
            }
        }
        None
    })
}

fn doc_element(doc: &Document) -> Option<NodeId> {
    doc.children(doc.root())
        .find(|n| matches!(doc.kind(*n), Kind::Element { .. }))
}

#[derive(Clone, Debug, PartialEq)]
enum Bucket {
    Id(String),
    Class(String),
    Tag(String),
    /// `[name…]`: an attribute the element must have.
    Attr(String),
    Universal,
}

#[derive(Default)]
struct Index {
    ids: HashMap<String, Vec<usize>>,
    classes: HashMap<String, Vec<usize>>,
    tags: HashMap<String, Vec<usize>>,
    attrs: HashMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

/// The id, class or type a complex selector's rightmost compound needs,
/// read from its text; universal when that can't be told simply.
fn bucket(selector: &str) -> Bucket {
    if selector.contains('\\') {
        return Bucket::Universal;
    }
    // The rightmost compound: after the last combinator outside brackets.
    let b = selector.as_bytes();
    let (mut depth, mut start) = (0i32, 0);
    for (i, c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b' ' | b'>' | b'+' | b'~' if depth == 0 => start = i + 1,
            _ => {}
        }
    }
    let compound = &selector[start..];
    let name_end = |s: &str| {
        s.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(s.len())
    };
    // Only what's outside any brackets counts.
    let (mut depth, mut i) = (0i32, 0);
    let cb = compound.as_bytes();
    let (mut id, mut class, mut attr) = (None, None, None);
    while i < cb.len() {
        match cb[i] {
            b'[' if depth == 0 && attr.is_none() => {
                let rest = compound[i + 1..].trim_start();
                let n = name_end(rest);
                // `[ns|name]` and the like stay universal.
                if n > 0 && !rest[n..].starts_with('|') {
                    attr = Some(rest[..n].to_ascii_lowercase());
                }
                depth += 1;
            }
            b'(' | b'[' => depth += 1,
            b')' | b']' => depth -= 1,
            b'#' if depth == 0 && id.is_none() => {
                let n = name_end(&compound[i + 1..]);
                if n > 0 {
                    id = Some(compound[i + 1..i + 1 + n].to_string());
                }
            }
            b'.' if depth == 0 && class.is_none() => {
                let n = name_end(&compound[i + 1..]);
                if n > 0 {
                    class = Some(compound[i + 1..i + 1 + n].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    if let Some(id) = id {
        return Bucket::Id(id);
    }
    if let Some(class) = class {
        return Bucket::Class(class);
    }
    let n = name_end(compound);
    if n > 0 && compound.as_bytes()[0].is_ascii_alphabetic() && !compound[n..].starts_with('|') {
        return Bucket::Tag(compound[..n].to_ascii_lowercase());
    }
    attr.map_or(Bucket::Universal, Bucket::Attr)
}

/// Cascade layers, in the order they were first named. Each layer's
/// rank sorts its sublayers before its own declarations, and unlayered
/// declarations last of all (§6.4).
#[derive(Default)]
struct Layers {
    /// Every layer's full name, `a.b`, in the order first seen; index 0
    /// is the unlayered declarations.
    names: Vec<Vec<String>>,
}

impl Layers {
    fn id(&mut self, path: &[String]) -> usize {
        if self.names.is_empty() {
            self.names.push(Vec::new());
        }
        // Naming `a.b` names `a` first.
        for n in 1..path.len() {
            if !self.names.iter().any(|p| p == &path[..n]) {
                self.names.push(path[..n].to_vec());
            }
        }
        match self.names.iter().position(|p| p == path) {
            Some(i) => i,
            None => {
                self.names.push(path.to_vec());
                self.names.len() - 1
            }
        }
    }

    /// Each layer's rank: higher wins for normal declarations.
    fn ranks(&mut self) -> Vec<u32> {
        if self.names.is_empty() {
            self.names.push(Vec::new());
        }
        // Order of first mention among siblings, depth first, a layer
        // after its sublayers; unlayered (the empty path) at the end.
        let mut order: Vec<usize> = (0..self.names.len()).collect();
        let first = |path: &[String], names: &[Vec<String>]| -> Vec<usize> {
            (1..=path.len())
                .map(|n| names.iter().position(|p| p[..] == path[..n]).unwrap())
                .collect()
        };
        let keys: Vec<Vec<usize>> = self
            .names
            .iter()
            .map(|p| {
                let mut k = first(p, &self.names);
                // A layer's own declarations come after its sublayers.
                k.push(usize::MAX);
                k
            })
            .collect();
        order.sort_by(|a, b| {
            let (ka, kb) = (&keys[*a], &keys[*b]);
            ka.cmp(kb)
        });
        let mut ranks = vec![0; self.names.len()];
        for (r, i) in order.iter().enumerate() {
            ranks[*i] = r as u32;
        }
        // The unlayered sort last: its key is just [MAX].
        ranks
    }
}

struct Collect<'c, 's> {
    env: &'c Environment,
    origin: Origin,
    layers: &'c mut Layers,
    entries: &'c mut Vec<Entry<'s>>,
    order: &'c mut u32,
}

/// A prelude's complex selectors, each as text.
fn complex_selectors(prelude: &[ComponentValue<'_>]) -> Vec<String> {
    prelude
        .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
        .map(|p| crate::to_css(p).trim().to_string())
        .collect()
}

/// A nested selector against its parent's: `&` replaced by `:is(parent)`,
/// or the parent put in front, as CSS Nesting reads them.
fn nest(selector: &str, parents: &[String]) -> String {
    if parents.is_empty() {
        return selector.to_string();
    }
    let parent = format!(":is({})", parents.join(", "));
    if selector.contains('&') {
        return selector.replace('&', &parent);
    }
    format!("{parent} {selector}")
}

impl<'s> Collect<'_, 's> {
    fn rules(&mut self, rules: &'s [Rule<'s, Block<'s>>], parents: &[String], layer: &[String]) {
        for r in rules {
            self.rule(r, parents, layer);
        }
    }

    fn items(&mut self, items: &'s [Item<'s, Block<'s>>], parents: &[String], layer: &[String]) {
        // Declarations straight in a nested group rule belong to the
        // parent's selector.
        let declarations: Vec<_> = items
            .iter()
            .filter_map(|i| match i {
                Item::Declaration(d) => Some((&*d.name, &d.value[..], d.important)),
                _ => None,
            })
            .collect();
        if !declarations.is_empty() && !parents.is_empty() {
            self.push(parents, declarations, layer);
        }
        for i in items {
            if let Item::Rule(r) = i {
                self.rule(r, parents, layer);
            }
        }
    }

    fn push(
        &mut self,
        selectors: &[String],
        declarations: Vec<(&'s str, &'s [ComponentValue<'s>], bool)>,
        layer: &[String],
    ) {
        let layer = self.layers.id(layer);
        for s in selectors {
            let Ok(list) = SelectorList::parse(s) else {
                continue;
            };
            *self.order += 1;
            self.entries.push(Entry {
                bucket: bucket(s),
                specificity: list.specificity(),
                selector: list,
                origin: self.origin,
                layer,
                order: *self.order,
                declarations: declarations.clone(),
            });
        }
    }

    fn rule(&mut self, r: &'s Rule<'s, Block<'s>>, parents: &[String], layer: &[String]) {
        match r {
            Rule::Qualified(q) => {
                let selectors: Vec<String> = complex_selectors(&q.prelude)
                    .iter()
                    .map(|s| nest(s, parents))
                    .collect();
                // A selector list with one bad selector is no rule at all.
                if selectors.iter().any(|s| SelectorList::parse(s).is_err()) {
                    return;
                }
                let declarations = q
                    .block
                    .items
                    .iter()
                    .filter_map(|i| match i {
                        Item::Declaration(d) => Some((&*d.name, &d.value[..], d.important)),
                        _ => None,
                    })
                    .collect();
                self.push(&selectors, declarations, layer);
                for i in &q.block.items {
                    if let Item::Rule(r) = i {
                        self.rule(r, &selectors, layer);
                    }
                }
            }
            Rule::At(a) => {
                let name = a.name.to_ascii_lowercase();
                match name.as_str() {
                    "media" => {
                        if MediaQueryList::parse(&a.prelude).matches(self.env)
                            && let Some(b) = &a.block
                        {
                            self.items(&b.items, parents, layer);
                        }
                    }
                    "supports" => {
                        if crate::supports::supports(&a.prelude) == Some(true)
                            && let Some(b) = &a.block
                        {
                            self.items(&b.items, parents, layer);
                        }
                    }
                    "layer" => {
                        let names: Vec<Vec<String>> = a
                            .prelude
                            .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
                            .map(|n| {
                                crate::to_css(n)
                                    .trim()
                                    .split('.')
                                    .map(|p| p.trim().to_string())
                                    .filter(|p| !p.is_empty())
                                    .collect()
                            })
                            .collect();
                        match &a.block {
                            // `@layer a, b;` names them, in that order.
                            None => {
                                for n in &names {
                                    let mut path = layer.to_vec();
                                    path.extend(n.iter().cloned());
                                    self.layers.id(&path);
                                }
                            }
                            Some(b) => {
                                let mut path = layer.to_vec();
                                match names.first() {
                                    Some(n) if !n.is_empty() && names.len() == 1 => {
                                        path.extend(n.iter().cloned())
                                    }
                                    Some(n) if !n.is_empty() => return,
                                    // An anonymous layer, its own each time.
                                    _ => path.push(format!("\u{0}{}", *self.order)),
                                }
                                *self.order += 1;
                                self.layers.id(&path);
                                self.items(&b.items, parents, &path);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

/// The CSS-wide keyword a value is, if it is one.
pub(crate) fn css_wide(v: &Specified<'_>) -> Option<CssWide> {
    match v {
        Specified::CssWide(k) => Some(*k),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style(html: &str, sel: &str, property: &str) -> String {
        let doc = parsec::parse(html, false);
        let styled = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .style(&doc);
        let el = SelectorList::parse(sel)
            .unwrap()
            .select_first(&doc, doc.root())
            .unwrap();
        styled.get(el, property).unwrap_or_default()
    }

    #[test]
    fn origins_importance_and_style_attributes() {
        let html = "<style>p { color: red !important } #a { color: blue }</style><p id=a style='color: green'>";
        assert_eq!(style(html, "p", "color"), "rgb(255, 0, 0)");
        let html = "<style>#a { color: blue }</style><p id=a style='color: green'>";
        assert_eq!(style(html, "p", "color"), "rgb(0, 128, 0)");
        let html = "<style>p { color: red !important }</style><p style='color: green !important'>";
        assert_eq!(style(html, "p", "color"), "rgb(0, 128, 0)");
        // The user agent's `display: block` loses to the author's.
        assert_eq!(
            style(
                "<style>div { display: inline }</style><div>",
                "div",
                "display"
            ),
            "inline"
        );
    }

    #[test]
    fn layers_order_and_importance() {
        let css = "@layer a, b; @layer b { p { color: green } } @layer a { p { color: red } }";
        assert_eq!(
            style(&format!("<style>{css}</style><p>"), "p", "color"),
            "rgb(0, 128, 0)"
        );
        // Unlayered beats layered, unless important, where it's the other way.
        let css = "p { color: green } @layer x { p { color: red } }";
        assert_eq!(
            style(&format!("<style>{css}</style><p>"), "p", "color"),
            "rgb(0, 128, 0)"
        );
        let css = "p { color: red !important } @layer x { p { color: green !important } }";
        assert_eq!(
            style(&format!("<style>{css}</style><p>"), "p", "color"),
            "rgb(0, 128, 0)"
        );
        // A layer's sublayers come before its own rules.
        let css = "@layer a { p { color: green } @layer b { p { color: red } } }";
        assert_eq!(
            style(&format!("<style>{css}</style><p>"), "p", "color"),
            "rgb(0, 128, 0)"
        );
    }

    #[test]
    fn revert_and_revert_layer() {
        let html = "<style>div { display: inline } div { display: revert }</style><div>";
        assert_eq!(style(html, "div", "display"), "block");
        let css = "@layer a { p { color: green } } @layer b { p { color: revert-layer } }";
        assert_eq!(
            style(&format!("<style>{css}</style><p>"), "p", "color"),
            "rgb(0, 128, 0)"
        );
        let html =
            "<style>.r { all: revert }</style><style>div { display: inline }</style><div class=r>";
        assert_eq!(style(html, "div", "display"), "block");
    }

    #[test]
    fn inheritance_and_keywords() {
        let html = "<div style='color: blue; margin-top: 3px; font-size: 20px'><p style='width: inherit'></div>";
        assert_eq!(style(html, "p", "color"), "rgb(0, 0, 255)");
        assert_eq!(style(html, "p", "margin-top"), "0px");
        let html = "<div style='font-size: 20px'><p style='font-size: 1.5em; line-height: 2em; margin-left: calc(1em + 2px)'></div>";
        assert_eq!(style(html, "p", "font-size"), "30px");
        assert_eq!(style(html, "p", "line-height"), "60px");
        assert_eq!(style(html, "p", "margin-left"), "32px");
        let html = "<div style='color: red'><p style='color: unset; border-top: 2px solid; border-bottom-width: 9px'></div>";
        assert_eq!(style(html, "p", "color"), "rgb(255, 0, 0)");
        assert_eq!(style(html, "p", "border-top-color"), "rgb(255, 0, 0)");
        assert_eq!(style(html, "p", "border-top-width"), "2px");
        // No style, no width.
        assert_eq!(style(html, "p", "border-bottom-width"), "0px");
        assert_eq!(style("<b><i>x", "b", "font-weight"), "700");
    }

    #[test]
    fn custom_properties_and_var() {
        let html = "<style>:root { --gap: 4px; --c: var(--d) } p { margin: calc(var(--gap) * 2) var(--nope, 1px); color: var(--c, green) }</style><p>";
        assert_eq!(style(html, "p", "margin-top"), "8px");
        assert_eq!(style(html, "p", "margin-right"), "1px");
        // --c refers to nothing, so it's guaranteed-invalid, and the
        // var() that refers to it takes its fallback.
        assert_eq!(style(html, "p", "color"), "rgb(0, 128, 0)");
        let html = "<style>p { color: red; color: var(--missing) }</style><div style='color: blue'><p></div>";
        // Invalid at computed-value time is `unset`: inherited here.
        assert_eq!(style(html, "p", "color"), "rgb(0, 0, 255)");
        assert_eq!(style(html, "p", "--missing"), "");
    }

    #[test]
    fn media_supports_and_nesting() {
        let css = "@media (min-width: 2000px) { p { color: red } } @supports (display: grid) { p { margin-top: 1px } } \
                   @supports (display: nonsense) { p { margin-top: 9px } } div { & > p { color: green } }";
        let html = format!("<style>{css}</style><div><p></div>");
        assert_eq!(style(&html, "p", "color"), "rgb(0, 128, 0)");
        assert_eq!(style(&html, "p", "margin-top"), "1px");
        let html = "<style media='print'>p { color: red }</style><p>";
        assert_eq!(style(html, "p", "color"), "rgb(0, 0, 0)");
    }

    #[test]
    fn buckets() {
        assert_eq!(bucket("div > p.x"), Bucket::Class("x".into()));
        assert_eq!(bucket("#a:hover"), Bucket::Id("a".into()));
        assert_eq!(bucket("a[href]"), Bucket::Tag("a".into()));
        assert_eq!(bucket("[data-x]"), Bucket::Attr("data-x".into()));
        assert_eq!(bucket(":is(.a, .b)"), Bucket::Universal);
        assert_eq!(bucket("svg|a"), Bucket::Universal);
    }
}
