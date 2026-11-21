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
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use encoding::{Encoding, Offsets, Parsed};
use html::css::SelectorList;
use html::{Document, Kind, NodeId, Ns, Options, ParseError};
use xpath::{Doc, Index, Item, NoExtensions, Value, XPath};

/// A parsed document. Made by `parsec_parse` and friends, freed by
/// `parsec_document_free`.
pub struct parsec_document {
    doc: Document,
    /// The XPath index of the tree as it is, built by the first query.
    /// Every change goes through `doc_mut`, which drops it.
    index: OnceLock<Index>,
    /// For a document parsed from bytes, the name of the encoding they
    /// were decoded from, NUL-terminated.
    encoding: Option<Box<str>>,
    /// With `PARSEC_SPANS` or `PARSEC_ERRORS`, for a document parsed from
    /// bytes or from text that wasn't all UTF-8, turns offsets into the
    /// text the parser read into offsets into the caller's bytes; None
    /// when they're the same.
    offsets: Option<Offsets>,
    /// How many `parsec_select` calls are running callbacks on it. While
    /// any are, changes are refused and freeing waits for the last.
    busy: AtomicUsize,
    /// Freed from a callback: the last `parsec_select` frees it.
    free_pending: AtomicBool,
}

impl parsec_document {
    fn new(doc: Document, encoding: Option<Box<str>>, offsets: Option<Offsets>) -> Self {
        parsec_document {
            doc,
            index: OnceLock::new(),
            encoding,
            offsets,
            busy: AtomicUsize::new(0),
            free_pending: AtomicBool::new(false),
        }
    }
}

/// A parse of bytes as they arrive. Made by `parsec_byte_parser_new`,
/// freed by `parsec_byte_parser_free`.
pub struct parsec_byte_parser {
    /// None once finished.
    parser: Option<encoding::ByteParser>,
    /// Every byte, for the offsets, when spans or errors are kept.
    bytes: Option<Vec<u8>>,
}

/// A compiled XPath 1.0 expression. Made by `parsec_xpath_compile`, freed
/// by `parsec_xpath_free`; like a selector, it can be shared.
pub struct parsec_xpath(XPath);

/// What an XPath expression evaluated to, copied out of the document, so
/// it stays valid after the document changes or is freed. Made by
/// `parsec_xpath_evaluate`, freed by `parsec_xpath_result_free`.
pub enum parsec_xpath_result {
    /// Each node, or each attribute as its element and its place there.
    Nodes(Vec<(parsec_node, u32)>),
    Str(String),
    Num(f64),
    Bool(bool),
}

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
/// The XPath expression doesn't parse, or couldn't be evaluated: it uses a
/// `$variable` (which the C API doesn't bind), or it ran past the work
/// limit that stops a query that would take hours.
pub const PARSEC_ERR_XPATH: parsec_status = 13;
/// The node was taken out with `parsec_remove`, or was inside one that
/// was, so it can't be used again: not edited, inserted, nor used as an
/// XPath context. `parsec_detach` takes a node out to move it.
pub const PARSEC_ERR_REMOVED: parsec_status = 14;
/// The document can't be changed from inside a `parsec_select` callback.
pub const PARSEC_ERR_BUSY: parsec_status = 15;

/// Parse with scripting on, as a browser with JavaScript does: `<noscript>`
/// holds raw text.
pub const PARSEC_SCRIPTING: u32 = 1;
/// Record where each node and attribute was written, for
/// `parsec_node_span`, `parsec_end_span`, `parsec_attr_span` and
/// `parsec_line_col`. It costs a few bytes per node.
pub const PARSEC_SPANS: u32 = 2;
/// Keep the parse errors the parser recovered from, for
/// `parsec_errors_count` and `parsec_errors_get`.
pub const PARSEC_ERRORS: u32 = 4;

/// A parse error the parser recovered from.
#[repr(C)]
pub struct parsec_syntax_error {
    /// Its name, `code_len` bytes with no NUL after them, static: the HTML
    /// Standard's (`eof-in-tag`), or for an error tree construction finds,
    /// html5lib's (`unexpected-end-tag`).
    pub code: *const c_char,
    pub code_len: usize,
    /// A byte offset into the input.
    pub offset: usize,
    /// From 1.
    pub line: usize,
    /// From 1, in characters.
    pub column: usize,
}

