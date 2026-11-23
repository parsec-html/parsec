// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec for Node.js, as a native addon. It's bindings/wasm/src/lib.rs
//! with Node-API in place of wasm-bindgen: the same names, options and
//! error messages, so the npm package loads this where a build for the
//! machine exists and the WebAssembly where none does, and code can't
//! tell which it got. What it adds is `parseAsync`, which parses on
//! Node's thread pool.

#[path = "../../wasm/src/units.rs"]
mod units;

use std::cell::{Ref, RefCell, RefMut};
use std::rc::Rc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use parsec::css::{MatchOptions, SelectorList};
use parsec::{AttrNs, Kind, NodeId, Ns, Options};
use parsec_encoding::{Confidence, Encoding};
use parsec_xpath::{Doc, Index, Item, NoExtensions, Value, XPath};
use units::Units;

type Shared = Rc<Tree>;

/// What a getter or method gives when there's nothing: `undefined`, as
/// wasm-bindgen gives for None, where Node-API's Option gives `null`.
type Maybe<T> = Either<T, Undefined>;

fn maybe<T>(value: Option<T>) -> Maybe<T> {
    value.map_or(Either::B(()), Either::A)
}

fn error(message: impl std::fmt::Display) -> Error {
    Error::from_reason(message.to_string())
}

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

/// Where something was written, as the WebAssembly build's `Span` has it.
#[napi]
#[derive(Clone, Copy)]
pub struct Span {
    #[napi(readonly)]
    pub start: u32,
    #[napi(readonly)]
    pub end: u32,
    #[napi(readonly)]
    pub line: u32,
    #[napi(readonly)]
    pub column: u32,
}

/// A parse error the parser recovered from.
#[napi]
pub struct ParseError {
    #[napi(readonly)]
    pub code: String,
    #[napi(readonly)]
    pub offset: u32,
    #[napi(readonly)]
    pub line: u32,
    #[napi(readonly)]
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

/// How to parse; the WebAssembly build's `ParseOptions`.
#[napi(object)]
pub struct ParseOptions {
    pub scripting: Option<bool>,
    pub max_depth: Option<u32>,
    pub spans: Option<bool>,
    pub errors: Option<bool>,
}

/// ParseOptions and the encoding's: the WebAssembly build's
/// `ByteParseOptions`.
#[napi(object)]
pub struct ByteParseOptions {
    pub scripting: Option<bool>,
    pub max_depth: Option<u32>,
    pub spans: Option<bool>,
    pub errors: Option<bool>,
    pub transport_encoding: Option<String>,
    pub default_encoding: Option<String>,
}

/// Limits for `select`, `selectFirst` and `matches`.
#[napi(object)]
pub struct SelectOptions {
    pub max_steps: Option<f64>,
}

/// A parsed document or fragment.
#[napi]
pub struct Document {
    doc: Shared,
}

/// A node in a [`Document`].
#[napi]
pub struct Node {
    doc: Shared,
    id: NodeId,
}

fn compile(selector: &str) -> Result<SelectorList> {
    SelectorList::parse(selector).map_err(|e| error(format!("{selector:?}: {e}")))
}

/// The match options from `options`.
fn match_options(options: Option<SelectOptions>) -> Result<MatchOptions<'static>> {
    let max_steps = options.and_then(|o| o.max_steps);
    if max_steps.is_some_and(|n| !(n >= 0.0 && n.fract() == 0.0)) {
        return Err(error("maxSteps must be a whole number, 0 or more"));
    }
    Ok(MatchOptions {
        max_steps: max_steps.map(|n| n as u64),
        ..MatchOptions::default()
    })
}

fn match_error(e: parsec::css::MatchTimeout) -> Error {
    error(format!("maxSteps: {e}"))
}

