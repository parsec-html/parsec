// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XML in the C API: `parsec_xml_parse` gives a `parsec_xml_document`,
//! read with `parsec_xml_*` functions that mirror the HTML ones for
//! walking the tree, names, attributes, text, serialization and source
//! positions. An XML document is read-only, so there are no edits, and
//! none of the busy-document bookkeeping HTML's need.

use std::ffi::c_char;
use std::ptr;

use encoding::Offsets;
use xml::{LineIndex, NodeId, NodeKind};

use crate::{
    PARSEC_ERR_INVALID, PARSEC_ERR_NULL, PARSEC_ERR_TOO_LARGE, PARSEC_NO_NODE, PARSEC_NODE_CDATA,
    PARSEC_NODE_COMMENT, PARSEC_NODE_DOCTYPE, PARSEC_NODE_DOCUMENT, PARSEC_NODE_ELEMENT,
    PARSEC_NODE_PROCESSING_INSTRUCTION, PARSEC_NODE_TEXT, borrowed, bytes, clear, guard, owned,
    parsec_node, parsec_status, parsec_string_free, parsec_syntax_error, put, status, text,
};

/// A parsed XML document. Made by `parsec_xml_parse`, freed by
/// `parsec_xml_document_free`. It can't be changed, so any number of
/// threads may read it at once.
pub struct parsec_xml_document {
    doc: xml::Document,
    /// Lines in the text the input decoded to, for line and column.
    lines: LineIndex<'static>,
    /// The encoding's name, NUL-terminated.
    encoding: Box<str>,
    /// Turns offsets into `text` into offsets into the input; None when
    /// they're the same (UTF-8 with no byte order mark).
    offsets: Option<Offsets>,
}

/// The XML isn't well-formed, nests deeper than its `max_depth`, or
/// expands entities past its limits: the `parsec_xml_error` says which,
/// and where.
pub const PARSEC_ERR_XML: parsec_status = 17;

/// What `parsec_xml_node_type` gives for a reference to an entity Parsec
/// didn't read (one declared external): the DOM's
/// `ENTITY_REFERENCE_NODE`.
pub const PARSEC_NODE_ENTITY_REFERENCE: u32 = 5;

/// Parse without Namespaces in XML: a name is any XML name, colons and
/// all, in no namespace.
pub const PARSEC_XML_NO_NAMESPACES: u32 = 1;

/// How `parsec_xml_parse` parses. NULL in its place is all zeros, and a
/// 0 field is its default.
#[repr(C)]
pub struct parsec_xml_options {
    /// 0, or `PARSEC_XML_NO_NAMESPACES`.
    pub flags: u32,
    /// How deep elements may nest; 0 is 2,048. Deeper fails with
    /// `PARSEC_ERR_XML` and the code `too-deep`.
    pub max_depth: u32,
    /// How deep entity references may nest in the entities they refer
    /// to; 0 is 16, and more than 64 is 64.
    pub max_entity_depth: u32,
    /// Entity expansion may add this many bytes to any document; 0 is
    /// 1 MiB...
    pub entity_allowance: u64,
    /// ...plus this many for each byte of the input; 0 is 8. Past either,
    /// the parse fails with `PARSEC_ERR_XML` and the code
    /// `entity-expansion-limit`, so a billion laughs is an error, not a
    /// hang.
    pub entity_ratio: u64,
}

/// Why XML didn't parse, and where. Made by `parsec_xml_parse`, freed by
/// `parsec_xml_error_free`, which frees its message too.
#[repr(C)]
pub struct parsec_xml_error {
    /// Its stable name, `code_len` bytes with no NUL after them, static:
    /// `mismatched-end-tag`, `too-deep`, `entity-expansion-limit` and the
    /// rest, as the wiki's XML page lists them.
    pub code: *const c_char,
    pub code_len: usize,
    /// A sentence saying what's wrong, NUL-terminated.
    pub message: *mut c_char,
    pub message_len: usize,
    /// From 1.
    pub line: usize,
    /// From 1, in characters.
    pub column: usize,
}

