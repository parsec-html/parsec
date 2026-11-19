// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Parsec's C API. `include/parsec.h` is generated from this file by
//! cbindgen, so the doc comments here are the header's documentation.
//!
//! Every handle is opaque and every function checks its pointers, so a
//! NULL or a node number that isn't in the document gives an error rather
//! than a crash. Nothing unwinds into C: each function runs inside
//! `catch_unwind`, and a panic comes back as `PARSEC_ERR_PANIC`.

// The C names are the API: snake_case types, as C writes them.
#![allow(non_camel_case_types)]
// The rules for every pointer argument are stated once, at the top of the
// header (cbindgen.toml), rather than on each function.
#![allow(clippy::missing_safety_doc)]

use std::borrow::Cow;
use std::ffi::{c_char, c_int, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;

use html::css::SelectorList;
use html::{Document, Kind, NodeId, Ns, Options, ParseError};

/// A parsed document. Made by `parsec_parse` and friends, freed by
/// `parsec_document_free`.
pub struct parsec_document(Document);

/// A compiled selector list. Made by `parsec_selector_compile`, freed by
/// `parsec_selector_free`; compile once and run it on as many documents as
/// you like.
pub struct parsec_selector(SelectorList);

/// A node, numbered within its document. The document itself is node 0.
/// A number stays valid, and keeps meaning the same node, for as long as
/// the document lives: removing a node only detaches it.
pub type parsec_node = u32;

/// No node: what navigation returns at the end of a list, and what you pass
/// to `parsec_insert_before` to insert last.
pub const PARSEC_NO_NODE: parsec_node = u32::MAX;

/// What a function that can fail returns. `parsec_status_message` names it.
pub type parsec_status = i32;

pub const PARSEC_OK: parsec_status = 0;
/// A pointer that must not be NULL was NULL.
pub const PARSEC_ERR_NULL: parsec_status = 1;
/// An argument was out of range: an unknown flag or namespace, or a length
/// over `PTRDIFF_MAX`.
pub const PARSEC_ERR_INVALID: parsec_status = 2;
/// The input is over the 512 MiB a document can hold.
pub const PARSEC_ERR_TOO_LARGE: parsec_status = 3;
/// The parse ran past the budget given to `parsec_parse_with`.
pub const PARSEC_ERR_TIMEOUT: parsec_status = 4;
/// The selector doesn't parse, or uses something Parsec doesn't support.
pub const PARSEC_ERR_SELECTOR: parsec_status = 5;
/// The node number isn't a node of this document.
pub const PARSEC_ERR_NO_SUCH_NODE: parsec_status = 6;
/// The node has to be an element for this call.
pub const PARSEC_ERR_NOT_AN_ELEMENT: parsec_status = 7;
/// The node would end up inside itself.
pub const PARSEC_ERR_CYCLE: parsec_status = 8;
/// The parent can't have children: it's text, a comment, a doctype or a
/// processing instruction.
pub const PARSEC_ERR_NOT_A_PARENT: parsec_status = 9;
/// The node to insert before isn't a child of the parent.
pub const PARSEC_ERR_NOT_A_CHILD: parsec_status = 10;
/// The node is a document or a fragment (the root, or a template's
/// contents), which is never anyone's child.
pub const PARSEC_ERR_NOT_INSERTABLE: parsec_status = 11;
/// A bug in Parsec: it panicked, and the call was stopped before the panic
/// reached C. A document that was being changed may be half-changed; free
/// it. Please report it.
pub const PARSEC_ERR_PANIC: parsec_status = 12;

/// Parse with scripting on, as a browser with JavaScript does: `<noscript>`
/// holds raw text.
pub const PARSEC_SCRIPTING: u32 = 1;

/// A namespace, for elements and fragment contexts. 0 is no namespace:
/// what `parsec_element_ns` gives for a node that isn't an element.
pub type parsec_ns = u32;
pub const PARSEC_NS_HTML: parsec_ns = 1;
pub const PARSEC_NS_SVG: parsec_ns = 2;
pub const PARSEC_NS_MATHML: parsec_ns = 3;

// What parsec_node_type returns: the DOM's `nodeType` numbers, which are
// Lexbor's `LXB_DOM_NODE_TYPE_*` too. 0 is no node.
pub const PARSEC_NODE_ELEMENT: u32 = 1;
pub const PARSEC_NODE_TEXT: u32 = 3;
pub const PARSEC_NODE_CDATA: u32 = 4;
pub const PARSEC_NODE_PROCESSING_INSTRUCTION: u32 = 7;
pub const PARSEC_NODE_COMMENT: u32 = 8;
pub const PARSEC_NODE_DOCUMENT: u32 = 9;
pub const PARSEC_NODE_DOCTYPE: u32 = 10;
pub const PARSEC_NODE_FRAGMENT: u32 = 11;

/// Called by `parsec_select` with each match, in document order, and the
/// `ctx` you passed. Return 0 to go on, anything else to stop there.
pub type parsec_match_fn =
    Option<unsafe extern "C" fn(node: parsec_node, ctx: *mut c_void) -> c_int>;

// ---- plumbing --------------------------------------------------------------

/// Runs `f`, or gives `fallback` if it panics, so nothing unwinds into C.
fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn status(r: Result<(), parsec_status>) -> parsec_status {
    r.err().unwrap_or(PARSEC_OK)
}

unsafe fn doc_ref<'a>(doc: *const parsec_document) -> Result<&'a Document, parsec_status> {
    // SAFETY: the caller passes NULL or a live document (header preamble).
    unsafe { doc.as_ref() }.map(|d| &d.0).ok_or(PARSEC_ERR_NULL)
}

unsafe fn doc_mut<'a>(doc: *mut parsec_document) -> Result<&'a mut Document, parsec_status> {
    // SAFETY: as doc_ref, and no other call is using the document (writes
    // are one at a time, the header says).
    unsafe { doc.as_mut() }
        .map(|d| &mut d.0)
        .ok_or(PARSEC_ERR_NULL)
}

fn node(doc: &Document, n: parsec_node) -> Result<NodeId, parsec_status> {
    if (n as usize) < doc.len() {
        Ok(n)
    } else {
        Err(PARSEC_ERR_NO_SUCH_NODE)
    }
}

/// `len` bytes at `s` as text. Bytes that aren't UTF-8 become U+FFFD, as
/// decoding a page as UTF-8 would make them. NULL is the empty string when
/// `len` is 0.
unsafe fn text<'a>(s: *const c_char, len: usize) -> Result<Cow<'a, str>, parsec_status> {
    if len > isize::MAX as usize {
        return Err(PARSEC_ERR_INVALID);
    }
    if s.is_null() {
        return if len == 0 {
            Ok(Cow::Borrowed(""))
        } else {
            Err(PARSEC_ERR_NULL)
        };
    }
    // SAFETY: the caller gives `len` readable bytes at `s`, which stay put
    // for the call; the length was checked above.
    let bytes = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) };
    Ok(String::from_utf8_lossy(bytes))
}

