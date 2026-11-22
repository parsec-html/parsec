// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The document: an arena of nodes linked by index, the attributes in one
//! vector, every string in one buffer, and names interned once each.

use std::collections::HashMap;
use std::fmt;

use parsec_base::{Diagnostic, Span};

/// The namespace the `xml` prefix is bound to, always.
pub const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
/// The namespace of `xmlns` and `xmlns:*` attributes.
pub const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

pub(crate) const NONE: u32 = u32::MAX;
/// Namespace ids: 0 is no namespace.
pub(crate) const NS_XML: u32 = 1;
pub(crate) const NS_XMLNS: u32 = 2;

/// A node of a [`Document`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub(crate) u32);

impl NodeId {
    /// Its place in the document's arena, which is document order.
    pub fn index(self) -> usize {
        self.0 as usize
    }

    /// The node at `index` in the arena: what [`NodeId::index`] gives
    /// back, and what an XPath result names a node by. It's only a node of
    /// the document whose `index` it came from.
    ///
    /// # Panics
    ///
    /// If `index` is over `u32::MAX`, which no document's ids reach.
    pub fn from_index(index: usize) -> NodeId {
        NodeId(u32::try_from(index).expect("a node index fits in a u32"))
    }
}

/// A range of [`Document`]'s string buffer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Str {
    pub start: u32,
    pub len: u32,
}

