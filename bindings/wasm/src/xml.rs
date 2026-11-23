// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XML in WebAssembly: `parseXml` and `parseXmlBytes` give an
//! `XmlDocument`, whose nodes have the DOM's names (`nodeName`,
//! `namespaceURI`, `getAttributeNS`) as the HTML `Node` does. An XML
//! document can't be changed. bindings/node/src/xml.rs is the same API
//! for the native build.

use std::rc::Rc;

use parsec_xml::{Document, Encoding, LineIndex, NodeId, NodeKind, Options};
use wasm_bindgen::prelude::*;

use crate::units::Units;
use crate::{ParseError, Span};

#[wasm_bindgen(typescript_custom_section)]
const XML_OPTIONS: &str = r#"
/** How `parseXml` and `parseXmlBytes` parse. */
export interface XmlParseOptions {
    /** Namespaces in XML, on unless `false`. Off, a name is any XML
     * name, colons and all, in no namespace. */
    namespaces?: boolean;
    /** How deep elements may nest: 2,048 unless given. */
    maxDepth?: number;
    /** How deep entity references may nest: 16 unless given, at most 64. */
    maxEntityDepth?: number;
    /** Bytes entity expansion may add to any document: 1 MiB unless
     * given... */
    entityAllowance?: number;
    /** ...plus this many per byte of input: 8 unless given. */
    entityRatio?: number;
}

/**
 * What `parseXml` and `parseXmlBytes` throw for XML that isn't
 * well-formed or runs past a limit: a SyntaxError with Parsec's code for
 * the error, and the `line` and `column` (both from 1, the column in
 * characters) where it is.
 */
export interface XmlSyntaxError extends SyntaxError {
    code: string;
    line: number;
    column: number;
}
"#;

#[wasm_bindgen]
extern "C" {
    /// A plain object, read with the getters below.
    #[wasm_bindgen(typescript_type = "XmlParseOptions")]
    pub type XmlParseOptions;
    #[wasm_bindgen(method, getter)]
    fn namespaces(this: &XmlParseOptions) -> Option<bool>;
    #[wasm_bindgen(method, getter, js_name = maxDepth)]
    fn max_depth(this: &XmlParseOptions) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = maxEntityDepth)]
    fn max_entity_depth(this: &XmlParseOptions) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = entityAllowance)]
    fn entity_allowance(this: &XmlParseOptions) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = entityRatio)]
    fn entity_ratio(this: &XmlParseOptions) -> Option<f64>;

    #[wasm_bindgen(js_name = SyntaxError)]
    type SyntaxError;
    #[wasm_bindgen(constructor, js_class = "SyntaxError")]
    fn new(message: &str) -> SyntaxError;
    #[wasm_bindgen(method, setter = code)]
    fn set_code(this: &SyntaxError, code: &str);
    #[wasm_bindgen(method, setter = line)]
    fn set_line(this: &SyntaxError, line: u32);
    #[wasm_bindgen(method, setter = column)]
    fn set_column(this: &SyntaxError, column: u32);
}

