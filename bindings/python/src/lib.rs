// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `parsec_html`: Parsec for Python, shaped like selectolax's Lexbor API
//! so a project can switch by changing its import.
//!
//! A parser owns its document behind a `RwLock`, and a node is the parser
//! plus an index, so nodes stay valid after edits and can be read from many
//! threads at once, free-threaded Python included. Two rules keep that free
//! of deadlocks: nothing waits for the lock while attached to the
//! interpreter (see `HtmlParser::read`), and nothing makes a Python object
//! while holding it, since that can need every other thread to pause.

mod units;

use std::borrow::Cow;
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, TryLockError};
use std::time::Duration;

use parsec::css::{MatchOptions, SelectorList};
use parsec::{AttrNs, Document, Kind, NodeId, Ns, Options, ParseError};
use parsec_encoding::{ByteParser, Confidence, Encoding};
use parsec_xpath::{Doc, Index, Item, NoExtensions, Value, XPath, XValue};
use pyo3::exceptions::{PyKeyError, PyTimeoutError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use units::Units;

use pyo3::types::{PyBool, PyBytes, PyDict, PyFloat, PyList, PyString, PyTuple};

pyo3::create_exception!(
    parsec_html,
    SelectorError,
    PyValueError,
    "A CSS selector that doesn't parse."
);

pyo3::create_exception!(
    parsec_html,
    XPathError,
    PyValueError,
    "An XPath expression that doesn't parse, or that can't be evaluated."
);

#[pyclass(frozen, module = "parsec_html", name = "HTMLParser")]
struct HtmlParser {
    doc: RwLock<Document>,
    fragment: bool,
    scripting: bool,
    /// The page's address, for `:target` and `:local-link`.
    url: Option<String>,
    /// The XPath index of the tree as it is, built by the first query
    /// after a change. `write` drops it, under the write lock, so a query
    /// (which holds the read lock) never sees one of an older tree.
    index: Mutex<Option<Arc<Index>>>,
    /// Turns the core's byte offsets into indexes of what was passed in.
    units: Units,
    /// What `bytes` were decoded from, and how sure that is; None for a
    /// `str`.
    encoding: Option<(&'static Encoding, Confidence)>,
}

/// A parse error the parser recovered from, as `HTMLParser.errors` lists
/// them. A record, not an exception: a page with errors still parses.
#[pyclass(frozen, get_all, module = "parsec_html", name = "ParseError")]
struct SyntaxIssue {
    /// The HTML Standard's name for it (`eof-in-tag`), or html5lib's for
    /// those tree construction finds (`unexpected-end-tag`).
    code: &'static str,
    /// An index into the input, as `Node.source_range` counts.
    offset: usize,
    /// From 1.
    line: usize,
    /// From 1, in characters.
    column: usize,
}

#[pymethods]
impl SyntaxIssue {
    fn __repr__(&self) -> String {
        format!(
            "<ParseError {} at {}:{}>",
            self.code, self.line, self.column
        )
    }

    fn __str__(&self) -> String {
        format!("{}:{}: {}", self.line, self.column, self.code)
    }
}

#[pyclass(frozen, module = "parsec_html")]
struct Node {
    parser: Py<HtmlParser>,
    id: NodeId,
}

/// An element's attributes as a mutable mapping, as `node.attrs`.
#[pyclass(frozen, module = "parsec_html", mapping)]
struct Attributes {
    parser: Py<HtmlParser>,
    id: NodeId,
}

fn compile(query: &str) -> PyResult<SelectorList> {
    SelectorList::parse(query).map_err(|e| SelectorError::new_err(format!("{query:?}: {e}")))
}

/// A `str` as it is, or `bytes` read as UTF-8 with bad sequences replaced,
/// as a browser decoding a UTF-8 page does.
fn text_of<'a>(value: &'a Bound<'_, PyAny>) -> PyResult<Option<Cow<'a, str>>> {
    if let Ok(s) = value.cast::<PyString>() {
        return s.to_cow().map(Some);
    }
    if let Ok(b) = value.cast::<PyBytes>() {
        return Ok(Some(String::from_utf8_lossy(b.as_bytes())));
    }
    Ok(None)
}

fn namespace(name: &str) -> PyResult<Ns> {
    match name {
        "html" | "http://www.w3.org/1999/xhtml" => Ok(Ns::Html),
        "svg" | "http://www.w3.org/2000/svg" => Ok(Ns::Svg),
        "math" | "http://www.w3.org/1998/Math/MathML" => Ok(Ns::MathMl),
        _ => Err(PyValueError::new_err(format!(
            "unknown fragment namespace {name:?}: use \"html\", \"svg\" or \"math\""
        ))),
    }
}

fn parse_error(e: ParseError) -> PyErr {
    match e {
        ParseError::Timeout { .. } => PyTimeoutError::new_err(e.to_string()),
        ParseError::TooLarge { .. } => PyValueError::new_err(e.to_string()),
    }
}

fn is_element(doc: &Document, id: NodeId) -> bool {
    matches!(doc.kind(id), Kind::Element { .. })
}

fn is_text(doc: &Document, id: NodeId) -> bool {
    matches!(doc.kind(id), Kind::Text(_))
}

fn ascii_space(s: &str) -> bool {
    s.bytes()
        .all(|b| matches!(b, b' ' | b'\t' | b'\n' | b'\x0c' | b'\r'))
}

/// Python's `str.strip()`: Rust's whitespace plus the four separators
/// Python also counts.
fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// `id` and its descendants in tree order, without recursion.
fn subtree(doc: &Document, id: NodeId) -> Vec<NodeId> {
    let (mut out, mut stack) = (Vec::new(), vec![id]);
    while let Some(n) = stack.pop() {
        out.push(n);
        let at = stack.len();
        stack.extend(doc.children(n));
        stack[at..].reverse();
    }
    out
}

/// selectolax's `text()`: text nodes in tree order (all of them, or only
/// the node's own children), each stripped if asked, joined by `separator`.
fn text(
    doc: &Document,
    id: NodeId,
    deep: bool,
    separator: &str,
    strip: bool,
    skip_empty: bool,
) -> String {
    if deep && separator.is_empty() && !strip && !skip_empty {
        return doc.text(id);
    }
    let nodes: Vec<NodeId> = if deep {
        subtree(doc, id)
    } else if is_text(doc, id) {
        vec![id]
    } else {
        doc.children(id).collect()
    };
    let mut parts = Vec::new();
    for n in nodes {
        if let Kind::Text(t) = doc.kind(n) {
            if skip_empty && ascii_space(t) {
                continue;
            }
            parts.push(if strip { py_strip(t) } else { t });
        }
    }
    parts.join(separator)
}

/// A subtree written down so it can be built again, in this document or
/// another: the copy is taken under a read lock and built under a write
/// lock, so copying within one document never holds both.
enum Step {
    Element(Ns, String, Vec<(AttrNs, String, String)>),
    /// What follows goes into the open template's contents.
    Contents,
    /// Closes the innermost open element or template contents.
    Close,
    Text(String),
    Comment(String),
    Cdata(String),
    Pi(String, String),
}

/// The steps that rebuild `id`, or a document's or fragment's children.
/// A doctype is left out: it can't be made, and only a document has one.
fn record(doc: &Document, id: NodeId) -> Vec<Step> {
    enum Visit {
        Node(NodeId),
        Contents(NodeId),
        Close,
    }
    let mut steps = Vec::new();
    let mut stack = vec![Visit::Node(id)];
    while let Some(v) = stack.pop() {
        let n = match v {
            Visit::Close => {
                steps.push(Step::Close);
                continue;
            }
            Visit::Contents(c) => {
                steps.push(Step::Contents);
                stack.push(Visit::Close);
                stack.extend(
                    doc.children(c)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(Visit::Node),
                );
                continue;
            }
            Visit::Node(n) => n,
        };
        match doc.kind(n) {
            Kind::Element { ns, name } => {
                // attrs() has the names as stored, except a namespaced
                // attribute's, which is stored with its prefix.
                let attrs = doc
                    .attrs(n)
                    .zip(doc.attrs_written(n))
                    .map(|((ans, local, value), (_, written, _))| {
                        let name = if ans == AttrNs::None { local } else { written };
                        (ans, name.to_string(), value.to_string())
                    })
                    .collect();
                steps.push(Step::Element(ns, name.to_string(), attrs));
                stack.push(Visit::Close);
                if let Some(c) = doc.template_contents(n) {
                    stack.push(Visit::Contents(c));
                }
                stack.extend(
                    doc.children(n)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(Visit::Node),
                );
            }
            Kind::Document | Kind::Fragment => {
                stack.extend(
                    doc.children(n)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .map(Visit::Node),
                );
            }
            Kind::Doctype { .. } => {}
            Kind::Text(t) => steps.push(Step::Text(t.to_string())),
            Kind::Comment(t) => steps.push(Step::Comment(t.to_string())),
            Kind::Cdata(t) => steps.push(Step::Cdata(t.to_string())),
            Kind::ProcessingInstruction { target, data } => {
                steps.push(Step::Pi(target.to_string(), data.to_string()))
            }
        }
    }
    steps
}

/// Builds `steps` as new, detached nodes and returns the top-level ones.
fn build(doc: &mut Document, steps: &[Step]) -> Vec<NodeId> {
    let (mut roots, mut open): (Vec<NodeId>, Vec<NodeId>) = (Vec::new(), Vec::new());
    for step in steps {
        let made = match step {
            Step::Element(ns, name, attrs) => {
                let el = doc.create_element(*ns, name);
                for (ans, name, value) in attrs {
                    doc.add_attr(el, *ans, name, value);
                }
                el
            }
            Step::Contents => {
                let el = *open.last().expect("contents follow their template");
                open.push(doc.template_contents(el).expect("a template has contents"));
                continue;
            }
            Step::Close => {
                open.pop();
                continue;
            }
            Step::Text(t) => doc.create_text(t),
            Step::Comment(t) => doc.create_comment(t),
            Step::Cdata(t) => doc.create_cdata(t),
            Step::Pi(target, data) => doc.create_processing_instruction(target, data),
        };
        match open.last() {
            Some(&parent) => doc
                .append(parent, made)
                .expect("a new node goes into a new element"),
            None => roots.push(made),
        }
        if matches!(step, Step::Element(..)) {
            open.push(made);
        }
    }
    roots
}

/// Every node with no parent: the document first, then any subtree that
/// was removed or made apart from it, so a query can start anywhere.
fn roots(doc: &Document) -> Vec<NodeId> {
    (0..doc.len() as NodeId)
        .filter(|&n| doc.parent(n).is_none())
        .collect()
}

/// An XPath result, copied out of the document.
enum Found {
    Nodes(Vec<Hit>),
    Str(String),
    Num(f64),
    Bool(bool),
}

enum Hit {
    Node(NodeId),
    Str(String),
}

impl Found {
    fn into_py(self, py: Python<'_>, parser: &Py<HtmlParser>) -> PyResult<Py<PyAny>> {
        Ok(match self {
            Found::Nodes(hits) => {
                let list = PyList::empty(py);
                for hit in hits {
                    match hit {
                        Hit::Node(id) => list.append(node(py, parser, id))?,
                        Hit::Str(s) => list.append(s)?,
                    }
                }
                list.into_any().unbind()
            }
            Found::Str(s) => PyString::new(py, &s).into_any().unbind(),
            Found::Num(n) => PyFloat::new(py, n).into_any().unbind(),
            Found::Bool(b) => PyBool::new(py, b).to_owned().into_any().unbind(),
        })
    }
}

/// lxml's `xpath(query, **variables)`: each keyword a `$variable`.
fn variables(given: Option<&Bound<'_, PyDict>>) -> PyResult<Vec<(String, XValue)>> {
    let mut out = Vec::new();
    for (k, v) in given.into_iter().flat_map(|d| d.iter()) {
        // bool before the numbers, since a bool is an int in Python.
        let value = if let Ok(b) = v.cast::<PyBool>() {
            XValue::Bool(b.is_true())
        } else if let Some(s) = text_of(&v)? {
            XValue::Str(s.into_owned())
        } else if let Ok(n) = v.extract::<f64>() {
            XValue::Num(n)
        } else {
            return Err(PyTypeError::new_err(format!(
                "${k}: an XPath variable is a str, a number or a bool"
            )));
        };
        out.push((k.extract()?, value));
    }
    Ok(out)
}

/// The place among an element's attributes of the one `key` names, found
/// as `Attributes` finds it.
fn attr_index(doc: &Document, id: NodeId, key: &str) -> Option<usize> {
    let html = matches!(doc.kind(id), Kind::Element { ns: Ns::Html, .. });
    doc.attrs_written(id).position(|(_, n, _)| {
        if html {
            n.eq_ignore_ascii_case(key)
        } else {
            n == key
        }
    })
}

fn options(
    scripting: bool,
    budget: Option<f64>,
    max_depth: Option<usize>,
    positions: bool,
    errors: bool,
) -> PyResult<Options> {
    let budget = budget
        .map(|s| {
            Duration::try_from_secs_f64(s)
                .map_err(|e| PyValueError::new_err(format!("budget: {e}")))
        })
        .transpose()?;
    Ok(Options {
        scripting,
        budget,
        max_depth,
        spans: positions,
        errors,
    })
}

fn label(what: &str, label: &str) -> PyResult<&'static Encoding> {
    Encoding::for_label(label.as_bytes()).ok_or_else(|| {
        PyValueError::new_err(format!(
            "{what}: {label:?} isn't an encoding label the Encoding Standard knows"
        ))
    })
}

fn byte_options(
    html: Options,
    transport: Option<&str>,
    default: Option<&str>,
) -> PyResult<parsec_encoding::Options> {
    Ok(parsec_encoding::Options {
        html,
        // As the HTML Standard has it, a transport layer's label that names
        // no encoding is passed over: it's usually a server's typo.
        transport: transport.and_then(|l| Encoding::for_label(l.as_bytes())),
        fallback: match default {
            Some(l) => label("default_encoding", l)?,
            None => parsec_encoding::Options::default().fallback,
        },
    })
}

fn confidence(c: Confidence) -> &'static str {
    match c {
        Confidence::Certain => "certain",
        Confidence::Tentative => "tentative",
    }
}