impl Str {
    pub fn end(self) -> u32 {
        self.start + self.len
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Data {
    Document,
    Doctype,
    Element {
        name: u32,
        ns: u32,
        attrs: u32,
        attrs_len: u32,
    },
    Text(Str),
    Cdata(Str),
    Comment(Str),
    Pi {
        target: Str,
        data: Str,
    },
    EntityRef(Str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Node {
    pub parent: u32,
    pub first: u32,
    pub last: u32,
    pub prev: u32,
    pub next: u32,
    pub span: Span,
    pub data: Data,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct AttrData {
    pub name: u32,
    pub ns: u32,
    pub value: Str,
    pub span: Span,
    pub specified: bool,
}

/// Prefix ids: 0 is no prefix.
pub(crate) const PREFIX_XML: u32 = 1;
pub(crate) const PREFIX_XMLNS: u32 = 2;

#[derive(Debug)]
pub(crate) struct QName {
    pub qname: Box<str>,
    /// Where the colon is: `NONE` for none, or with namespaces off.
    pub colon: u32,
    pub prefix: u32,
}

/// Element and attribute names, prefixes and namespace URIs, each stored
/// once.
#[derive(Debug)]
pub(crate) struct Names {
    qnames: Vec<QName>,
    qname_ids: HashMap<Box<str>, u32>,
    prefix_ids: HashMap<Box<str>, u32>,
    uris: Vec<Box<str>>,
    uri_ids: HashMap<Box<str>, u32>,
}

/// Names a document can have before they're looked up by hash.
const SCAN: usize = 16;

/// The URIs and prefixes every document has, which take no allocation.
const BUILTIN_URIS: [&str; 3] = ["", XML_NS, XMLNS_NS];
const BUILTIN_PREFIXES: [&str; 3] = ["", "xml", "xmlns"];

impl Names {
    pub fn new() -> Names {
        Names {
            qnames: Vec::new(),
            qname_ids: HashMap::new(),
            prefix_ids: HashMap::new(),
            uris: Vec::new(),
            uri_ids: HashMap::new(),
        }
    }

    pub fn get(&self, qname: &str) -> Option<u32> {
        // A small document's few names are quicker found by looking at
        // each than by hashing; the map is built once there are more.
        if self.qnames.len() <= SCAN {
            return self
                .qnames
                .iter()
                .position(|q| &*q.qname == qname)
                .map(|i| i as u32);
        }
        self.qname_ids.get(qname).copied()
    }

    /// Adds a name [`Names::get`] didn't find.
    pub fn insert(&mut self, qname: &str, colon: u32) -> u32 {
        let prefix = match colon {
            NONE => 0,
            c => self.prefix(&qname[..c as usize]),
        };
        let id = self.qnames.len() as u32;
        self.qnames.push(QName {
            qname: qname.into(),
            colon,
            prefix,
        });
        if self.qnames.len() == SCAN + 1 {
            self.qname_ids = self
                .qnames
                .iter()
                .enumerate()
                .map(|(i, q)| (q.qname.clone(), i as u32))
                .collect();
        } else if self.qnames.len() > SCAN + 1 {
            self.qname_ids.insert(qname.into(), id);
        }
        id
    }

    pub fn prefix(&mut self, prefix: &str) -> u32 {
        if let Some(i) = BUILTIN_PREFIXES.iter().position(|&p| p == prefix) {
            return i as u32;
        }
        if let Some(&id) = self.prefix_ids.get(prefix) {
            return id;
        }
        let id = (BUILTIN_PREFIXES.len() + self.prefix_ids.len()) as u32;
        self.prefix_ids.insert(prefix.into(), id);
        id
    }

    pub fn uri(&mut self, uri: &str) -> u32 {
        if let Some(i) = BUILTIN_URIS.iter().position(|&u| u == uri) {
            return i as u32;
        }
        if let Some(&id) = self.uri_ids.get(uri) {
            return id;
        }
        let id = (BUILTIN_URIS.len() + self.uris.len()) as u32;
        self.uris.push(uri.into());
        self.uri_ids.insert(uri.into(), id);
        id
    }

    pub fn qname_info(&self, id: u32) -> &QName {
        &self.qnames[id as usize]
    }

    pub fn qname_str(&self, id: u32) -> &str {
        &self.qnames[id as usize].qname
    }

    /// The part after the colon, or all of it.
    pub fn local(&self, id: u32) -> &str {
        let q = &self.qnames[id as usize];
        match q.colon {
            NONE => &q.qname,
            c => &q.qname[c as usize + 1..],
        }
    }

    pub fn uri_str(&self, id: u32) -> &str {
        match BUILTIN_URIS.get(id as usize) {
            Some(u) => u,
            None => &self.uris[id as usize - BUILTIN_URIS.len()],
        }
    }

    fn name(&self, qname: u32, ns: u32) -> Name<'_> {
        let q = &self.qnames[qname as usize];
        let (prefix, local) = match q.colon {
            NONE => ("", &*q.qname),
            c => (&q.qname[..c as usize], &q.qname[c as usize + 1..]),
        };
        Name {
            qname: &q.qname,
            prefix,
            local,
            namespace: self.uri_str(ns),
        }
    }
}

/// An element's or attribute's name: as written, and resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Name<'a> {
    /// The name as written, prefix and all: `atom:link`.
    pub qname: &'a str,
    /// The prefix, `""` for none.
    pub prefix: &'a str,
    /// The local part: `link`.
    pub local: &'a str,
    /// The namespace URI the prefix was bound to, `""` for none.
    pub namespace: &'a str,
}

impl fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.qname)
    }
}

/// An attribute of an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Attribute<'a> {
    pub name: Name<'a>,
    /// The value, normalized as XML says: references replaced, and each
    /// tab, line end and newline a space (and, for an attribute the
    /// internal subset declares as anything but `CDATA`, spaces collapsed).
    pub value: &'a str,
    /// From the name to the closing quote; for a default from the DTD,
    /// the element's start tag.
    pub span: Span,
    /// False for a default the DTD supplied.
    pub specified: bool,
}

impl Attribute<'_> {
    /// Whether this is a namespace declaration, `xmlns` or `xmlns:*`.
    pub fn is_namespace_declaration(&self) -> bool {
        self.name.namespace == XMLNS_NS
    }
}

