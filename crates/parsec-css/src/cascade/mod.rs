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
//! `@scope` applies with scoping proximity in the cascade order;
//! `@container`, and the `cq*` units, against the sizes
//! [`Styler::container_sizes`] gives;
//! `@import` through the fetch [`Styler::fetch`] gives, with `layer()`,
//! `supports()` and media; `@property` registers custom properties, which
//! then take their syntax, initial value and inheritance from it; and
//! [`Styler::at_rules`] gives the `@keyframes`, `@counter-style` and
//! `@property` rules that win.
//!
//! Pseudo-elements are styled too, CSS Pseudo-Elements 4: a rule whose
//! selector ends in `::before`, `::after`, `::marker`, `::placeholder`,
//! `::first-line`, `::first-letter`, `::selection`, `::backdrop`,
//! `::file-selector-button` or `::highlight()` applies to that
//! pseudo-element of the elements the rest of it matches, and
//! [`Styled::get_pseudo`] gives its computed values, inherited from the
//! originating element, with only the properties that apply to it
//! counted, and `content` worked out. They're computed when asked for,
//! not before.
//!
//! What it doesn't do is layout: a value that needs boxes to work out,
//! `width: 50%` as pixels say, stays as its computed value, and a
//! container's size is the caller's to give.

mod compute;
mod pseudo;

use std::collections::HashMap;
use std::sync::Arc;

use parsec::css::SelectorList;
use parsec::{Document, Kind, NodeId};

use crate::media::{Environment, MediaQueryList};
use crate::parser::{Block, ComponentValue, Item, Rule};
use crate::rules::{
    ContainerQuery, CounterStyle, Import, Keyframes, PropertyRule, ScopeBounds, Syntax,
};
use crate::tokenizer::Token;
use crate::values::{CssWide, Specified, expand, property};

pub use compute::Computed;
pub use pseudo::Pseudo;
use pseudo::{PseudoRule, PseudoStyles, Split, split_pseudo};

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
    /// The `<style>` it came from, whose parent an `@scope` with no root
    /// is scoped to.
    owner: Option<NodeId>,
}

/// What an `@import`'s URL gives: the stylesheet's text, or `None`.
pub type Fetch = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A container's size, width and height in CSS pixels, from the element
/// and its computed values; `None` for one whose size isn't known.
pub type ContainerSize = Arc<dyn Fn(NodeId, &Computed) -> Option<(f64, f64)> + Send + Sync>;

/// Collects stylesheets and styles documents with them.
#[derive(Default)]
pub struct Styler {
    env: Environment,
    sheets: Vec<Sheet>,
    /// Whether `style=""` attributes count, as they do in a browser.
    style_attributes: bool,
    fetch: Option<Fetch>,
    container_size: Option<ContainerSize>,
}

/// How deep `@import`s may nest, and how many one styling may follow:
/// past either, the rest aren't fetched.
const MAX_IMPORT_DEPTH: usize = 16;
const MAX_IMPORTS: usize = 256;

/// The `@keyframes`, `@counter-style` and `@property` rules that win, by
/// name: the last of a name in the cascade's order, layers counted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AtRules {
    pub keyframes: Vec<Keyframes>,
    pub counter_styles: Vec<CounterStyle>,
    pub properties: Vec<PropertyRule>,
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
    /// Scoping proximity, CSS Cascade 6 §6.6: nearer roots win, and an
    /// unscoped rule is the farthest of all.
    proximity: u32,
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
    scope: Option<Arc<Scope>>,
    /// The `@container` rules it's in, each a list of queries any of
    /// which may hold.
    containers: Vec<Arc<Vec<ContainerQuery>>>,
    /// The pseudo-element a rule styles, whose originating element
    /// `selector` matches.
    pseudo: Option<Pseudo>,
    /// Whether the selector is `*`, which every element matches.
    any: bool,
}

/// An `@scope` a rule is in.
struct Scope {
    start: ScopeStart,
    end: Option<SelectorList>,
    /// The `@scope` this one is in, which its roots must be in scope of.
    outer: Option<Arc<Scope>>,
}

enum ScopeStart {
    Selector(SelectorList),
    /// No `<scope-start>`: the parent of the `<style>` it's in, or the
    /// document for a sheet with none.
    Owner(Option<NodeId>),
}