/// What `parsec_xpath_result_type` returns; 0 is a NULL result.
pub type parsec_xpath_type = u32;
/// A node-set: `parsec_xpath_result_count` items, in document order.
pub const PARSEC_XPATH_NODES: parsec_xpath_type = 1;
pub const PARSEC_XPATH_STRING: parsec_xpath_type = 2;
pub const PARSEC_XPATH_NUMBER: parsec_xpath_type = 3;
pub const PARSEC_XPATH_BOOLEAN: parsec_xpath_type = 4;

/// What `parsec_xpath_result_item` gives as the attribute index of an item
/// that's a node rather than an attribute.
pub const PARSEC_NO_ATTR: u32 = u32::MAX;

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

unsafe fn handle<'a>(doc: *const parsec_document) -> Result<&'a parsec_document, parsec_status> {
    // SAFETY: the caller passes NULL or a live document (header preamble).
    unsafe { doc.as_ref() }.ok_or(PARSEC_ERR_NULL)
}

unsafe fn doc_ref<'a>(doc: *const parsec_document) -> Result<&'a Document, parsec_status> {
    // SAFETY: as handle.
    unsafe { handle(doc) }.map(|d| &d.doc)
}

unsafe fn doc_mut<'a>(doc: *mut parsec_document) -> Result<&'a mut Document, parsec_status> {
    // SAFETY: as doc_ref. A `parsec_select` callback may be calling, with
    // the document borrowed, so look before borrowing it mutably.
    if unsafe { handle(doc)? }.busy.load(Ordering::Acquire) != 0 {
        return Err(PARSEC_ERR_BUSY);
    }
    // SAFETY: as doc_ref, and no other call is using the document (writes
    // are one at a time, the header says).
    let d = unsafe { doc.as_mut() }.ok_or(PARSEC_ERR_NULL)?;
    // Whatever the caller changes, the next query indexes the tree again.
    d.index.take();
    Ok(&mut d.doc)
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
    // SAFETY: as the caller vouches for `s` and `len`.
    Ok(String::from_utf8_lossy(unsafe { bytes(s, len)? }))
}

/// `len` bytes at `s`. NULL is no bytes when `len` is 0.
unsafe fn bytes<'a>(s: *const c_char, len: usize) -> Result<&'a [u8], parsec_status> {
    if len > isize::MAX as usize {
        return Err(PARSEC_ERR_INVALID);
    }
    if s.is_null() {
        return if len == 0 {
            Ok(&[])
        } else {
            Err(PARSEC_ERR_NULL)
        };
    }
    // SAFETY: the caller gives `len` readable bytes at `s`, which stay put
    // for the call; the length was checked above.
    Ok(unsafe { std::slice::from_raw_parts(s.cast::<u8>(), len) })
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
        html::EditError::Removed => PARSEC_ERR_REMOVED,
        // The C API attaches no shadow roots, so this can't happen yet.
        html::EditError::NotAShadowHost => PARSEC_ERR_INVALID,
        html::EditError::InvalidName => PARSEC_ERR_INVALID,
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

fn options(flags: u32) -> Result<Options, parsec_status> {
    if flags & !(PARSEC_SCRIPTING | PARSEC_SPANS | PARSEC_ERRORS) != 0 {
        return Err(PARSEC_ERR_INVALID);
    }
    Ok(Options {
        scripting: flags & PARSEC_SCRIPTING != 0,
        spans: flags & PARSEC_SPANS != 0,
        errors: flags & PARSEC_ERRORS != 0,
        ..Options::default()
    })
}

fn parse_status(e: ParseError) -> parsec_status {
    match e {
        ParseError::TooLarge { .. } => PARSEC_ERR_TOO_LARGE,
        ParseError::Timeout { .. } => PARSEC_ERR_TIMEOUT,
    }
}

/// `html` is the text the parser read, made from `input`; when that
/// replaced bytes that weren't UTF-8, and spans or errors are kept, the
/// document keeps the offsets back to `input`.
fn parsed(
    r: Result<Document, ParseError>,
    (input, html): (&[u8], &Cow<'_, str>),
    flags: u32,
    out: &mut *mut parsec_document,
) -> Result<(), parsec_status> {
    let keep = flags & (PARSEC_SPANS | PARSEC_ERRORS) != 0;
    let offsets = (keep && matches!(html, Cow::Owned(_))).then(|| Offsets::lossy_utf8(input));
    *out = Box::into_raw(Box::new(parsec_document::new(
        r.map_err(parse_status)?,
        None,
        offsets,
    )));
    Ok(())
}

/// As `parsed`, for a document from `bytes`, which are given when its
/// offsets are kept.
fn parsed_bytes(
    r: Result<Parsed, ParseError>,
    bytes: Option<&[u8]>,
    out: &mut *mut parsec_document,
) -> Result<&'static Encoding, parsec_status> {
    let p = r.map_err(parse_status)?;
    let offsets = bytes
        .map(|b| Offsets::new(b, p.encoding))
        .filter(|o| !o.is_identity());
    *out = Box::into_raw(Box::new(parsec_document::new(
        p.document,
        Some(format!("{}\0", p.encoding.name()).into()),
        offsets,
    )));
    Ok(p.encoding)
}

/// The encoding options for bytes: `label` (NULL or empty for none) as
/// the transport layer's encoding, and `flags`. As the HTML Standard has
/// it, a label that names no encoding is passed over.
unsafe fn byte_options(
    label: *const c_char,
    label_len: usize,
    flags: u32,
) -> Result<encoding::Options, parsec_status> {
    // SAFETY: as the caller vouches for `label` and `label_len`.
    let label = unsafe { bytes(label, label_len)? };
    let transport = Encoding::for_label(label);
    Ok(encoding::Options {
        html: options(flags)?,
        transport,
        ..encoding::Options::default()
    })
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

/// The library's version, such as "0.2.0", NUL-terminated and static.
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
        PARSEC_ERR_XPATH => "the XPath expression isn't valid, or couldn't be evaluated\0",
        PARSEC_ERR_REMOVED => "the node was removed from its document, so it can't be used again\0",
        PARSEC_ERR_BUSY => "the document can't be changed from a parsec_select callback\0",
        _ => "unknown status\0",
    };
    s.as_ptr().cast()
}