/// What a node is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind<'a> {
    /// The root of the tree, above the document element.
    Document,
    /// The `<!DOCTYPE>`; [`Document::doctype`] has what it said.
    Doctype,
    Element(Name<'a>),
    /// Character data, with line ends normalized and references replaced.
    /// Adjacent character data is one node.
    Text(&'a str),
    /// A CDATA section's contents.
    CData(&'a str),
    Comment(&'a str),
    ProcessingInstruction {
        target: &'a str,
        data: &'a str,
    },
    /// A reference to an entity this parser didn't read: one declared
    /// external, or one an unread part of the DTD might declare. The
    /// document's warnings say which.
    EntityRef(&'a str),
}

/// The XML declaration, `<?xml version="1.0"?>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XmlDecl {
    pub version: String,
    pub encoding: Option<String>,
    pub standalone: Option<bool>,
    pub span: Span,
}

/// What the `<!DOCTYPE>` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doctype {
    /// The document element's name it gives.
    pub name: String,
    pub public_id: Option<String>,
    /// The external subset's address. Parsec never reads it.
    pub system_id: Option<String>,
    /// The internal subset as written, without its brackets.
    pub internal_subset: Option<String>,
    pub span: Span,
}

/// An entity the internal subset declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntityDecl {
    pub name: String,
    /// A parameter entity (`<!ENTITY % name ...>`), for use in the DTD.
    pub parameter: bool,
    /// An internal entity's replacement text.
    pub value: Option<String>,
    pub public_id: Option<String>,
    /// An external entity's address. Parsec never reads it.
    pub system_id: Option<String>,
    /// An unparsed entity's notation (`NDATA`).
    pub notation: Option<String>,
    pub span: Span,
}

/// A notation the internal subset declared.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notation {
    pub name: String,
    pub public_id: Option<String>,
    pub system_id: Option<String>,
    pub span: Span,
}

/// A parsed XML document.
///
/// Nodes are numbered in document order, from the document node at 0.
/// Every node and attribute has the [`Span`] of the source it came from;
/// a node an entity reference produced has the span of that reference.
#[derive(Debug)]
pub struct Document {
    pub(crate) nodes: Vec<Node>,
    pub(crate) attrs: Vec<AttrData>,
    pub(crate) text: String,
    pub(crate) names: Names,
    pub(crate) decl: Option<XmlDecl>,
    pub(crate) doctype: Option<Doctype>,
    pub(crate) entities: Vec<EntityDecl>,
    pub(crate) notations: Vec<Notation>,
    /// The attribute the internal subset declares as an ID for each
    /// element, both by qualified name id.
    pub(crate) id_attrs: HashMap<u32, u32>,
    /// For XPath, built on first use: see `xpath_tree.rs`.
    #[cfg(feature = "xpath")]
    pub(crate) xpath_attrs: std::sync::OnceLock<crate::xpath_tree::Declared>,
    pub(crate) warnings: Vec<Diagnostic>,
    pub(crate) namespaces: bool,
}

impl Document {
    /// A document for a source of `len` bytes, whose text is rarely
    /// longer than that.
    pub(crate) fn new(namespaces: bool, len: usize) -> Document {
        Document {
            nodes: vec![Node {
                parent: NONE,
                first: NONE,
                last: NONE,
                prev: NONE,
                next: NONE,
                span: Span::default(),
                data: Data::Document,
            }],
            attrs: Vec::new(),
            text: String::with_capacity(len),
            names: Names::new(),
            decl: None,
            doctype: None,
            entities: Vec::new(),
            notations: Vec::new(),
            id_attrs: HashMap::new(),
            #[cfg(feature = "xpath")]
            xpath_attrs: std::sync::OnceLock::new(),
            warnings: Vec::new(),
            namespaces,
        }
    }

    pub(crate) fn str(&self, s: Str) -> &str {
        &self.text[s.start as usize..s.end() as usize]
    }

    /// Adds a node as `parent`'s last child.
    pub(crate) fn append(&mut self, parent: u32, span: Span, data: Data) -> u32 {
        let id = self.nodes.len() as u32;
        let prev = self.nodes[parent as usize].last;
        self.nodes.push(Node {
            parent,
            first: NONE,
            last: NONE,
            prev,
            next: NONE,
            span,
            data,
        });
        if prev == NONE {
            self.nodes[parent as usize].first = id;
        } else {
            self.nodes[prev as usize].next = id;
        }
        self.nodes[parent as usize].last = id;
        id
    }

    /// The document node, the root of the tree.
    pub fn root(&self) -> NodeId {
        NodeId(0)
    }

    /// The document element.
    pub fn root_element(&self) -> NodeId {
        self.children(self.root())
            .find(|&n| matches!(self.nodes[n.index()].data, Data::Element { .. }))
            .expect("a parsed document has a document element")
    }

