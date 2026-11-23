// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XML for Node.js: bindings/wasm/src/xml.rs with Node-API in place of
//! wasm-bindgen, the same names, options and errors.

use std::rc::Rc;

use napi::JsValue;
use napi::bindgen_prelude::*;
use napi_derive::napi;
use parsec_xml::{Document, Encoding, LineIndex, NodeId, NodeKind, Options};

use crate::units::Units;
use crate::{Bytes, Maybe, ParseError, Span, error, maybe};

/// How `parseXml` and `parseXmlBytes` parse; the WebAssembly build's
/// `XmlParseOptions`.
#[napi(object)]
pub struct XmlParseOptions {
    pub namespaces: Option<bool>,
    pub max_depth: Option<f64>,
    pub max_entity_depth: Option<f64>,
    pub entity_allowance: Option<f64>,
    pub entity_ratio: Option<f64>,
}

struct Tree {
    doc: Document,
    /// Lines in the text the input was, or decoded to.
    lines: LineIndex<'static>,
    units: Option<Units>,
    /// What bytes were decoded from; None for a string.
    encoding: Option<&'static Encoding>,
}

impl Tree {
    fn span(&self, s: parsec_xml::Span) -> Span {
        let out = |o: u32| {
            self.units
                .as_ref()
                .map_or(o as usize, |u| u.out(o as usize)) as u32
        };
        let at = self.lines.line_col(s.start);
        Span {
            start: out(s.start),
            end: out(s.end),
            line: at.line,
            column: at.column,
        }
    }
}

/// A parsed XML document.
#[napi]
pub struct XmlDocument {
    doc: Rc<Tree>,
}

/// A node in an [`XmlDocument`].
#[napi]
pub struct XmlNode {
    doc: Rc<Tree>,
    id: NodeId,
}

/// An attribute of an element, as `XmlNode.attributes` gives them.
#[napi]
pub struct XmlAttribute {
    #[napi(readonly)]
    pub name: String,
    #[napi(readonly, js_name = "localName")]
    pub local_name: String,
    prefix: Option<String>,
    namespace_uri: Option<String>,
    #[napi(readonly)]
    pub value: String,
    #[napi(readonly)]
    pub specified: bool,
    span: Span,
}

#[napi]
impl XmlAttribute {
    #[napi(getter, catch_unwind)]
    pub fn prefix(&self) -> Maybe<String> {
        maybe(self.prefix.clone())
    }

    #[napi(getter, js_name = "namespaceURI", catch_unwind)]
    pub fn namespace_uri(&self) -> Maybe<String> {
        maybe(self.namespace_uri.clone())
    }

    /// From its name to its closing quote.
    #[napi(getter, catch_unwind)]
    pub fn span(&self) -> Span {
        self.span
    }
}

fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn options(o: Option<XmlParseOptions>) -> Result<Options> {
    let mut options = Options::default();
    let Some(o) = o else { return Ok(options) };
    let whole = |name: &str, n: Option<f64>, max: f64| match n {
        Some(n) if !((0.0..=max).contains(&n) && n.fract() == 0.0) => {
            Err(error(format!("{name} must be a whole number, 0 or more")))
        }
        n => Ok(n),
    };
    options.namespaces = o.namespaces.unwrap_or(true);
    if let Some(d) = whole("maxDepth", o.max_depth, u32::MAX.into())? {
        options.limits.max_depth = Some(d as u32);
    }
    if let Some(d) = whole("maxEntityDepth", o.max_entity_depth, u32::MAX.into())? {
        options.max_entity_depth = d as u32;
    }
    if let Some(n) = whole("entityAllowance", o.entity_allowance, u64::MAX as f64)? {
        options.entity_allowance = n as u64;
    }
    if let Some(n) = whole("entityRatio", o.entity_ratio, u64::MAX as f64)? {
        options.entity_ratio = n as u64;
    }
    Ok(options)
}

/// A SyntaxError with the error's `code`, `line` and `column`, as the
/// WebAssembly build throws.
fn syntax_error(env: &Env, e: &parsec_xml::Error) -> Error {
    let build = || -> Result<Error> {
        let ctor: Function<String, Unknown> =
            env.get_global()?.get_named_property("SyntaxError")?;
        let thrown = ctor.new_instance(e.diagnostic.message.to_string())?;
        let mut object = Object::from_unknown(thrown)?;
        object.set_named_property("code", e.code())?;
        object.set_named_property("line", e.position.line)?;
        object.set_named_property("column", e.position.column)?;
        Ok(Error::from(object.to_unknown()))
    };
    build().unwrap_or_else(|failed| failed)
}

