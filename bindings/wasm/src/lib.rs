// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec for JavaScript. Names follow the DOM where there's a DOM name
//! for the thing (`textContent`, `outerHTML`, `nextSibling`), so code
//! written against a browser's tree reads the same; selectors are `select`
//! and `selectFirst`.
//!
//! A node is its document plus an index, so it stays valid after edits:
//! a removed node can still be read, and put back.

mod units;

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use parsec::css::{MatchOptions, SelectorList};
use parsec::{AttrNs, Kind, NodeId, Ns, Options};
use parsec_encoding::{Confidence, Encoding};
use parsec_xpath::{Doc, Index, Item, NoExtensions, Value, XPath};
use units::Units;
use wasm_bindgen::prelude::*;

type Shared = Rc<Tree>;

/// A document and the XPath index of it as it is, which the first query
/// builds and any change drops.
struct Tree {
    doc: RefCell<parsec::Document>,
    index: RefCell<Option<Rc<Index>>>,
    /// Turns the core's byte offsets into UTF-16 indexes of a string, or
    /// byte offsets into bytes; None when they're the same (ASCII), or
    /// when nothing reads them.
    units: Option<Units>,
    /// What bytes were decoded from, and how sure that is; None for a
    /// string.
    encoding: Option<(&'static Encoding, Confidence)>,
}

impl Tree {
    fn borrow(&self) -> Ref<'_, parsec::Document> {
        self.doc.borrow()
    }

    /// The document to change, so the index goes.
    fn borrow_mut(&self) -> RefMut<'_, parsec::Document> {
        self.index.take();
        self.doc.borrow_mut()
    }

    /// A core offset as an index into the JavaScript string.
    fn out(&self, offset: usize) -> u32 {
        self.units.as_ref().map_or(offset, |u| u.out(offset)) as u32
    }

    fn index(&self) -> Rc<Index> {
        let doc = self.doc.borrow();
        self.index
            .borrow_mut()
            .get_or_insert_with(|| {
                // Every node without a parent, so a query can start in a
                // removed subtree as well as in the document.
                let roots: Vec<NodeId> = (0..doc.len() as NodeId)
                    .filter(|&n| doc.parent(n).is_none())
                    .collect();
                Rc::new(Index::build(&*doc, &roots))
            })
            .clone()
    }
}

/// Where something was written: `start` to `end` as indexes into the
/// string that was parsed, so `html.slice(start, end)` is its source (or
/// byte offsets into the bytes, for `parseBytes` and `ByteParser`), and
/// the `line` and `column` of `start`, both from 1, the column counting
/// characters (code points, so an emoji is one).
#[wasm_bindgen]
#[derive(Clone, Copy)]
pub struct Span {
    #[wasm_bindgen(readonly)]
    pub start: u32,
    #[wasm_bindgen(readonly)]
    pub end: u32,
    #[wasm_bindgen(readonly)]
    pub line: u32,
    #[wasm_bindgen(readonly)]
    pub column: u32,
}

/// A parse error the parser recovered from.
#[wasm_bindgen]
pub struct ParseError {
    /// The HTML Standard's name for it (`eof-in-tag`), or html5lib's for
    /// one tree construction finds (`unexpected-end-tag`).
    #[wasm_bindgen(readonly, getter_with_clone)]
    pub code: String,
    /// An index into the string that was parsed, or the bytes, as `Span`
    /// counts.
    #[wasm_bindgen(readonly)]
    pub offset: u32,
    #[wasm_bindgen(readonly)]
    pub line: u32,
    /// From 1, in characters.
    #[wasm_bindgen(readonly)]
    pub column: u32,
}

fn span(tree: &Tree, doc: &parsec::Document, r: Option<std::ops::Range<usize>>) -> Option<Span> {
    let r = r?;
    let (line, column) = doc.line_col(r.start);
    Some(Span {
        start: tree.out(r.start),
        end: tree.out(r.end),
        line: line as u32,
        column: column as u32,
    })
}