/// Frees a string a `parsec_*_html` or `parsec_text` call returned. NULL
/// is fine and does nothing. Anything else, a borrowed string or one
/// already freed, is undefined behaviour: the size is read from the bytes
/// just before the pointer.
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
/// dropped. `flags` is 0, or any of `PARSEC_SCRIPTING`, `PARSEC_SPANS` and
/// `PARSEC_ERRORS` or'd together.
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
            let input = unsafe { bytes(html, len)? };
            let html = String::from_utf8_lossy(input);
            let options = Options {
                budget: (budget_us != 0).then(|| std::time::Duration::from_micros(budget_us)),
                max_depth: (max_depth != 0).then_some(max_depth as usize),
                ..options(flags)?
            };
            parsed(html::parse_with(&html, options), (input, &html), flags, out)
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
            let input = unsafe { bytes(html, len)? };
            let html = String::from_utf8_lossy(input);
            let context = unsafe { text(context, context_len)? };
            let options = options(flags)?;
            parsed(
                html::parse_fragment_with(&html, namespace(ns)?, &context, options),
                (input, &html),
                flags,
                out,
            )
        })())
    })
}

/// Parses `len` bytes as a whole document, decoding them first as a
/// browser does: in the encoding of a byte order mark, else of
/// `transport` (the `charset` of an HTTP `Content-Type`, say:
/// `transport_len` bytes naming any of the Encoding Standard's labels,
/// such as "shift_jis" or "latin1"; NULL with a length of 0 for none, and
/// one that's no label is passed over, as browsers do), else of a
/// `<meta>` in the page, else UTF-8 if the bytes are valid UTF-8, else
/// windows-1252. `flags` are
/// `parsec_parse`'s, and spans and error offsets are byte offsets into
/// these bytes, whatever their encoding.
///
/// If `encoding` isn't NULL, `*encoding` is the name of the encoding the
/// bytes were decoded from, as `parsec_document_encoding` gives it.
/// Fails as `parsec_parse` does. On any error `*out` is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_parse_bytes(
    bytes: *const c_char,
    len: usize,
    transport: *const c_char,
    transport_len: usize,
    flags: u32,
    out: *mut *mut parsec_document,
    encoding: *mut *const c_char,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            let out = unsafe { clear(out)? };
            let input = unsafe { self::bytes(bytes, len)? };
            let options = unsafe { byte_options(transport, transport_len, flags)? };
            let keep = options.html.spans || options.html.errors;
            parsed_bytes(
                encoding::parse_bytes(input, options),
                keep.then_some(input),
                out,
            )?;
            // SAFETY: `*out` was just made; `encoding` NULL or writable.
            unsafe { put(encoding, parsec_document_encoding(*out)) };
            Ok(())
        })())
    })
}