struct Tree {
    doc: Document,
    /// Lines in the text the input was, or decoded to.
    lines: LineIndex<'static>,
    /// Turns the parser's byte offsets into UTF-16 indexes of a string,
    /// or byte offsets into bytes; None when they're the same.
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
#[wasm_bindgen]
pub struct XmlDocument {
    doc: Rc<Tree>,
}

/// A node in an [`XmlDocument`]: an element, text, a CDATA section, a
/// comment, a processing instruction, the doctype, an entity reference,
/// or the document itself.
#[wasm_bindgen]
pub struct XmlNode {
    doc: Rc<Tree>,
    id: NodeId,
}

/// An attribute of an element, as `XmlNode.attributes` gives them.
#[wasm_bindgen]
pub struct XmlAttribute {
    /// The name as written: `xlink:href`.
    #[wasm_bindgen(readonly, getter_with_clone)]
    pub name: String,
    /// `href`.
    #[wasm_bindgen(readonly, getter_with_clone, js_name = localName)]
    pub local_name: String,
    /// `xlink`, or undefined.
    #[wasm_bindgen(readonly, getter_with_clone)]
    pub prefix: Option<String>,
    /// The namespace the prefix is bound to, or undefined.
    #[wasm_bindgen(readonly, getter_with_clone, js_name = namespaceURI)]
    pub namespace_uri: Option<String>,
    /// Normalized as XML says: references replaced, white space made
    /// spaces.
    #[wasm_bindgen(readonly, getter_with_clone)]
    pub value: String,
    /// False for a default the DTD supplied.
    #[wasm_bindgen(readonly)]
    pub specified: bool,
    /// From its name to its closing quote.
    #[wasm_bindgen(readonly)]
    pub span: Span,
}

fn some(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

fn options(o: Option<&XmlParseOptions>) -> Result<Options, JsValue> {
    let mut options = Options::default();
    let Some(o) = o else { return Ok(options) };
    let whole = |name: &str, n: Option<f64>, max: f64| match n {
        Some(n) if !((0.0..=max).contains(&n) && n.fract() == 0.0) => Err(JsValue::from(
            JsError::new(&format!("{name} must be a whole number, 0 or more")),
        )),
        n => Ok(n),
    };
    options.namespaces = o.namespaces().unwrap_or(true);
    if let Some(d) = whole("maxDepth", o.max_depth(), u32::MAX.into())? {
        options.limits.max_depth = Some(d as u32);
    }
    if let Some(d) = whole("maxEntityDepth", o.max_entity_depth(), u32::MAX.into())? {
        options.max_entity_depth = d as u32;
    }
    if let Some(n) = whole("entityAllowance", o.entity_allowance(), u64::MAX as f64)? {
        options.entity_allowance = n as u64;
    }
    if let Some(n) = whole("entityRatio", o.entity_ratio(), u64::MAX as f64)? {
        options.entity_ratio = n as u64;
    }
    Ok(options)
}

fn syntax_error(e: &parsec_xml::Error) -> JsValue {
    let err = SyntaxError::new(&e.diagnostic.message);
    err.set_code(e.code());
    err.set_line(e.position.line);
    err.set_column(e.position.column);
    err.into()
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

/// Parses an XML document given as a string. Throws an `XmlSyntaxError`
/// for XML that isn't well-formed or runs past a limit in `options`.
/// Nothing is ever fetched: the external DTD and external entities are
/// left unread, and `warnings` says so.
#[wasm_bindgen(js_name = parseXml)]
pub fn parse_xml(source: &str, options: Option<XmlParseOptions>) -> Result<XmlDocument, JsValue> {
    let options = self::options(options.as_ref())?;
    let doc = parsec_xml::parse_with(source, &options).map_err(|e| syntax_error(&e))?;
    Ok(document(
        doc,
        source.to_string(),
        Units::of_str(source),
        None,
    ))
}

/// Parses an XML document given as bytes, decoded as XML says: from a
/// byte order mark, else the encoding the XML declaration names, else
/// UTF-8, strictly. Spans are byte offsets into the bytes.
#[wasm_bindgen(js_name = parseXmlBytes)]
pub fn parse_xml_bytes(
    bytes: &[u8],
    options: Option<XmlParseOptions>,
) -> Result<XmlDocument, JsValue> {
    let options = self::options(options.as_ref())?;
    let p = parsec_xml::parse_bytes(bytes, &options).map_err(|e| syntax_error(&e))?;
    let units = Units::of_bytes(bytes, p.encoding);
    Ok(document(
        p.document,
        p.text.into_owned(),
        units,
        Some(p.encoding),
    ))
}

fn node(doc: &Rc<Tree>, id: Option<NodeId>) -> Option<XmlNode> {
    id.map(|id| XmlNode {
        doc: doc.clone(),
        id,
    })
}

#[wasm_bindgen]
impl XmlDocument {
    /// The document node, the document element's parent.
    #[wasm_bindgen(getter)]
    pub fn root(&self) -> XmlNode {
        XmlNode {
            doc: self.doc.clone(),
            id: self.doc.doc.root(),
        }
    }

    /// The document element.
    #[wasm_bindgen(getter, js_name = documentElement)]
    pub fn document_element(&self) -> XmlNode {
        XmlNode {
            doc: self.doc.clone(),
            id: self.doc.doc.root_element(),
        }
    }

    /// What bytes were decoded from, by the Encoding Standard's name
    /// (`"windows-1252"`); `"UTF-8"` for a string.
    #[wasm_bindgen(getter, js_name = characterSet)]
    pub fn character_set(&self) -> String {
        self.doc.encoding.map_or("UTF-8", |e| e.name()).to_string()
    }

    /// What the parser didn't read and says so, as the XML spec asks of
    /// a processor that doesn't validate: the external DTD, an external
    /// entity. None of them is an error.
    #[wasm_bindgen(getter)]
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

    /// The whole document as XML, its XML declaration and `<!DOCTYPE>`
    /// included.
    #[wasm_bindgen(getter)]
    pub fn xml(&self) -> String {
        self.doc.doc.to_xml()
    }
}

#[wasm_bindgen]
impl XmlNode {
    /// The DOM's `nodeType`: 1 for an element, 3 text, 4 a CDATA
    /// section, 5 a reference to an entity Parsec didn't read, 7 a
    /// processing instruction, 8 a comment, 9 the document, 10 the
    /// doctype.
    #[wasm_bindgen(getter, js_name = nodeType)]
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

    /// The DOM's `nodeName`: an element's name as written (`atom:link`),
    /// `#text`, `#cdata-section`, `#comment`, `#document`, the doctype's
    /// name, a processing instruction's target, or an entity's name.
    #[wasm_bindgen(getter, js_name = nodeName)]
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

    /// An element's local name: `link` for `atom:link`.
    #[wasm_bindgen(getter, js_name = localName)]
    pub fn local_name(&self) -> Option<String> {
        self.doc.doc.name(self.id).map(|n| n.local.to_string())
    }

    /// An element's prefix: `atom` for `atom:link`.
    #[wasm_bindgen(getter)]
    pub fn prefix(&self) -> Option<String> {
        self.doc.doc.name(self.id).and_then(|n| some(n.prefix))
    }

    /// The namespace an element's name is in.
    #[wasm_bindgen(getter, js_name = namespaceURI)]
    pub fn namespace_uri(&self) -> Option<String> {
        self.doc.doc.name(self.id).and_then(|n| some(n.namespace))
    }

    /// A text, CDATA or comment node's text, or a processing
    /// instruction's data.
    #[wasm_bindgen(getter, js_name = nodeValue)]
    pub fn node_value(&self) -> Option<String> {
        match self.doc.doc.kind(self.id) {
            NodeKind::Text(s) | NodeKind::CData(s) | NodeKind::Comment(s) => Some(s.to_string()),
            NodeKind::ProcessingInstruction { data, .. } => Some(data.to_string()),
            _ => None,
        }
    }

    /// The text of the node and everything under it: character data and
    /// CDATA, in order.
    #[wasm_bindgen(getter, js_name = textContent)]
    pub fn text_content(&self) -> String {
        self.doc.doc.text(self.id)
    }

    #[wasm_bindgen(getter, js_name = parentNode)]
    pub fn parent_node(&self) -> Option<XmlNode> {
        node(&self.doc, self.doc.doc.parent(self.id))
    }

    #[wasm_bindgen(getter, js_name = firstChild)]
    pub fn first_child(&self) -> Option<XmlNode> {
        node(&self.doc, self.doc.doc.first_child(self.id))
    }

    #[wasm_bindgen(getter, js_name = lastChild)]
    pub fn last_child(&self) -> Option<XmlNode> {
        node(&self.doc, self.doc.doc.last_child(self.id))
    }

    #[wasm_bindgen(getter, js_name = nextSibling)]
    pub fn next_sibling(&self) -> Option<XmlNode> {
        node(&self.doc, self.doc.doc.next_sibling(self.id))
    }

    #[wasm_bindgen(getter, js_name = previousSibling)]
    pub fn previous_sibling(&self) -> Option<XmlNode> {
        node(&self.doc, self.doc.doc.prev_sibling(self.id))
    }

    /// Every child, of every kind.
    #[wasm_bindgen(getter, js_name = childNodes)]
    pub fn child_nodes(&self) -> Vec<XmlNode> {
        let doc = &self.doc.doc;
        doc.children(self.id)
            .filter_map(|id| node(&self.doc, Some(id)))
            .collect()
    }

    /// The children that are elements.
    #[wasm_bindgen(getter)]
    pub fn children(&self) -> Vec<XmlNode> {
        let doc = &self.doc.doc;
        doc.children(self.id)
            .filter(|&id| doc.is_element(id))
            .filter_map(|id| node(&self.doc, Some(id)))
            .collect()
    }

    /// The attributes with their namespaces, in the order of the start
    /// tag, then the defaults the DTD supplied; namespace declarations
    /// included.
    #[wasm_bindgen(getter)]
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

    /// The value of the attribute with this name as written
    /// (`xlink:href`).
    #[wasm_bindgen(js_name = getAttribute)]
    pub fn get_attribute(&self, name: &str) -> Option<String> {
        self.doc
            .doc
            .attribute_named(self.id, name)
            .map(str::to_string)
    }

    /// The value of the attribute with this local name in `namespace`
    /// (null or `""` for none), whatever its prefix.
    #[wasm_bindgen(js_name = getAttributeNS)]
    pub fn get_attribute_ns(&self, namespace: Option<String>, local_name: &str) -> Option<String> {
        let ns = namespace.as_deref().unwrap_or("");
        self.doc
            .doc
            .attribute(self.id, ns, local_name)
            .map(str::to_string)
    }

    #[wasm_bindgen(js_name = hasAttribute)]
    pub fn has_attribute(&self, name: &str) -> bool {
        self.doc.doc.attribute_named(self.id, name).is_some()
    }

    /// The attributes' names as written, in order.
    #[wasm_bindgen(js_name = getAttributeNames)]
    pub fn get_attribute_names(&self) -> Vec<String> {
        self.doc
            .doc
            .attributes(self.id)
            .map(|a| a.name.qname.to_string())
            .collect()
    }

    /// The node and everything under it as XML. An element takes along
    /// the namespace declarations it's in the scope of, so it means the
    /// same on its own.
    #[wasm_bindgen(getter)]
    pub fn xml(&self) -> String {
        let mut out = String::new();
        self.doc.doc.write_xml(self.id, &mut out);
        out
    }

    /// Where the node was written: an element from its start tag's `<` to
    /// its end tag's `>`.
    #[wasm_bindgen(getter)]
    pub fn span(&self) -> Span {
        self.doc.span(self.doc.doc.span(self.id))
    }

    /// Whether `other` is this same node.
    #[wasm_bindgen(js_name = isSameNode)]
    pub fn is_same_node(&self, other: &XmlNode) -> bool {
        Rc::ptr_eq(&self.doc, &other.doc) && self.id == other.id
    }
}