#[wasm_bindgen(typescript_custom_section)]
const PARSE_OPTIONS: &str = r#"
/** How to parse. */
export interface ParseOptions {
    /** Parse `<noscript>` as a browser with JavaScript on does. */
    scripting?: boolean;
    /** Keep the tree at most this deep, as Chromium does at 512. */
    maxDepth?: number;
    /** Record where each node and attribute was written, for `span`. */
    spans?: boolean;
    /** Keep the parse errors, for `errors`. */
    errors?: boolean;
}

/**
 * How to parse bytes. Their encoding is the one a browser would pick: a
 * byte order mark's, else `transportEncoding`, else a `<meta>`'s, else
 * UTF-8 if the bytes are valid UTF-8, else `defaultEncoding`. Spans and error offsets are byte offsets into the
 * bytes.
 */
export interface ByteParseOptions extends ParseOptions {
    /** The encoding the transport layer gave, such as the `charset` of an
     * HTTP `Content-Type`: any label the Encoding Standard knows. One it
     * doesn't is passed over, as browsers do. */
    transportEncoding?: string;
    /** The encoding when nothing names one and the bytes aren't valid
     * UTF-8: `"windows-1252"` unless given, as in browsers set to
     * English. */
    defaultEncoding?: string;
}

/**
 * Limits for `select`, `selectFirst` and `matches`, for a selector or a
 * page you don't control: a `:has()` of more than one compound over a very
 * deep tree can take seconds.
 */
export interface SelectOptions {
    /** Give up after matching this many compounds, by throwing an Error
     * that names `maxSteps`, never with part of the answer. A walk of the
     * tree is a compound or so an element; a few hundred million is
     * seconds of work. There's no time limit, as wasm32 has no clock. */
    maxSteps?: number;
}

/**
 * What `xpath()` gives: an array for a node-set (attributes as their
 * values, namespace nodes as their URIs), or a string, a number or a
 * boolean.
 */
export type XPathResult = (Node | string)[] | string | number | boolean;
"#;

#[wasm_bindgen]
extern "C" {
    /// A plain object, read with the getters below.
    #[wasm_bindgen(typescript_type = "ParseOptions")]
    pub type ParseOptions;
    #[wasm_bindgen(method, getter)]
    fn scripting(this: &ParseOptions) -> Option<bool>;
    #[wasm_bindgen(method, getter, js_name = maxDepth)]
    fn max_depth(this: &ParseOptions) -> Option<u32>;
    #[wasm_bindgen(method, getter)]
    fn spans(this: &ParseOptions) -> Option<bool>;
    #[wasm_bindgen(method, getter)]
    fn errors(this: &ParseOptions) -> Option<bool>;

    /// ParseOptions and the encoding's.
    #[wasm_bindgen(extends = ParseOptions, typescript_type = "ByteParseOptions")]
    pub type ByteParseOptions;
    #[wasm_bindgen(method, getter, js_name = transportEncoding)]
    fn transport_encoding(this: &ByteParseOptions) -> Option<String>;
    #[wasm_bindgen(method, getter, js_name = defaultEncoding)]
    fn default_encoding(this: &ByteParseOptions) -> Option<String>;

    /// A plain object, read with the getter below.
    #[wasm_bindgen(typescript_type = "SelectOptions")]
    pub type SelectOptions;
    #[wasm_bindgen(method, getter, js_name = maxSteps)]
    fn max_steps(this: &SelectOptions) -> Option<f64>;
}

/// A parsed document or fragment.
#[wasm_bindgen]
pub struct Document {
    doc: Shared,
}

/// A node in a [`Document`]: an element, text, a comment, a doctype, or
/// the document itself.
#[wasm_bindgen]
pub struct Node {
    doc: Shared,
    id: NodeId,
}

fn compile(selector: &str) -> Result<SelectorList, JsError> {
    SelectorList::parse(selector).map_err(|e| JsError::new(&format!("{selector:?}: {e}")))
}