/// The name of the encoding a document's bytes were decoded from, in the
/// Encoding Standard's spelling ("UTF-8", "Shift_JIS", "windows-1252"),
/// NUL-terminated and valid for as long as the document. NULL for a
/// document parsed from text, or a NULL handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_document_encoding(doc: *const parsec_document) -> *const c_char {
    guard(ptr::null(), || {
        // SAFETY: NULL or a live document.
        unsafe { handle(doc) }
            .ok()
            .and_then(|d| d.encoding.as_deref())
            .map_or(ptr::null(), |name| name.as_ptr().cast())
    })
}

/// Starts parsing a document given as bytes a piece at a time, as a
/// browser does while it downloads one, into `*out`: feed it each piece
/// with `parsec_byte_parser_feed`, then get the document from
/// `parsec_byte_parser_finish`, the one `parsec_parse_bytes` gives for all
/// the pieces joined, however they were cut. `transport` and `flags` are
/// `parsec_parse_bytes`'s. On any error `*out` is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_byte_parser_new(
    transport: *const c_char,
    transport_len: usize,
    flags: u32,
    out: *mut *mut parsec_byte_parser,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            let out = unsafe { clear(out)? };
            let options = unsafe { byte_options(transport, transport_len, flags)? };
            let keep = options.html.spans || options.html.errors;
            *out = Box::into_raw(Box::new(parsec_byte_parser {
                parser: Some(encoding::ByteParser::new(options)),
                bytes: keep.then(Vec::new),
            }));
            Ok(())
        })())
    })
}

/// Gives the parser the next `len` bytes of the page.
/// `PARSEC_ERR_INVALID` once it has finished.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_byte_parser_feed(
    parser: *mut parsec_byte_parser,
    bytes: *const c_char,
    len: usize,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live parser no other call is using.
            let p = unsafe { parser.as_mut() }.ok_or(PARSEC_ERR_NULL)?;
            let chunk = unsafe { self::bytes(bytes, len)? };
            p.parser.as_mut().ok_or(PARSEC_ERR_INVALID)?.feed(chunk);
            if let Some(kept) = &mut p.bytes {
                kept.extend_from_slice(chunk);
            }
            Ok(())
        })())
    })
}

/// Ends the page and puts the document in `*out`. A parser finishes once:
/// after that every call but `parsec_byte_parser_free` fails with
/// `PARSEC_ERR_INVALID`, and it still has to be freed. On any error `*out`
/// is NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_byte_parser_finish(
    parser: *mut parsec_byte_parser,
    out: *mut *mut parsec_document,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: as the header says for every argument.
            let out = unsafe { clear(out)? };
            let p = unsafe { parser.as_mut() }.ok_or(PARSEC_ERR_NULL)?;
            let running = p.parser.take().ok_or(PARSEC_ERR_INVALID)?;
            let bytes = p.bytes.take();
            parsed_bytes(running.finish(), bytes.as_deref(), out)?;
            Ok(())
        })())
    })
}

/// Frees a byte parser, finished or not. NULL is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_byte_parser_free(parser: *mut parsec_byte_parser) {
    if !parser.is_null() {
        // SAFETY: `parser` came from parsec_byte_parser_new and hasn't
        // been freed.
        guard((), || drop(unsafe { Box::from_raw(parser) }))
    }
}

/// Frees a document and everything in it: its nodes, and every string
/// borrowed from it. NULL is fine and does nothing. Called from a
/// `parsec_select` callback, it frees the document when `parsec_select`
/// returns.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_document_free(doc: *mut parsec_document) {
    if !doc.is_null() {
        guard((), || {
            // SAFETY: `doc` came from a parse and hasn't been freed.
            let d = unsafe { &*doc };
            if d.busy.load(Ordering::Acquire) != 0 {
                d.free_pending.store(true, Ordering::Release);
                return;
            }
            // SAFETY: as above, and no select is using it.
            drop(unsafe { Box::from_raw(doc) })
        })
    }
}

