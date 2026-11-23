// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `parsec_html.xml`: XML for Python. `parse()` gives an `XMLDocument`,
//! whose nodes have the HTML parser's names where there are the same
//! things to name (`tag`, `parent`, `next`, `attributes`, `text()`,
//! `source_range`), plus what namespaces add.
//!
//! A parsed XML document can't be changed, so it needs no lock: any
//! number of threads read it at once, and a node is the document plus an
//! index.

use parsec_xml::{Document, LineIndex, NodeId, NodeKind, Options};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyString};

use crate::SyntaxIssue;
use crate::units::Units;

pyo3::create_exception!(
    parsec_html.xml,
    XMLSyntaxError,
    PyValueError,
    "XML that isn't well-formed, or that runs past a limit given to \
     parse(): `code` names the error, `message` says what's wrong, and \
     `line` and `column` (both from 1, the column in characters) say \
     where."
);

/// A parsed XML document. Its nodes stay valid as long as any of them
/// is held. Any number of threads may read it at once.
#[pyclass(frozen, module = "parsec_html.xml", name = "XMLDocument")]
pub struct XmlDocument {
    doc: Document,
    /// Lines in the text the input was, or decoded to.
    lines: LineIndex<'static>,
    /// Turns the parser's byte offsets into indexes of what was passed in.
    units: Units,
    /// What `bytes` were decoded from; None for a `str`.
    encoding: Option<&'static str>,
}

/// A node of an `XMLDocument`: an element, text, CDATA, a comment, a
/// processing instruction, the doctype, an entity reference, or the
/// document itself.
#[pyclass(frozen, module = "parsec_html.xml", name = "XMLNode")]
pub struct XmlNode {
    doc: Py<XmlDocument>,
    id: NodeId,
}

/// An attribute of an element, as `XMLNode.attribute_list` gives them.
#[pyclass(frozen, get_all, module = "parsec_html.xml", name = "XMLAttribute")]
pub struct XmlAttribute {
    /// The name as written: `xlink:href`.
    name: String,
    /// `href`.
    local_name: String,
    /// `xlink`, or None.
    prefix: Option<String>,
    /// The namespace URI the prefix is bound to, or None.
    namespace: Option<String>,
    /// Normalized as XML says: references replaced, white space made
    /// spaces.
    value: String,
    /// False for a default the DTD supplied.
    specified: bool,
}

#[pymethods]
impl XmlAttribute {
    fn __repr__(&self) -> String {
        format!("<XMLAttribute {}={:?}>", self.name, self.value)
    }
}

fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Parses an XML document. A `str` is parsed as it is; `bytes` are
/// decoded as XML says: from a byte order mark, else the encoding the
/// XML declaration names, else UTF-8, strictly. Nothing is ever fetched:
/// the external DTD and external entities are left unread, and
/// `warnings` says so.
///
/// `max_depth` (2,048 by default) bounds how deep elements nest;
/// `max_entity_depth` (16, at most 64) how deep entity references nest;
/// and expansion may add `entity_allowance` bytes (1 MiB) plus
/// `entity_ratio` (8) per byte of input. Past any of them, or for XML
/// that isn't well-formed, it raises `XMLSyntaxError`.
#[pyfunction]
#[pyo3(signature = (
    source, *, namespaces = true, max_depth = None, max_entity_depth = None,
    entity_allowance = None, entity_ratio = None,
))]
fn parse(
    py: Python<'_>,
    source: &Bound<'_, PyAny>,
    namespaces: bool,
    max_depth: Option<u32>,
    max_entity_depth: Option<u32>,
    entity_allowance: Option<u64>,
    entity_ratio: Option<u64>,
) -> PyResult<XmlDocument> {
    let mut options = Options {
        namespaces,
        ..Options::default()
    };
    if max_depth.is_some() {
        options.limits.max_depth = max_depth;
    }
    options.max_entity_depth = max_entity_depth.unwrap_or(options.max_entity_depth);
    options.entity_allowance = entity_allowance.unwrap_or(options.entity_allowance);
    options.entity_ratio = entity_ratio.unwrap_or(options.entity_ratio);
    let parsed = if let Ok(s) = source.cast::<PyString>() {
        let src = s.to_cow()?;
        py.detach(|| {
            parsec_xml::parse_with(&src, &options).map(|doc| XmlDocument {
                doc,
                units: Units::of_str(&src),
                lines: LineIndex::owned(src.to_string()),
                encoding: None,
            })
        })
    } else if let Ok(b) = source.cast::<PyBytes>() {
        let bytes = b.as_bytes();
        py.detach(|| {
            parsec_xml::parse_bytes(bytes, &options).map(|p| XmlDocument {
                doc: p.document,
                units: Units::of_bytes(bytes, p.encoding),
                lines: LineIndex::owned(p.text.into_owned()),
                encoding: Some(p.encoding.name()),
            })
        })
    } else {
        return Err(pyo3::exceptions::PyTypeError::new_err(
            "parse() takes str or bytes",
        ));
    };
    parsed.map_err(|e| syntax_error(py, &e))
}