unsafe fn put<T>(out: *mut T, value: T) {
    if !out.is_null() {
        // SAFETY: a non-NULL out pointer points at writable space for a T.
        unsafe { out.write(value) }
    }
}

/// A string borrowed from the document: its bytes and, through `len`, its
/// length. NULL, with a length of 0, for none.
unsafe fn borrowed(s: Option<&str>, len: *mut usize) -> *const c_char {
    if len.is_null() {
        return ptr::null();
    }
    let (p, n) = s.map_or((ptr::null(), 0), |s| (s.as_ptr().cast(), s.len()));
    // SAFETY: checked non-NULL above.
    unsafe { len.write(n) };
    p
}

/// The bytes ahead of an owned string's text, holding the allocation's size
/// so `parsec_string_free` needs only the pointer.
const HEADER: usize = size_of::<usize>();

/// A copy of `s` the caller frees with `parsec_string_free`: NUL-terminated,
/// with its length (without the NUL) through `len` if that isn't NULL.
unsafe fn owned(s: &str, len: *mut usize) -> *mut c_char {
    let size = HEADER + s.len() + 1;
    let mut bytes = Vec::with_capacity(size);
    bytes.extend_from_slice(&size.to_ne_bytes());
    bytes.extend_from_slice(s.as_bytes());
    bytes.push(0);
    let base = Box::into_raw(bytes.into_boxed_slice()).cast::<u8>();
    // SAFETY: `len` is NULL or writable (put checks).
    unsafe { put(len, s.len()) };
    // SAFETY: the allocation is `size` > HEADER bytes long.
    unsafe { base.add(HEADER) }.cast()
}