/// Marks a document busy for a `parsec_select`'s callbacks, and on drop,
/// panic or not, unmarks it and frees it if a callback asked to.
struct Busy(*const parsec_document);

impl Busy {
    fn new(doc: &parsec_document) -> Busy {
        doc.busy.fetch_add(1, Ordering::AcqRel);
        Busy(doc)
    }
}

impl Drop for Busy {
    fn drop(&mut self) {
        // SAFETY: the document can't have been freed while busy.
        let d = unsafe { &*self.0 };
        if d.busy.fetch_sub(1, Ordering::AcqRel) == 1 && d.free_pending.load(Ordering::Acquire) {
            // SAFETY: it came from Box::into_raw, and this was the last use.
            drop(unsafe { Box::from_raw(self.0.cast_mut()) });
        }
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
/// that's been removed, detached or not yet inserted.
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
            // A shadow root is a DocumentFragment in the DOM.
            Kind::Fragment | Kind::ShadowRoot(_) => PARSEC_NODE_FRAGMENT,
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
/// don't want that part. `PARSEC_ERR_INVALID` if `index` is past the last,
/// `PARSEC_ERR_REMOVED` for a removed element.
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
            let n = element(d, node)?;
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
/// lowercased. A name `setAttribute` refuses (empty, or with whitespace,
/// `/`, `>`, `=`, a quote or a control character in it) is
/// `PARSEC_ERR_INVALID`.
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
            d.set_attr(n, &name, &value).map_err(edit_status)
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
    if doc.is_removed(n) {
        return Err(PARSEC_ERR_REMOVED);
    }
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
/// The callback may read the document but not change it: a change from
/// there fails with `PARSEC_ERR_BUSY`, so collect the nodes and change it
/// afterwards. `parsec_document_free` from there frees it when this
/// returns.
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
            let h = unsafe { handle(doc)? };
            let s = unsafe { selector.as_ref() }.ok_or(PARSEC_ERR_NULL)?;
            let callback = callback.ok_or(PARSEC_ERR_NULL)?;
            let scope = node(&h.doc, scope)?;
            let _busy = Busy::new(h);
            for n in s.0.select_iter(&h.doc, scope) {
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
/// document, an unknown namespace, or a name `createElement` refuses: an
/// empty one, or one with whitespace, `/`, `>`, `=`, a quote or a control
/// character in it.
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
            d.create_element(namespace(ns)?, name).map_err(edit_status)
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

/// Takes the node, and everything in it, out of the document for good, as
/// `lxb_dom_node_destroy_deep` would. Their numbers stay valid but read
/// as nothing: each keeps its node type, and an element its name, but
/// has no parent, children, siblings, text, attributes, spans or markup,
/// matches no selector, and editing or inserting it is
/// `PARSEC_ERR_REMOVED`. So a node you still hold after something else
/// removed it can't quietly read stale data. A template's contents and a
/// shadow host's shadow tree go with it. Memory goes when the document is
/// freed. Removing the root, a template's contents or a node already
/// removed does nothing. To move a node, use `parsec_detach`.
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

/// Takes the node out of the tree, as `lxb_dom_node_remove` does. It and
/// everything in it stay usable: insert it again with `parsec_append` or
/// `parsec_insert_before` to move it. `PARSEC_ERR_REMOVED` for a node
/// `parsec_remove` took out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_detach(
    doc: *mut parsec_document,
    node: parsec_node,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document no other call is using.
            let d = unsafe { doc_mut(doc)? };
            let n = self::node(d, node)?;
            if d.is_removed(n) {
                return Err(PARSEC_ERR_REMOVED);
            }
            d.detach(n);
            Ok(())
        })())
    })
}

/// Whether `parsec_remove` took the node out, or one it was inside. False
/// for a detached node, a NULL handle or a bad node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_is_removed(
    doc: *const parsec_document,
    node: parsec_node,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or a live document.
        let Ok(d) = (unsafe { doc_ref(doc) }) else {
            return false;
        };
        node_of(d, node).is_some_and(|n| d.is_removed(n))
    })
}

// ---- source positions ------------------------------------------------------

/// An offset into the parsed text as one into the input.
fn source(d: &parsec_document, offset: usize) -> usize {
    d.offsets.as_ref().map_or(offset, |o| o.to_source(offset))
}