fn syntax_error(py: Python<'_>, e: &parsec_xml::Error) -> PyErr {
    let at = e.position;
    let message = &*e.diagnostic.message;
    let err = XMLSyntaxError::new_err(format!("{}:{}: {message}", at.line, at.column));
    let value = err.value(py);
    let set = || -> PyResult<()> {
        value.setattr("code", e.code())?;
        value.setattr("message", message)?;
        value.setattr("line", at.line)?;
        value.setattr("column", at.column)
    };
    match set() {
        Ok(()) => err,
        Err(e) => e,
    }
}

fn node(doc: &Bound<'_, XmlDocument>, id: Option<NodeId>) -> Option<XmlNode> {
    id.map(|id| XmlNode {
        doc: doc.clone().unbind(),
        id,
    })
}

impl XmlDocument {
    fn position(&self, offset: u32) -> (u32, u32) {
        let at = self.lines.line_col(offset);
        (at.line, at.column)
    }
}

#[pymethods]
impl XmlDocument {
    /// The document element.
    #[getter]
    fn root(slf: &Bound<'_, Self>) -> Option<XmlNode> {
        node(slf, Some(slf.get().doc.root_element()))
    }

    /// The document node, the document element's parent.
    #[getter]
    fn document(slf: &Bound<'_, Self>) -> Option<XmlNode> {
        node(slf, Some(slf.get().doc.root()))
    }

    /// The Encoding Standard's name for what `bytes` were decoded from,
    /// such as "UTF-8" or "windows-1252"; None for a `str`.
    #[getter]
    fn encoding(&self) -> Option<&'static str> {
        self.encoding
    }

    /// What the parser didn't read and says so, as the XML spec asks of
    /// a processor that doesn't validate: the external DTD, an external
    /// entity. None of them is an error.
    #[getter]
    fn warnings(&self) -> Vec<SyntaxIssue> {
        self.doc
            .warnings()
            .iter()
            .map(|w| {
                let (line, column) = self.position(w.span.start);
                SyntaxIssue {
                    code: w.code,
                    offset: self.units.out(w.span.start as usize),
                    line: line as usize,
                    column: column as usize,
                }
            })
            .collect()
    }

    /// The whole document as XML, its XML declaration and `<!DOCTYPE>`
    /// included.
    #[getter]
    fn xml(&self) -> String {
        self.doc.to_xml()
    }

    /// The line and column, from 1, of an index into the input, as
    /// `source_range` counts.
    fn line_col(&self, offset: usize) -> (u32, u32) {
        let offset = self.units.back(offset).min(u32::MAX as usize);
        self.position(offset as u32)
    }

    fn __repr__(&self) -> String {
        let root = self.doc.root_element();
        format!(
            "<XMLDocument {}>",
            self.doc.name(root).map_or("", |n| n.qname)
        )
    }
}

impl XmlNode {
    fn d(&self) -> &XmlDocument {
        self.doc.get()
    }

    fn link(
        &self,
        py: Python<'_>,
        step: fn(&Document, NodeId) -> Option<NodeId>,
    ) -> Option<XmlNode> {
        node(self.doc.bind(py), step(&self.d().doc, self.id))
    }
}