impl HtmlParser {
    fn made(
        doc: Document,
        fragment: bool,
        scripting: bool,
        url: Option<String>,
        units: Units,
        encoding: Option<(&'static Encoding, Confidence)>,
    ) -> HtmlParser {
        HtmlParser {
            doc: RwLock::new(doc),
            fragment,
            scripting,
            url,
            index: Mutex::new(None),
            units,
            encoding,
        }
    }

    /// A span of the core's as `(start, end)` in the input's own units.
    fn range(&self, r: std::ops::Range<usize>) -> (usize, usize) {
        (self.units.out(r.start), self.units.out(r.end))
    }

    /// The document to read. It waits for a writer detached from the
    /// interpreter, then takes the lock attached, so a thread holding it can
    /// always get the interpreter's attention.
    fn read(&self, py: Python<'_>) -> RwLockReadGuard<'_, Document> {
        loop {
            match self.doc.try_read() {
                Ok(guard) => return guard,
                Err(TryLockError::Poisoned(p)) => return p.into_inner(),
                Err(TryLockError::WouldBlock) => py.detach(|| drop(self.doc.read())),
            }
        }
    }

    fn write(&self, py: Python<'_>) -> RwLockWriteGuard<'_, Document> {
        let guard = loop {
            match self.doc.try_write() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(p)) => break p.into_inner(),
                Err(TryLockError::WouldBlock) => py.detach(|| drop(self.doc.write())),
            }
        };
        // Whatever the caller changes, the next query indexes the tree again.
        self.index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        guard
    }

    /// Runs `query` from `scope`, detached from the interpreter as `select`
    /// is, and copies the result out before the lock is let go.
    fn run_xpath(
        &self,
        py: Python<'_>,
        query: &str,
        scope: NodeId,
        vars: &[(String, XValue)],
    ) -> PyResult<Found> {
        let compiled = XPath::new(query).map_err(|e| XPathError::new_err(e.to_string()))?;
        py.detach(|| {
            let doc = self.doc.read().unwrap_or_else(PoisonError::into_inner);
            let index = self
                .index
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get_or_insert_with(|| Arc::new(Index::build(&doc, &roots(&doc))))
                .clone();
            let value =
                compiled.evaluate(Doc::html(&doc, &index), scope, vars, &[], &NoExtensions)?;
            Ok(match value {
                Value::Nodes(items) => Found::Nodes(
                    items
                        .into_iter()
                        .map(|item| match item {
                            // As lxml gives them: text and attributes as strings.
                            Item::Node(n) => match doc.kind(n) {
                                Kind::Text(t) | Kind::Cdata(t) => Hit::Str(t.to_string()),
                                _ => Hit::Node(n),
                            },
                            Item::Attr { value, .. } => Hit::Str(value.to_string()),
                        })
                        .collect(),
                ),
                Value::Str(s) => Found::Str(s.into_owned()),
                Value::Num(n) => Found::Num(n),
                Value::Bool(b) => Found::Bool(b),
            })
        })
        .map_err(|e: parsec_xpath::XPathError| XPathError::new_err(e.to_string()))
    }

    fn options(&self) -> MatchOptions<'_> {
        MatchOptions {
            url: self.url.as_deref(),
            ..MatchOptions::default()
        }
    }