fn edit_status(e: html::EditError) -> parsec_status {
    match e {
        html::EditError::Cycle => PARSEC_ERR_CYCLE,
        html::EditError::NotAParent => PARSEC_ERR_NOT_A_PARENT,
        html::EditError::NotAChild => PARSEC_ERR_NOT_A_CHILD,
        html::EditError::NotInsertable => PARSEC_ERR_NOT_INSERTABLE,
    }
}

fn namespace(ns: parsec_ns) -> Result<Ns, parsec_status> {
    match ns {
        PARSEC_NS_HTML => Ok(Ns::Html),
        PARSEC_NS_SVG => Ok(Ns::Svg),
        PARSEC_NS_MATHML => Ok(Ns::MathMl),
        _ => Err(PARSEC_ERR_INVALID),
    }
}

fn scripting(flags: u32) -> Result<bool, parsec_status> {
    if flags & !PARSEC_SCRIPTING != 0 {
        return Err(PARSEC_ERR_INVALID);
    }
    Ok(flags & PARSEC_SCRIPTING != 0)
}

fn parsed(
    r: Result<Document, ParseError>,
    out: &mut *mut parsec_document,
) -> Result<(), parsec_status> {
    let doc = r.map_err(|e| match e {
        ParseError::TooLarge { .. } => PARSEC_ERR_TOO_LARGE,
        ParseError::Timeout { .. } => PARSEC_ERR_TIMEOUT,
    })?;
    *out = Box::into_raw(Box::new(parsec_document(doc)));
    Ok(())
}

/// The out pointer as a reference, with `*out` cleared first so it's NULL
/// on every error.
unsafe fn clear<'a, T>(out: *mut *mut T) -> Result<&'a mut *mut T, parsec_status> {
    // SAFETY: the caller passes NULL or a pointer to writable space for a
    // pointer, which nothing else touches during the call.
    let out = unsafe { out.as_mut() }.ok_or(PARSEC_ERR_NULL)?;
    *out = ptr::null_mut();
    Ok(out)
}

// ---- the library -----------------------------------------------------------

/// The library's version, such as "0.1.0", NUL-terminated and static.
#[unsafe(no_mangle)]
pub extern "C" fn parsec_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// A sentence saying what `status` means, NUL-terminated and static.
#[unsafe(no_mangle)]
pub extern "C" fn parsec_status_message(status: parsec_status) -> *const c_char {
    let s: &'static str = match status {
        PARSEC_OK => "no error\0",
        PARSEC_ERR_NULL => "a pointer that must not be NULL was NULL\0",
        PARSEC_ERR_INVALID => "an argument was out of range\0",
        PARSEC_ERR_TOO_LARGE => "the input is over the 512 MiB limit\0",
        PARSEC_ERR_TIMEOUT => "the parse ran past its budget\0",
        PARSEC_ERR_SELECTOR => "the selector isn't valid, or isn't supported\0",
        PARSEC_ERR_NO_SUCH_NODE => "no such node in this document\0",
        PARSEC_ERR_NOT_AN_ELEMENT => "the node isn't an element\0",
        PARSEC_ERR_CYCLE => "a node can't be inserted into itself or its descendants\0",
        PARSEC_ERR_NOT_A_PARENT => "only an element, a document or a fragment can have children\0",
        PARSEC_ERR_NOT_A_CHILD => "the node to insert before isn't a child of the parent\0",
        PARSEC_ERR_NOT_INSERTABLE => {
            "a document or a fragment can't be inserted, only its children\0"
        }
        PARSEC_ERR_PANIC => "internal error in Parsec (please report it)\0",
        _ => "unknown status\0",
    };
    s.as_ptr().cast()
}

/// Frees a string a `parsec_*_html` or `parsec_text` call returned. NULL
/// is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_string_free(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    guard((), || {
        // SAFETY: `s` came from `owned`, so HEADER bytes before it start an
        // allocation of the size stored there, made by a boxed slice.
        unsafe {
            let base = s.cast::<u8>().sub(HEADER);
            let size = usize::from_ne_bytes(base.cast::<[u8; HEADER]>().read());
            drop(Box::from_raw(ptr::slice_from_raw_parts_mut(base, size)));
        }
    })
}