/// The match options from `options`.
fn match_options(options: Option<&SelectOptions>) -> Result<MatchOptions<'static>, JsError> {
    let max_steps = options.and_then(SelectOptions::max_steps);
    if max_steps.is_some_and(|n| !(n >= 0.0 && n.fract() == 0.0)) {
        return Err(JsError::new("maxSteps must be a whole number, 0 or more"));
    }
    Ok(MatchOptions {
        max_steps: max_steps.map(|n| n as u64),
        ..MatchOptions::default()
    })
}

fn match_error(e: parsec::css::MatchTimeout) -> JsError {
    JsError::new(&format!("maxSteps: {e}"))
}

fn options(options: Option<&ParseOptions>) -> Options {
    Options {
        scripting: options.and_then(ParseOptions::scripting).unwrap_or(false),
        budget: None,
        max_depth: options
            .and_then(ParseOptions::max_depth)
            .map(|d| d as usize),
        spans: options.and_then(ParseOptions::spans).unwrap_or(false),
        errors: options.and_then(ParseOptions::errors).unwrap_or(false),
        ..Options::default()
    }
}

fn document(
    doc: parsec::Document,
    units: Option<Units>,
    encoding: Option<(&'static Encoding, Confidence)>,
) -> Document {
    Document {
        doc: Rc::new(Tree {
            doc: RefCell::new(doc),
            index: RefCell::new(None),
            units,
            encoding,
        }),
    }
}

fn run(
    html: &str,
    options: Option<&ParseOptions>,
    context: Option<&str>,
) -> Result<Document, JsError> {
    let parse = self::options(options);
    let doc = match context {
        Some(context) => {
            let context = if context.is_empty() { "div" } else { context };
            parsec::parse_fragment_with(html, Ns::Html, context, parse)
        }
        None => parsec::parse_with(html, parse),
    }
    .map_err(|e| JsError::new(&e.to_string()))?;
    let units = (parse.spans || parse.errors)
        .then(|| Units::of_str(html))
        .flatten();
    Ok(document(doc, units, None))
}

fn label(what: &str, label: Option<String>) -> Result<Option<&'static Encoding>, JsError> {
    label
        .map(|l| {
            Encoding::for_label(l.as_bytes()).ok_or_else(|| {
                JsError::new(&format!(
                    "{what}: {l:?} isn't an encoding label the Encoding Standard knows"
                ))
            })
        })
        .transpose()
}

fn byte_options(options: Option<&ByteParseOptions>) -> Result<parsec_encoding::Options, JsError> {
    let base = options.map(|o| o.as_ref());
    Ok(parsec_encoding::Options {
        html: self::options(base),
        // As the HTML Standard has it, a transport layer's label that names
        // no encoding is passed over: it's usually a server's typo.
        transport: options
            .and_then(ByteParseOptions::transport_encoding)
            .and_then(|l| Encoding::for_label(l.as_bytes())),
        fallback: label(
            "defaultEncoding",
            options.and_then(ByteParseOptions::default_encoding),
        )?
        .unwrap_or(parsec_encoding::Options::default().fallback),
    })
}

/// A document from bytes, with offsets into `bytes` if it keeps any.
fn from_bytes(parsed: parsec_encoding::Parsed, bytes: Option<&[u8]>) -> Document {
    let units = bytes.and_then(|b| Units::of_bytes(b, parsed.encoding));
    document(
        parsed.document,
        units,
        Some((parsed.encoding, parsed.confidence)),
    )
}

/// Parses a whole document given as bytes, decoding them first as a
/// browser would.
#[wasm_bindgen(js_name = parseBytes)]
pub fn parse_bytes(bytes: &[u8], options: Option<ByteParseOptions>) -> Result<Document, JsError> {
    let options = byte_options(options.as_ref())?;
    let keep = options.html.spans || options.html.errors;
    let parsed =
        parsec_encoding::parse_bytes(bytes, options).map_err(|e| JsError::new(&e.to_string()))?;
    Ok(from_bytes(parsed, keep.then_some(bytes)))
}