/// An attribute, as `parsec_xml_attr_at` gives it. Every string is
/// borrowed from the document, as `parsec_node_name` says, and is empty,
/// not NULL, when the attribute has no prefix or no namespace.
#[repr(C)]
pub struct parsec_xml_attr {
    /// The name as written: `xlink:href`.
    pub name: *const c_char,
    pub name_len: usize,
    /// `href`.
    pub local_name: *const c_char,
    pub local_name_len: usize,
    /// `xlink`.
    pub prefix: *const c_char,
    pub prefix_len: usize,
    /// The namespace the prefix is bound to:
    /// `http://www.w3.org/1999/xlink`.
    pub namespace_uri: *const c_char,
    pub namespace_uri_len: usize,
    /// The value, normalized as XML says: references replaced, and white
    /// space characters made spaces.
    pub value: *const c_char,
    pub value_len: usize,
    /// Where it was written, as byte offsets into the input from its name
    /// to its closing quote; the start tag's, for a default from the DTD.
    pub start: usize,
    pub end: usize,
    /// False for a default the DTD supplied.
    pub specified: bool,
}

unsafe fn handle<'a>(
    doc: *const parsec_xml_document,
) -> Result<&'a parsec_xml_document, parsec_status> {
    // SAFETY: the caller passes NULL or a live document (header preamble).
    unsafe { doc.as_ref() }.ok_or(PARSEC_ERR_NULL)
}

fn node_of(d: &parsec_xml_document, n: parsec_node) -> Option<NodeId> {
    d.doc.node(n as usize)
}

/// An offset into the decoded text as one into the input.
fn source(d: &parsec_xml_document, offset: u32) -> usize {
    let offset = offset as usize;
    d.offsets.as_ref().map_or(offset, |o| o.to_source(offset))
}

/// Puts `e` in `*err`, if `err` isn't NULL, and gives the status for it.
unsafe fn xml_error(e: &xml::Error, err: *mut *mut parsec_xml_error) -> parsec_status {
    if !err.is_null() {
        let mut message_len = 0;
        // SAFETY: the length is written to a local.
        let message = unsafe { owned(&e.diagnostic.message, &mut message_len) };
        let boxed = Box::new(parsec_xml_error {
            code: e.code().as_ptr().cast(),
            code_len: e.code().len(),
            message,
            message_len,
            line: e.position.line as usize,
            column: e.position.column as usize,
        });
        // SAFETY: a non-NULL `err` points at writable space (header).
        unsafe { err.write(Box::into_raw(boxed)) };
    }
    match e.code() {
        "input-too-large" => PARSEC_ERR_TOO_LARGE,
        _ => PARSEC_ERR_XML,
    }
}