// ---- parsing ---------------------------------------------------------------

/// Parses `len` bytes of UTF-8 as a whole document and puts it in `*out`.
/// Bad markup never fails: the parser repairs it as a browser does. Bytes
/// that aren't UTF-8 become U+FFFD, and a leading byte order mark is
/// dropped. `flags` is 0 or `PARSEC_SCRIPTING`.
///
/// Fails with `PARSEC_ERR_TOO_LARGE` over 512 MiB. On any error `*out` is
/// NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_parse(
    html: *const c_char,
    len: usize,
    flags: u32,
    out: *mut *mut parsec_document,
) -> parsec_status {
    // SAFETY: the pointers are the caller's to vouch for (header preamble).
    unsafe { parsec_parse_with(html, len, flags, 0, 0, out) }
}

/// `parsec_parse`, with limits for input you don't control. Past
/// `budget_us` microseconds the parse gives up with `PARSEC_ERR_TIMEOUT`
/// (0 is no budget; the clock is read every 4,096 tokenizer steps, so a
/// small input always finishes). Past `max_depth` open elements, a new node
/// goes beside the current one instead of inside it, as Chromium does at
/// 512 (0 is no limit).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_parse_with(
    html: *const c_char,
    len: usize,
    flags: u32,
    budget_us: u64,
    max_depth: u32,
    out: *mut *mut parsec_document,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            let out = unsafe { clear(out)? };
            let html = unsafe { text(html, len)? };
            let options = Options {
                scripting: scripting(flags)?,
                budget: (budget_us != 0).then(|| std::time::Duration::from_micros(budget_us)),
                max_depth: (max_depth != 0).then_some(max_depth as usize),
            };
            parsed(html::parse_with(&html, options), out)
        })())
    })
}

/// Parses `len` bytes as the contents of an element named `context` in
/// namespace `ns`, as setting `innerHTML` on it does: "tbody" in
/// `PARSEC_NS_HTML` reads `<tr>` as a row. The result is a document of its
/// own whose root (node 0) is a fragment holding what was parsed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_parse_fragment(
    html: *const c_char,
    len: usize,
    ns: parsec_ns,
    context: *const c_char,
    context_len: usize,
    flags: u32,
    out: *mut *mut parsec_document,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            let out = unsafe { clear(out)? };
            let html = unsafe { text(html, len)? };
            let context = unsafe { text(context, context_len)? };
            let options = Options {
                scripting: scripting(flags)?,
                ..Options::default()
            };
            parsed(
                html::parse_fragment_with(&html, namespace(ns)?, &context, options),
                out,
            )
        })())
    })
}

/// Frees a document and everything in it: its nodes, and every string
/// borrowed from it. NULL is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_document_free(doc: *mut parsec_document) {
    if !doc.is_null() {
        // SAFETY: `doc` came from a parse and hasn't been freed.
        guard((), || drop(unsafe { Box::from_raw(doc) }))
    }
}

/// How many nodes the document holds, removed ones included: every node
/// number below this is valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_count(doc: *const parsec_document) -> usize {
    // SAFETY: NULL or a live document.
    guard(0, || unsafe { doc_ref(doc) }.map_or(0, |d| d.len()))
}

// ---- the tree --------------------------------------------------------------

/// Follows one link from `node`, or gives PARSEC_NO_NODE.
unsafe fn step(
    doc: *const parsec_document,
    node: parsec_node,
    link: fn(&Document, NodeId) -> Option<NodeId>,
) -> parsec_node {
    guard(PARSEC_NO_NODE, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return PARSEC_NO_NODE;
        };
        node_of(d, node)
            .and_then(|n| link(d, n))
            .unwrap_or(PARSEC_NO_NODE)
    })
}

fn node_of(doc: &Document, n: parsec_node) -> Option<NodeId> {
    node(doc, n).ok()
}

/// The document itself, which is always node 0.
#[unsafe(no_mangle)]
pub extern "C" fn parsec_root() -> parsec_node {
    0
}

/// The node's parent, or `PARSEC_NO_NODE` for the root and for a node
/// that's been removed or not yet inserted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_parent(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::parent) }
}

/// The node's first child, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_first_child(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::first_child) }
}

/// The node's last child, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_last_child(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::last_child) }
}

