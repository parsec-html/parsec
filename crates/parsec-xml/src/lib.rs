// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XML for Parsec: [XML 1.0 (Fifth Edition)](https://www.w3.org/TR/xml/)
//! and [Namespaces in XML 1.0 (Third Edition)](https://www.w3.org/TR/xml-names/),
//! parsed to a tree with namespace-resolved names and a span on every
//! node and attribute.
//!
//! ```
//! let feed = r#"<?xml version="1.0"?>
//! <feed xmlns="http://www.w3.org/2005/Atom" xml:lang="en">
//!   <entry><title>Ships &amp; ports</title></entry>
//! </feed>"#;
//! let doc = parsec_xml::parse(feed).unwrap();
//! let root = doc.root_element();
//! let name = doc.name(root).unwrap();
//! assert_eq!((name.local, name.namespace), ("feed", "http://www.w3.org/2005/Atom"));
//! assert_eq!(doc.xml_lang(root), Some("en"));
//! assert_eq!(doc.text(root).trim(), "Ships & ports");
//! ```
//!
//! XML is a trust boundary, and the parser treats it as one:
//!
//! - It doesn't validate, and it never fetches or opens anything. The
//!   external subset and external entities are left unread, and the
//!   document's [`warnings`](Document::warnings) say so, as the spec asks
//!   of a processor that doesn't read them.
//! - Entities the internal subset declares are expanded, no deeper than
//!   [`Options::max_entity_depth`] and to no more text in all than
//!   [`Options::entity_allowance`] plus [`Options::entity_ratio`] bytes per
//!   byte of input, so a billion laughs or a quadratic blowup stops with
//!   an error.
//! - Elements nest no deeper than [`Limits::max_depth`]: 2,048 unless you
//!   say otherwise. The parser keeps open elements on a stack of its own,
//!   so even a million levels with no limit cost time in proportion and
//!   no recursion.
//! - Anything that isn't well-formed is an [`Error`], fatal as the spec
//!   says, with a code, a span, and the line and column.
//!
//! ## XPath
//!
//! With the `xpath` feature (`parsec-xml = { version = "0.2", features =
//! ["xpath"] }`), a [`Document`] is a tree `xpath` can query, and
//! `parsec_xml::xpath` is `parsec-xpath` re-exported. No HTML parser
//! comes with it. Names compare exactly, a query's prefixes are bound by
//! the namespaces it's run with, `xmlns` and `xmlns:*` aren't attributes,
//! `id()` finds `xml:id` and attributes the internal subset declares an
//! `ID`, and `lang()` reads `xml:lang`. The namespace axis is empty.
//!
//! ```
//! # #[cfg(feature = "xpath")] {
//! use parsec_xml::xpath::{Doc, Index, Item, NoExtensions, Value, XPath};
//!
//! let doc = parsec_xml::parse(r#"<feed xmlns="http://www.w3.org/2005/Atom">
//!   <entry><title>Ships</title></entry><entry><title>Ports</title></entry>
//! </feed>"#).unwrap();
//! // Build the index once per document; every query reads it.
//! let index = Index::build(&doc, &[0]);
//! let atom = [("a".to_string(), "http://www.w3.org/2005/Atom".to_string())];
//! let query = XPath::new("//a:entry[2]/a:title").unwrap();
//! let Ok(Value::Nodes(hits)) =
//!     query.evaluate(Doc::new(&doc, &index), 0, &[], &atom, &NoExtensions)
//! else {
//!     panic!("a node-set")
//! };
//! let Item::Node(title) = hits[0] else { panic!("a node") };
//! let title = parsec_xml::NodeId::from_index(title as usize);
//! assert_eq!(doc.text(title), "Ports");
//! # }
//! ```

mod bytes;
mod chars;
mod dtd;
mod parse;
mod serialize;
mod tree;
#[cfg(feature = "xpath")]
mod xpath_tree;

use std::fmt;

pub use bytes::{Parsed, parse_bytes};
pub use encoding_rs::Encoding;
pub use parse::ENTITY_DEPTH_CEILING;
pub use parsec_base::{Diagnostic, Limits, LineCol, LineIndex, Severity, Span};
#[cfg(feature = "xpath")]
#[doc(inline)]
pub use parsec_xpath as xpath;
pub use tree::{
    Attribute, Doctype, Document, EntityDecl, Name, NodeId, NodeKind, Notation, XML_NS, XMLNS_NS,
    XmlDecl,
};

/// How elements may nest unless you say otherwise: lxml's limit with
/// `huge_tree`, deeper than any document we know of that isn't an attack.
pub const DEFAULT_MAX_DEPTH: u32 = 2048;

/// How to parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    /// The input's size, how deep elements nest
    /// ([`DEFAULT_MAX_DEPTH`]), and the steps a parse may take (one for
    /// each node, attribute and entity expansion). Unlimited but for the
    /// depth.
    pub limits: Limits,
    /// Namespaces in XML: names are qualified names, and their prefixes
    /// resolve. On by default. Off, a name is any XML name, colons and
    /// all, in no namespace.
    pub namespaces: bool,
    /// How deep entity references may nest in the entities they refer
    /// to: 16 by default, and never past [`ENTITY_DEPTH_CEILING`].
    pub max_entity_depth: u32,
    /// Entity expansion may add this many bytes to any document, 1 MiB by
    /// default...
    pub entity_allowance: u64,
    /// ...plus this many for each byte of the document itself, 8 by
    /// default. Every expansion counts, of the same entity too.
    pub entity_ratio: u64,
}

impl Default for Options {
    fn default() -> Options {
        Options {
            limits: Limits {
                max_depth: Some(DEFAULT_MAX_DEPTH),
                ..Limits::default()
            },
            namespaces: true,
            max_entity_depth: 16,
            entity_allowance: 1 << 20,
            entity_ratio: 8,
        }
    }
}

/// Why a document isn't well-formed XML, or couldn't be parsed within its
/// limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    /// What went wrong, with a stable `code` and its span in the input.
    pub diagnostic: Diagnostic,
    /// Where the span starts, from 1, the column in characters.
    pub position: LineCol,
}

impl Error {
    pub(crate) fn new(diagnostic: Diagnostic, src: &str) -> Error {
        let position = LineIndex::new(src).line_col(diagnostic.span.start);
        Error {
            diagnostic,
            position,
        }
    }

    pub fn code(&self) -> &'static str {
        self.diagnostic.code
    }

    pub fn span(&self) -> Span {
        self.diagnostic.span
    }
}

impl fmt::Display for Error {
    /// `3:14: message [code]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} [{}]",
            self.position, self.diagnostic.message, self.diagnostic.code
        )
    }
}

impl std::error::Error for Error {}

/// Parses a document with the default [`Options`].
pub fn parse(src: &str) -> Result<Document, Error> {
    parse_with(src, &Options::default())
}

/// Parses a document. A byte order mark at the start is skipped; an
/// encoding the XML declaration names is ignored, since the text is
/// decoded already.
///
/// ```
/// let e = parsec_xml::parse("<a>\n  <b></a>").unwrap_err();
/// assert_eq!(e.code(), "mismatched-end-tag");
/// assert_eq!(e.to_string(), "2:6: `</a>` closes `<b>` [mismatched-end-tag]");
/// ```
pub fn parse_with(src: &str, options: &Options) -> Result<Document, Error> {
    let mut parser = parse::Parser::new(src, options);
    match parser.document() {
        Ok(()) => Ok(parser.doc),
        Err(d) => Err(Error::new(d, src)),
    }
}