/// Parses `len` bytes as an XML document into `*out`, to free with
/// `parsec_xml_document_free`. The bytes are decoded as XML says: from
/// the byte order mark, else the first bytes' pattern and the encoding
/// the XML declaration names (`<?xml version="1.0"
/// encoding="ISO-8859-1"?>`), else UTF-8; bytes that aren't that
/// encoding are an error, not U+FFFD. `opts` may be NULL. Nothing is
/// ever fetched: the external subset and external entities are left
/// unread, and the document's warnings say so.
///
/// Fails with `PARSEC_ERR_XML` for XML that isn't well-formed or runs
/// past a limit in `opts`, and `PARSEC_ERR_TOO_LARGE` over 4 GiB; each
/// puts a `parsec_xml_error` in `*err`, if `err` isn't NULL, to free with
/// `parsec_xml_error_free`. An unknown flag fails with
/// `PARSEC_ERR_INVALID`. On any error `*out` is NULL, and `*err` is NULL
/// unless set as above.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_parse(
    src: *const c_char,
    len: usize,
    opts: *const parsec_xml_options,
    out: *mut *mut parsec_xml_document,
    err: *mut *mut parsec_xml_error,
) -> parsec_status {
    guard(crate::PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            if let Some(err) = unsafe { err.as_mut() } {
                *err = ptr::null_mut();
            }
            let out = unsafe { clear(out)? };
            let input = unsafe { bytes(src, len)? };
            let o = unsafe { opts.as_ref() };
            let flags = o.map_or(0, |o| o.flags);
            if flags & !PARSEC_XML_NO_NAMESPACES != 0 {
                return Err(PARSEC_ERR_INVALID);
            }
            let mut options = xml::Options {
                namespaces: flags & PARSEC_XML_NO_NAMESPACES == 0,
                ..xml::Options::default()
            };
            if let Some(o) = o {
                let or = |v: u64, d: u64| if v == 0 { d } else { v };
                if o.max_depth != 0 {
                    options.limits.max_depth = Some(o.max_depth);
                }
                options.max_entity_depth = or(o.max_entity_depth.into(), 16) as u32;
                options.entity_allowance = or(o.entity_allowance, options.entity_allowance);
                options.entity_ratio = or(o.entity_ratio, options.entity_ratio);
            }
            let parsed =
                xml::parse_bytes(input, &options).map_err(|e| unsafe { xml_error(&e, err) })?;
            let offsets = Some(Offsets::new(input, parsed.encoding)).filter(|o| !o.is_identity());
            *out = Box::into_raw(Box::new(parsec_xml_document {
                encoding: format!("{}\0", parsed.encoding.name()).into(),
                lines: LineIndex::owned(parsed.text.into_owned()),
                doc: parsed.document,
                offsets,
            }));
            Ok(())
        })())
    })
}

/// Frees a `parsec_xml_error` and its message. NULL is fine and does
/// nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_error_free(err: *mut parsec_xml_error) {
    if err.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: `err` came from `xml_error`'s Box, its message from
        // `owned`.
        let e = unsafe { Box::from_raw(err) };
        unsafe { parsec_string_free(e.message) };
    })
}

/// Frees a document. NULL is fine and does nothing. Every node number,
/// and every string borrowed from it, is invalid afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_document_free(doc: *mut parsec_xml_document) {
    if doc.is_null() {
        return;
    }
    // SAFETY: `doc` came from parsec_xml_parse's Box.
    guard((), || drop(unsafe { Box::from_raw(doc) }))
}

/// The Encoding Standard's name for what the bytes were decoded from,
/// such as "UTF-8" or "windows-1252", NUL-terminated and valid as long as
/// the document; NULL for a NULL handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_document_encoding(
    doc: *const parsec_xml_document,
) -> *const c_char {
    // SAFETY: NULL or a live document.
    guard(ptr::null(), || {
        unsafe { handle(doc) }.map_or(ptr::null(), |d| d.encoding.as_ptr().cast())
    })
}

/// How many nodes the document holds: every node number below this is
/// valid. The document itself is node 0, as `parsec_root` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_node_count(doc: *const parsec_xml_document) -> usize {
    // SAFETY: NULL or a live document.
    guard(0, || unsafe { handle(doc) }.map_or(0, |d| d.doc.len()))
}

/// Follows one link from `node`, or gives PARSEC_NO_NODE.
unsafe fn step(
    doc: *const parsec_xml_document,
    node: parsec_node,
    link: fn(&xml::Document, NodeId) -> Option<NodeId>,
) -> parsec_node {
    guard(PARSEC_NO_NODE, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { handle(doc) }) else {
            return PARSEC_NO_NODE;
        };
        node_of(d, node)
            .and_then(|n| link(&d.doc, n))
            .map_or(PARSEC_NO_NODE, |n| n.index() as parsec_node)
    })
}

/// The document element, the one element at the top.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_document_element(
    doc: *const parsec_xml_document,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, 0, |d, _| Some(d.root_element())) }
}