fn document(
    doc: Document,
    text: String,
    units: Option<Units>,
    encoding: Option<&'static Encoding>,
) -> XmlDocument {
    XmlDocument {
        doc: Rc::new(Tree {
            doc,
            lines: LineIndex::owned(text),
            units,
            encoding,
        }),
    }
}

/// Parses an XML document given as a string.
#[napi(js_name = "parseXml", catch_unwind)]
pub fn parse_xml(
    env: Env,
    source: String,
    options: Option<XmlParseOptions>,
) -> Result<XmlDocument> {
    let options = self::options(options)?;
    let doc = parsec_xml::parse_with(&source, &options).map_err(|e| syntax_error(&env, &e))?;
    let units = Units::of_str(&source);
    Ok(document(doc, source, units, None))
}

/// Parses an XML document given as bytes, decoded as XML says.
#[napi(js_name = "parseXmlBytes", catch_unwind)]
pub fn parse_xml_bytes(
    env: Env,
    bytes: Bytes,
    options: Option<XmlParseOptions>,
) -> Result<XmlDocument> {
    let bytes = crate::bytes(&bytes);
    let options = self::options(options)?;
    let p = parsec_xml::parse_bytes(bytes, &options).map_err(|e| syntax_error(&env, &e))?;
    let units = Units::of_bytes(bytes, p.encoding);
    Ok(document(
        p.document,
        p.text.into_owned(),
        units,
        Some(p.encoding),
    ))
}

fn node(doc: &Rc<Tree>, id: Option<NodeId>) -> Maybe<XmlNode> {
    maybe(id.map(|id| XmlNode {
        doc: doc.clone(),
        id,
    }))
}

#[napi]
impl XmlDocument {
    /// The document node, the document element's parent.
    #[napi(getter, catch_unwind)]
    pub fn root(&self) -> XmlNode {
        XmlNode {
            doc: self.doc.clone(),
            id: self.doc.doc.root(),
        }
    }

    /// The document element.
    #[napi(getter, js_name = "documentElement", catch_unwind)]
    pub fn document_element(&self) -> XmlNode {
        XmlNode {
            doc: self.doc.clone(),
            id: self.doc.doc.root_element(),
        }
    }

    #[napi(getter, js_name = "characterSet", catch_unwind)]
    pub fn character_set(&self) -> String {
        self.doc.encoding.map_or("UTF-8", |e| e.name()).to_string()
    }

    #[napi(getter, catch_unwind)]
    pub fn warnings(&self) -> Vec<ParseError> {
        self.doc
            .doc
            .warnings()
            .iter()
            .map(|w| {
                let s = self.doc.span(w.span);
                ParseError {
                    code: w.code.to_string(),
                    offset: s.start,
                    line: s.line,
                    column: s.column,
                }
            })
            .collect()
    }

    #[napi(getter, catch_unwind)]
    pub fn xml(&self) -> String {
        self.doc.doc.to_xml()
    }
}

#[napi]
impl XmlNode {
    #[napi(getter, js_name = "nodeType", catch_unwind)]
    pub fn node_type(&self) -> u8 {
        match self.doc.doc.kind(self.id) {
            NodeKind::Element(_) => 1,
            NodeKind::Text(_) => 3,
            NodeKind::CData(_) => 4,
            NodeKind::EntityRef(_) => 5,
            NodeKind::ProcessingInstruction { .. } => 7,
            NodeKind::Comment(_) => 8,
            NodeKind::Document => 9,
            NodeKind::Doctype => 10,
        }
    }

    #[napi(getter, js_name = "nodeName", catch_unwind)]
    pub fn node_name(&self) -> String {
        let doc = &self.doc.doc;
        match doc.kind(self.id) {
            NodeKind::Element(name) => name.qname,
            NodeKind::Text(_) => "#text",
            NodeKind::CData(_) => "#cdata-section",
            NodeKind::Comment(_) => "#comment",
            NodeKind::Document => "#document",
            NodeKind::Doctype => doc.doctype().map_or("", |t| t.name.as_str()),
            NodeKind::ProcessingInstruction { target, .. } => target,
            NodeKind::EntityRef(name) => name,
        }
        .to_string()
    }

    #[napi(getter, js_name = "localName", catch_unwind)]
    pub fn local_name(&self) -> Maybe<String> {
        maybe(self.doc.doc.name(self.id).map(|n| n.local.to_string()))
    }

    #[napi(getter, catch_unwind)]
    pub fn prefix(&self) -> Maybe<String> {
        maybe(self.doc.doc.name(self.id).and_then(|n| some(n.prefix)))
    }

    #[napi(getter, js_name = "namespaceURI", catch_unwind)]
    pub fn namespace_uri(&self) -> Maybe<String> {
        maybe(self.doc.doc.name(self.id).and_then(|n| some(n.namespace)))
    }