/// Parses a document given as bytes a piece at a time, as a browser does
/// while it downloads one: `feed()` each piece as it comes, then
/// `finish()` for the document `parseBytes` gives for all of them.
#[wasm_bindgen]
pub struct ByteParser {
    /// None once finished.
    parser: Option<parsec_encoding::ByteParser>,
    /// Every byte, for the spans' offsets, when they're kept.
    bytes: Option<Vec<u8>>,
}

#[wasm_bindgen]
impl ByteParser {
    #[wasm_bindgen(constructor)]
    pub fn new(options: Option<ByteParseOptions>) -> Result<ByteParser, JsError> {
        let options = byte_options(options.as_ref())?;
        let keep = options.html.spans || options.html.errors;
        Ok(ByteParser {
            parser: Some(parsec_encoding::ByteParser::new(options)),
            bytes: keep.then(Vec::new),
        })
    }

    /// The next piece of the input.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(), JsError> {
        let parser = self.parser.as_mut().ok_or_else(finished)?;
        parser.feed(chunk);
        if let Some(kept) = &mut self.bytes {
            kept.extend_from_slice(chunk);
        }
        Ok(())
    }

    /// The end of the input: the document. A parser finishes once.
    pub fn finish(&mut self) -> Result<Document, JsError> {
        let parser = self.parser.take().ok_or_else(finished)?;
        let parsed = parser.finish().map_err(|e| JsError::new(&e.to_string()))?;
        let bytes = self.bytes.take();
        Ok(from_bytes(parsed, bytes.as_deref()))
    }
}

fn finished() -> JsError {
    JsError::new("the ByteParser has finished")
}

/// Parses a whole document, as a browser does.
#[wasm_bindgen]
pub fn parse(html: &str, options: Option<ParseOptions>) -> Result<Document, JsError> {
    run(html, options.as_ref(), None)
}

/// Parses `html` as the contents of a `context` element (`"div"` if not
/// given), as setting `innerHTML` does.
#[wasm_bindgen(js_name = parseFragment)]
pub fn parse_fragment(
    html: &str,
    context: Option<String>,
    options: Option<ParseOptions>,
) -> Result<Document, JsError> {
    run(
        html,
        options.as_ref(),
        Some(context.as_deref().unwrap_or("")),
    )
}

fn node(doc: &Shared, id: Option<NodeId>) -> Option<Node> {
    id.map(|id| Node {
        doc: doc.clone(),
        id,
    })
}

fn nodes(doc: &Shared, ids: Vec<NodeId>) -> Vec<Node> {
    ids.into_iter()
        .map(|id| Node {
            doc: doc.clone(),
            id,
        })
        .collect()
}

fn html_child(doc: &parsec::Document, name: &str) -> Option<NodeId> {
    let html = doc
        .children(doc.root())
        .find(|&n| doc.lower_name(n) == Some("html"))?;
    doc.children(html).find(|&n| {
        matches!(doc.kind(n), Kind::Element { ns: Ns::Html, .. }) && doc.lower_name(n) == Some(name)
    })
}

#[wasm_bindgen]
impl Document {
    /// The document node, or the fragment's root, which holds its nodes.
    #[wasm_bindgen(getter)]
    pub fn root(&self) -> Node {
        Node {
            doc: self.doc.clone(),
            id: 0,
        }
    }