fn options(options: Option<&ParseOptions>) -> Options {
    Options {
        scripting: options.and_then(|o| o.scripting).unwrap_or(false),
        budget: None,
        max_depth: options.and_then(|o| o.max_depth).map(|d| d as usize),
        spans: options.and_then(|o| o.spans).unwrap_or(false),
        errors: options.and_then(|o| o.errors).unwrap_or(false),
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

/// A string parsed, with what its offsets need; no JavaScript is touched,
/// so `parseAsync` runs this on the thread pool.
fn run(
    html: &str,
    parse: Options,
    context: Option<&str>,
) -> Result<(parsec::Document, Option<Units>)> {
    let doc = match context {
        Some(context) => {
            let context = if context.is_empty() { "div" } else { context };
            parsec::parse_fragment_with(html, Ns::Html, context, parse)
        }
        None => parsec::parse_with(html, parse),
    }
    .map_err(error)?;
    let units = (parse.spans || parse.errors)
        .then(|| Units::of_str(html))
        .flatten();
    Ok((doc, units))
}

fn label(what: &str, label: Option<String>) -> Result<Option<&'static Encoding>> {
    label
        .map(|l| {
            Encoding::for_label(l.as_bytes()).ok_or_else(|| {
                error(format!(
                    "{what}: {l:?} isn't an encoding label the Encoding Standard knows"
                ))
            })
        })
        .transpose()
}

fn byte_options(options: Option<ByteParseOptions>) -> Result<parsec_encoding::Options> {
    let (base, transport, fallback) = match options {
        Some(o) => (
            Some(ParseOptions {
                scripting: o.scripting,
                max_depth: o.max_depth,
                spans: o.spans,
                errors: o.errors,
            }),
            o.transport_encoding,
            o.default_encoding,
        ),
        None => (None, None, None),
    };
    Ok(parsec_encoding::Options {
        html: self::options(base.as_ref()),
        // As the HTML Standard has it, a transport layer's label that names
        // no encoding is passed over: it's usually a server's typo.
        transport: transport.and_then(|l| Encoding::for_label(l.as_bytes())),
        fallback: label("defaultEncoding", fallback)?
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

/// A `Uint8Array` (a `Buffer` is one), or an array of numbers, as
/// wasm-bindgen takes for `&[u8]`.
type Bytes<'a> = Either<&'a [u8], Vec<u8>>;

fn bytes<'a>(b: &'a Bytes<'_>) -> &'a [u8] {
    match b {
        Either::A(b) => b,
        Either::B(b) => b,
    }
}

/// Parses a whole document given as bytes, decoding them first as a
/// browser would.
#[napi(js_name = "parseBytes", catch_unwind)]
pub fn parse_bytes(bytes: Bytes, options: Option<ByteParseOptions>) -> Result<Document> {
    let bytes = self::bytes(&bytes);
    let options = byte_options(options)?;
    let keep = options.html.spans || options.html.errors;
    let parsed = parsec_encoding::parse_bytes(bytes, options).map_err(error)?;
    Ok(from_bytes(parsed, keep.then_some(bytes)))
}

/// Parses a document given as bytes a piece at a time.
#[napi]
pub struct ByteParser {
    /// None once finished.
    parser: Option<parsec_encoding::ByteParser>,
    /// Every byte, for the spans' offsets, when they're kept.
    bytes: Option<Vec<u8>>,
}

#[napi]
impl ByteParser {
    #[napi(constructor, catch_unwind)]
    pub fn new(options: Option<ByteParseOptions>) -> Result<ByteParser> {
        let options = byte_options(options)?;
        let keep = options.html.spans || options.html.errors;
        Ok(ByteParser {
            parser: Some(parsec_encoding::ByteParser::new(options)),
            bytes: keep.then(Vec::new),
        })
    }

    /// The next piece of the input.
    #[napi(catch_unwind)]
    pub fn feed(&mut self, chunk: Bytes) -> Result<()> {
        let chunk = bytes(&chunk);
        let parser = self.parser.as_mut().ok_or_else(finished)?;
        parser.feed(chunk);
        if let Some(kept) = &mut self.bytes {
            kept.extend_from_slice(chunk);
        }
        Ok(())
    }

    /// The end of the input: the document. A parser finishes once.
    #[napi(catch_unwind)]
    pub fn finish(&mut self) -> Result<Document> {
        let parser = self.parser.take().ok_or_else(finished)?;
        let parsed = parser.finish().map_err(error)?;
        let bytes = self.bytes.take();
        Ok(from_bytes(parsed, bytes.as_deref()))
    }
}

fn finished() -> Error {
    error("the ByteParser has finished")
}

/// Parses a whole document, as a browser does.
#[napi(catch_unwind)]
pub fn parse(html: String, options: Option<ParseOptions>) -> Result<Document> {
    let (doc, units) = run(&html, self::options(options.as_ref()), None)?;
    Ok(document(doc, units, None))
}

/// Parses `html` as the contents of a `context` element (`"div"` if not
/// given), as setting `innerHTML` does.
#[napi(js_name = "parseFragment", catch_unwind)]
pub fn parse_fragment(
    html: String,
    context: Option<String>,
    options: Option<ParseOptions>,
) -> Result<Document> {
    let context = context.as_deref().unwrap_or("");
    let (doc, units) = run(&html, self::options(options.as_ref()), Some(context))?;
    Ok(document(doc, units, None))
}

/// Runs `f`, turning a panic into an error rather than letting it end the
/// process.
fn guard<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or_else(|panic| {
        let what = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        Err(Error::from_reason(format!("parsec panicked: {what}")))
    })
}

/// `parse` on the thread pool.
pub struct Parse {
    html: String,
    options: Options,
}

impl Task for Parse {
    type Output = (parsec::Document, Option<Units>);
    type JsValue = Document;

    fn compute(&mut self) -> Result<Self::Output> {
        // napi-rs runs this on a pool thread with no catch_unwind of its
        // own, and a panic out of its extern "C" callback aborts the
        // process: reject the promise instead, as the WebAssembly build
        // throws and Python raises.
        guard(|| run(&self.html, self.options, None))
    }

    fn resolve(&mut self, _: Env, (doc, units): Self::Output) -> Result<Document> {
        Ok(document(doc, units, None))
    }
}

/// Parses a whole document on one of Node's worker threads, so the main
/// thread carries on meanwhile: a promise of what `parse` gives.
#[napi(
    js_name = "parseAsync",
    ts_return_type = "Promise<Document>",
    catch_unwind
)]
pub fn parse_async(html: String, options: Option<ParseOptions>) -> AsyncTask<Parse> {
    AsyncTask::new(Parse {
        html,
        options: self::options(options.as_ref()),
    })
}

fn node(doc: &Shared, id: Option<NodeId>) -> Maybe<Node> {
    maybe(id.map(|id| Node {
        doc: doc.clone(),
        id,
    }))
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

/// What `xpath()` gives: an array for a node-set (attributes as their
/// values), or a string, a number or a boolean.
type XPathResult = Either4<Vec<Either<Node, String>>, String, f64, bool>;

#[napi]
impl Document {
    /// The document node, or the fragment's root, which holds its nodes.
    #[napi(getter, catch_unwind)]
    pub fn root(&self) -> Node {
        Node {
            doc: self.doc.clone(),
            id: 0,
        }
    }

    /// The `<html>` element.
    #[napi(getter, js_name = "documentElement", catch_unwind)]
    pub fn document_element(&self) -> Maybe<Node> {
        let id = {
            let doc = self.doc.borrow();
            doc.children(0)
                .find(|&n| matches!(doc.kind(n), Kind::Element { .. }))
        };
        node(&self.doc, id)
    }

    #[napi(getter, catch_unwind)]
    pub fn head(&self) -> Maybe<Node> {
        let id = html_child(&self.doc.borrow(), "head");
        node(&self.doc, id)
    }

    #[napi(getter, catch_unwind)]
    pub fn body(&self) -> Maybe<Node> {
        let id = html_child(&self.doc.borrow(), "body");
        node(&self.doc, id)
    }

    /// Every element that matches, in document order.
    #[napi(catch_unwind)]
    pub fn select(&self, selector: String, options: Option<SelectOptions>) -> Result<Vec<Node>> {
        self.root().select(selector, options)
    }

    /// The first element that matches.
    #[napi(js_name = "selectFirst", catch_unwind)]
    pub fn select_first(
        &self,
        selector: String,
        options: Option<SelectOptions>,
    ) -> Result<Maybe<Node>> {
        self.root().select_first(selector, options)
    }

    /// The whole document as markup.
    #[napi(getter, catch_unwind)]
    pub fn html(&self) -> String {
        self.doc.borrow().inner_html(0)
    }

    /// Every text node's text, in tree order.
    #[napi(getter, catch_unwind)]
    pub fn text(&self) -> String {
        self.doc.borrow().text(0)
    }

    /// The encoding the document was decoded from: "UTF-8" for a string.
    #[napi(getter, js_name = "characterSet", catch_unwind)]
    pub fn character_set(&self) -> String {
        self.doc.encoding.map_or("UTF-8", |(e, _)| e.name()).into()
    }

    /// `"certain"` or `"tentative"`; undefined for a string.
    #[napi(getter, js_name = "encodingConfidence", catch_unwind)]
    pub fn encoding_confidence(&self) -> Maybe<String> {
        maybe(self.doc.encoding.map(|(_, c)| {
            match c {
                Confidence::Certain => "certain",
                Confidence::Tentative => "tentative",
            }
            .into()
        }))
    }

    /// The parse errors, in source order: empty unless parsed with
    /// `{ errors: true }`.
    #[napi(getter, catch_unwind)]
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
    #[napi(catch_unwind)]
    pub fn xpath(&self, expr: String) -> Result<XPathResult> {
        self.root().xpath(expr)
    }
}

#[napi]
impl Node {
    /// The DOM's `nodeType`.
    #[napi(getter, js_name = "nodeType", catch_unwind)]
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

    /// The DOM's `nodeName`.
    #[napi(getter, js_name = "nodeName", catch_unwind)]
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

    /// An element's name, lowercase for HTML.
    #[napi(getter, js_name = "localName", catch_unwind)]
    pub fn local_name(&self) -> Maybe<String> {
        maybe(match self.doc.borrow().kind(self.id) {
            Kind::Element { name, .. } => Some(name.to_string()),
            _ => None,
        })
    }

    #[napi(getter, js_name = "parentNode", catch_unwind)]
    pub fn parent_node(&self) -> Maybe<Node> {
        let id = self.doc.borrow().parent(self.id);
        node(&self.doc, id)
    }

    #[napi(getter, js_name = "firstChild", catch_unwind)]
    pub fn first_child(&self) -> Maybe<Node> {
        let id = self.doc.borrow().first_child(self.id);
        node(&self.doc, id)
    }

    #[napi(getter, js_name = "lastChild", catch_unwind)]
    pub fn last_child(&self) -> Maybe<Node> {
        let id = self.doc.borrow().last_child(self.id);
        node(&self.doc, id)
    }

    #[napi(getter, js_name = "nextSibling", catch_unwind)]
    pub fn next_sibling(&self) -> Maybe<Node> {
        let id = self.doc.borrow().next_sibling(self.id);
        node(&self.doc, id)
    }

    #[napi(getter, js_name = "previousSibling", catch_unwind)]
    pub fn previous_sibling(&self) -> Maybe<Node> {
        let id = self.doc.borrow().prev_sibling(self.id);
        node(&self.doc, id)
    }

    /// Every child, text and comments included.
    #[napi(getter, js_name = "childNodes", catch_unwind)]
    pub fn child_nodes(&self) -> Vec<Node> {
        let ids = self.doc.borrow().children(self.id).collect();
        nodes(&self.doc, ids)
    }

    /// The children that are elements.
    #[napi(getter, catch_unwind)]
    pub fn children(&self) -> Vec<Node> {
        let ids = {
            let doc = self.doc.borrow();
            doc.children(self.id)
                .filter(|&n| matches!(doc.kind(n), Kind::Element { .. }))
                .collect()
        };
        nodes(&self.doc, ids)
    }

    /// A template element's contents.
    #[napi(getter, catch_unwind)]
    pub fn content(&self) -> Maybe<Node> {
        let id = self.doc.borrow().template_contents(self.id);
        node(&self.doc, id)
    }

    /// The elements under this node that match, in document order.
    #[napi(catch_unwind)]
    pub fn select(&self, selector: String, options: Option<SelectOptions>) -> Result<Vec<Node>> {
        let (list, options) = (compile(&selector)?, match_options(options)?);
        let ids = list.try_select_with(&self.doc.borrow(), self.id, &options);
        Ok(nodes(&self.doc, ids.map_err(match_error)?))
    }

    #[napi(js_name = "selectFirst", catch_unwind)]
    pub fn select_first(
        &self,
        selector: String,
        options: Option<SelectOptions>,
    ) -> Result<Maybe<Node>> {
        let (list, options) = (compile(&selector)?, match_options(options)?);
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

    /// Runs an XPath 1.0 expression with this node as the context.
    #[napi(catch_unwind)]
    pub fn xpath(&self, expr: String) -> Result<XPathResult> {
        let compiled = XPath::new(&expr).map_err(error)?;
        let index = self.doc.index();
        let doc = self.doc.borrow();
        let value = compiled
            .evaluate(Doc::html(&doc, &index), self.id, &[], &[], &NoExtensions)
            .map_err(error)?;
        Ok(match value {
            Value::Nodes(items) => Either4::A(
                items
                    .into_iter()
                    .map(|item| match item {
                        Item::Node(id) => Either::A(Node {
                            doc: self.doc.clone(),
                            id,
                        }),
                        Item::Attr { owner, index, .. } => {
                            Either::B(attr_value(&doc, owner, index).to_string())
                        }
                    })
                    .collect(),
            ),
            Value::Str(s) => Either4::B(s.to_string()),
            Value::Num(n) => Either4::C(n),
            Value::Bool(b) => Either4::D(b),
        })
    }

    /// Where the node was written.
    #[napi(getter, catch_unwind)]
    pub fn span(&self) -> Maybe<Span> {
        let doc = self.doc.borrow();
        maybe(span(&self.doc, &doc, doc.span(self.id)))
    }

    /// Where an element's end tag was written, if it had one.
    #[napi(getter, js_name = "endSpan", catch_unwind)]
    pub fn end_span(&self) -> Maybe<Span> {
        let doc = self.doc.borrow();
        maybe(span(&self.doc, &doc, doc.end_span(self.id)))
    }

    /// Where the attribute `name` was written.
    #[napi(js_name = "attributeSpan", catch_unwind)]
    pub fn attribute_span(&self, name: String) -> Maybe<Span> {
        let doc = self.doc.borrow();
        let html = matches!(doc.kind(self.id), Kind::Element { ns: Ns::Html, .. });
        let i = doc.attrs_written(self.id).position(|(_, n, _)| {
            if html {
                n.eq_ignore_ascii_case(&name)
            } else {
                n == name
            }
        });
        maybe(i.and_then(|i| span(&self.doc, &doc, doc.attr_span(self.id, i))))
    }

    /// Whether this element matches, with the limit `select` takes.
    #[napi(catch_unwind)]
    pub fn matches(&self, selector: String, options: Option<SelectOptions>) -> Result<bool> {
        let (list, options) = (compile(&selector)?, match_options(options)?);
        let doc = self.doc.borrow();
        Ok(matches!(doc.kind(self.id), Kind::Element { .. })
            && list
                .try_matches_with(&doc, self.id, None, &options)
                .map_err(match_error)?)
    }

    #[napi(js_name = "getAttribute", catch_unwind)]
    pub fn get_attribute(&self, name: String) -> Maybe<String> {
        maybe(self.attribute(&name))
    }

    #[napi(js_name = "hasAttribute", catch_unwind)]
    pub fn has_attribute(&self, name: String) -> bool {
        self.attribute(&name).is_some()
    }

    /// The attributes' names as written, in source order.
    #[napi(js_name = "getAttributeNames", catch_unwind)]
    pub fn get_attribute_names(&self) -> Vec<String> {
        self.doc
            .borrow()
            .attrs_written(self.id)
            .map(|(_, n, _)| n.to_string())
            .collect()
    }

    /// Sets an attribute, as `setAttribute` does. Only elements have them.
    #[napi(js_name = "setAttribute", catch_unwind)]
    pub fn set_attribute(&self, name: String, value: String) -> Result<()> {
        let mut doc = self.doc.borrow_mut();
        if !matches!(doc.kind(self.id), Kind::Element { .. }) {
            return Err(error("only an element has attributes"));
        }
        doc.set_attr(self.id, &name, &value)
            .map_err(|e| error(format!("{name:?}: {e}")))
    }

    /// Removes an attribute; false if there wasn't one.
    #[napi(js_name = "removeAttribute", catch_unwind)]
    pub fn remove_attribute(&self, name: String) -> bool {
        self.doc.borrow_mut().remove_attr(self.id, &name)
    }

    /// The `id` attribute, or `""`.
    #[napi(getter, catch_unwind)]
    pub fn id(&self) -> String {
        let doc = self.doc.borrow();
        doc.attrs(self.id)
            .find(|&(ns, n, _)| ns == AttrNs::None && n == "id")
            .map_or(String::new(), |(_, _, v)| v.to_string())
    }

    /// The text of this node and everything under it, as `textContent`.
    #[napi(getter, js_name = "textContent", catch_unwind)]
    pub fn text_content(&self) -> String {
        self.doc.borrow().text(self.id)
    }

    /// The node and its contents as markup.
    #[napi(getter, js_name = "outerHTML", catch_unwind)]
    pub fn outer_html(&self) -> String {
        self.doc.borrow().outer_html(self.id)
    }

    /// The contents as markup.
    #[napi(getter, js_name = "innerHTML", catch_unwind)]
    pub fn inner_html(&self) -> String {
        self.doc.borrow().inner_html(self.id)
    }

    /// Takes the node out of the tree. It can still be read, and put back.
    #[napi(catch_unwind)]
    pub fn remove(&self) {
        self.doc.borrow_mut().detach(self.id);
    }

    /// Moves `child`, a node of the same document, to the end of this
    /// one's children.
    #[napi(js_name = "appendChild", catch_unwind)]
    pub fn append_child(&self, child: &Node) -> Result<()> {
        self.insert(child, None)
    }

    /// Moves `child`, a node of the same document, in before `reference`,
    /// one of this node's children.
    #[napi(js_name = "insertBefore", catch_unwind)]
    pub fn insert_before(&self, child: &Node, reference: &Node) -> Result<()> {
        self.insert(child, Some(reference))
    }

    /// Whether `other` is this same node.
    #[napi(js_name = "isSameNode", catch_unwind)]
    pub fn is_same_node(&self, other: &Node) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc) && self.id == other.id
    }
}

impl Node {
    fn attribute(&self, name: &str) -> Option<String> {
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

    fn insert(&self, child: &Node, reference: Option<&Node>) -> Result<()> {
        if !Rc::ptr_eq(&self.doc, &child.doc)
            || reference
                .as_ref()
                .is_some_and(|r| !Rc::ptr_eq(&self.doc, &r.doc))
        {
            return Err(error("nodes can only move within their own document"));
        }
        self.doc
            .borrow_mut()
            .insert_before(self.id, child.id, reference.map(|r| r.id))
            .map_err(error)
    }
}

/// The value of an XPath attribute item, which names its attribute by
/// place on the element rather than carrying it.
fn attr_value(doc: &parsec::Document, owner: parsec::NodeId, index: u32) -> &str {
    doc.attrs(owner)
        .nth(index as usize)
        .map_or("", |(_, _, value)| value)
}

#[cfg(test)]
mod tests {
    use super::guard;

    #[test]
    fn a_panic_becomes_an_error() {
        let err = guard::<()>(|| panic!("boom")).unwrap_err();
        assert_eq!(err.reason, "parsec panicked: boom");
        let err = guard::<()>(|| panic!("{}", String::from("formatted"))).unwrap_err();
        assert_eq!(err.reason, "parsec panicked: formatted");
        assert_eq!(guard(|| Ok(7)).unwrap(), 7);
    }
}