/// Writes a byte range out, as offsets into the input, or says there's
/// none.
unsafe fn range(
    d: &parsec_document,
    r: Option<std::ops::Range<usize>>,
    start: *mut usize,
    end: *mut usize,
) -> bool {
    let Some(r) = r else { return false };
    // SAFETY: NULL or writable (put checks).
    unsafe {
        put(start, source(d, r.start));
        put(end, source(d, r.end));
    }
    true
}

/// Where `node` was written, as byte offsets into the input from `*start`
/// up to `*end`: an element's start tag from `<` to `>`, or the whole of a
/// text node, comment, doctype or processing instruction. False, leaving
/// both alone, for a document parsed without `PARSEC_SPANS`, a node the
/// parser implied (`<html>`, `<head>`, `<body>`, `<tbody>`) or an edit
/// made, a NULL handle or a bad node.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_node_span(
    doc: *const parsec_document,
    node: parsec_node,
    start: *mut usize,
    end: *mut usize,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or a live document; out pointers NULL or writable.
        let Ok(h) = (unsafe { handle(doc) }) else {
            return false;
        };
        let r = node_of(&h.doc, node).and_then(|n| h.doc.span(n));
        unsafe { range(h, r, start, end) }
    })
}

/// Where an element's end tag was written, as `parsec_node_span` gives a
/// start tag: the end tag that closed it (`</p>`, or a `</body>` that
/// closed it too). False when it had none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_end_span(
    doc: *const parsec_document,
    node: parsec_node,
    start: *mut usize,
    end: *mut usize,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or a live document; out pointers NULL or writable.
        let Ok(h) = (unsafe { handle(doc) }) else {
            return false;
        };
        let r = node_of(&h.doc, node).and_then(|n| h.doc.end_span(n));
        unsafe { range(h, r, start, end) }
    })
}

/// Where an element's `index`th attribute (as `parsec_attr_at` counts
/// them) was written: from its name to the end of its value, past a
/// closing quote. False for one an edit set, or as `parsec_node_span`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_attr_span(
    doc: *const parsec_document,
    node: parsec_node,
    index: usize,
    start: *mut usize,
    end: *mut usize,
) -> bool {
    guard(false, || {
        // SAFETY: NULL or a live document; out pointers NULL or writable.
        let Ok(h) = (unsafe { handle(doc) }) else {
            return false;
        };
        let r = node_of(&h.doc, node).and_then(|n| h.doc.attr_span(n, index));
        unsafe { range(h, r, start, end) }
    })
}

/// The line and column of a byte offset into the input, as an editor shows
/// them: both from 1, a line ending at an LF, a CRLF or a lone CR, and the
/// column counted in characters, not bytes. It reads the input up to
/// `offset` each time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_line_col(
    doc: *const parsec_document,
    offset: usize,
    line: *mut usize,
    column: *mut usize,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; out pointers NULL or writable.
            let h = unsafe { handle(doc)? };
            let offset = h.offsets.as_ref().map_or(offset, |o| o.to_decoded(offset));
            let (l, c) = h.doc.line_col(offset);
            unsafe {
                put(line, l);
                put(column, c);
            }
            Ok(())
        })())
    })
}

// ---- parse errors ----------------------------------------------------------

/// How many parse errors the input had: 0 unless it was parsed with
/// `PARSEC_ERRORS`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_errors_count(doc: *const parsec_document) -> usize {
    // SAFETY: NULL or a live document.
    guard(0, || {
        unsafe { doc_ref(doc) }.map_or(0, |d| d.errors().len())
    })
}

/// The `index`th parse error, in source order, into `*out`.
/// `PARSEC_ERR_INVALID` if `index` is past the last.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_errors_get(
    doc: *const parsec_document,
    index: usize,
    out: *mut parsec_syntax_error,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live document; `out` NULL or writable.
            let h = unsafe { handle(doc)? };
            if out.is_null() {
                return Err(PARSEC_ERR_NULL);
            }
            let e = h.doc.errors().get(index).ok_or(PARSEC_ERR_INVALID)?;
            unsafe {
                out.write(parsec_syntax_error {
                    code: e.code.as_ptr().cast(),
                    code_len: e.code.len(),
                    offset: source(h, e.offset),
                    line: e.line,
                    column: e.column,
                })
            };
            Ok(())
        })())
    })
}