    /// Matches under `scope`, and `scope` itself first if `include` and it
    /// matches, as selectolax's node queries count the node.
    fn select(
        &self,
        py: Python<'_>,
        list: &SelectorList,
        scope: NodeId,
        include: bool,
    ) -> Vec<NodeId> {
        py.detach(|| {
            let doc = self.doc.read().unwrap_or_else(PoisonError::into_inner);
            let options = self.options();
            let mut out = Vec::new();
            if include
                && is_element(&doc, scope)
                && list.matches_with(&doc, scope, Some(scope), &options)
            {
                out.push(scope);
            }
            out.extend(list.select_with(&doc, scope, &options));
            out
        })
    }

    fn select_first(
        &self,
        py: Python<'_>,
        list: &SelectorList,
        scope: NodeId,
        include: bool,
    ) -> Option<NodeId> {
        let doc = self.read(py);
        let options = self.options();
        if include
            && is_element(&doc, scope)
            && list.matches_with(&doc, scope, Some(scope), &options)
        {
            return Some(scope);
        }
        if self.url.is_some() {
            list.select_with(&doc, scope, &options).first().copied()
        } else {
            list.select_first(&doc, scope)
        }
    }

    /// The document element, or a fragment's first node.
    fn root_id(&self, py: Python<'_>) -> Option<NodeId> {
        let doc = self.read(py);
        let mut children = doc.children(doc.root());
        if self.fragment {
            children.next()
        } else {
            children.find(|&n| is_element(&doc, n))
        }
    }

    /// The html element's child named `name`.
    fn html_child(&self, py: Python<'_>, name: &str) -> Option<NodeId> {
        if self.fragment {
            return None;
        }
        let html = self.root_id(py)?;
        let doc = self.read(py);
        doc.children(html).find(|&n| {
            matches!(doc.kind(n), Kind::Element { ns: Ns::Html, .. })
                && doc.lower_name(n) == Some(name)
        })
    }
}

fn node(py: Python<'_>, parser: &Py<HtmlParser>, id: NodeId) -> Node {
    Node {
        parser: parser.clone_ref(py),
        id,
    }
}

fn nodes(py: Python<'_>, parser: &Py<HtmlParser>, ids: Vec<NodeId>) -> Vec<Node> {
    ids.into_iter().map(|id| node(py, parser, id)).collect()
}

fn first_or_default(
    py: Python<'_>,
    parser: &Py<HtmlParser>,
    found: Vec<NodeId>,
    default: Option<Py<PyAny>>,
    strict: bool,
) -> PyResult<Py<PyAny>> {
    if strict && found.len() > 1 {
        return Err(PyValueError::new_err(format!(
            "Expected 1 match, but found {} matches",
            found.len()
        )));
    }
    match found.first() {
        Some(&id) => Ok(Bound::new(py, node(py, parser, id))?.into_any().unbind()),
        None => Ok(default.unwrap_or_else(|| py.None())),
    }
}

/// Each of `tags`' matches under `scope`, gathered before any is changed.
fn matches_of(
    py: Python<'_>,
    parser: &HtmlParser,
    tags: &Bound<'_, PyAny>,
    scope: NodeId,
) -> PyResult<Vec<NodeId>> {
    let mut out = Vec::new();
    for tag in tags.try_iter()? {
        let tag = tag?;
        out.extend(parser.select(py, &compile(&tag.extract::<String>()?)?, scope, false));
    }
    Ok(out)
}

fn unwrap(doc: &mut Document, id: NodeId, delete_empty: bool) {
    let Some(parent) = doc.parent(id) else { return };
    if doc.first_child(id).is_none() {
        if delete_empty {
            doc.remove(id);
        }
        return;
    }
    while let Some(child) = doc.first_child(id) {
        // `id` is a child of `parent`, and its child can't hold its parent.
        let _ = doc.insert_before(parent, child, Some(id));
    }
    doc.remove(id);
}

/// Joins each run of adjacent text nodes under `id` into one.
fn merge_text_nodes(doc: &mut Document, id: NodeId) {
    for parent in subtree(doc, id) {
        let mut run: Vec<NodeId> = Vec::new();
        let children: Vec<NodeId> = doc.children(parent).collect();
        for child in children.into_iter().map(Some).chain([None]) {
            if let Some(c) = child
                && is_text(doc, c)
            {
                run.push(c);
                continue;
            }
            if run.len() > 1 {
                let joined: String = run
                    .iter()
                    .map(|&t| match doc.kind(t) {
                        Kind::Text(s) => s,
                        _ => "",
                    })
                    .collect();
                let merged = doc.create_text(&joined);
                let _ = doc.insert_before(parent, merged, Some(run[0]));
                for &t in &run {
                    doc.remove(t);
                }
            }
            run.clear();
        }
    }
}