/// Every element's computed values.
pub struct Styled {
    styles: HashMap<NodeId, Computed>,
    pseudo: PseudoStyles,
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
            style_attributes: true,
            ..Styler::default()
        };
        s.add(HTML_UA_STYLESHEET, Origin::UserAgent);
        s
    }

    /// A styler with no stylesheet at all, not even the user agent's.
    pub fn bare(env: Environment) -> Styler {
        Styler {
            env,
            style_attributes: true,
            ..Styler::default()
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
            owner: None,
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
            owner: None,
        });
        self
    }

    /// Fetches `@import`ed stylesheets with `fetch`, given each URL as
    /// written; without it, `@import` is ignored.
    pub fn fetch(
        mut self,
        fetch: impl Fn(&str) -> Option<String> + Send + Sync + 'static,
    ) -> Styler {
        self.fetch = Some(Arc::new(fetch));
        self
    }

    /// Gives each container's size, for `@container` and the `cq*` units:
    /// `size` is asked for an element whose `container-type` makes it a
    /// container, with its computed values, and gives its width and height
    /// in CSS pixels. A query on a container with no size given is false,
    /// and `cq*` units look past it to the next container out, or the
    /// viewport.
    pub fn container_sizes(
        mut self,
        size: impl Fn(NodeId, &Computed) -> Option<(f64, f64)> + Send + Sync + 'static,
    ) -> Styler {
        self.container_size = Some(Arc::new(size));
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
            if let Some(last) = self.sheets.last_mut() {
                last.owner = Some(el);
            }
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

    /// Each sheet that applies, with what it `@import`s before it, in the
    /// cascade's order.
    fn units(&self) -> Vec<Unit> {
        let mut out = Vec::new();
        let mut count = 0;
        for s in &self.sheets {
            if s.origin == Origin::UserAgent && s.css == HTML_UA_STYLESHEET {
                continue;
            }
            if !s.media.as_ref().is_none_or(|m| m.matches(&self.env)) {
                continue;
            }
            self.expand(
                s.css.clone(),
                (s.origin, s.owner),
                (Vec::new(), Vec::new()),
                0,
                &mut count,
                &mut out,
            );
        }
        out
    }

    /// A sheet, after the sheets its `@import`s bring in, each in the
    /// layer the import puts it.
    #[allow(clippy::too_many_arguments)]
    fn expand(
        &self,
        css: String,
        (origin, owner): (Origin, Option<NodeId>),
        (layer, scopes): (Vec<String>, Vec<ScopeBounds>),
        depth: usize,
        count: &mut usize,
        out: &mut Vec<Unit>,
    ) {
        if let Some(fetch) = &self.fetch {
            let sheet = crate::parse_stylesheet(&css);
            // `@layer` statements before the imports name their layers
            // first.
            let mut declared = Vec::new();
            for rule in &sheet.rules {
                let Rule::At(a) = rule else { break };
                match a.name.to_ascii_lowercase().as_str() {
                    "charset" => {}
                    "layer" if a.block.is_none() => {
                        for n in layer_names(&a.prelude) {
                            let mut path = layer.clone();
                            path.extend(n);
                            declared.push(path);
                        }
                    }
                    "import" => {
                        let Some(import) = Import::parse(a) else {
                            continue;
                        };
                        if !import.applies(&self.env)
                            || depth >= MAX_IMPORT_DEPTH
                            || *count >= MAX_IMPORTS
                        {
                            continue;
                        }
                        *count += 1;
                        let mut path = layer.clone();
                        match &import.layer {
                            Some(Some(n)) => path.extend(n.split('.').map(String::from)),
                            // Each anonymous layer its own.
                            Some(None) => path.push(format!("\u{0}import{count}")),
                            None => {}
                        }
                        // The layer is named whether the sheet loads or not.
                        if import.layer.is_some() {
                            declared.push(path.clone());
                        }
                        out.push(Unit::layers(std::mem::take(&mut declared)));
                        let Some(text) = fetch(&import.url) else {
                            continue;
                        };
                        let mut inner = scopes.clone();
                        inner.extend(import.scope.clone());
                        self.expand(text, (origin, owner), (path, inner), depth + 1, count, out);
                    }
                    _ => break,
                }
            }
        }
        out.push(Unit {
            css,
            origin,
            owner,
            layer,
            declare: Vec::new(),
            scopes,
        });
    }

    /// The rules of every sheet that applies, collected.
    fn collect<'u>(
        &self,
        parsed: &'u [(&'u Unit, crate::Stylesheet<'u, Block<'u>>)],
        layers: &mut Layers,
        entries: &mut Vec<Entry<'u>>,
        at: &mut Collected,
    ) {
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
        for (unit, parsed) in parsed {
            for path in &unit.declare {
                layers.id(path);
            }
            let mut cx = Collect {
                env: &self.env,
                origin: unit.origin,
                layers,
                entries,
                order: &mut order,
                owner: unit.owner,
                scope: None,
                containers: Vec::new(),
                at,
            };
            // An `@import … scope()`'s sheet is as if in that `@scope`.
            if unit.scopes.iter().all(|b| cx.enter_scope(b, &[])) {
                cx.rules(&parsed.rules, &[], &unit.layer);
            }
        }
    }

    /// The `@keyframes`, `@counter-style` and `@property` rules in force:
    /// for each name, the one the cascade's order puts last.
    pub fn at_rules(&self) -> AtRules {
        let units = self.units();
        let parsed: Vec<_> = units
            .iter()
            .map(|u| (u, crate::parse_stylesheet_deep(&u.css)))
            .collect();
        let mut layers = Layers::default();
        let mut entries = Vec::new();
        let mut at = Collected::default();
        self.collect(&parsed, &mut layers, &mut entries, &mut at);
        let ranks = layers.ranks();
        at.resolved(&ranks)
    }

    fn style_some(&self, doc: &Document, only: Option<&[NodeId]>) -> Styled {
        let units = self.units();
        let parsed: Vec<_> = units
            .iter()
            .map(|u| (u, crate::parse_stylesheet_deep(&u.css)))
            .collect();
        let mut layers = Layers::default();
        let mut entries = Vec::new();
        let mut at = Collected::default();
        self.collect(&parsed, &mut layers, &mut entries, &mut at);
        let ranks = layers.ranks();
        let registered: HashMap<String, (PropertyRule, Syntax)> = at
            .resolved(&ranks)
            .properties
            .into_iter()
            .filter(PropertyRule::registers)
            .filter_map(|p| {
                let syntax = Syntax::parse(&p.syntax)?;
                Some((p.name.clone(), (p, syntax)))
            })
            .collect();
        let mut scope_cache = ScopeCache::default();
        let mut index = Index::default();
        let mut pseudo = PseudoStyles {
            env: self.env.clone(),
            root: doc_element(doc),
            ..PseudoStyles::default()
        };
        // Each pseudo-element rule's place in `pseudo.rules`, where it's
        // copied the first time an element matches it.
        let mut rule_of: HashMap<usize, usize> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            // On every element alike: kept once, not per element.
            if e.pseudo.is_some() && e.any && e.scope.is_none() && e.containers.is_empty() {
                let rule = PseudoRule::of(e);
                if !rule.needs_attrs {
                    pseudo.rules.push(rule);
                    let key = self.key(e, &ranks, 0);
                    pseudo.always.push((pseudo.rules.len() - 1, key));
                    continue;
                }
            }
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
            let mut pseudo_candidates = Vec::new();
            for &i in &candidates {
                let e = &entries[i];
                if e.pseudo.is_some() {
                    // Once this element's style is known: a pseudo-element's
                    // container may be its originating element.
                    pseudo_candidates.push(i);
                    continue;
                }
                let Some(proximity) =
                    self.entry_matches(e, doc, el, &styles, &mut scope_cache, false)
                else {
                    continue;
                };
                let key = self.key(e, &ranks, proximity);
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
                            proximity: 0,
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
            let parent_id = doc.parent(el);
            let parent = parent_id.and_then(|p| styles.get(&p));
            let parent_id = parent_id.filter(|_| parent.is_some());
            let root_style = if Some(el) == root {
                None
            } else {
                root.and_then(|r| styles.get(&r))
            };
            let mut computed = compute::compute(compute::Input {
                declared: &declared,
                custom: &custom,
                parent,
                root: root_style,
                is_root: Some(el) == root,
                env: &self.env,
                cq: parent.map_or((self.env.width, self.env.height), |p| p.cq_children),
                registered: &registered,
            });
            // A size container is what its descendants' `cq*` units are
            // relative to, on the axes it contains; one with no size given
            // passes its own on.
            let (w, h) = computed.size_axes();
            if (w || h)
                && let Some((width, height)) =
                    self.container_size.as_ref().and_then(|f| f(el, &computed))
            {
                if w {
                    computed.cq_children.0 = width;
                }
                if h {
                    computed.cq_children.1 = height;
                }
            }
            computed.parent = parent_id;
            styles.insert(el, computed);
            let mut matched = Vec::new();
            for i in pseudo_candidates {
                let e = &entries[i];
                if let Some(proximity) =
                    self.entry_matches(e, doc, el, &styles, &mut scope_cache, true)
                {
                    let r = *rule_of.entry(i).or_insert_with(|| {
                        pseudo.rules.push(PseudoRule::of(e));
                        pseudo.rules.len() - 1
                    });
                    if pseudo.rules[r].needs_attrs && !pseudo.attrs.contains_key(&el) {
                        let attrs = doc
                            .attrs(el)
                            .map(|(_, n, v)| (n.to_string(), v.to_string()))
                            .collect();
                        pseudo.attrs.insert(el, attrs);
                    }
                    matched.push((r, self.key(e, &ranks, proximity)));
                }
            }
            if !matched.is_empty() {
                pseudo.matched.insert(el, matched);
            }
        }
        pseudo.registered = registered;
        Styled { styles, pseudo }
    }

    /// Where a rule's declarations go in the cascade's order.
    #[inline]
    fn key(&self, e: &Entry<'_>, ranks: &[u32], proximity: u32) -> Key {
        Key {
            origin: e.origin as u8,
            attached: false,
            layer: ranks[e.layer],
            specificity: e.specificity,
            proximity,
            order: e.order,
        }
    }

    /// Whether a rule applies to `el`, with its scoping proximity if so.
    /// `inclusive` is for a pseudo-element's rule, whose containers start
    /// at `el`, the originating element, which must be styled already.
    #[inline]
    fn entry_matches(
        &self,
        e: &Entry<'_>,
        doc: &Document,
        el: NodeId,
        styles: &HashMap<NodeId, Computed>,
        scope_cache: &mut ScopeCache,
        inclusive: bool,
    ) -> Option<u32> {
        if !e
            .containers
            .iter()
            .all(|c| self.container_holds(c, doc, el, styles, inclusive))
        {
            return None;
        }
        match &e.scope {
            None => e.selector.matches(doc, el, None).then_some(0),
            Some(scope) => {
                let levels = scope_roots(scope, doc, el, scope_cache);
                let roots = levels.last().map_or(&[][..], |r| &r[..]);
                roots
                    .iter()
                    .find(|(r, _)| e.selector.matches(doc, el, Some(*r)))
                    .map(|(_, hops)| u32::MAX - hops)
            }
        }
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
        let mut at = Collected::default();
        let mut cx = Collect {
            env: &env,
            origin: Origin::UserAgent,
            layers: &mut layers,
            entries: &mut entries,
            order: &mut order,
            owner: None,
            scope: None,
            containers: Vec::new(),
            at: &mut at,
        };
        cx.rules(&sheet.rules, &[], &[]);
        (entries, order)
    })
}

