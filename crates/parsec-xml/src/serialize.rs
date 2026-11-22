// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Writing a document back out as XML, with every prefix and namespace
//! declaration as it was, so it parses back to the same tree.

use std::collections::HashSet;

use crate::tree::{Data, Document, NONE, NodeId, XMLNS_NS};

fn escape_text(s: &str, out: &mut String) {
    let mut from = 0;
    for (i, b) in s.bytes().enumerate() {
        let r = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            // A CR would read back as a line end.
            b'\r' => "&#13;",
            _ => continue,
        };
        out.push_str(&s[from..i]);
        out.push_str(r);
        from = i + 1;
    }
    out.push_str(&s[from..]);
}

fn escape_attr(s: &str, out: &mut String) {
    let mut from = 0;
    for (i, b) in s.bytes().enumerate() {
        let r = match b {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'"' => "&quot;",
            // White space that normalization would turn into spaces.
            b'\t' => "&#9;",
            b'\n' => "&#10;",
            b'\r' => "&#13;",
            _ => continue,
        };
        out.push_str(&s[from..i]);
        out.push_str(r);
        from = i + 1;
    }
    out.push_str(&s[from..]);
}

fn quote(s: &str, out: &mut String) {
    let q = if s.contains('"') { '\'' } else { '"' };
    out.push(q);
    out.push_str(s);
    out.push(q);
}

impl Document {
    /// The whole document as XML: the XML declaration (without an
    /// encoding, since this is a string), the `<!DOCTYPE>` with its
    /// internal subset as written, and the tree. Entity references come
    /// out expanded, but for those to entities Parsec didn't read.
    /// Attributes the DTD defaulted are left to the DTD.
    ///
    /// ```
    /// let src = r#"<feed xmlns="http://www.w3.org/2005/Atom" xmlns:m="urn:m"><m:x a="1 &amp; 2"/></feed>"#;
    /// let doc = parsec_xml::parse(src).unwrap();
    /// assert_eq!(doc.to_xml(), src);
    /// ```
    pub fn to_xml(&self) -> String {
        let mut out = String::new();
        if let Some(decl) = &self.decl {
            out.push_str("<?xml version=\"");
            out.push_str(&decl.version);
            out.push('"');
            if let Some(sd) = decl.standalone {
                out.push_str(if sd {
                    " standalone=\"yes\""
                } else {
                    " standalone=\"no\""
                });
            }
            out.push_str("?>");
        }
        for (i, child) in self.children(self.root()).enumerate() {
            if i > 0 || self.decl.is_some() {
                out.push('\n');
            }
            self.write(child, false, &mut out);
        }
        out
    }

    /// A node and everything under it as XML. An element written alone
    /// takes along the namespace declarations it's in the scope of, and
    /// the attributes the DTD defaulted on it and under it, so that it
    /// means the same on its own.
    pub fn write_xml(&self, node: NodeId, out: &mut String) {
        if node == self.root() {
            out.push_str(&self.to_xml());
        } else {
            self.write(node, true, out);
        }
    }

    fn write(&self, top: NodeId, alone: bool, out: &mut String) {
        let mut n = top.0;
        loop {
            let node = &self.nodes[n as usize];
            self.open(NodeId(n), alone && n == top.0, alone, out);
            if node.first != NONE {
                n = node.first;
                continue;
            }
            loop {
                if n == top.0 {
                    return;
                }
                let node = &self.nodes[n as usize];
                if node.next != NONE {
                    n = node.next;
                    break;
                }
                n = node.parent;
                self.close(NodeId(n), out);
            }
        }
    }

    fn open(&self, n: NodeId, top: bool, defaults: bool, out: &mut String) {
        match self.nodes[n.index()].data {
            Data::Element { .. } => {
                let name = self.name(n).unwrap();
                out.push('<');
                out.push_str(name.qname);
                for a in self.attributes(n) {
                    if a.specified || defaults {
                        out.push(' ');
                        out.push_str(a.name.qname);
                        out.push_str("=\"");
                        escape_attr(a.value, out);
                        out.push('"');
                    }
                }
                if top {
                    self.inherited_declarations(n, out);
                }
                out.push_str(if self.nodes[n.index()].first == NONE {
                    "/>"
                } else {
                    ">"
                });
            }
            Data::Text(s) => escape_text(self.str(s), out),
            Data::Cdata(s) => {
                out.push_str("<![CDATA[");
                out.push_str(self.str(s));
                out.push_str("]]>");
            }
            Data::Comment(s) => {
                out.push_str("<!--");
                out.push_str(self.str(s));
                out.push_str("-->");
            }
            Data::Pi { target, data } => {
                out.push_str("<?");
                out.push_str(self.str(target));
                if data.len > 0 {
                    out.push(' ');
                    out.push_str(self.str(data));
                }
                out.push_str("?>");
            }
            Data::EntityRef(s) => {
                out.push('&');
                out.push_str(self.str(s));
                out.push(';');
            }
            Data::Doctype => {
                let Some(d) = &self.doctype else { return };
                out.push_str("<!DOCTYPE ");
                out.push_str(&d.name);
                match (&d.public_id, &d.system_id) {
                    (Some(p), Some(s)) => {
                        out.push_str(" PUBLIC ");
                        quote(p, out);
                        out.push(' ');
                        quote(s, out);
                    }
                    (None, Some(s)) => {
                        out.push_str(" SYSTEM ");
                        quote(s, out);
                    }
                    _ => {}
                }
                if let Some(subset) = &d.internal_subset {
                    out.push_str(" [");
                    out.push_str(subset);
                    out.push(']');
                }
                out.push('>');
            }
            Data::Document => {}
        }
    }

    fn close(&self, n: NodeId, out: &mut String) {
        if let Some(name) = self.name(n) {
            out.push_str("</");
            out.push_str(name.qname);
            out.push('>');
        }
    }

    /// The declarations in scope at `n` from its ancestors that it
    /// doesn't make itself, nearest first.
    fn inherited_declarations(&self, n: NodeId, out: &mut String) {
        let mut seen: HashSet<&str> = self
            .attributes(n)
            .filter(|a| a.is_namespace_declaration())
            .map(|a| a.name.qname)
            .collect();
        for up in self.ancestors(n) {
            for a in self.attributes(up) {
                if a.name.namespace != XMLNS_NS || !seen.insert(a.name.qname) {
                    continue;
                }
                // No default namespace above is the same as none at all.
                if a.name.qname == "xmlns" && a.value.is_empty() {
                    continue;
                }
                out.push(' ');
                out.push_str(a.name.qname);
                out.push_str("=\"");
                escape_attr(a.value, out);
                out.push('"');
            }
        }
    }
}