// ---- XPath -----------------------------------------------------------------

/// Compiles an XPath 1.0 expression (`len` bytes, such as
/// `"//a[@href]/@href"`) into `*out`. On `PARSEC_ERR_XPATH`, if `message`
/// isn't NULL, `*message` is a sentence saying what's wrong, yours to free
/// with `parsec_string_free`. On any error `*out` is NULL.
///
/// Names match as lxml matches them in HTML: lowercase, in no namespace,
/// so `//div` finds every div and `//DIV` none.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_compile(
    expr: *const c_char,
    len: usize,
    out: *mut *mut parsec_xpath,
    message: *mut *mut c_char,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: `out` and `message` NULL or writable; `expr` has `len`
            // bytes.
            unsafe { put(message, ptr::null_mut()) };
            let out = unsafe { clear(out)? };
            let expr = unsafe { text(expr, len)? };
            let compiled = XPath::new(&expr).map_err(|e| unsafe { xpath_failed(&e, message) })?;
            *out = Box::into_raw(Box::new(parsec_xpath(compiled)));
            Ok(())
        })())
    })
}

/// Puts an XPath error's sentence in `*message`, if wanted.
unsafe fn xpath_failed(e: &xpath::XPathError, message: *mut *mut c_char) -> parsec_status {
    if !message.is_null() {
        // SAFETY: checked non-NULL; the caller gives writable space.
        unsafe { message.write(owned(&e.to_string(), ptr::null_mut())) };
    }
    PARSEC_ERR_XPATH
}

/// Frees a compiled XPath expression. NULL is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_free(xpath: *mut parsec_xpath) {
    if !xpath.is_null() {
        // SAFETY: `xpath` came from parsec_xpath_compile and hasn't been
        // freed.
        guard((), || drop(unsafe { Box::from_raw(xpath) }))
    }
}

/// Evaluates `xpath` with `context` as the context node (`parsec_root()`
/// for the whole document) and puts the result in `*out`. Errors as
/// `parsec_xpath_compile`; on any error `*out` is NULL.
///
/// The first query after a change indexes the document, which takes about
/// as long as a pass over it; later ones reuse the index. Like any read,
/// it may run on many threads at once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_evaluate(
    doc: *const parsec_document,
    xpath: *const parsec_xpath,
    context: parsec_node,
    out: *mut *mut parsec_xpath_result,
    message: *mut *mut c_char,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or live handles; out pointers NULL or writable.
            unsafe { put(message, ptr::null_mut()) };
            let out = unsafe { clear(out)? };
            let d = unsafe { doc.as_ref() }.ok_or(PARSEC_ERR_NULL)?;
            let x = unsafe { xpath.as_ref() }.ok_or(PARSEC_ERR_NULL)?;
            let context = node(&d.doc, context)?;
            if d.doc.is_removed(context) {
                return Err(PARSEC_ERR_REMOVED);
            }
            let index = d.index.get_or_init(|| {
                // Every node without a parent, so a query can start in a
                // detached subtree as well as in the document. Removed
                // nodes have no parent either, but no one can query them.
                let roots: Vec<NodeId> = (0..d.doc.len() as NodeId)
                    .filter(|&n| d.doc.parent(n).is_none() && !d.doc.is_removed(n))
                    .collect();
                Index::build(&d.doc, &roots)
            });
            let value =
                x.0.evaluate(Doc::html(&d.doc, index), context, &[], &[], &NoExtensions)
                    .map_err(|e| unsafe { xpath_failed(&e, message) })?;
            let result = match value {
                Value::Nodes(items) => parsec_xpath_result::Nodes(
                    items
                        .into_iter()
                        .map(|item| match item {
                            Item::Node(n) => (n, PARSEC_NO_ATTR),
                            Item::Attr { owner, index, .. } => (owner, index),
                        })
                        .collect(),
                ),
                Value::Str(s) => parsec_xpath_result::Str(s.into_owned()),
                Value::Num(n) => parsec_xpath_result::Num(n),
                Value::Bool(b) => parsec_xpath_result::Bool(b),
            };
            *out = Box::into_raw(Box::new(result));
            Ok(())
        })())
    })
}

/// Frees an XPath result. NULL is fine and does nothing.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_free(result: *mut parsec_xpath_result) {
    if !result.is_null() {
        // SAFETY: `result` came from parsec_xpath_evaluate and hasn't been
        // freed.
        guard((), || drop(unsafe { Box::from_raw(result) }))
    }
}