    #[napi(getter, js_name = "nodeValue", catch_unwind)]
    pub fn node_value(&self) -> Maybe<String> {
        maybe(match self.doc.doc.kind(self.id) {
            NodeKind::Text(s) | NodeKind::CData(s) | NodeKind::Comment(s) => Some(s.to_string()),
            NodeKind::ProcessingInstruction { data, .. } => Some(data.to_string()),
            _ => None,
        })
    }

    #[napi(getter, js_name = "textContent", catch_unwind)]
    pub fn text_content(&self) -> String {
        self.doc.doc.text(self.id)
    }

    #[napi(getter, js_name = "parentNode", catch_unwind)]
    pub fn parent_node(&self) -> Maybe<XmlNode> {
        node(&self.doc, self.doc.doc.parent(self.id))
    }

    #[napi(getter, js_name = "firstChild", catch_unwind)]
    pub fn first_child(&self) -> Maybe<XmlNode> {
        node(&self.doc, self.doc.doc.first_child(self.id))
    }

    #[napi(getter, js_name = "lastChild", catch_unwind)]
    pub fn last_child(&self) -> Maybe<XmlNode> {
        node(&self.doc, self.doc.doc.last_child(self.id))
    }

    #[napi(getter, js_name = "nextSibling", catch_unwind)]
    pub fn next_sibling(&self) -> Maybe<XmlNode> {
        node(&self.doc, self.doc.doc.next_sibling(self.id))
    }

    #[napi(getter, js_name = "previousSibling", catch_unwind)]
    pub fn previous_sibling(&self) -> Maybe<XmlNode> {
        node(&self.doc, self.doc.doc.prev_sibling(self.id))
    }

    #[napi(getter, js_name = "childNodes", catch_unwind)]
    pub fn child_nodes(&self) -> Vec<XmlNode> {
        let doc = &self.doc.doc;
        doc.children(self.id)
            .map(|id| XmlNode {
                doc: self.doc.clone(),
                id,
            })
            .collect()
    }

    #[napi(getter, catch_unwind)]
    pub fn children(&self) -> Vec<XmlNode> {
        let doc = &self.doc.doc;
        doc.children(self.id)
            .filter(|&id| doc.is_element(id))
            .map(|id| XmlNode {
                doc: self.doc.clone(),
                id,
            })
            .collect()
    }

    #[napi(getter, catch_unwind)]
    pub fn attributes(&self) -> Vec<XmlAttribute> {
        self.doc
            .doc
            .attributes(self.id)
            .map(|a| XmlAttribute {
                name: a.name.qname.to_string(),
                local_name: a.name.local.to_string(),
                prefix: some(a.name.prefix),
                namespace_uri: some(a.name.namespace),
                value: a.value.to_string(),
                specified: a.specified,
                span: self.doc.span(a.span),
            })
            .collect()
    }

    #[napi(js_name = "getAttribute", catch_unwind)]
    pub fn get_attribute(&self, name: String) -> Maybe<String> {
        maybe(
            self.doc
                .doc
                .attribute_named(self.id, &name)
                .map(str::to_string),
        )
    }

    /// `namespace` null, undefined or `""` for none.
    #[napi(js_name = "getAttributeNS", catch_unwind)]
    pub fn get_attribute_ns(
        &self,
        namespace: Option<Either<String, Null>>,
        local_name: String,
    ) -> Maybe<String> {
        let ns = match &namespace {
            Some(Either::A(ns)) => ns.as_str(),
            _ => "",
        };
        maybe(
            self.doc
                .doc
                .attribute(self.id, ns, &local_name)
                .map(str::to_string),
        )
    }

    #[napi(js_name = "hasAttribute", catch_unwind)]
    pub fn has_attribute(&self, name: String) -> bool {
        self.doc.doc.attribute_named(self.id, &name).is_some()
    }

    #[napi(js_name = "getAttributeNames", catch_unwind)]
    pub fn get_attribute_names(&self) -> Vec<String> {
        self.doc
            .doc
            .attributes(self.id)
            .map(|a| a.name.qname.to_string())
            .collect()
    }

    #[napi(getter, catch_unwind)]
    pub fn xml(&self) -> String {
        let mut out = String::new();
        self.doc.doc.write_xml(self.id, &mut out);
        out
    }

    #[napi(getter, catch_unwind)]
    pub fn span(&self) -> Span {
        self.doc.span(self.doc.doc.span(self.id))
    }

    #[napi(js_name = "isSameNode", catch_unwind)]
    pub fn is_same_node(&self, other: &XmlNode) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc) && self.id == other.id
    }
}