/// The next node with the same parent, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_next_sibling(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::next_sibling) }
}

/// The previous node with the same parent, or `PARSEC_NO_NODE`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_prev_sibling(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::prev_sibling) }
}

/// A `<template>` element's contents, a fragment whose children are what
/// the template holds; `PARSEC_NO_NODE` for any other node. A
/// template's children in the markup are here, not under the element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_template_contents(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { step(doc, node, Document::template_contents) }
}

/// What the node is, as a `PARSEC_NODE_*` number; 0 if `node` isn't in the
/// document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_type(doc: *const parsec_document, node: parsec_node) -> u32 {
    guard(0, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return 0;
        };
        let Some(n) = node_of(d, node) else { return 0 };
        match d.kind(n) {
            Kind::Element { .. } => PARSEC_NODE_ELEMENT,
            Kind::Text(_) => PARSEC_NODE_TEXT,
            Kind::Cdata(_) => PARSEC_NODE_CDATA,
            Kind::ProcessingInstruction { .. } => PARSEC_NODE_PROCESSING_INSTRUCTION,
            Kind::Comment(_) => PARSEC_NODE_COMMENT,
            Kind::Document => PARSEC_NODE_DOCUMENT,
            Kind::Doctype { .. } => PARSEC_NODE_DOCTYPE,
            Kind::Fragment => PARSEC_NODE_FRAGMENT,
        }
    })
}

/// An element's namespace as a `PARSEC_NS_*` number; 0 for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_element_ns(
    doc: *const parsec_document,
    node: parsec_node,
) -> parsec_ns {
    guard(0, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return 0;
        };
        match node_of(d, node).map(|n| d.kind(n)) {
            Some(Kind::Element { ns: Ns::Html, .. }) => PARSEC_NS_HTML,
            Some(Kind::Element { ns: Ns::Svg, .. }) => PARSEC_NS_SVG,
            Some(Kind::Element { ns: Ns::MathMl, .. }) => PARSEC_NS_MATHML,
            _ => 0,
        }
    })
}

/// Reads one borrowed string about a node, or NULL with length 0.
unsafe fn about(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
    pick: impl FnOnce(Kind<'_>) -> Option<&str>,
) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document; `len` NULL or writable.
        let d = unsafe { doc_ref(doc) }.ok();
        let s = d.and_then(|d| node_of(d, node).and_then(|n| pick(d.kind(n))));
        unsafe { borrowed(s, len) }
    })
}

/// The node's name: an element's as the markup gives it (lowercase for
/// HTML, `foreignObject` for SVG's), a doctype's, or a processing
/// instruction's target. NULL for any other node.
///
/// Like every string borrowed from a document, it's `*len` bytes of UTF-8
/// with no NUL after them, and it's valid until the document is changed or
/// freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_name(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |k| match k {
            Kind::Element { name, .. } => Some(name),
            Kind::Doctype { name, .. } => Some(name),
            Kind::ProcessingInstruction { target, .. } => Some(target),
            _ => None,
        })
    }
}

/// A text, comment or CDATA node's text, or a processing instruction's
/// data, borrowed as `parsec_node_name` says. NULL for any other node; for
/// an element's text, `parsec_text`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_value(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |k| match k {
            Kind::Text(s) | Kind::Comment(s) | Kind::Cdata(s) => Some(s),
            Kind::ProcessingInstruction { data, .. } => Some(data),
            _ => None,
        })
    }
}

/// A doctype's public identifier, borrowed; empty when it has none, NULL
/// for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_doctype_public_id(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |k| match k {
            Kind::Doctype { public_id, .. } => Some(public_id),
            _ => None,
        })
    }
}

/// A doctype's system identifier, borrowed; empty when it has none, NULL
/// for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_doctype_system_id(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: passed on as given.
    unsafe {
        about(doc, node, len, |k| match k {
            Kind::Doctype { system_id, .. } => Some(system_id),
            _ => None,
        })
    }
}

// ---- attributes ------------------------------------------------------------

/// How many attributes an element has; 0 for any other node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_count(
    doc: *const parsec_document,
    node: parsec_node,
) -> usize {
    guard(0, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return 0;
        };
        node_of(d, node).map_or(0, |n| d.attrs(n).count())
    })
}