impl Styler {
    /// Whether one of an `@container`'s queries holds for `el`: its
    /// nearest container of the name with the axes the query needs, of
    /// the size [`Styler::container_sizes`] gives.
    fn container_holds(
        &self,
        queries: &[ContainerQuery],
        doc: &Document,
        el: NodeId,
        styles: &HashMap<NodeId, Computed>,
        inclusive: bool,
    ) -> bool {
        queries.iter().any(|q| {
            let axes = q.axes();
            let sized = axes.inline || axes.block;
            let mut at = if inclusive { Some(el) } else { doc.parent(el) };
            while let Some(a) = at {
                at = doc.parent(a);
                let Some(c) = styles.get(&a) else { continue };
                let ty = c.get("container-type").unwrap_or_default();
                let size = ty.split_ascii_whitespace().any(|w| w == "size");
                let inline = size || ty.split_ascii_whitespace().any(|w| w == "inline-size");
                // Every element is a style container; a size container
                // needs the type for the axes asked of it, and a box.
                let boxed = !matches!(c.get("display").as_deref(), Some("none" | "contents"));
                let eligible = if axes.block {
                    size && boxed
                } else if axes.inline {
                    inline && boxed
                } else {
                    true
                };
                let named = q.name.as_ref().is_none_or(|n| {
                    c.get("container-name")
                        .is_some_and(|names| names.split_ascii_whitespace().any(|x| x == n))
                });
                if !eligible || !named {
                    continue;
                }
                let size = match sized {
                    true => match self.container_size.as_ref().and_then(|f| f(a, c)) {
                        Some(s) => Some(s),
                        None => return false,
                    },
                    false => None,
                };
                let get = |name: &str| c.get(name);
                let number = |value: &str| c.range_value(value);
                return q.evaluate(size, &c.query_lengths(), &get, &number) == Some(true);
            }
            false
        })
    }
}

