// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! A [`Document`] as a tree XPath can query, with the `xpath` feature.

use parsec_xpath::{Attr, NodeId as Id, NodeType, TagKey, Tree};

use crate::tree::{AttrData, Data, Document, NS_XMLNS, NodeId};

/// The elements that declare namespaces, and where their other attributes
/// are, so XPath reads an attribute by place in constant time on them too.
/// Few elements declare namespaces, so this is small: built once, on the
/// first query that reads an attribute, in one pass over the attributes.
#[derive(Debug, Default)]
pub(crate) struct Declared {
    /// Node ids, ascending.
    elements: Vec<u32>,
    /// Where each element's run of `positions` starts, and one past the
    /// last element's.
    starts: Vec<u32>,
    /// Indexes into `Document::attrs`.
    positions: Vec<u32>,
    /// The elements with an `xml:lang`, ascending, and its index into
    /// `Document::attrs`: `lang()` reads it from an attribute's owner, so
    /// a scan of the owner's attributes would make `@*[lang('en')]`
    /// quadratic on one wide element.
    langs: Vec<(u32, u32)>,
}

impl Declared {
    fn build(doc: &Document) -> Declared {
        let mut d = Declared::default();
        let xml_lang = doc.names.get("xml:lang");
        for (n, node) in doc.nodes.iter().enumerate() {
            let Data::Element {
                attrs, attrs_len, ..
            } = node.data
            else {
                continue;
            };
            let range = attrs..attrs + attrs_len;
            let own = &doc.attrs[range.start as usize..range.end as usize];
            if let Some(at) = own.iter().position(|a| Some(a.name) == xml_lang) {
                d.langs.push((n as u32, attrs + at as u32));
            }
            if !own.iter().any(|a| a.ns == NS_XMLNS) {
                continue;
            }
            d.elements.push(n as u32);
            d.starts.push(d.positions.len() as u32);
            d.positions
                .extend(range.filter(|&i| doc.attrs[i as usize].ns != NS_XMLNS));
        }
        d.starts.push(d.positions.len() as u32);
        d
    }
}

enum Attrs<'d> {
    Run(std::ops::Range<usize>),
    Picked(&'d [u32]),
}

impl Document {
    fn element(&self, n: Id) -> Option<(u32, u32)> {
        match self.nodes[n as usize].data {
            Data::Element { name, ns, .. } => Some((name, ns)),
            _ => None,
        }
    }

    fn declared(&self) -> &Declared {
        self.xpath_attrs.get_or_init(|| Declared::build(self))
    }

    /// The element's attributes as a range of `self.attrs`.
    fn attr_range(&self, n: Id) -> std::ops::Range<usize> {
        match self.nodes[n as usize].data {
            Data::Element {
                attrs, attrs_len, ..
            } => attrs as usize..(attrs + attrs_len) as usize,
            _ => 0..0,
        }
    }

    /// Where in `self.attrs` the element's attributes are, as XPath sees
    /// them: its own run of them, or, on an element that declares
    /// namespaces, the declarations left out.
    fn xpath_attrs(&self, n: Id) -> Attrs<'_> {
        let range = self.attr_range(n);
        if !self.namespaces || range.is_empty() {
            return Attrs::Run(range);
        }
        let declared = self.declared();
        match declared.elements.binary_search(&n) {
            Ok(k) => {
                let (a, b) = (declared.starts[k], declared.starts[k + 1]);
                Attrs::Picked(&declared.positions[a as usize..b as usize])
            }
            Err(_) => Attrs::Run(range),
        }
    }

    fn xpath_attr(&self, n: Id, i: u32) -> Option<&AttrData> {
        let at = match self.xpath_attrs(n) {
            Attrs::Run(r) => r.start.checked_add(i as usize).filter(|&at| at < r.end)?,
            Attrs::Picked(p) => *p.get(i as usize)? as usize,
        };
        Some(&self.attrs[at])
    }

    /// `n`, or the sibling `step` gives if `n` is the doctype: XPath's
    /// data model has no node for it, so the links XPath reads go round it.
    fn past_doctype(&self, n: NodeId, step: fn(&Document, NodeId) -> Option<NodeId>) -> Option<Id> {
        match self.nodes[n.index()].data {
            Data::Doctype => step(self, n).map(|s| s.0),
            _ => Some(n.0),
        }
    }
}