/// An element's `index`th attribute, in source order: its name as written
/// (`xlink:href`, `viewBox`) and its value, both borrowed as
/// `parsec_node_name` says. Any of the four out pointers may be NULL if you
/// don't want that part. `PARSEC_ERR_INVALID` if `index` is past the last.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_at(
    doc: *const parsec_document,
    node: parsec_node,
    index: usize,
    name: *mut *const c_char,
    name_len: *mut usize,
    value: *mut *const c_char,
    value_len: *mut usize,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; out pointers NULL or writable.
            let d = unsafe { doc_ref(doc)? };
            let n = self::node(d, node)?;
            if !matches!(d.kind(n), Kind::Element { .. }) {
                return Err(PARSEC_ERR_NOT_AN_ELEMENT);
            }
            let (_, k, v) = d.attrs_written(n).nth(index).ok_or(PARSEC_ERR_INVALID)?;
            unsafe {
                put(name, k.as_ptr().cast());
                put(name_len, k.len());
                put(value, v.as_ptr().cast());
                put(value_len, v.len());
            }
            Ok(())
        })())
    })
}

/// The value of the element's attribute named `name`, as `getAttribute`
/// finds it: by the name as written (`xlink:href`), in any case on an HTML
/// element. Borrowed as `parsec_node_name` says; NULL if there's no such
/// attribute, or the node isn't an element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_get(
    doc: *const parsec_document,
    node: parsec_node,
    name: *const c_char,
    name_len: usize,
    value_len: *mut usize,
) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document; `name` has `name_len` bytes.
        let d = unsafe { doc_ref(doc) }.ok();
        let want = unsafe { text(name, name_len) }.ok();
        let found = d.zip(want).and_then(|(d, want)| {
            let n = node_of(d, node)?;
            let html = matches!(d.kind(n), Kind::Element { ns: Ns::Html, .. });
            d.attrs_written(n)
                .find(|(_, k, _)| {
                    if html {
                        k.eq_ignore_ascii_case(&want)
                    } else {
                        *k == want
                    }
                })
                .map(|(_, _, v)| v)
        });
        unsafe { borrowed(found, value_len) }
    })
}

/// Sets an attribute, as `setAttribute` does: it replaces the first one of
/// that name or adds one at the end. HTML elements' attribute names are
/// lowercased.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_set(
    doc: *mut parsec_document,
    node: parsec_node,
    name: *const c_char,
    name_len: usize,
    value: *const c_char,
    value_len: usize,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document no other call is using; the
            // strings have the lengths given.
            let d = unsafe { doc_mut(doc)? };
            let n = element(d, node)?;
            let (name, value) = unsafe { (text(name, name_len)?, text(value, value_len)?) };
            d.set_attr(n, &name, &value);
            Ok(())
        })())
    })
}

/// Removes the element's attribute named `name`, matched as
/// `parsec_attr_get` matches it. `*removed`, if `removed` isn't NULL, says
/// whether there was one; there not being one isn't an error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_remove(
    doc: *mut parsec_document,
    node: parsec_node,
    name: *const c_char,
    name_len: usize,
    removed: *mut bool,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as parsec_attr_set; `removed` NULL or writable.
            let d = unsafe { doc_mut(doc)? };
            let n = element(d, node)?;
            let name = unsafe { text(name, name_len)? };
            let gone = d.remove_attr(n, &name);
            unsafe { put(removed, gone) };
            Ok(())
        })())
    })
}

fn element(doc: &Document, n: parsec_node) -> Result<NodeId, parsec_status> {
    let n = node(doc, n)?;
    match doc.kind(n) {
        Kind::Element { .. } => Ok(n),
        _ => Err(PARSEC_ERR_NOT_AN_ELEMENT),
    }
}

// ---- text and serialization ------------------------------------------------

unsafe fn written(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
    write: fn(&Document, NodeId) -> String,
) -> *mut c_char {
    guard(ptr::null_mut(), || {
        // SAFETY: NULL or a live document; `len` NULL or writable.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return ptr::null_mut();
        };
        let Some(n) = node_of(d, node) else {
            return ptr::null_mut();
        };
        unsafe { owned(&write(d, n), len) }
    })
}