/// Each nesting level's scoping roots, outermost first, each root with
/// how many generations up it is.
type Levels = Vec<Vec<(NodeId, u32)>>;

/// Each element's scoping roots for each `@scope`, worked out once: an
/// element's are its parent's, a generation farther, less those it limits,
/// and itself if it's a root.
#[derive(Default)]
struct ScopeCache {
    /// By the scope's address and the node: the roots at each level of
    /// nesting, outermost first.
    roots: HashMap<(usize, NodeId), Arc<Levels>>,
}

/// The scoping roots `el` is in scope of for `scope`, nearest first, each
/// with how many generations up it is.
fn scope_roots(scope: &Scope, doc: &Document, el: NodeId, cache: &mut ScopeCache) -> Arc<Levels> {
    let key = |n: NodeId| (scope as *const Scope as usize, n);
    // Up to the nearest element already done, then down from there, so
    // nothing recurses on the document's depth.
    let mut chain = vec![el];
    let mut start = None;
    let mut at = el;
    if let Some(r) = cache.roots.get(&key(el)) {
        return r.clone();
    }
    while let Some(p) = doc.parent(at) {
        if let Some(r) = cache.roots.get(&key(p)) {
            start = Some(r.clone());
            break;
        }
        chain.push(p);
        at = p;
    }
    let mut levels: Vec<&Scope> = Vec::new();
    let mut s = Some(scope);
    while let Some(x) = s {
        levels.push(x);
        s = x.outer.as_deref();
    }
    levels.reverse();
    let owner_root = |o: &Option<NodeId>| o.and_then(|o| doc.parent(o)).unwrap_or(doc.root());
    let mut roots: Levels = match start {
        Some(r) => (*r).clone(),
        None => vec![Vec::new(); levels.len()],
    };
    for &n in chain.iter().rev() {
        let element = matches!(doc.kind(n), Kind::Element { .. });
        for i in 0..levels.len() {
            let sc = levels[i];
            for r in &mut roots[i] {
                r.1 += 1;
            }
            // A limit, and what's under it, is out of its root's scope.
            if let Some(end) = &sc.end
                && element
            {
                roots[i].retain(|(r, _)| !end.matches(doc, n, Some(*r)));
            }
            let is_root = match &sc.start {
                ScopeStart::Owner(o) => n == owner_root(o),
                ScopeStart::Selector(sel) => {
                    element
                        && if i == 0 {
                            sel.matches(doc, n, None)
                        } else {
                            roots[i - 1]
                                .iter()
                                .any(|(r, _)| sel.matches(doc, n, Some(*r)))
                        }
                }
            };
            // A root that's its own limit, `to (:scope)`, scopes nothing.
            let own_limit = is_root
                && sc
                    .end
                    .as_ref()
                    .is_some_and(|end| end.matches(doc, n, Some(n)));
            if is_root && !own_limit {
                roots[i].insert(0, (n, 0));
            }
        }
        cache.roots.insert(key(n), Arc::new(roots.clone()));
    }
    cache.roots[&key(el)].clone()
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
    owner: Option<NodeId>,
    scope: Option<Arc<Scope>>,
    containers: Vec<Arc<Vec<ContainerQuery>>>,
    at: &'c mut Collected,
}