/// Which of `PARSEC_XPATH_NODES`, `_STRING`, `_NUMBER` and `_BOOLEAN` the
/// result is; 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_type(
    result: *const parsec_xpath_result,
) -> parsec_xpath_type {
    // SAFETY: NULL or a live result.
    match unsafe { result.as_ref() } {
        None => 0,
        Some(parsec_xpath_result::Nodes(_)) => PARSEC_XPATH_NODES,
        Some(parsec_xpath_result::Str(_)) => PARSEC_XPATH_STRING,
        Some(parsec_xpath_result::Num(_)) => PARSEC_XPATH_NUMBER,
        Some(parsec_xpath_result::Bool(_)) => PARSEC_XPATH_BOOLEAN,
    }
}

/// How many items a node-set holds; 0 for any other result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_count(result: *const parsec_xpath_result) -> usize {
    // SAFETY: NULL or a live result.
    match unsafe { result.as_ref() } {
        Some(parsec_xpath_result::Nodes(items)) => items.len(),
        _ => 0,
    }
}

/// The node-set's `index`th item, in document order. A node comes back as
/// `*node` with `*attr` set to `PARSEC_NO_ATTR`; an attribute as its
/// element in `*node` and its place among the element's attributes in
/// `*attr`, for `parsec_attr_at`. Either out pointer may be NULL.
/// `PARSEC_ERR_INVALID` if `index` is past the last item or the result
/// isn't a node-set.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_item(
    result: *const parsec_xpath_result,
    index: usize,
    node: *mut parsec_node,
    attr: *mut u32,
) -> parsec_status {
    guard(PARSEC_ERR_PANIC, || {
        status((|| {
            // SAFETY: NULL or a live result; out pointers NULL or writable.
            let r = unsafe { result.as_ref() }.ok_or(PARSEC_ERR_NULL)?;
            let parsec_xpath_result::Nodes(items) = r else {
                return Err(PARSEC_ERR_INVALID);
            };
            let &(n, a) = items.get(index).ok_or(PARSEC_ERR_INVALID)?;
            unsafe {
                put(node, n);
                put(attr, a);
            }
            Ok(())
        })())
    })
}

/// A string result, borrowed from the result (valid until it's freed),
/// `*len` bytes with no NUL after them; NULL for any other result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_string(
    result: *const parsec_xpath_result,
    len: *mut usize,
) -> *const c_char {
    // SAFETY: NULL or a live result; `len` NULL or writable.
    let s = match unsafe { result.as_ref() } {
        Some(parsec_xpath_result::Str(s)) => Some(s.as_str()),
        _ => None,
    };
    unsafe { borrowed(s, len) }
}

/// A number result; NaN for any other result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_number(result: *const parsec_xpath_result) -> f64 {
    // SAFETY: NULL or a live result.
    match unsafe { result.as_ref() } {
        Some(parsec_xpath_result::Num(n)) => *n,
        _ => f64::NAN,
    }
}

/// A boolean result; false for any other result.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn parsec_xpath_result_boolean(result: *const parsec_xpath_result) -> bool {
    // SAFETY: NULL or a live result.
    matches!(
        unsafe { result.as_ref() },
        Some(parsec_xpath_result::Bool(true))
    )
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
    fn the_header_has_this_version() {
        let header = include_str!("../include/parsec.h");
        let v: Vec<&str> = env!("CARGO_PKG_VERSION").split('.').collect();
        for line in [
            format!("#define PARSEC_VERSION \"{}\"", env!("CARGO_PKG_VERSION")),
            format!("#define PARSEC_VERSION_MAJOR {}", v[0]),
            format!("#define PARSEC_VERSION_MINOR {}", v[1]),
            format!("#define PARSEC_VERSION_PATCH {}", v[2]),
        ] {
            assert!(
                header.contains(&line),
                "include/parsec.h lacks {line}: update cbindgen.toml"
            );
        }
    }

    #[test]
    fn every_status_has_a_message() {
        for s in PARSEC_OK..=PARSEC_ERR_BUSY {
            let m = unsafe { std::ffi::CStr::from_ptr(parsec_status_message(s)) };
            assert_ne!(m.to_str().unwrap(), "unknown status", "{s}");
        }
    }
}