#[pymethods]
impl HtmlParser {
    /// Parses `html` as a whole document, or as a fragment inside
    /// `fragment_tag` when `is_fragment`. `budget` is a time limit in
    /// seconds and `max_depth` caps the tree's depth; `url` is the page's
    /// address, for `:target` and `:local-link`.
    ///
    /// A `str` is parsed as it is. `bytes` are decoded first, in the
    /// encoding a browser would pick: a byte order mark's, else
    /// `transport_encoding` (the `charset` of an HTTP `Content-Type`,
    /// passed over if it names no encoding, as browsers do), else a
    /// `<meta>` naming one, else UTF-8 if the bytes are valid UTF-8, else
    /// `default_encoding` (windows-1252 unless given). The names are
    /// html5lib's.
    ///
    /// `positions` records where each node and attribute was written, for
    /// `Node.source_range` and the like; `errors` keeps the parse errors,
    /// for `errors`.
    #[new]
    #[pyo3(signature = (html, is_fragment = false, fragment_tag = "div", fragment_namespace = "html", scripting = false, budget = None, max_depth = None, url = None, positions = false, errors = false, transport_encoding = None, default_encoding = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        py: Python<'_>,
        html: &Bound<'_, PyAny>,
        is_fragment: bool,
        fragment_tag: &str,
        fragment_namespace: &str,
        scripting: bool,
        budget: Option<f64>,
        max_depth: Option<usize>,
        url: Option<String>,
        positions: bool,
        errors: bool,
        transport_encoding: Option<&str>,
        default_encoding: Option<&str>,
    ) -> PyResult<Self> {
        let options = options(scripting, budget, max_depth, positions, errors)?;
        let ns = namespace(fragment_namespace)?;
        let keep = positions || errors;
        if let Ok(b) = html.cast::<PyBytes>() {
            let bytes = b.as_bytes();
            let options = byte_options(options, transport_encoding, default_encoding)?;
            let (doc, encoding, confidence, units) = py
                .detach(|| -> Result<_, ParseError> {
                    let (doc, encoding, confidence) = if is_fragment {
                        // A fragment has no restart on a later `<meta>`:
                        // that's a document's.
                        let (encoding, confidence) =
                            parsec_encoding::sniff(bytes, options.transport, options.fallback);
                        let (text, _) = encoding.decode_with_bom_removal(bytes);
                        let html = options.html;
                        let doc = parsec::parse_fragment_with(&text, ns, fragment_tag, html)?;
                        (doc, encoding, confidence)
                    } else {
                        let p = parsec_encoding::parse_bytes(bytes, options)?;
                        (p.document, p.encoding, p.confidence)
                    };
                    let units = if keep {
                        Units::of_bytes(bytes, encoding)
                    } else {
                        Units::Same
                    };
                    Ok((doc, encoding, confidence, units))
                })
                .map_err(parse_error)?;
            return Ok(HtmlParser::made(
                doc,
                is_fragment,
                scripting,
                url,
                units,
                Some((encoding, confidence)),
            ));
        }
        let Ok(html) = html.cast::<PyString>() else {
            return Err(PyTypeError::new_err("html must be str or bytes"));
        };
        if transport_encoding.is_some() || default_encoding.is_some() {
            // As html5lib: a str has no encoding to choose.
            return Err(PyTypeError::new_err(
                "transport_encoding and default_encoding are for bytes; a str is already decoded",
            ));
        }
        let html = html.to_cow()?;
        let units = if keep {
            Units::of_str(&html)
        } else {
            Units::Same
        };
        let doc = py
            .detach(|| {
                if is_fragment {
                    parsec::parse_fragment_with(&html, ns, fragment_tag, options)
                } else {
                    parsec::parse_with(&html, options)
                }
            })
            .map_err(parse_error)?;
        Ok(HtmlParser::made(
            doc,
            is_fragment,
            scripting,
            url,
            units,
            None,
        ))
    }

    /// The encoding `bytes` were decoded from, by its name in the Encoding
    /// Standard ("UTF-8", "Shift_JIS", "windows-1252"): what a browser's
    /// `document.characterSet` says. None for a `str`.
    #[getter]
    fn encoding(&self) -> Option<&'static str> {
        self.encoding.map(|(e, _)| e.name())
    }

    /// "certain" when a byte order mark, `transport_encoding` or a `<meta>`
    /// the parser met decided the encoding, "tentative" when it's from the
    /// first 1,024 bytes' `<meta>` alone or `default_encoding`, as the
    /// HTML Standard calls them. None for a `str`.
    #[getter]
    fn encoding_confidence(&self) -> Option<&'static str> {
        self.encoding.map(|(_, c)| confidence(c))
    }

    /// Runs an XPath 1.0 expression over the document, as lxml's
    /// `tree.xpath()` does: a list for a node-set (elements and comments
    /// as nodes, text and attributes as strings), else a `str`, `float`
    /// or `bool`. Keywords bind `$variables`.
    #[pyo3(signature = (query, **variables))]
    fn xpath(
        slf: &Bound<'_, Self>,
        query: &str,
        variables: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let vars = self::variables(variables)?;
        slf.get()
            .run_xpath(slf.py(), query, 0, &vars)?
            .into_py(slf.py(), slf.as_unbound())
    }

    /// The parse errors the parser recovered from, in source order; empty
    /// unless parsed with `errors=True`.
    #[getter]
    fn errors(&self, py: Python<'_>) -> Vec<SyntaxIssue> {
        // Copied under the lock, made into objects after it's let go.
        let found: Vec<parsec::SyntaxError> = self.read(py).errors().to_vec();
        found
            .into_iter()
            .map(|e| SyntaxIssue {
                code: e.code,
                offset: self.units.out(e.offset),
                line: e.line,
                column: e.column,
            })
            .collect()
    }

    /// The line and column of an offset into the input, counted as
    /// `source_range` counts: both from 1, the column in characters.
    fn line_col(&self, py: Python<'_>, offset: usize) -> (usize, usize) {
        self.read(py).line_col(self.units.back(offset))
    }

    /// The `<html>` element; a fragment's first node.
    #[getter]
    fn root(slf: &Bound<'_, Self>) -> Option<Node> {
        let py = slf.py();
        slf.get()
            .root_id(py)
            .map(|id| node(py, slf.as_unbound(), id))
    }

    #[getter]
    fn head(slf: &Bound<'_, Self>) -> Option<Node> {
        let py = slf.py();
        slf.get()
            .html_child(py, "head")
            .map(|id| node(py, slf.as_unbound(), id))
    }

    #[getter]
    fn body(slf: &Bound<'_, Self>) -> Option<Node> {
        let py = slf.py();
        slf.get()
            .html_child(py, "body")
            .map(|id| node(py, slf.as_unbound(), id))
    }

    /// The whole document's markup, or every node of a fragment's.
    #[getter]
    fn html(&self, py: Python<'_>) -> String {
        let doc = self.read(py);
        doc.inner_html(doc.root())
    }

    #[pyo3(signature = (deep = true, separator = "", strip = false, skip_empty = false))]
    fn text(
        &self,
        py: Python<'_>,
        deep: bool,
        separator: &str,
        strip: bool,
        skip_empty: bool,
    ) -> String {
        let doc = self.read(py);
        text(&doc, doc.root(), deep, separator, strip, skip_empty)
    }

    fn css(slf: &Bound<'_, Self>, query: &str) -> PyResult<Vec<Node>> {
        let list = compile(query)?;
        let found = slf.get().select(slf.py(), &list, 0, false);
        Ok(nodes(slf.py(), slf.as_unbound(), found))
    }

    #[pyo3(signature = (query, default = None, strict = false))]
    fn css_first(
        slf: &Bound<'_, Self>,
        query: &str,
        default: Option<Py<PyAny>>,
        strict: bool,
    ) -> PyResult<Py<PyAny>> {
        let (py, list) = (slf.py(), compile(query)?);
        let found = if strict {
            slf.get().select(py, &list, 0, false)
        } else {
            slf.get()
                .select_first(py, &list, 0, false)
                .into_iter()
                .collect()
        };
        first_or_default(py, slf.as_unbound(), found, default, strict)
    }

    /// Whether anything in the document matches.
    fn css_matches(&self, py: Python<'_>, selector: &str) -> PyResult<bool> {
        Ok(self
            .select_first(py, &compile(selector)?, 0, false)
            .is_some())
    }

    fn any_css_matches(&self, py: Python<'_>, selectors: &Bound<'_, PyAny>) -> PyResult<bool> {
        for s in selectors.try_iter()? {
            if self
                .select_first(py, &compile(&s?.extract::<String>()?)?, 0, false)
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Every element named `name`, in document order.
    fn tags(slf: &Bound<'_, Self>, name: &str) -> PyResult<Vec<Node>> {
        if name.is_empty() {
            return Err(PyValueError::new_err("tag name can't be empty"));
        }
        let py = slf.py();
        let found: Vec<NodeId> = {
            let doc = slf.get().read(py);
            let want = name.to_ascii_lowercase();
            subtree(&doc, doc.root())
                .into_iter()
                .filter(|&n| doc.lower_name(n).is_some_and(|l| l == want))
                .collect()
        };
        Ok(nodes(py, slf.as_unbound(), found))
    }

    /// A new element, not yet in the tree.
    fn create_node(slf: &Bound<'_, Self>, tag: &str) -> PyResult<Node> {
        if tag.is_empty() {
            return Err(PyValueError::new_err("tag name can't be empty"));
        }
        let id = slf.get().write(slf.py()).create_element(Ns::Html, tag);
        Ok(node(slf.py(), slf.as_unbound(), id))
    }

    fn merge_text_nodes(&self, py: Python<'_>) {
        let mut doc = self.write(py);
        let root = doc.root();
        merge_text_nodes(&mut doc, root);
    }

    /// Removes every element matching each of `tags`, with its contents.
    #[pyo3(signature = (tags, recursive = false))]
    fn strip_tags(&self, py: Python<'_>, tags: &Bound<'_, PyAny>, recursive: bool) -> PyResult<()> {
        let _ = recursive;
        let found = matches_of(py, self, tags, 0)?;
        let mut doc = self.write(py);
        found.into_iter().for_each(|n| doc.remove(n));
        Ok(())
    }

    /// Replaces every element matching each of `tags` with its contents.
    #[pyo3(signature = (tags, delete_empty = false))]
    fn unwrap_tags(
        &self,
        py: Python<'_>,
        tags: &Bound<'_, PyAny>,
        delete_empty: bool,
    ) -> PyResult<()> {
        let found = matches_of(py, self, tags, 0)?;
        let mut doc = self.write(py);
        found
            .into_iter()
            .for_each(|n| unwrap(&mut doc, n, delete_empty));
        Ok(())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!(
            "<HTMLParser {} nodes{}>",
            self.read(py).len(),
            if self.fragment { ", fragment" } else { "" }
        )
    }
}

impl Node {
    fn p(&self) -> &HtmlParser {
        self.parser.get()
    }

    fn at(&self, py: Python<'_>, id: Option<NodeId>) -> Option<Node> {
        id.map(|id| node(py, &self.parser, id))
    }

    /// The nodes `value` stands for in this document: new text for a
    /// string, else the node itself when `move` and it's already here,
    /// else a copy (a node moved from another document is taken out of it).
    fn incoming(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        move_: bool,
    ) -> PyResult<Vec<NodeId>> {
        if let Some(t) = text_of(value)? {
            return Ok(vec![self.p().write(py).create_text(&t)]);
        }
        let Ok(other) = value.cast::<Node>() else {
            return Err(PyTypeError::new_err("expected str, bytes or Node"));
        };
        let other = other.get();
        let same = other.parser.is(&self.parser);
        if same && move_ {
            return Ok(vec![other.id]);
        }
        let steps = record(&other.p().read(py), other.id);
        let made = build(&mut self.p().write(py), &steps);
        if move_ {
            other.p().write(py).remove(other.id);
        }
        Ok(made)
    }

    /// Puts `value` into `parent` before `before` (last for `None`).
    fn put(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        move_: bool,
        parent: NodeId,
        before: Option<NodeId>,
    ) -> PyResult<()> {
        let ids = self.incoming(py, value, move_)?;
        let mut doc = self.p().write(py);
        for id in ids {
            doc.insert_before(parent, id, before)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        Ok(())
    }
}

#[pymethods]
impl Node {
    /// The element's name; `-text`, `-comment`, `-doctype` or `-document`
    /// for those nodes.
    #[getter]
    fn tag(&self, py: Python<'_>) -> Option<String> {
        let doc = self.p().read(py);
        Some(
            match doc.kind(self.id) {
                Kind::Element { name, .. } => name,
                Kind::Text(_) => "-text",
                Kind::Comment(_) => "-comment",
                Kind::Doctype { .. } => "-doctype",
                Kind::Document => "-document",
                Kind::Fragment => "-document-fragment",
                Kind::Cdata(_) | Kind::ProcessingInstruction { .. } => return None,
            }
            .to_string(),
        )
    }

    /// The `id` attribute.
    #[getter]
    fn id(&self, py: Python<'_>) -> Option<String> {
        let doc = self.p().read(py);
        doc.attrs(self.id)
            .find(|&(ns, name, _)| ns == AttrNs::None && name == "id")
            .map(|(_, _, v)| v.to_string())
    }

    /// The attributes as a new `dict`, names as written (`xlink:href`).
    #[getter]
    fn attributes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        AttrCopy::with(self.p(), py, self.id, |attrs| {
            let dict = PyDict::new(py);
            for (n, v) in attrs.iter() {
                dict.set_item(n, v)?;
            }
            Ok(dict)
        })
    }

    /// The attributes as a live mapping: setting or deleting a key edits
    /// the element.
    #[getter]
    fn attrs(&self, py: Python<'_>) -> Attributes {
        Attributes {
            parser: self.parser.clone_ref(py),
            id: self.id,
        }
    }

    /// The node and its contents as markup, as `outerHTML`.
    #[getter]
    fn html(&self, py: Python<'_>) -> String {
        self.p().read(py).outer_html(self.id)
    }

    /// The contents as markup, as `innerHTML`; setting it parses the markup
    /// in this element's context and replaces the contents.
    #[getter]
    fn get_inner_html(&self, py: Python<'_>) -> String {
        self.p().read(py).inner_html(self.id)
    }

    #[setter]
    fn set_inner_html(&self, py: Python<'_>, html: &Bound<'_, PyAny>) -> PyResult<()> {
        let Some(html) = text_of(html)? else {
            return Err(PyTypeError::new_err("inner_html must be str or bytes"));
        };
        let (ns, context, target) = {
            let doc = self.p().read(py);
            let Kind::Element { ns, .. } = doc.kind(self.id) else {
                return Err(PyValueError::new_err(
                    "only an element has inner_html to set",
                ));
            };
            let name = doc.lower_name(self.id).unwrap_or("div").to_string();
            (ns, name, doc.template_contents(self.id).unwrap_or(self.id))
        };
        let scripting = self.p().scripting;
        let fragment = py.detach(|| {
            parsec::parse_fragment_with(
                &html,
                ns,
                &context,
                Options {
                    scripting,
                    ..Options::default()
                },
            )
        });
        let fragment = fragment.map_err(parse_error)?;
        let steps = record(&fragment, fragment.root());
        let mut doc = self.p().write(py);
        while let Some(child) = doc.first_child(target) {
            doc.remove(child);
        }
        for id in build(&mut doc, &steps) {
            doc.append(target, id)
                .map_err(|e| PyValueError::new_err(e.to_string()))?;
        }
        Ok(())
    }

    #[pyo3(signature = (deep = true, separator = "", strip = false, skip_empty = false))]
    fn text(
        &self,
        py: Python<'_>,
        deep: bool,
        separator: &str,
        strip: bool,
        skip_empty: bool,
    ) -> String {
        text(
            &self.p().read(py),
            self.id,
            deep,
            separator,
            strip,
            skip_empty,
        )
    }

    /// A text node's text; `None` for any other node.
    #[getter]
    fn text_content(&self, py: Python<'_>) -> Option<String> {
        match self.p().read(py).kind(self.id) {
            Kind::Text(t) => Some(t.to_string()),
            _ => None,
        }
    }

    /// A comment's text, stripped; `None` for any other node.
    #[getter]
    fn comment_content(&self, py: Python<'_>) -> Option<String> {
        match self.p().read(py).kind(self.id) {
            Kind::Comment(t) => Some(py_strip(t).to_string()),
            _ => None,
        }
    }

    /// The parent, or `None` at the top of the tree or of a fragment.
    #[getter]
    fn parent(&self, py: Python<'_>) -> Option<Node> {
        let doc = self.p().read(py);
        let parent = doc
            .parent(self.id)
            .filter(|&p| doc.kind(p) != Kind::Fragment);
        drop(doc);
        self.at(py, parent)
    }

    /// The next sibling, text and comments included.
    #[getter]
    fn next(&self, py: Python<'_>) -> Option<Node> {
        let id = self.p().read(py).next_sibling(self.id);
        self.at(py, id)
    }

    #[getter]
    fn prev(&self, py: Python<'_>) -> Option<Node> {
        let id = self.p().read(py).prev_sibling(self.id);
        self.at(py, id)
    }

    #[getter]
    fn first_child(&self, py: Python<'_>) -> Option<Node> {
        let id = self.p().read(py).first_child(self.id);
        self.at(py, id)
    }

    /// The first child, as selectolax's older name for it.
    #[getter]
    fn child(&self, py: Python<'_>) -> Option<Node> {
        self.first_child(py)
    }

    #[getter]
    fn last_child(&self, py: Python<'_>) -> Option<Node> {
        let id = self.p().read(py).last_child(self.id);
        self.at(py, id)
    }

    #[getter]
    fn parser(&self, py: Python<'_>) -> Py<HtmlParser> {
        self.parser.clone_ref(py)
    }

    #[getter]
    fn is_element_node(&self, py: Python<'_>) -> bool {
        is_element(&self.p().read(py), self.id)
    }

    #[getter]
    fn is_text_node(&self, py: Python<'_>) -> bool {
        is_text(&self.p().read(py), self.id)
    }

    #[getter]
    fn is_comment_node(&self, py: Python<'_>) -> bool {
        matches!(self.p().read(py).kind(self.id), Kind::Comment(_))
    }

    #[getter]
    fn is_document_node(&self, py: Python<'_>) -> bool {
        matches!(
            self.p().read(py).kind(self.id),
            Kind::Document | Kind::Fragment
        )
    }

    /// A text node of nothing but ASCII whitespace.
    #[getter]
    fn is_empty_text_node(&self, py: Python<'_>) -> bool {
        matches!(self.p().read(py).kind(self.id), Kind::Text(t) if ascii_space(t))
    }

    /// The children: elements only, unless `include_text`, which adds text
    /// and comments (whitespace-only text left out if `skip_empty`).
    #[pyo3(signature = (include_text = false, skip_empty = false))]
    fn iter<'py>(
        &self,
        py: Python<'py>,
        include_text: bool,
        skip_empty: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ids: Vec<NodeId> = {
            let doc = self.p().read(py);
            doc.children(self.id)
                .filter(|&n| keep(&doc, n, include_text, skip_empty))
                .collect()
        };
        Ok(PyList::new(py, nodes(py, &self.parser, ids))?
            .try_iter()?
            .into_any())
    }

    /// The node and everything under it in tree order, filtered as `iter`.
    #[pyo3(signature = (include_text = false, skip_empty = false))]
    fn traverse<'py>(
        &self,
        py: Python<'py>,
        include_text: bool,
        skip_empty: bool,
    ) -> PyResult<Bound<'py, PyAny>> {
        let ids: Vec<NodeId> = {
            let doc = self.p().read(py);
            subtree(&doc, self.id)
                .into_iter()
                .filter(|&n| keep(&doc, n, include_text, skip_empty))
                .collect()
        };
        Ok(PyList::new(py, nodes(py, &self.parser, ids))?
            .try_iter()?
            .into_any())
    }

    /// Runs an XPath 1.0 expression with this node as the context, as
    /// lxml's `element.xpath()` does; see `HTMLParser.xpath`.
    #[pyo3(signature = (query, **variables))]
    fn xpath(
        &self,
        py: Python<'_>,
        query: &str,
        variables: Option<&Bound<'_, PyDict>>,
    ) -> PyResult<Py<PyAny>> {
        let vars = self::variables(variables)?;
        self.p()
            .run_xpath(py, query, self.id, &vars)?
            .into_py(py, &self.parser)
    }

    /// Where the node was written, as `(start, end)` indexes into the input
    /// as it was passed, so slicing it gives the source: code points into
    /// a `str`, bytes into `bytes`. An element's start tag, or a whole text
    /// node or comment. `None` unless parsed with
    /// `positions=True`, and for nodes the parser implied or an edit made.
    #[getter]
    fn source_range(&self, py: Python<'_>) -> Option<(usize, usize)> {
        let r = self.p().read(py).span(self.id);
        r.map(|r| self.p().range(r))
    }

    /// Where an element's end tag was written, if it had one.
    #[getter]
    fn end_tag_range(&self, py: Python<'_>) -> Option<(usize, usize)> {
        let r = self.p().read(py).end_span(self.id);
        r.map(|r| self.p().range(r))
    }

    /// The line and column where the node starts, both from 1, as an
    /// editor counts them; `None` when `source_range` is.
    #[getter]
    fn position(&self, py: Python<'_>) -> Option<(usize, usize)> {
        let doc = self.p().read(py);
        doc.span(self.id).map(|r| doc.line_col(r.start))
    }

    /// Where the attribute named `name` was written, from its name to the
    /// end of its value.
    fn attribute_range(&self, py: Python<'_>, name: &str) -> Option<(usize, usize)> {
        let doc = self.p().read(py);
        let i = attr_index(&doc, self.id, name)?;
        doc.attr_span(self.id, i).map(|r| self.p().range(r))
    }

    /// The node, if it matches, then the matches under it.
    fn css(&self, py: Python<'_>, query: &str) -> PyResult<Vec<Node>> {
        let found = self.p().select(py, &compile(query)?, self.id, true);
        Ok(nodes(py, &self.parser, found))
    }

    #[pyo3(signature = (query, default = None, strict = false))]
    fn css_first(
        &self,
        py: Python<'_>,
        query: &str,
        default: Option<Py<PyAny>>,
        strict: bool,
    ) -> PyResult<Py<PyAny>> {
        let list = compile(query)?;
        let found = if strict {
            self.p().select(py, &list, self.id, true)
        } else {
            self.p()
                .select_first(py, &list, self.id, true)
                .into_iter()
                .collect()
        };
        first_or_default(py, &self.parser, found, default, strict)
    }

    /// Whether the node or anything under it matches.
    fn css_matches(&self, py: Python<'_>, selector: &str) -> PyResult<bool> {
        Ok(self
            .p()
            .select_first(py, &compile(selector)?, self.id, true)
            .is_some())
    }

    fn any_css_matches(&self, py: Python<'_>, selectors: &Bound<'_, PyAny>) -> PyResult<bool> {
        for s in selectors.try_iter()? {
            if self
                .p()
                .select_first(py, &compile(&s?.extract::<String>()?)?, self.id, true)
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Takes the node out of the tree. It stays readable, and can be
    /// inserted again.
    #[pyo3(signature = (recursive = true))]
    fn decompose(&self, py: Python<'_>, recursive: bool) {
        let _ = recursive;
        self.p().write(py).remove(self.id);
    }

    #[pyo3(signature = (recursive = true))]
    fn remove(&self, py: Python<'_>, recursive: bool) {
        self.decompose(py, recursive);
    }

    /// Replaces the element with its contents. An empty one stays unless
    /// `delete_empty`.
    #[pyo3(signature = (delete_empty = false))]
    fn unwrap(&self, py: Python<'_>, delete_empty: bool) {
        unwrap(&mut self.p().write(py), self.id, delete_empty);
    }

    #[pyo3(signature = (tags, delete_empty = false))]
    fn unwrap_tags(
        &self,
        py: Python<'_>,
        tags: &Bound<'_, PyAny>,
        delete_empty: bool,
    ) -> PyResult<()> {
        let found = matches_of(py, self.p(), tags, self.id)?;
        let mut doc = self.p().write(py);
        found
            .into_iter()
            .for_each(|n| unwrap(&mut doc, n, delete_empty));
        Ok(())
    }

    #[pyo3(signature = (tags, recursive = false))]
    fn strip_tags(&self, py: Python<'_>, tags: &Bound<'_, PyAny>, recursive: bool) -> PyResult<()> {
        let _ = recursive;
        let found = matches_of(py, self.p(), tags, self.id)?;
        let mut doc = self.p().write(py);
        found.into_iter().for_each(|n| doc.remove(n));
        Ok(())
    }

    /// Puts `value` in the node's place: a `str` as text, a node as a copy
    /// (moved instead if `move`).
    #[pyo3(signature = (value, r#move = false))]
    fn replace_with(&self, py: Python<'_>, value: &Bound<'_, PyAny>, r#move: bool) -> PyResult<()> {
        if let Ok(other) = value.cast::<Node>()
            && r#move
            && other.get().parser.is(&self.parser)
            && other.get().id == self.id
        {
            return Ok(());
        }
        let Some(parent) = self.p().read(py).parent(self.id) else {
            return Ok(());
        };
        self.put(py, value, r#move, parent, Some(self.id))?;
        self.p().write(py).remove(self.id);
        Ok(())
    }

    #[pyo3(signature = (value, r#move = false))]
    fn insert_before(
        &self,
        py: Python<'_>,
        value: &Bound<'_, PyAny>,
        r#move: bool,
    ) -> PyResult<()> {
        let Some(parent) = self.p().read(py).parent(self.id) else {
            return Ok(());
        };
        self.put(py, value, r#move, parent, Some(self.id))
    }

    #[pyo3(signature = (value, r#move = false))]
    fn insert_after(&self, py: Python<'_>, value: &Bound<'_, PyAny>, r#move: bool) -> PyResult<()> {
        let (parent, next) = {
            let doc = self.p().read(py);
            (doc.parent(self.id), doc.next_sibling(self.id))
        };
        let Some(parent) = parent else { return Ok(()) };
        self.put(py, value, r#move, parent, next)
    }

    /// Adds `value` after the node's last child.
    #[pyo3(signature = (value, r#move = false))]
    fn insert_child(&self, py: Python<'_>, value: &Bound<'_, PyAny>, r#move: bool) -> PyResult<()> {
        self.put(py, value, r#move, self.id, None)
    }

    fn merge_text_nodes(&self, py: Python<'_>) {
        merge_text_nodes(&mut self.p().write(py), self.id);
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Node>()
            .is_ok_and(|o| o.get().parser.is(&self.parser) && o.get().id == self.id)
    }

    fn __hash__(&self) -> u64 {
        (self.parser.as_ptr() as u64).rotate_left(32) ^ u64::from(self.id)
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        format!("<Node {}>", self.tag(py).unwrap_or_default())
    }
}

/// An element's attributes, names as written, copied into one buffer so
/// the lock is let go before any Python string is made: making one can run
/// a finalizer, and one that edits this document would wait on us forever.
/// The buffer is kept per thread, as allocating one per call cost more
/// than the copy.
#[derive(Default)]
struct AttrCopy {
    buf: String,
    /// Where each name ends and its value ends, in `buf`.
    ends: Vec<(usize, usize)>,
}

thread_local! {
    static ATTRS: std::cell::Cell<AttrCopy> = std::cell::Cell::default();
}

impl AttrCopy {
    fn with<R>(
        parser: &HtmlParser,
        py: Python<'_>,
        id: NodeId,
        f: impl FnOnce(&AttrCopy) -> R,
    ) -> R {
        // Taken, not borrowed: a finalizer that lands here gets a new one.
        let mut copy = ATTRS.take();
        copy.buf.clear();
        copy.ends.clear();
        for (_, n, v) in parser.read(py).attrs_written(id) {
            copy.buf.push_str(n);
            let name_end = copy.buf.len();
            copy.buf.push_str(v);
            copy.ends.push((name_end, copy.buf.len()));
        }
        let out = f(&copy);
        ATTRS.set(copy);
        out
    }

    fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        let starts = std::iter::once(0).chain(self.ends.iter().map(|&(_, v)| v));
        starts
            .zip(&self.ends)
            .map(|(start, &(n, v))| (&self.buf[start..n], &self.buf[n..v]))
    }
}

/// Whether `iter` and `traverse` yield `n`.
fn keep(doc: &Document, n: NodeId, include_text: bool, skip_empty: bool) -> bool {
    match doc.kind(n) {
        Kind::Element { .. } => true,
        Kind::Text(t) => include_text && !(skip_empty && ascii_space(t)),
        _ => include_text,
    }
}

impl Attributes {
    fn pairs(&self, py: Python<'_>) -> Vec<(String, String)> {
        self.parser
            .get()
            .read(py)
            .attrs_written(self.id)
            .map(|(_, n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    fn lookup(&self, py: Python<'_>, key: &str) -> Option<String> {
        let doc = self.parser.get().read(py);
        // HTML attribute names are stored lowercase, so look up as setAttribute does.
        let html = matches!(doc.kind(self.id), Kind::Element { ns: Ns::Html, .. });
        let key: Cow<str> = if html {
            key.to_ascii_lowercase().into()
        } else {
            key.into()
        };
        doc.attrs_written(self.id)
            .find(|&(_, n, _)| n == key)
            .map(|(_, _, v)| v.to_string())
    }
}

#[pymethods]
impl Attributes {
    fn __getitem__(&self, py: Python<'_>, key: &str) -> PyResult<String> {
        self.lookup(py, key)
            .ok_or_else(|| PyKeyError::new_err(key.to_string()))
    }

    /// Sets an attribute; `None` sets it empty, which is how it serializes.
    fn __setitem__(&self, py: Python<'_>, key: &str, value: Option<&str>) -> PyResult<()> {
        let mut doc = self.parser.get().write(py);
        if !is_element(&doc, self.id) {
            return Err(PyTypeError::new_err("only an element has attributes"));
        }
        doc.set_attr(self.id, key, value.unwrap_or(""));
        Ok(())
    }

    fn __delitem__(&self, py: Python<'_>, key: &str) -> PyResult<()> {
        if self.parser.get().write(py).remove_attr(self.id, key) {
            Ok(())
        } else {
            Err(PyKeyError::new_err(key.to_string()))
        }
    }

    fn __contains__(&self, py: Python<'_>, key: &str) -> bool {
        self.lookup(py, key).is_some()
    }

    fn __len__(&self, py: Python<'_>) -> usize {
        self.parser.get().read(py).attrs(self.id).count()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(PyList::new(py, self.keys(py))?.try_iter()?.into_any())
    }

    fn keys(&self, py: Python<'_>) -> Vec<String> {
        self.pairs(py).into_iter().map(|(n, _)| n).collect()
    }

    fn values(&self, py: Python<'_>) -> Vec<String> {
        self.pairs(py).into_iter().map(|(_, v)| v).collect()
    }

    fn items<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyTuple>>> {
        self.pairs(py)
            .into_iter()
            .map(|(n, v)| PyTuple::new(py, [n, v]))
            .collect()
    }

    #[pyo3(signature = (key, default = None))]
    fn get(&self, py: Python<'_>, key: &str, default: Option<Py<PyAny>>) -> PyResult<Py<PyAny>> {
        match self.lookup(py, key) {
            Some(v) => Ok(PyString::new(py, &v).into_any().unbind()),
            None => Ok(default.unwrap_or_else(|| py.None())),
        }
    }

    /// As `get`, with `""` as the default.
    #[pyo3(signature = (key, default = ""))]
    fn sget(&self, py: Python<'_>, key: &str, default: &str) -> String {
        self.lookup(py, key).unwrap_or_else(|| default.to_string())
    }

    fn __repr__(&self, py: Python<'_>) -> String {
        let doc = self.parser.get().read(py);
        let name = match doc.kind(self.id) {
            Kind::Element { name, .. } => name.to_string(),
            _ => String::new(),
        };
        format!("<{name} attributes, {} items>", doc.attrs(self.id).count())
    }
}

/// Parses a page given as `bytes` a piece at a time, as a browser does
/// while it downloads one: `feed()` each piece as it comes, then
/// `close()` for the `HTMLParser`, the same one `HTMLParser(b"".join(
/// pieces), ...)` gives, however the bytes were cut. The names are those
/// of lxml's feed parser and the standard library's `html.parser`.
#[pyclass(frozen, module = "parsec_html", name = "StreamParser")]
struct StreamParser {
    /// None once closed. Locked only while detached from the interpreter.
    state: Mutex<Option<Stream>>,
    scripting: bool,
    url: Option<String>,
}

struct Stream {
    parser: ByteParser,
    /// Every byte, for the offsets, when positions or errors are kept.
    bytes: Option<Vec<u8>>,
}

#[pymethods]
impl StreamParser {
    /// Takes what `HTMLParser` takes, but for the page and the fragment
    /// arguments, which a stream doesn't have.
    #[new]
    #[pyo3(signature = (scripting = false, budget = None, max_depth = None, url = None, positions = false, errors = false, transport_encoding = None, default_encoding = None))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        scripting: bool,
        budget: Option<f64>,
        max_depth: Option<usize>,
        url: Option<String>,
        positions: bool,
        errors: bool,
        transport_encoding: Option<&str>,
        default_encoding: Option<&str>,
    ) -> PyResult<Self> {
        let html = options(scripting, budget, max_depth, positions, errors)?;
        let options = byte_options(html, transport_encoding, default_encoding)?;
        Ok(StreamParser {
            state: Mutex::new(Some(Stream {
                parser: ByteParser::new(options),
                bytes: (positions || errors).then(Vec::new),
            })),
            scripting,
            url,
        })
    }

    /// The next piece of the page.
    fn feed(&self, py: Python<'_>, data: &[u8]) -> PyResult<()> {
        py.detach(|| {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            let stream = state.as_mut().ok_or_else(closed)?;
            stream.parser.feed(data);
            if let Some(kept) = &mut stream.bytes {
                kept.extend_from_slice(data);
            }
            Ok(())
        })
    }

    /// The end of the page: the parsed document. A parser closes once.
    fn close(&self, py: Python<'_>) -> PyResult<HtmlParser> {
        let (parsed, units) = py.detach(|| {
            let stream = self
                .state
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take()
                .ok_or_else(closed)?;
            let parsed = stream.parser.finish().map_err(parse_error)?;
            let units = match stream.bytes {
                Some(bytes) => Units::of_bytes(&bytes, parsed.encoding),
                None => Units::Same,
            };
            Ok::<_, PyErr>((parsed, units))
        })?;
        Ok(HtmlParser::made(
            parsed.document,
            false,
            self.scripting,
            self.url.clone(),
            units,
            Some((parsed.encoding, parsed.confidence)),
        ))
    }
}

fn closed() -> PyErr {
    PyValueError::new_err("the StreamParser is closed")
}

/// The encoding a browser set to the language `locale` (a BCP 47 tag such
/// as "ja", "ru-RU" or "zh-Hant") falls back to for a page that names
/// none, from the HTML Standard's table: pass it as `default_encoding`.
#[pyfunction]
fn encoding_for_locale(locale: &str) -> &'static str {
    parsec_encoding::locale_default(locale).name()
}

#[pymodule(gil_used = false)]
fn parsec_html(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<HtmlParser>()?;
    m.add_class::<Node>()?;
    m.add_class::<Attributes>()?;
    m.add_class::<SyntaxIssue>()?;
    m.add_class::<StreamParser>()?;
    m.add_function(wrap_pyfunction!(encoding_for_locale, m)?)?;
    m.add("SelectorError", m.py().get_type::<SelectorError>())?;
    m.add("XPathError", m.py().get_type::<XPathError>())?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