/// The node's parent, or `PARSEC_NO_NODE` for the document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_parent(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, xml::Document::parent) }
}

/// The node's first child, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_first_child(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, xml::Document::first_child) }
}

/// The node's last child, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_last_child(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, xml::Document::last_child) }
}

/// The next node with the same parent, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_next_sibling(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, xml::Document::next_sibling) }
}

/// The previous node with the same parent, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_prev_sibling(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, xml::Document::prev_sibling) }
}

/// What the node is, as a `PARSEC_NODE_*` number (`PARSEC_NODE_CDATA`
/// for a CDATA section, `PARSEC_NODE_ENTITY_REFERENCE` for an entity
/// Parsec didn't read); 0 if `node` isn't in the document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_node_type(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> u32 {
    guard(0, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { handle(doc) }) else {
            return 0;
        };
        let Some(n) = node_of(d, node) else { return 0 };
        match d.doc.kind(n) {
            NodeKind::Document => PARSEC_NODE_DOCUMENT,
            NodeKind::Doctype => PARSEC_NODE_DOCTYPE,
            NodeKind::Element(_) => PARSEC_NODE_ELEMENT,
            NodeKind::Text(_) => PARSEC_NODE_TEXT,
            NodeKind::CData(_) => PARSEC_NODE_CDATA,
            NodeKind::Comment(_) => PARSEC_NODE_COMMENT,
            NodeKind::ProcessingInstruction { .. } => PARSEC_NODE_PROCESSING_INSTRUCTION,
            NodeKind::EntityRef(_) => PARSEC_NODE_ENTITY_REFERENCE,
        }
    })
}

/// Reads one borrowed string about a node, or NULL with length 0.
unsafe fn about(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
    pick: impl FnOnce(&xml::Document, NodeId) -> Option<&str>,
) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document; `len` NULL or writable.
        let d = unsafe { handle(doc) }.ok();
        let s = d.and_then(|d| node_of(d, node).and_then(|n| pick(&d.doc, n)));
        unsafe { borrowed(s, len) }
    })
}

/// The node's name: an element's as written, prefix and all
/// (`atom:link`), the doctype's, a processing instruction's target, or
/// the entity an entity reference names. NULL for any other node.
/// Borrowed, as every string from a document is: `*len` bytes of UTF-8
/// with no NUL after them, valid until the document is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_node_name(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |d, n| match d.kind(n) {
            NodeKind::Element(name) => Some(name.qname),
            NodeKind::Doctype => d.doctype().map(|t| t.name.as_str()),
            NodeKind::ProcessingInstruction { target, .. } => Some(target),
            NodeKind::EntityRef(name) => Some(name),
            _ => None,
        })
    }
}

/// An element's local name, `link` for `atom:link`, borrowed; NULL for
/// any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_local_name(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe { about(doc, node, len, |d, n| d.name(n).map(|name| name.local)) }
}

/// An element's prefix, `atom` for `atom:link`, borrowed; empty for none,
/// NULL for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_prefix(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe { about(doc, node, len, |d, n| d.name(n).map(|name| name.prefix)) }
}

/// The namespace URI an element's prefix (or the default namespace) is
/// bound to, borrowed; empty for none, NULL for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_namespace_uri(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe { about(doc, node, len, |d, n| d.name(n).map(|name| name.namespace)) }
}

/// A text, CDATA or comment node's text, or a processing instruction's
/// data, borrowed; NULL for any other node. For an element's text,
/// `parsec_xml_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_node_value(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |d, n| match d.kind(n) {
            NodeKind::Text(s) | NodeKind::CData(s) | NodeKind::Comment(s) => Some(s),
            NodeKind::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        })
    }
}

/// How many attributes an element has, namespace declarations and the
/// DTD's defaults included; 0 for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_attr_count(
    doc: *const parsec_xml_document,
    node: parsec_node,
) -> usize {
    guard(0, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { handle(doc) }) else {
            return 0;
        };
        node_of(d, node).map_or(0, |n| d.doc.attributes(n).len())
    })
}