    /// The `<html>` element.
    #[wasm_bindgen(getter, js_name = documentElement)]
    pub fn document_element(&self) -> Option<Node> {
        let id = {
            let doc = self.doc.borrow();
            doc.children(0)
                .find(|&n| matches!(doc.kind(n), Kind::Element { .. }))
        };
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter)]
    pub fn head(&self) -> Option<Node> {
        let id = html_child(&self.doc.borrow(), "head");
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter)]
    pub fn body(&self) -> Option<Node> {
        let id = html_child(&self.doc.borrow(), "body");
        node(&self.doc, id)
    }

    /// Every element that matches, in document order. `options.maxSteps`
    /// bounds the work: past it, this throws rather than give part of the
    /// answer.
    pub fn select(
        &self,
        selector: &str,
        options: Option<SelectOptions>,
    ) -> Result<Vec<Node>, JsError> {
        self.root().select(selector, options)
    }

    /// The first element that matches, found without looking further
    /// unless there's a `maxSteps`, which only the whole walk reads.
    #[wasm_bindgen(js_name = selectFirst)]
    pub fn select_first(
        &self,
        selector: &str,
        options: Option<SelectOptions>,
    ) -> Result<Option<Node>, JsError> {
        self.root().select_first(selector, options)
    }

    /// The whole document as markup.
    #[wasm_bindgen(getter)]
    pub fn html(&self) -> String {
        self.doc.borrow().inner_html(0)
    }

    /// Every text node's text, in tree order.
    #[wasm_bindgen(getter)]
    pub fn text(&self) -> String {
        self.doc.borrow().text(0)
    }

    /// The encoding the document was decoded from, by the Encoding
    /// Standard's name ("UTF-8", "Shift_JIS", "windows-1252"), as the
    /// DOM's `document.characterSet`: "UTF-8" for a string.
    #[wasm_bindgen(getter, js_name = characterSet)]
    pub fn character_set(&self) -> String {
        self.doc.encoding.map_or("UTF-8", |(e, _)| e.name()).into()
    }

    /// `"certain"` when a byte order mark, `transportEncoding` or a
    /// `<meta>` the parser met decided the encoding, `"tentative"` when
    /// it's from the first 1,024 bytes' `<meta>` alone or
    /// `defaultEncoding`, as the HTML Standard calls them; undefined for a
    /// string.
    #[wasm_bindgen(getter, js_name = encodingConfidence, unchecked_return_type = "\"certain\" | \"tentative\" | undefined")]
    pub fn encoding_confidence(&self) -> Option<String> {
        self.doc.encoding.map(|(_, c)| {
            match c {
                Confidence::Certain => "certain",
                Confidence::Tentative => "tentative",
            }
            .into()
        })
    }

    /// The parse errors, in source order: empty unless parsed with
    /// `{ errors: true }`.
    #[wasm_bindgen(getter)]
    pub fn errors(&self) -> Vec<ParseError> {
        self.doc
            .borrow()
            .errors()
            .iter()
            .map(|e| ParseError {
                code: e.code.to_string(),
                offset: self.doc.out(e.offset),
                line: e.line as u32,
                column: e.column as u32,
            })
            .collect()
    }

    /// Runs an XPath 1.0 expression over the document.
    #[wasm_bindgen(unchecked_return_type = "XPathResult")]
    pub fn xpath(&self, expr: &str) -> Result<JsValue, JsError> {
        self.root().xpath(expr)
    }
}