    /// How many nodes there are, the document node included.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Always false: there is at least the document node.
    pub fn is_empty(&self) -> bool {
        false
    }

    pub fn kind(&self, node: NodeId) -> NodeKind<'_> {
        match self.nodes[node.index()].data {
            Data::Document => NodeKind::Document,
            Data::Doctype => NodeKind::Doctype,
            Data::Element { name, ns, .. } => NodeKind::Element(self.names.name(name, ns)),
            Data::Text(s) => NodeKind::Text(self.str(s)),
            Data::Cdata(s) => NodeKind::CData(self.str(s)),
            Data::Comment(s) => NodeKind::Comment(self.str(s)),
            Data::Pi { target, data } => NodeKind::ProcessingInstruction {
                target: self.str(target),
                data: self.str(data),
            },
            Data::EntityRef(s) => NodeKind::EntityRef(self.str(s)),
        }
    }

    pub fn is_element(&self, node: NodeId) -> bool {
        matches!(self.nodes[node.index()].data, Data::Element { .. })
    }

    /// An element's name.
    pub fn name(&self, node: NodeId) -> Option<Name<'_>> {
        match self.nodes[node.index()].data {
            Data::Element { name, ns, .. } => Some(self.names.name(name, ns)),
            _ => None,
        }
    }

    /// Where the node came from: an element's from its start tag's `<` to
    /// its end tag's `>`.
    pub fn span(&self, node: NodeId) -> Span {
        self.nodes[node.index()].span
    }

    fn link(id: u32) -> Option<NodeId> {
        (id != NONE).then_some(NodeId(id))
    }

    pub fn parent(&self, node: NodeId) -> Option<NodeId> {
        Self::link(self.nodes[node.index()].parent)
    }

    pub fn first_child(&self, node: NodeId) -> Option<NodeId> {
        Self::link(self.nodes[node.index()].first)
    }

    pub fn last_child(&self, node: NodeId) -> Option<NodeId> {
        Self::link(self.nodes[node.index()].last)
    }

    pub fn next_sibling(&self, node: NodeId) -> Option<NodeId> {
        Self::link(self.nodes[node.index()].next)
    }

    pub fn prev_sibling(&self, node: NodeId) -> Option<NodeId> {
        Self::link(self.nodes[node.index()].prev)
    }

    pub fn children(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.first_child(node), |&n| self.next_sibling(n))
    }

    /// The node's ancestors, nearest first, the document node last.
    pub fn ancestors(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        std::iter::successors(self.parent(node), |&n| self.parent(n))
    }

    /// The node and everything under it, in document order.
    pub fn descendants(&self, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        // Ids are document order, and a subtree is a run of them: it ends
        // where the next node after it, outside it, starts.
        let mut end = node;
        let stop = loop {
            if let Some(next) = self.next_sibling(end) {
                break next.0;
            }
            match self.parent(end) {
                Some(p) => end = p,
                None => break self.nodes.len() as u32,
            }
        };
        (node.0..stop).map(NodeId)
    }

    /// An element's attributes, in the order of its start tag, then the
    /// defaults the DTD supplied.
    pub fn attributes(&self, node: NodeId) -> impl ExactSizeIterator<Item = Attribute<'_>> + '_ {
        let range = match self.nodes[node.index()].data {
            Data::Element {
                attrs, attrs_len, ..
            } => attrs as usize..(attrs + attrs_len) as usize,
            _ => 0..0,
        };
        self.attrs[range].iter().map(|a| Attribute {
            name: self.names.name(a.name, a.ns),
            value: self.str(a.value),
            span: a.span,
            specified: a.specified,
        })
    }

    /// The value of the attribute with this namespace (`""` for none)
    /// and local name.
    pub fn attribute(&self, node: NodeId, namespace: &str, local: &str) -> Option<&str> {
        self.attributes(node)
            .find(|a| a.name.local == local && a.name.namespace == namespace)
            .map(|a| a.value)
    }

    /// The value of the attribute with this name as written, prefix and
    /// all.
    pub fn attribute_named(&self, node: NodeId, qname: &str) -> Option<&str> {
        self.attributes(node)
            .find(|a| a.name.qname == qname)
            .map(|a| a.value)
    }

    /// The text of the node and everything under it, as XPath's
    /// `string()` reads it: character data and CDATA, in order.
    pub fn text(&self, node: NodeId) -> String {
        let mut out = String::new();
        for n in self.descendants(node) {
            if let Data::Text(s) | Data::Cdata(s) = self.nodes[n.index()].data {
                out.push_str(self.str(s));
            }
        }
        out
    }

    /// The namespace `prefix` (`""` for the default namespace) is bound to
    /// at `node`, from the declarations on it and its ancestors.
    pub fn lookup_namespace(&self, node: NodeId, prefix: &str) -> Option<&str> {
        if prefix == "xml" {
            return Some(XML_NS);
        }
        for n in std::iter::once(node).chain(self.ancestors(node)) {
            for a in self.attributes(n) {
                if a.is_namespace_declaration()
                    && (a.name.local == prefix && a.name.prefix == "xmlns"
                        || prefix.is_empty() && a.name.qname == "xmlns")
                {
                    return (!a.value.is_empty()).then_some(a.value);
                }
            }
        }
        None
    }

    /// The nearest `xml:` attribute named `local` on the node or above it.
    fn inherited(&self, node: NodeId, local: &str) -> Option<&str> {
        std::iter::once(node)
            .chain(self.ancestors(node))
            .find_map(|n| self.attribute(n, XML_NS, local))
    }

    /// The node's language: the nearest `xml:lang` on it or above it.
    pub fn xml_lang(&self, node: NodeId) -> Option<&str> {
        self.inherited(node, "lang")
    }

    /// Whether white space at the node is to be kept: the nearest
    /// `xml:space` on it or above it is `preserve`.
    pub fn preserves_space(&self, node: NodeId) -> bool {
        self.inherited(node, "space") == Some("preserve")
    }

    /// The XML declaration, if the document has one.
    pub fn xml_decl(&self) -> Option<&XmlDecl> {
        self.decl.as_ref()
    }

    pub fn doctype(&self) -> Option<&Doctype> {
        self.doctype.as_ref()
    }

    /// The entities the internal subset declared, in order, the first
    /// declaration of each name only (later ones don't count).
    pub fn entities(&self) -> &[EntityDecl] {
        &self.entities
    }

    pub fn notations(&self) -> &[Notation] {
        &self.notations
    }

    /// What the parser didn't read and says so, as the XML spec asks of
    /// a processor that doesn't validate: the external subset, external
    /// entities, and declarations it skipped after a parameter entity it
    /// didn't read. None of these is an error.
    pub fn warnings(&self) -> &[Diagnostic] {
        &self.warnings
    }

    /// Whether the document was parsed with namespaces on.
    pub fn namespaces(&self) -> bool {
        self.namespaces
    }
}

// Documents go between threads.
const _: () = {
    const fn send_sync<T: Send + Sync>() {}
    send_sync::<Document>();
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_found_either_side_of_the_map() {
        let mut names = Names::new();
        let ids: Vec<u32> = (0..40)
            .map(|i| names.insert(&format!("n{i}"), NONE))
            .collect();
        for (i, &id) in ids.iter().enumerate() {
            assert_eq!(names.get(&format!("n{i}")), Some(id));
            assert_eq!(names.qname_str(id), format!("n{i}"));
        }
        assert_eq!(names.get("n40"), None);
        assert_eq!(
            (names.uri(""), names.uri(XML_NS), names.uri(XMLNS_NS)),
            (0, NS_XML, NS_XMLNS)
        );
        let u = names.uri("urn:x");
        assert_eq!(
            (names.uri("urn:x"), names.uri_str(u), names.uri_str(NS_XML)),
            (u, "urn:x", XML_NS)
        );
        assert_eq!(
            (
                names.prefix("xml"),
                names.prefix("xmlns"),
                names.prefix("p")
            ),
            (PREFIX_XML, PREFIX_XMLNS, 3)
        );
        assert_eq!(names.prefix("p"), 3);
    }
}