/// The markup for the node's children, as `innerHTML` gives it. The string
/// is yours: NUL-terminated, its length without the NUL in `*len` (if `len`
/// isn't NULL), and freed with `parsec_string_free`. NULL if `node` isn't
/// in the document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_inner_html(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *mut c_char {
    // SAFETY: passed on as given.
    unsafe { written(doc, node, len, Document::inner_html) }
}

/// The markup for the node and its children, as `outerHTML` gives it; the
/// whole page for the root. Yours to free, as `parsec_inner_html` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_outer_html(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *mut c_char {
    // SAFETY: passed on as given.
    unsafe { written(doc, node, len, Document::outer_html) }
}

/// The text of the node and everything under it, as `textContent` gives
/// it. Yours to free, as `parsec_inner_html` says.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_text(
    doc: *const parsec_document,
    node: parsec_node,
    len: *mut usize,
) -> *mut c_char {
    // SAFETY: passed on as given.
    unsafe { written(doc, node, len, Document::text) }
}

// ---- selectors -------------------------------------------------------------

/// Compiles a selector list (`len` bytes, such as `"ul > li.hit, p"`) into
/// `*out`. On `PARSEC_ERR_SELECTOR`, if `message` isn't NULL, `*message` is
/// a sentence saying what's wrong, yours to free with `parsec_string_free`.
/// On any error `*out` is NULL.
///
/// A selector can be shared: run it on any number of documents, from any
/// number of threads at once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_selector_compile(
    selector: *const c_char,
    len: usize,
    out: *mut *mut parsec_selector,
    message: *mut *mut c_char,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: `out` and `message` NULL or writable; `selector` has
            // `len` bytes.
            unsafe { put(message, ptr::null_mut()) };
            let out = unsafe { clear(out)? };
            let selector = unsafe { text(selector, len)? };
            match SelectorList::parse(&selector) {
                Ok(list) => {
                    *out = Box::into_raw(Box::new(parsec_selector(list)));
                    Ok(())
                }
                Err(e) => {
                    if !message.is_null() {
                        unsafe { message.write(owned(&e.reason, ptr::null_mut())) };
                    }
                    Err(PARSEC_ERR_SELECTOR)
                }
            }
        })())
    })
}

/// Frees a compiled selector. NULL is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_selector_free(selector: *mut parsec_selector) {
    if !selector.is_null() {
        // SAFETY: `selector` came from parsec_selector_compile and hasn't
        // been freed.
        guard((), || drop(unsafe { Box::from_raw(selector) }))
    }
}

/// Calls `callback` with each element under `scope` that matches, in
/// document order, and stops early if it returns nonzero. `scope` itself is
/// never a match, and is what `:scope` matches. As in a browser,
/// combinators look past it: `article .price` run on an article finds the
/// prices inside it.
///
/// Don't change the document from the callback; collect the nodes and
/// change it afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_select(
    doc: *const parsec_document,
    selector: *const parsec_selector,
    scope: parsec_node,
    callback: parsec_match_fn,
    ctx: *mut c_void,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or live handles.
            let d = unsafe { doc_ref(doc)? };
            let s = unsafe { selector.as_ref() }.ok_or(PARSEC_ERR_NULL)?;
            let callback = callback.ok_or(PARSEC_ERR_NULL)?;
            let scope = node(d, scope)?;
            for n in s.0.select_iter(d, scope) {
                // SAFETY: the caller's callback, with the caller's context.
                if unsafe { callback(n, ctx) } != 0 {
                    break;
                }
            }
            Ok(())
        })())
    })
}

/// The first element under `scope` that matches, without looking further;
/// `PARSEC_NO_NODE` if none does, or on a NULL handle or a bad `scope`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_select_first(
    doc: *const parsec_document,
    selector: *const parsec_selector,
    scope: parsec_node,
) -> parsec_node {
    guard(PARSEC_NO_NODE, || {
        // SAFETY: NULL or live handles.
        let (Ok(d), Some(s)) = (unsafe { doc_ref(doc) }, unsafe { selector.as_ref() }) else {
            return PARSEC_NO_NODE;
        };
        node_of(d, scope).map_or(PARSEC_NO_NODE, |scope| {
            s.0.select_first(d, scope).unwrap_or(PARSEC_NO_NODE)
        })
    })
}