/// Names compare exactly, prefixes resolve to the namespaces the document
/// declared, and namespace declarations aren't attributes. With namespaces
/// off ([`crate::Options::namespaces`]), every name is its own local name
/// in no namespace and `xmlns` attributes are attributes like any other.
/// The doctype isn't in the tree XPath reads, as XPath's data model has
/// no node for it. An [`EntityRef`](crate::NodeKind::EntityRef) is a node
/// XPath has no kind for, so no node test matches it.
impl Tree for Document {
    fn len(&self) -> usize {
        self.nodes.len()
    }

    fn kind(&self, n: Id) -> NodeType {
        match self.nodes[n as usize].data {
            Data::Document => NodeType::Document,
            Data::Doctype => NodeType::Doctype,
            Data::Element { .. } => NodeType::Element,
            Data::Text(_) => NodeType::Text,
            Data::Cdata(_) => NodeType::Cdata,
            Data::Comment(_) => NodeType::Comment,
            Data::Pi { .. } => NodeType::ProcessingInstruction,
            Data::EntityRef(_) => NodeType::Other,
        }
    }

    fn parent(&self, n: Id) -> Option<Id> {
        Document::parent(self, NodeId(n)).map(|p| p.0)
    }

    fn first_child(&self, n: Id) -> Option<Id> {
        let first = Document::first_child(self, NodeId(n))?;
        self.past_doctype(first, Document::next_sibling)
    }

    fn next_sibling(&self, n: Id) -> Option<Id> {
        let next = Document::next_sibling(self, NodeId(n))?;
        self.past_doctype(next, Document::next_sibling)
    }

    fn prev_sibling(&self, n: Id) -> Option<Id> {
        let prev = Document::prev_sibling(self, NodeId(n))?;
        self.past_doctype(prev, Document::prev_sibling)
    }

    fn qualified_name(&self, n: Id) -> &str {
        self.element(n)
            .map_or("", |(name, _)| self.names.qname_str(name))
    }

    fn local_name(&self, n: Id) -> &str {
        self.element(n)
            .map_or("", |(name, _)| self.names.local(name))
    }

    fn namespace_uri(&self, n: Id) -> &str {
        self.element(n).map_or("", |(_, ns)| self.names.uri_str(ns))
    }

    fn tag_key(&self, n: Id) -> TagKey<'_> {
        TagKey::Name(self.local_name(n))
    }

    fn attribute_count(&self, n: Id) -> u32 {
        match self.xpath_attrs(n) {
            Attrs::Run(r) => r.len() as u32,
            Attrs::Picked(p) => p.len() as u32,
        }
    }

    fn attribute(&self, n: Id, i: u32) -> Option<Attr<'_>> {
        let a = self.xpath_attr(n, i)?;
        let q = self.names.qname_info(a.name);
        Some(Attr {
            qualified_name: &q.qname,
            local_name: self.names.local(a.name),
            namespace_uri: self.names.uri_str(a.ns),
            value: self.str(a.value),
        })
    }

    fn data(&self, n: Id) -> &str {
        match self.nodes[n as usize].data {
            Data::Text(s) | Data::Cdata(s) | Data::Comment(s) | Data::Pi { data: s, .. } => {
                self.str(s)
            }
            _ => "",
        }
    }

    fn pi_target(&self, n: Id) -> Option<&str> {
        match self.nodes[n as usize].data {
            Data::Pi { target, .. } => Some(self.str(target)),
            _ => None,
        }
    }

    fn is_xml(&self) -> bool {
        true
    }

    fn id(&self, n: Id) -> Option<&str> {
        let (name, _) = self.element(n)?;
        if let Some(v) = self.attribute_named(NodeId(n), "xml:id") {
            return Some(v);
        }
        let declared = *self.id_attrs.get(&name)?;
        self.attrs[self.attr_range(n)]
            .iter()
            .find(|a| a.name == declared)
            .map(|a| self.str(a.value))
    }

    fn lang(&self, n: Id) -> Option<&str> {
        self.element(n)?;
        let langs = &self.declared().langs;
        let k = langs.binary_search_by_key(&n, |&(e, _)| e).ok()?;
        Some(self.str(self.attrs[langs[k].1 as usize].value))
    }
}
