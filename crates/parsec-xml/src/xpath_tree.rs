// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! A [`Document`] as a tree XPath can query, with the `xpath` feature.

use parsec_xpath::{Attr, NodeId as Id, NodeType, TagKey, Tree};

use crate::tree::{Data, Document, NS_XMLNS, NodeId};

impl Document {
    fn element(&self, n: Id) -> Option<(u32, u32)> {
        match self.nodes[n as usize].data {
            Data::Element { name, ns, .. } => Some((name, ns)),
            _ => None,
        }
    }

    /// The attributes XPath sees: the element's, namespace declarations
    /// left out.
    // shortcut: finding the ith skips declarations one by one, which is
    // linear in the element's attributes; index them if elements with
    // hundreds of attributes show in profiles.
    fn xpath_attributes(&self, n: Id) -> impl Iterator<Item = crate::Attribute<'_>> + '_ {
        self.attributes(NodeId(n))
            .filter(|a| !a.is_namespace_declaration())
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
        match self.nodes[n as usize].data {
            Data::Element {
                attrs, attrs_len, ..
            } => {
                let all = &self.attrs[attrs as usize..(attrs + attrs_len) as usize];
                all.iter().filter(|a| a.ns != NS_XMLNS).count() as u32
            }
            _ => 0,
        }
    }

    fn attribute(&self, n: Id, i: u32) -> Option<Attr<'_>> {
        self.xpath_attributes(n).nth(i as usize).map(|a| Attr {
            qualified_name: a.name.qname,
            local_name: a.name.local,
            namespace_uri: a.name.namespace,
            value: a.value,
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
        let Data::Element {
            attrs, attrs_len, ..
        } = self.nodes[n as usize].data
        else {
            return None;
        };
        self.attrs[attrs as usize..(attrs + attrs_len) as usize]
            .iter()
            .find(|a| a.name == declared)
            .map(|a| self.str(a.value))
    }

    fn lang(&self, n: Id) -> Option<&str> {
        self.element(n)?;
        self.attribute_named(NodeId(n), "xml:lang")
    }
}