#[wasm_bindgen]
impl Node {
    /// The DOM's `nodeType`: 1 for an element, 3 text, 4 a CDATA section
    /// (in SVG or MathML), 7 a processing instruction, 8 a comment, 9 the
    /// document, 10 a doctype, 11 a fragment or a shadow root.
    #[wasm_bindgen(getter, js_name = nodeType)]
    pub fn node_type(&self) -> u8 {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { .. } => 1,
            Kind::Text(_) => 3,
            Kind::Cdata(_) => 4,
            Kind::ProcessingInstruction { .. } => 7,
            Kind::Comment(_) => 8,
            Kind::Document => 9,
            Kind::Doctype { .. } => 10,
            // A shadow root is a DocumentFragment in the DOM.
            Kind::Fragment | Kind::ShadowRoot(_) => 11,
        }
    }

    /// The DOM's `nodeName`: `DIV` for an HTML element, the name as
    /// written for SVG and MathML (`foreignObject`), `#text`, `#comment`,
    /// `#document`, or the doctype's name.
    #[wasm_bindgen(getter, js_name = nodeName)]
    pub fn node_name(&self) -> String {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { ns: Ns::Html, name } => name.to_ascii_uppercase(),
            Kind::Element { name, .. } => name.to_string(),
            Kind::Text(_) => "#text".into(),
            Kind::Cdata(_) => "#cdata-section".into(),
            Kind::ProcessingInstruction { target, .. } => target.to_string(),
            Kind::Comment(_) => "#comment".into(),
            Kind::Document => "#document".into(),
            Kind::Doctype { name, .. } => name.to_string(),
            Kind::Fragment | Kind::ShadowRoot(_) => "#document-fragment".into(),
        }
    }

    /// An element's name, lowercase for HTML: `div`, `foreignObject`.
    #[wasm_bindgen(getter, js_name = localName)]
    pub fn local_name(&self) -> Option<String> {
        match self.doc.borrow().kind(self.id) {
            Kind::Element { name, .. } => Some(name.to_string()),
            _ => None,
        }
    }

    #[wasm_bindgen(getter, js_name = parentNode)]
    pub fn parent_node(&self) -> Option<Node> {
        let id = self.doc.borrow().parent(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = firstChild)]
    pub fn first_child(&self) -> Option<Node> {
        let id = self.doc.borrow().first_child(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = lastChild)]
    pub fn last_child(&self) -> Option<Node> {
        let id = self.doc.borrow().last_child(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = nextSibling)]
    pub fn next_sibling(&self) -> Option<Node> {
        let id = self.doc.borrow().next_sibling(self.id);
        node(&self.doc, id)
    }

    #[wasm_bindgen(getter, js_name = previousSibling)]
    pub fn previous_sibling(&self) -> Option<Node> {
        let id = self.doc.borrow().prev_sibling(self.id);
        node(&self.doc, id)
    }

    /// Every child, text and comments included.
    #[wasm_bindgen(getter, js_name = childNodes)]
    pub fn child_nodes(&self) -> Vec<Node> {
        let ids = self.doc.borrow().children(self.id).collect();
        nodes(&self.doc, ids)
    }

    /// The children that are elements.
    #[wasm_bindgen(getter)]
    pub fn children(&self) -> Vec<Node> {
        let ids = {
            let doc = self.doc.borrow();
            doc.children(self.id)
                .filter(|&n| matches!(doc.kind(n), Kind::Element { .. }))
                .collect()
        };
        nodes(&self.doc, ids)
    }

    /// A template element's contents, which selectors and `childNodes`
    /// don't reach, as in a browser.
    #[wasm_bindgen(getter)]
    pub fn content(&self) -> Option<Node> {
        let id = self.doc.borrow().template_contents(self.id);
        node(&self.doc, id)
    }

    /// The elements under this node that match, in document order.
    /// Combinators look past it, as `querySelectorAll` does.
    /// `options.maxSteps` is as `Document.select` takes it.
    pub fn select(
        &self,
        selector: &str,
        options: Option<SelectOptions>,
    ) -> Result<Vec<Node>, JsError> {
        let (list, options) = (compile(selector)?, match_options(options.as_ref())?);
        let ids = list.try_select_with(&self.doc.borrow(), self.id, &options);
        Ok(nodes(&self.doc, ids.map_err(match_error)?))
    }

    #[wasm_bindgen(js_name = selectFirst)]
    pub fn select_first(
        &self,
        selector: &str,
        options: Option<SelectOptions>,
    ) -> Result<Option<Node>, JsError> {
        let (list, options) = (compile(selector)?, match_options(options.as_ref())?);
        let doc = self.doc.borrow();
        let id = match options.max_steps {
            None => list.select_first(&doc, self.id),
            Some(_) => {
                let ids = list.try_select_with(&doc, self.id, &options);
                ids.map_err(match_error)?.first().copied()
            }
        };
        Ok(node(&self.doc, id))
    }

    /// Runs an XPath 1.0 expression with this node as the context. A
    /// node-set comes back as an array in document order, with attributes
    /// as their values and namespace nodes as their URIs; names match
    /// lowercase, as lxml matches HTML.
    #[wasm_bindgen(unchecked_return_type = "XPathResult")]
    pub fn xpath(&self, expr: &str) -> Result<JsValue, JsError> {
        let compiled = XPath::new(expr).map_err(|e| JsError::new(&e.to_string()))?;
        let index = self.doc.index();
        let doc = self.doc.borrow();
        let view = Doc::html(&doc, &index);
        let value = compiled
            .evaluate(view, self.id, &[], &[], &NoExtensions)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(match value {
            Value::Nodes(items) => items
                .into_iter()
                .map(|item| match item {
                    Item::Node(id) => JsValue::from(Node {
                        doc: self.doc.clone(),
                        id,
                    }),
                    Item::Attr { owner, index, .. } => {
                        JsValue::from_str(attr_value(&doc, owner, index))
                    }
                    Item::Ns { decl, .. } => JsValue::from_str(view.namespace_at(decl).1),
                })
                .collect::<Vec<_>>()
                .into(),
            Value::Str(s) => JsValue::from_str(&s),
            Value::Num(n) => JsValue::from_f64(n),
            Value::Bool(b) => JsValue::from_bool(b),
        })
    }

    /// Where the node was written: an element's start tag, or a whole text
    /// node or comment. Only with `{ spans: true }`, and never for a node
    /// the parser implied (`<html>`, `<head>`, `<body>`) or an edit made.
    #[wasm_bindgen(getter)]
    pub fn span(&self) -> Option<Span> {
        let doc = self.doc.borrow();
        span(&self.doc, &doc, doc.span(self.id))
    }

    /// Where an element's end tag was written, if it had one.
    #[wasm_bindgen(getter, js_name = endSpan)]
    pub fn end_span(&self) -> Option<Span> {
        let doc = self.doc.borrow();
        span(&self.doc, &doc, doc.end_span(self.id))
    }

    /// Where the attribute `name` was written, from its name to the end
    /// of its value.
    #[wasm_bindgen(js_name = attributeSpan)]
    pub fn attribute_span(&self, name: &str) -> Option<Span> {
        let doc = self.doc.borrow();
        let html = matches!(doc.kind(self.id), Kind::Element { ns: Ns::Html, .. });
        let i = doc.attrs_written(self.id).position(|(_, n, _)| {
            if html {
                n.eq_ignore_ascii_case(name)
            } else {
                n == name
            }
        })?;
        span(&self.doc, &doc, doc.attr_span(self.id, i))
    }

    /// Whether this element matches, with the limit `select` takes.
    pub fn matches(&self, selector: &str, options: Option<SelectOptions>) -> Result<bool, JsError> {
        let (list, options) = (compile(selector)?, match_options(options.as_ref())?);
        let doc = self.doc.borrow();
        Ok(matches!(doc.kind(self.id), Kind::Element { .. })
            && list
                .try_matches_with(&doc, self.id, None, &options)
                .map_err(match_error)?)
    }

    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        let doc = self.doc.borrow();
        let html = matches!(doc.kind(self.id), Kind::Element { ns: Ns::Html, .. });
        let name = if html {
            name.to_ascii_lowercase()
        } else {
            name.to_string()
        };
        doc.attrs_written(self.id)
            .find(|&(_, n, _)| n == name)
            .map(|(_, _, v)| v.to_string())
    }

    #[wasm_bindgen(js_name = hasAttribute)]
    pub fn has_attribute(&self, name: &str) -> bool {
        self.get_attribute(name).is_some()
    }

    /// The attributes' names as written, in source order (`xlink:href`).
    #[wasm_bindgen(js_name = getAttributeNames)]
    pub fn get_attribute_names(&self) -> Vec<String> {
        self.doc
            .borrow()
            .attrs_written(self.id)
            .map(|(_, n, _)| n.to_string())
            .collect()
    }

    /// Sets an attribute, as `setAttribute` does. Only elements have them.
    #[wasm_bindgen(js_name = setAttribute)]
    pub fn set_attribute(&self, name: &str, value: &str) -> Result<(), JsError> {
        let mut doc = self.doc.borrow_mut();
        if !matches!(doc.kind(self.id), Kind::Element { .. }) {
            return Err(JsError::new("only an element has attributes"));
        }
        doc.set_attr(self.id, name, value)
            .map_err(|e| JsError::new(&format!("{name:?}: {e}")))
    }

    /// Removes an attribute; false if there wasn't one.
    #[wasm_bindgen(js_name = removeAttribute)]
    pub fn remove_attribute(&self, name: &str) -> bool {
        self.doc.borrow_mut().remove_attr(self.id, name)
    }

    /// The `id` attribute, or `""`.
    #[wasm_bindgen(getter)]
    pub fn id(&self) -> String {
        let doc = self.doc.borrow();
        doc.attrs(self.id)
            .find(|&(ns, n, _)| ns == AttrNs::None && n == "id")
            .map_or(String::new(), |(_, _, v)| v.to_string())
    }

    /// The text of this node and everything under it, as `textContent`.
    #[wasm_bindgen(getter, js_name = textContent)]
    pub fn text_content(&self) -> String {
        self.doc.borrow().text(self.id)
    }

    /// The node and its contents as markup.
    #[wasm_bindgen(getter, js_name = outerHTML)]
    pub fn outer_html(&self) -> String {
        self.doc.borrow().outer_html(self.id)
    }

    /// The contents as markup.
    #[wasm_bindgen(getter, js_name = innerHTML)]
    pub fn inner_html(&self) -> String {
        self.doc.borrow().inner_html(self.id)
    }

    /// Takes the node out of the tree. It can still be read, and put back,
    /// as the DOM's `ChildNode.remove()` leaves it.
    pub fn remove(&self) {
        self.doc.borrow_mut().detach(self.id);
    }

    /// Moves `child`, a node of the same document, to the end of this
    /// one's children.
    #[wasm_bindgen(js_name = appendChild)]
    pub fn append_child(&self, child: &Node) -> Result<(), JsError> {
        self.insert(child, None)
    }

    /// Moves `child`, a node of the same document, in before `reference`,
    /// one of this node's children.
    #[wasm_bindgen(js_name = insertBefore)]
    pub fn insert_before(&self, child: &Node, reference: &Node) -> Result<(), JsError> {
        self.insert(child, Some(reference))
    }

    /// Whether `other` is this same node.
    #[wasm_bindgen(js_name = isSameNode)]
    pub fn is_same_node(&self, other: &Node) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc) && self.id == other.id
    }
}

impl Node {
    fn insert(&self, child: &Node, reference: Option<&Node>) -> Result<(), JsError> {
        if !Rc::ptr_eq(&self.doc, &child.doc)
            || reference
                .as_ref()
                .is_some_and(|r| !Rc::ptr_eq(&self.doc, &r.doc))
        {
            return Err(JsError::new(
                "nodes can only move within their own document",
            ));
        }
        self.doc
            .borrow_mut()
            .insert_before(self.id, child.id, reference.map(|r| r.id))
            .map_err(|e| JsError::new(&e.to_string()))
    }
}

/// The value of an XPath attribute item, which names its attribute by
/// place on the element rather than carrying it.
fn attr_value(doc: &parsec::Document, owner: parsec::NodeId, index: u32) -> &str {
    doc.attrs(owner)
        .nth(index as usize)
        .map_or("", |(_, _, value)| value)
}