/// One stylesheet to collect: its text, where it's from, and the layer
/// an `@import` put it in; or, with no text, layers to name first.
struct Unit {
    css: String,
    origin: Origin,
    owner: Option<NodeId>,
    layer: Vec<String>,
    declare: Vec<Vec<String>>,
    /// The `scope()`s `@import`s put its rules in, outermost first.
    scopes: Vec<ScopeBounds>,
}

impl Unit {
    fn layers(declare: Vec<Vec<String>>) -> Unit {
        Unit {
            css: String::new(),
            origin: Origin::Author,
            owner: None,
            layer: Vec::new(),
            declare,
            scopes: Vec::new(),
        }
    }
}

/// `@layer a.b, c`'s names, each as its parts.
fn layer_names(prelude: &[ComponentValue<'_>]) -> Vec<Vec<String>> {
    prelude
        .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
        .map(|n| {
            crate::to_css(n)
                .trim()
                .split('.')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .collect()
}

/// The named at-rules as collected, each with its layer and order.
#[derive(Default)]
struct Collected {
    keyframes: Vec<(usize, u32, Keyframes)>,
    counter_styles: Vec<(usize, u32, CounterStyle)>,
    properties: Vec<(usize, u32, PropertyRule)>,
}

impl Collected {
    /// For each name, the rule that's last in the cascade's order.
    fn resolved(&self, ranks: &[u32]) -> AtRules {
        fn last<T: Clone>(
            items: &[(usize, u32, T)],
            ranks: &[u32],
            name: impl Fn(&T) -> &str,
        ) -> Vec<T> {
            let mut sorted: Vec<&(usize, u32, T)> = items.iter().collect();
            sorted.sort_by_key(|(l, o, _)| (ranks.get(*l).copied().unwrap_or(0), *o));
            let mut out: Vec<T> = Vec::new();
            for (_, _, t) in sorted {
                out.retain(|x| name(x) != name(t));
                out.push(t.clone());
            }
            out
        }
        AtRules {
            keyframes: last(&self.keyframes, ranks, |k| &k.name),
            counter_styles: last(&self.counter_styles, ranks, |c| &c.name),
            properties: last(&self.properties, ranks, |p| &p.name),
        }
    }
}

/// The parent "selector" of the rules straight inside an `@scope`.
const IN_SCOPE: &str = "\u{1}scope";

/// How deep `@scope`s may nest; deeper ones are ignored, since each level
/// multiplies the work of finding roots.
const MAX_SCOPE_DEPTH: usize = 8;

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
    // In `@scope`, CSS Cascade 6 §3.5.1: `&` is `:where(:scope)`, and a
    // selector without `:scope` or `&` is under the root.
    if parents == [IN_SCOPE] {
        let scope = ":where(:scope)";
        if selector.contains('&') {
            return selector.replace('&', scope);
        }
        if selector.contains(":scope") {
            return selector.to_string();
        }
        return format!("{scope} {selector}");
    }
    let parent = format!(":is({})", parents.join(", "));
    if selector.contains('&') {
        return selector.replace('&', &parent);
    }
    format!("{parent} {selector}")
}

impl<'s> Collect<'_, 's> {
    /// Makes `bounds` the scope rules are collected in, inside the one
    /// they're in now; `false` if its selectors don't parse or scopes
    /// nest too deep.
    fn enter_scope(&mut self, bounds: &ScopeBounds, parents: &[String]) -> bool {
        let mut depth = 0;
        let mut s = self.scope.as_deref();
        while let Some(x) = s {
            depth += 1;
            s = x.outer.as_deref();
        }
        if depth >= MAX_SCOPE_DEPTH {
            return false;
        }
        // Real parents: a style rule's selectors.
        let parents: Vec<String> = parents.iter().filter(|p| *p != IN_SCOPE).cloned().collect();
        let start = match &bounds.start {
            Some(sel) => match SelectorList::parse(&nest(sel, &parents)) {
                Ok(l) => ScopeStart::Selector(l),
                Err(_) => return false,
            },
            None if !parents.is_empty() => match SelectorList::parse(&parents.join(", ")) {
                Ok(l) => ScopeStart::Selector(l),
                Err(_) => return false,
            },
            None => ScopeStart::Owner(self.owner),
        };
        let end = match &bounds.end {
            Some(sel) => match SelectorList::parse(sel) {
                Ok(l) => Some(l),
                Err(_) => return false,
            },
            None => None,
        };
        self.scope = Some(Arc::new(Scope {
            start,
            end,
            outer: self.scope.take(),
        }));
        true
    }

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
            let s = if s == IN_SCOPE {
                ":where(:scope)"
            } else {
                s.as_str()
            };
            let Ok(list) = SelectorList::parse(s) else {
                continue;
            };
            // A pseudo-element's rule matches its originating element, and
            // counts the pseudo-element in its specificity.
            let specificity = list.specificity();
            let mut origin_any = false;
            let (bucket, list, pseudo) = match split_pseudo(s) {
                Split::No => (bucket(s), list, None),
                // Only after a state, or as part of another pseudo-element.
                Split::Unstyled => continue,
                Split::Pseudo(origin, p) => match SelectorList::parse(&origin) {
                    Ok(l) => {
                        origin_any = origin == "*";
                        (bucket(&origin), l, Some(p))
                    }
                    Err(_) => continue,
                },
            };
            *self.order += 1;
            self.entries.push(Entry {
                bucket,
                specificity,
                selector: list,
                origin: self.origin,
                layer,
                order: *self.order,
                declarations: declarations.clone(),
                scope: self.scope.clone(),
                containers: self.containers.clone(),
                any: origin_any,
                pseudo,
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
                    "container" => {
                        if let (Some(queries), Some(b)) =
                            (ContainerQuery::parse_list(&a.prelude), &a.block)
                        {
                            self.containers.push(Arc::new(queries));
                            self.items(&b.items, parents, layer);
                            self.containers.pop();
                        }
                    }
                    "scope" => {
                        let (Some(bounds), Some(b)) = (ScopeBounds::parse(&a.prelude), &a.block)
                        else {
                            return;
                        };
                        let outer = self.scope.clone();
                        if self.enter_scope(&bounds, parents) {
                            self.items(&b.items, &[IN_SCOPE.to_string()], layer);
                        }
                        self.scope = outer;
                    }
                    "keyframes" | "-webkit-keyframes" | "counter-style" | "property" => {
                        let l = self.layers.id(layer);
                        *self.order += 1;
                        let o = *self.order;
                        if let Some(k) = Keyframes::parse(a) {
                            self.at.keyframes.push((l, o, k));
                        } else if let Some(c) = CounterStyle::parse(a) {
                            self.at.counter_styles.push((l, o, c));
                        } else if let Some(p) = PropertyRule::parse(a) {
                            self.at.properties.push((l, o, p));
                        }
                    }
                    "layer" => {
                        let names = layer_names(&a.prelude);
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
    fn scope_and_proximity() {
        let html = "<style>@scope (.a) { p { color: green } } @scope (.b) to (.c) { p { color: blue } }</style>\
                    <div class=a><p id=one></p><div class=b><p id=two></p><div class=c><p id=three></p></div></div></div><p id=four>";
        assert_eq!(style(html, "#one", "color"), "rgb(0, 128, 0)");
        // .b is nearer than .a, so its rule wins though it's not later.
        let html2 = html
            .replace("@scope (.b) to (.c) { p { color: blue } }", "")
            .replace(
                "<style>",
                "<style>@scope (.b) to (.c) { p { color: blue } }",
            );
        assert_eq!(style(&html2, "#two", "color"), "rgb(0, 0, 255)");
        // Past the limit only .a's applies.
        assert_eq!(style(html, "#three", "color"), "rgb(0, 128, 0)");
        assert_eq!(style(html, "#four", "color"), "rgb(0, 0, 0)");
        // `:scope` itself, and declarations straight in the rule.
        let html = "<style>@scope (.a) { :scope { color: green } } @scope (.b) { color: blue }</style><div class=a></div><div class=b>";
        assert_eq!(style(html, ".a", "color"), "rgb(0, 128, 0)");
        assert_eq!(style(html, ".b", "color"), "rgb(0, 0, 255)");
        // With no root: the <style>'s parent.
        let html = "<div id=x><style>@scope { p { color: green } }</style><p></p></div><p id=y>";
        assert_eq!(style(html, "#x p", "color"), "rgb(0, 128, 0)");
        assert_eq!(style(html, "#y", "color"), "rgb(0, 0, 0)");
    }

    #[test]
    fn container_units_and_style_ranges() {
        // `cqw` from the inline-size container, `cqh` from the size one
        // beyond it, and the small viewport's with no container.
        let doc = parsec::parse(
            "<style>.s { container-type: size } .i { container-type: inline-size } \
             p { width: 10cqw; height: 10cqh; margin-left: 10cqi; margin-top: 10cqmax }</style>\
             <div class=s id=s><div class=i id=i><p></p></div></div><p id=bare>",
            false,
        );
        let find = |s: &str| {
            SelectorList::parse(s)
                .unwrap()
                .select_first(&doc, doc.root())
                .unwrap()
        };
        let styled = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .container_sizes(|_, c| {
                Some(match c.get("container-type").as_deref() {
                    Some("size") => (1000.0, 500.0),
                    _ => (200.0, 50.0),
                })
            })
            .style(&doc);
        let p = find("#i p");
        assert_eq!(styled.get(p, "width").as_deref(), Some("20px"));
        assert_eq!(styled.get(p, "height").as_deref(), Some("50px"));
        assert_eq!(styled.get(p, "margin-left").as_deref(), Some("20px"));
        assert_eq!(styled.get(p, "margin-top").as_deref(), Some("50px"));
        let bare = find("#bare");
        assert_eq!(styled.get(bare, "width").as_deref(), Some("128px"));
        assert_eq!(styled.get(bare, "height").as_deref(), Some("72px"));

        // A range in `style()`, with a custom property's value and `em`
        // on the container.
        let doc = parsec::parse(
            "<style>.c { --n: 30px; font-size: 10px } @container style(2em < --n <= 3em) { p { color: green } } \
             @container style(--n > 5) { p { background-color: red } }</style><div class=c><p>",
            false,
        );
        let p = SelectorList::parse("p")
            .unwrap()
            .select_first(&doc, doc.root())
            .unwrap();
        let styled = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .style(&doc);
        assert_eq!(styled.get(p, "color").as_deref(), Some("rgb(0, 128, 0)"));
        // A length against a number is false, not unknown.
        assert_eq!(
            styled.get(p, "background-color").as_deref(),
            Some("rgba(0, 0, 0, 0)")
        );
    }

    #[test]
    fn containers_imports_and_registrations() {
        let doc = parsec::parse(
            "<style>.c { container: card / inline-size } @container card (width > 400px) { p { color: green } }</style><div class=c><p>",
            false,
        );
        let p = SelectorList::parse("p")
            .unwrap()
            .select_first(&doc, doc.root())
            .unwrap();
        let wide = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .container_sizes(|_, _| Some((500.0, 100.0)))
            .style(&doc);
        assert_eq!(wide.get(p, "color").as_deref(), Some("rgb(0, 128, 0)"));
        let narrow = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .container_sizes(|_, _| Some((300.0, 100.0)))
            .style(&doc);
        assert_eq!(narrow.get(p, "color").as_deref(), Some("rgb(0, 0, 0)"));

        let doc = parsec::parse(
            "<style>@import 'a.css' layer(x); p { color: red } @layer x { p { color: blue } }</style><p>",
            false,
        );
        let p = SelectorList::parse("p")
            .unwrap()
            .select_first(&doc, doc.root())
            .unwrap();
        let styled = Styler::new(Environment::default())
            .with_document_styles(&doc)
            .fetch(|url| {
                (url == "a.css").then(|| "p { margin-top: 3px; color: green }".to_string())
            })
            .style(&doc);
        assert_eq!(styled.get(p, "margin-top").as_deref(), Some("3px"));
        // Unlayered beats layer x, imports included.
        assert_eq!(styled.get(p, "color").as_deref(), Some("rgb(255, 0, 0)"));

        let html = "<style>@property --w { syntax: '<length>'; inherits: false; initial-value: 2px } \
                    div { --w: 2em; font-size: 10px } p { --w: oops }</style><div><p></p></div>";
        assert_eq!(style(html, "div", "--w"), "20px");
        // Not inherited, and an invalid value is the initial one.
        assert_eq!(style(html, "p", "--w"), "2px");
    }

    #[test]
    fn at_rules() {
        let mut s = Styler::bare(Environment::default());
        s.add(
            "@keyframes a { to { opacity: 0 } } @layer l { @keyframes a { to { opacity: 1 } } } \
             @counter-style c { system: cyclic; symbols: x } @media (min-width: 9999px) { @keyframes b {} }",
            Origin::Author,
        );
        let r = s.at_rules();
        assert_eq!(r.keyframes.len(), 1);
        assert_eq!(r.keyframes[0].frames[0].declarations[0].1, "0");
        assert_eq!(r.counter_styles[0].name, "c");
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