#[pymethods]
impl XmlNode {
    /// What the node is: `"element"`, `"text"`, `"cdata"`, `"comment"`,
    /// `"processing-instruction"`, `"doctype"`, `"entity-reference"` (to
    /// an entity Parsec didn't read) or `"document"`.
    #[getter]
    fn kind(&self) -> &'static str {
        match self.d().doc.kind(self.id) {
            NodeKind::Document => "document",
            NodeKind::Doctype => "doctype",
            NodeKind::Element(_) => "element",
            NodeKind::Text(_) => "text",
            NodeKind::CData(_) => "cdata",
            NodeKind::Comment(_) => "comment",
            NodeKind::ProcessingInstruction { .. } => "processing-instruction",
            NodeKind::EntityRef(_) => "entity-reference",
        }
    }

    /// An element's name as written, prefix and all (`atom:link`); a
    /// processing instruction's target, the doctype's name, or the entity
    /// an entity reference names. None for any other node.
    #[getter]
    fn tag(&self) -> Option<&str> {
        let doc = &self.d().doc;
        match doc.kind(self.id) {
            NodeKind::Element(name) => Some(name.qname),
            NodeKind::Doctype => doc.doctype().map(|t| t.name.as_str()),
            NodeKind::ProcessingInstruction { target, .. } => Some(target),
            NodeKind::EntityRef(name) => Some(name),
            _ => None,
        }
    }

    /// An element's local name: `link` for `atom:link`.
    #[getter]
    fn local_name(&self) -> Option<&str> {
        self.d().doc.name(self.id).map(|n| n.local)
    }

    /// An element's prefix: `atom` for `atom:link`, None for none.
    #[getter]
    fn prefix(&self) -> Option<&str> {
        self.d()
            .doc
            .name(self.id)
            .map(|n| n.prefix)
            .filter(|p| !p.is_empty())
    }

    /// The namespace URI an element's name is in, or None.
    #[getter]
    fn namespace(&self) -> Option<&str> {
        let name = self.d().doc.name(self.id)?;
        Some(name.namespace).filter(|ns| !ns.is_empty())
    }

    /// A text, CDATA or comment node's text, or a processing
    /// instruction's data; None for any other node.
    #[getter]
    fn value(&self) -> Option<&str> {
        match self.d().doc.kind(self.id) {
            NodeKind::Text(s) | NodeKind::CData(s) | NodeKind::Comment(s) => Some(s),
            NodeKind::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        }
    }

    /// The attributes as a new `dict`, names as written (`xlink:href`),
    /// namespace declarations included.
    #[getter]
    fn attributes<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let dict = PyDict::new(py);
        for a in self.d().doc.attributes(self.id) {
            dict.set_item(a.name.qname, a.value)?;
        }
        Ok(dict)
    }

    /// The attributes with their namespaces, in the order of the start
    /// tag, then the defaults the DTD supplied.
    #[getter]
    fn attribute_list(&self) -> Vec<XmlAttribute> {
        self.d()
            .doc
            .attributes(self.id)
            .map(|a| XmlAttribute {
                name: a.name.qname.to_string(),
                local_name: a.name.local.to_string(),
                prefix: some(a.name.prefix),
                namespace: some(a.name.namespace),
                value: a.value.to_string(),
                specified: a.specified,
            })
            .collect()
    }

    /// The value of the attribute with this local name in `namespace`
    /// (None for no namespace), whatever its prefix; None if there's none.
    #[pyo3(signature = (local_name, namespace = None))]
    fn attribute(&self, local_name: &str, namespace: Option<&str>) -> Option<&str> {
        self.d()
            .doc
            .attribute(self.id, namespace.unwrap_or(""), local_name)
    }

    /// The text of the node and everything under it: character data and
    /// CDATA, in order.
    fn text(&self) -> String {
        self.d().doc.text(self.id)
    }

    /// The node and everything under it as XML. An element takes along
    /// the namespace declarations it's in the scope of, so it means the
    /// same on its own.
    #[getter]
    fn xml(&self) -> String {
        let mut out = String::new();
        self.d().doc.write_xml(self.id, &mut out);
        out
    }

    #[getter]
    fn parent(&self, py: Python<'_>) -> Option<XmlNode> {
        self.link(py, Document::parent)
    }

    #[getter]
    fn next(&self, py: Python<'_>) -> Option<XmlNode> {
        self.link(py, Document::next_sibling)
    }

    #[getter]
    fn prev(&self, py: Python<'_>) -> Option<XmlNode> {
        self.link(py, Document::prev_sibling)
    }

    #[getter]
    fn first_child(&self, py: Python<'_>) -> Option<XmlNode> {
        self.link(py, Document::first_child)
    }

    #[getter]
    fn last_child(&self, py: Python<'_>) -> Option<XmlNode> {
        self.link(py, Document::last_child)
    }

    /// Every child, of every kind.
    #[getter]
    fn children(&self, py: Python<'_>) -> Vec<XmlNode> {
        let doc = self.doc.bind(py);
        self.d()
            .doc
            .children(self.id)
            .filter_map(|id| node(doc, Some(id)))
            .collect()
    }

    /// The document the node is in.
    #[getter]
    fn document(&self, py: Python<'_>) -> Py<XmlDocument> {
        self.doc.clone_ref(py)
    }

    /// Where the node was written, as `(start, end)` indexes into the
    /// input (a `str`'s characters, or bytes): an element from its start
    /// tag's `<` to its end tag's `>`.
    #[getter]
    fn source_range(&self) -> (usize, usize) {
        let span = self.d().doc.span(self.id);
        let units = &self.d().units;
        (units.out(span.start as usize), units.out(span.end as usize))
    }

    /// The line and column where the node starts, both from 1.
    #[getter]
    fn position(&self) -> (u32, u32) {
        self.d().position(self.d().doc.span(self.id).start)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<XmlNode>()
            .is_ok_and(|o| o.get().doc.is(&self.doc) && o.get().id == self.id)
    }

    fn __hash__(&self) -> u64 {
        (self.doc.as_ptr() as u64).rotate_left(32) ^ self.id.index() as u64
    }

    fn __repr__(&self) -> String {
        format!("<XMLNode {}>", self.tag().unwrap_or(self.kind()))
    }
}

/// Adds `parsec_html.xml` to `parent`, importable as a module too.
pub fn add(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "parsec_html.xml")?;
    m.add_function(wrap_pyfunction!(parse, &m)?)?;
    m.add_class::<XmlDocument>()?;
    m.add_class::<XmlNode>()?;
    m.add_class::<XmlAttribute>()?;
    m.add("XMLSyntaxError", py.get_type::<XMLSyntaxError>())?;
    parent.add("xml", &m)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("parsec_html.xml", &m)
}