/// An element's `index`th attribute into `*out`: in the order of its
/// start tag, then the defaults the DTD supplied. `PARSEC_ERR_INVALID` if
/// `index` is past the last, or the node isn't an element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_attr_at(
    doc: *const parsec_xml_document,
    node: parsec_node,
    index: usize,
    out: *mut parsec_xml_attr,
) -> parsec_status {
    guard(crate::PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; `out` NULL or writable.
            let d = unsafe { handle(doc)? };
            if out.is_null() {
                return Err(PARSEC_ERR_NULL);
            }
            let n = node_of(d, node).ok_or(crate::PARSEC_ERR_NO_SUCH_NODE)?;
            let a = d.doc.attributes(n).nth(index).ok_or(PARSEC_ERR_INVALID)?;
            let s = |s: &str| -> (*const c_char, usize) { (s.as_ptr().cast(), s.len()) };
            let (name, name_len) = s(a.name.qname);
            let (local_name, local_name_len) = s(a.name.local);
            let (prefix, prefix_len) = s(a.name.prefix);
            let (namespace_uri, namespace_uri_len) = s(a.name.namespace);
            let (value, value_len) = s(a.value);
            unsafe {
                out.write(parsec_xml_attr {
                    name,
                    name_len,
                    local_name,
                    local_name_len,
                    prefix,
                    prefix_len,
                    namespace_uri,
                    namespace_uri_len,
                    value,
                    value_len,
                    start: source(d, a.span.start),
                    end: source(d, a.span.end),
                    specified: a.specified,
                })
            };
            Ok(())
        })())
    })
}

/// The value of the element's attribute named `name` as written, prefix
/// and all (`xlink:href`), borrowed; NULL if there's no such attribute,
/// or the node isn't an element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_attr_get(
    doc: *const parsec_xml_document,
    node: parsec_node,
    name: *const c_char,
    name_len: usize,
    value_len: *mut usize,
) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document; `name` has `name_len` bytes.
        let d = unsafe { handle(doc) }.ok();
        let want = unsafe { text(name, name_len) }.ok();
        let found = d
            .zip(want)
            .and_then(|(d, want)| d.doc.attribute_named(node_of(d, node)?, &want));
        unsafe { borrowed(found, value_len) }
    })
}

/// The value of the element's attribute in namespace `ns` (empty, or
/// NULL with a length of 0, for none) with local name `local`, whatever
/// its prefix, borrowed; NULL if there's no such attribute.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_attr_get_ns(
    doc: *const parsec_xml_document,
    node: parsec_node,
    ns: *const c_char,
    ns_len: usize,
    local: *const c_char,
    local_len: usize,
    value_len: *mut usize,
) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document; the strings have their lengths.
        let d = unsafe { handle(doc) }.ok();
        let ns = unsafe { text(ns, ns_len) }.ok();
        let local = unsafe { text(local, local_len) }.ok();
        let found = d
            .zip(ns.zip(local))
            .and_then(|(d, (ns, local))| d.doc.attribute(node_of(d, node)?, &ns, &local));
        unsafe { borrowed(found, value_len) }
    })
}

unsafe fn written(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
    write: fn(&xml::Document, NodeId) -> String,
) -> *mut c_char {
    guard(ptr::null_mut(), || {
        // SAFETY: NULL or a live document; `len` NULL or writable.
        let Ok(d) = (unsafe { handle(doc) }) else {
            return ptr::null_mut();
        };
        let Some(n) = node_of(d, node) else {
            return ptr::null_mut();
        };
        unsafe { owned(&write(&d.doc, n), len) }
    })
}