/// Whether `node` matches the selector, with the root element as `:scope`.
/// False for a NULL handle, a node that isn't an element, or one that isn't
/// in the document.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_matches(
    doc: *const parsec_document,
    selector: *const parsec_selector,
    node: parsec_node,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or live handles.
        let (Ok(d), Some(s)) = (unsafe { doc_ref(doc) }, unsafe { selector.as_ref() }) else {
            return false;
        };
        node_of(d, node)
            .is_some_and(|n| matches!(d.kind(n), Kind::Element { .. }) && s.0.matches(d, n, None))
    })
}

// ---- editing ---------------------------------------------------------------

/// Makes a node with `make`, or gives PARSEC_NO_NODE.
unsafe fn create(
    doc: *mut parsec_document,
    s: *const c_char,
    len: usize,
    make: impl FnOnce(&mut Document, &str) -> Result<NodeId, parsec_status>,
) -> parsec_node {
    guard(PARSEC_NO_NODE, || {
        // SAFETY: NULL or a live document no other call is using; `s` has
        // `len` bytes.
        let (Ok(d), Ok(s)) = (unsafe { doc_mut(doc) }, unsafe { text(s, len) }) else {
            return PARSEC_NO_NODE;
        };
        make(d, &s).unwrap_or(PARSEC_NO_NODE)
    })
}

/// A new element named `name` in namespace `ns`, not yet in the tree;
/// insert it with `parsec_append` or `parsec_insert_before`. HTML names are
/// lowercased, as `createElement` does. `PARSEC_NO_NODE` on a NULL
/// document or an unknown namespace.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_create_element(
    doc: *mut parsec_document,
    ns: parsec_ns,
    name: *const c_char,
    len: usize,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe {
        create(doc, name, len, |d, name| {
            Ok(d.create_element(namespace(ns)?, name))
        })
    }
}

/// A new text node, not yet in the tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_create_text(
    doc: *mut parsec_document,
    text: *const c_char,
    len: usize,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { create(doc, text, len, |d, t| Ok(d.create_text(t))) }
}

/// A new comment, not yet in the tree.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_create_comment(
    doc: *mut parsec_document,
    text: *const c_char,
    len: usize,
) -> parsec_node {
    // SAFETY: passed on as given.
    unsafe { create(doc, text, len, |d, t| Ok(d.create_comment(t))) }
}

/// Moves `child` to be the last child of `parent`, taking it from wherever
/// it was.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_append(
    doc: *mut parsec_document,
    parent: parsec_node,
    child: parsec_node,
) -> parsec_status {
    // SAFETY: passed on as given.
    unsafe { parsec_insert_before(doc, parent, child, PARSEC_NO_NODE) }
}

/// Moves `child` into `parent`, just before `before`, which must be one of
/// `parent`'s children; `PARSEC_NO_NODE` puts it last. Refuses, with the
/// tree unchanged, to put a node inside itself, to give children to a node
/// that can't have them, or to insert a document or fragment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_insert_before(
    doc: *mut parsec_document,
    parent: parsec_node,
    child: parsec_node,
    before: parsec_node,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document no other call is using.
            let d = unsafe { doc_mut(doc)? };
            let (parent, child) = (node(d, parent)?, node(d, child)?);
            let before = match before {
                PARSEC_NO_NODE => None,
                b => Some(node(d, b)?),
            };
            d.insert_before(parent, child, before).map_err(edit_status)
        })())
    })
}

/// Takes the node out of the tree. Its number stays valid and it can be
/// inserted again; its memory goes when the document is freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_remove(
    doc: *mut parsec_document,
    node: parsec_node,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document no other call is using.
            let d = unsafe { doc_mut(doc)? };
            let n = self::node(d, node)?;
            d.remove(n);
            Ok(())
        })())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_strings_round_trip() {
        let mut len = 0;
        let s = unsafe { owned("a\0b", &mut len) };
        assert_eq!(len, 3);
        let back = unsafe { std::slice::from_raw_parts(s.cast::<u8>(), 4) };
        assert_eq!(back, b"a\0b\0");
        unsafe { parsec_string_free(s) };
    }

    #[test]
    fn every_status_has_a_message() {
        for s in PARSEC_OK..=PARSEC_ERR_PANIC {
            let m = unsafe { std::ffi::CStr::from_ptr(parsec_status_message(s)) };
            assert_ne!(m.to_str().unwrap(), "unknown status", "{s}");
        }
    }
}