/// The node and everything under it as XML; the whole document, its XML
/// declaration and `<!DOCTYPE>` included, for the root. An element on
/// its own takes along the namespace declarations it's in the scope of,
/// so it means the same. The string is yours: NUL-terminated, its length
/// without the NUL in `*len` (if `len` isn't NULL), freed with
/// `parsec_string_free`. NULL if `node` isn't in the document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_serialize(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *mut c_char {
    // SAFETY: passed on as given.
    unsafe {
        written(doc, node, len, |d, n| {
            let mut out = String::new();
            d.write_xml(n, &mut out);
            out
        })
    }
}

/// The text of the node and everything under it, character data and
/// CDATA in order, as XPath's `string()` reads it. Yours to free, as
/// `parsec_xml_serialize` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_text(
    doc: *const parsec_xml_document,
    node: parsec_node,
    len: *mut usize,
) -> *mut c_char {
    // SAFETY: passed on as given.
    unsafe { written(doc, node, len, xml::Document::text) }
}

/// Where `node` was written, as byte offsets into the input from
/// `*start` up to `*end`: an element from its start tag's `<` to its end
/// tag's `>`. A node an entity reference produced has the reference's
/// place. False, leaving both alone, for a NULL handle or a bad node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_node_span(
    doc: *const parsec_xml_document,
    node: parsec_node,
    start: *mut usize,
    end: *mut usize,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or a live document; out pointers NULL or writable.
        let Ok(d) = (unsafe { handle(doc) }) else {
            return false;
        };
        let Some(n) = node_of(d, node) else {
            return false;
        };
        let span = d.doc.span(n);
        unsafe {
            put(start, source(d, span.start));
            put(end, source(d, span.end));
        }
        true
    })
}

/// The line and column of a byte offset into the input, as
/// `parsec_line_col` gives them for HTML. It reads the input up to
/// `offset` each time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_line_col(
    doc: *const parsec_xml_document,
    offset: usize,
    line: *mut usize,
    column: *mut usize,
) -> parsec_status {
    guard(crate::PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; out pointers NULL or writable.
            let d = unsafe { handle(doc)? };
            let offset = d.offsets.as_ref().map_or(offset, |o| o.to_decoded(offset));
            let at = d.lines.line_col(offset.min(u32::MAX as usize) as u32);
            unsafe {
                put(line, at.line as usize);
                put(column, at.column as usize);
            }
            Ok(())
        })())
    })
}

/// How many warnings the parse gave: what it didn't read and says so, as
/// the XML spec asks of a processor that doesn't validate (the external
/// subset, an external entity). None of them is an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_warnings_count(doc: *const parsec_xml_document) -> usize {
    // SAFETY: NULL or a live document.
    guard(0, || {
        unsafe { handle(doc) }.map_or(0, |d| d.doc.warnings().len())
    })
}

/// The `index`th warning, in source order, into `*out`: its code (such
/// as `external-dtd-not-loaded`), and where it is in the input.
/// `PARSEC_ERR_INVALID` if `index` is past the last.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xml_warnings_get(
    doc: *const parsec_xml_document,
    index: usize,
    out: *mut parsec_syntax_error,
) -> parsec_status {
    guard(crate::PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; `out` NULL or writable.
            let d = unsafe { handle(doc)? };
            if out.is_null() {
                return Err(PARSEC_ERR_NULL);
            }
            let w = d.doc.warnings().get(index).ok_or(PARSEC_ERR_INVALID)?;
            let at = d.lines.line_col(w.span.start);
            unsafe {
                out.write(parsec_syntax_error {
                    code: w.code.as_ptr().cast(),
                    code_len: w.code.len(),
                    offset: source(d, w.span.start),
                    line: at.line as usize,
                    column: at.column as usize,
                })
            };
            Ok(())
        })())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_xml_status_has_a_message() {
        let m = unsafe { std::ffi::CStr::from_ptr(crate::parsec_status_message(PARSEC_ERR_XML)) };
        assert_ne!(m.to_str().unwrap(), "unknown status");
    }
}
