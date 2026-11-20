// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The Lexbor calls the benchmark needs, for parsing and for selectors,
//! and Lexbor's allocations routed through Rust's global allocator so
//! they're counted like the others.

use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, realloc};
use std::ffi::c_void;
use std::ptr::null_mut;

#[repr(C)]
struct LxbHtmlDocument {
    _private: [u8; 0],
}

type Malloc = unsafe extern "C" fn(usize) -> *mut c_void;
type Realloc = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
type Calloc = unsafe extern "C" fn(usize, usize) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);

#[repr(C)]
struct LxbCssParser {
    _private: [u8; 0],
}

#[repr(C)]
struct LxbCssMemory {
    _private: [u8; 0],
}

#[repr(C)]
struct LxbCssSelectors {
    _private: [u8; 0],
}

#[repr(C)]
struct LxbSelectors {
    _private: [u8; 0],
}

#[repr(C)]
struct LxbCssSelectorList {
    _private: [u8; 0],
}

/// `lxb_dom_node_t`; a document is one, at the start of its struct.
#[repr(C)]
struct LxbDomNode {
    _private: [u8; 0],
}

type FindCallback = unsafe extern "C" fn(*mut LxbDomNode, u32, *mut c_void) -> u32;

/// `LXB_SELECTORS_OPT_MATCH_FIRST`: report a node once, however many of a
/// list's selectors match it, as `querySelectorAll` does.
const MATCH_FIRST: u32 = 1 << 2;

unsafe extern "C" {
    fn lxb_css_parser_create() -> *mut LxbCssParser;
    fn lxb_css_parser_init(parser: *mut LxbCssParser, tkz: *mut c_void) -> u32;
    fn lxb_css_parser_destroy(parser: *mut LxbCssParser, self_destroy: bool) -> *mut LxbCssParser;
    fn lxb_css_selectors_parse(
        parser: *mut LxbCssParser,
        data: *const u8,
        len: usize,
    ) -> *mut LxbCssSelectorList;
    fn lxb_css_memory_create() -> *mut LxbCssMemory;
    fn lxb_css_memory_init(memory: *mut LxbCssMemory, prepare_count: usize) -> u32;
    fn lxb_css_memory_destroy(memory: *mut LxbCssMemory, self_destroy: bool) -> *mut LxbCssMemory;
    fn lxb_css_parser_memory_set_noi(parser: *mut LxbCssParser, memory: *mut LxbCssMemory);
    fn lxb_css_selectors_create() -> *mut LxbCssSelectors;
    fn lxb_css_selectors_init(selectors: *mut LxbCssSelectors) -> u32;
    fn lxb_css_selectors_destroy(
        selectors: *mut LxbCssSelectors,
        self_destroy: bool,
    ) -> *mut LxbCssSelectors;
    fn lxb_css_parser_selectors_set_noi(parser: *mut LxbCssParser, selectors: *mut LxbCssSelectors);
    fn lxb_selectors_create() -> *mut LxbSelectors;
    fn lxb_selectors_init(selectors: *mut LxbSelectors) -> u32;
    fn lxb_selectors_opt_set_noi(selectors: *mut LxbSelectors, opt: u32);
    fn lxb_selectors_destroy(selectors: *mut LxbSelectors, self_destroy: bool)
    -> *mut LxbSelectors;
    fn lxb_selectors_find(
        selectors: *mut LxbSelectors,
        root: *mut LxbDomNode,
        list: *const LxbCssSelectorList,
        cb: FindCallback,
        ctx: *mut c_void,
    ) -> u32;
    fn lxb_html_document_create() -> *mut LxbHtmlDocument;
    fn lxb_html_document_parse(doc: *mut LxbHtmlDocument, html: *const u8, size: usize) -> u32;
    fn lxb_html_document_destroy(doc: *mut LxbHtmlDocument) -> *mut LxbHtmlDocument;
    fn lexbor_memory_setup(m: Malloc, r: Realloc, c: Calloc, f: Free) -> u32;
    fn lxb_dom_node_first_child_noi(node: *mut LxbDomNode) -> *mut LxbDomNode;
    fn lxb_dom_node_next_noi(node: *mut LxbDomNode) -> *mut LxbDomNode;
    fn lxb_dom_node_parent_noi(node: *mut LxbDomNode) -> *mut LxbDomNode;
    fn lxb_dom_node_type_noi(node: *mut LxbDomNode) -> u32;
    fn lxb_encoding_bom_sniff(begin: *const u8, length: usize) -> u32;
    fn lxb_encoding_data_by_pre_name(name: *const u8, length: usize) -> *const LxbEncodingData;
    fn lxb_encoding_data_prescan_validate(name: *const u8, length: usize)
    -> *const LxbEncodingData;
    fn lxb_html_encoding_init(em: *mut HtmlEncoding) -> u32;
    fn lxb_html_encoding_destroy(em: *mut HtmlEncoding, self_destroy: bool) -> *mut HtmlEncoding;
    fn lxb_html_encoding_prescan(
        em: *mut HtmlEncoding,
        data: *const u8,
        end: *const u8,
        out_length: *mut usize,
    ) -> *const u8;
}

/// `lxb_encoding_data_t`, whose first field is its `lxb_encoding_t`.
#[repr(C)]
struct LxbEncodingData {
    encoding: u32,
}

/// Room for `lxb_html_encoding_t`, two `lexbor_array_obj_t` (64 bytes);
/// `lxb_html_encoding_init` fills it in.
#[repr(C, align(16))]
struct HtmlEncoding([u8; 128]);

/// `LXB_DOM_NODE_TYPE_ELEMENT`.
const ELEMENT: u32 = 1;
/// `lxb_encoding_t` values from lexbor/encoding/const.h.
const LXB_DEFAULT: u32 = 0x00;
const LXB_UTF_16BE: u32 = 0x19;
const LXB_UTF_16LE: u32 = 0x1a;
const LXB_UTF_8: u32 = 0x1b;
const LXB_WINDOWS_1252: u32 = 0x21;

/// The encoding Lexbor's own pieces choose for `bytes`, as the HTML
/// Standard orders them: its BOM sniff, then the transport's label, then
/// its prescan of the first 1,024 bytes, then windows-1252, the spec's
/// default (Lexbor has no detector to guess UTF-8 from the bytes). Also
/// how many bytes of BOM to skip.
pub fn choose_encoding(bytes: &[u8], transport: Option<&str>) -> (u32, usize) {
    // SAFETY: each call reads only the slice it's given; the prescan's
    // state is initialised before use and destroyed after, and the name it
    // returns points into `bytes`, which is still alive.
    unsafe {
        match lxb_encoding_bom_sniff(bytes.as_ptr(), bytes.len()) {
            LXB_UTF_8 => return (LXB_UTF_8, 3),
            bom @ (LXB_UTF_16BE | LXB_UTF_16LE) => return (bom, 2),
            _ => {}
        }
        if let Some(label) = transport {
            let data = lxb_encoding_data_by_pre_name(label.as_ptr(), label.len());
            if !data.is_null() {
                return ((*data).encoding, 0);
            }
        }
        let head = &bytes[..bytes.len().min(1024)];
        let mut em = HtmlEncoding([0; 128]);
        assert_eq!(lxb_html_encoding_init(&mut em), 0);
        let mut len = 0;
        let name = lxb_html_encoding_prescan(
            &mut em,
            head.as_ptr(),
            head.as_ptr().add(head.len()),
            &mut len,
        );
        let data = if name.is_null() {
            std::ptr::null()
        } else {
            lxb_encoding_data_prescan_validate(name, len)
        };
        lxb_html_encoding_destroy(&mut em, false);
        match data.as_ref() {
            Some(d) if d.encoding != LXB_DEFAULT => (d.encoding, 0),
            _ => (LXB_WINDOWS_1252, 0),
        }
    }
}

impl Document {
    /// A page as a crawler gets it, bytes and maybe a transport label,
    /// through Lexbor's own encoding detection and decoders and then its
    /// parser, which takes UTF-8. UTF-8, and ASCII in an encoding that
    /// keeps ASCII as it is, go to the parser as they are; anything else
    /// is decoded to UTF-8 by Lexbor's encoding module first. A failed
    /// parse is its status.
    pub fn parse_bytes(bytes: &[u8], transport: Option<&str>) -> Result<Document, u32> {
        let (encoding, bom) = choose_encoding(bytes, transport);
        let bytes = &bytes[bom..];
        let decoded;
        let utf8 = if encoding == LXB_UTF_8
            || (bytes.is_ascii() && encoding != LXB_UTF_16BE && encoding != LXB_UTF_16LE)
        {
            bytes
        } else {
            decoded = crate::encoding::lexbor_utf8(encoding, bytes);
            &decoded
        };
        // SAFETY: as in `parse`.
        unsafe {
            let doc = lxb_html_document_create();
            if doc.is_null() {
                return Err(u32::MAX);
            }
            let status = lxb_html_document_parse(doc, utf8.as_ptr(), utf8.len());
            if status != 0 {
                lxb_html_document_destroy(doc);
                return Err(status);
            }
            Ok(Document(doc))
        }
    }

    /// Elements in the tree, walked from the document node in tree order
    /// (a template's contents aren't its children, so they're not counted).
    pub fn elements(&self) -> usize {
        let root: *mut LxbDomNode = self.0.cast();
        let mut n = 0;
        // SAFETY: every pointer comes from Lexbor's own tree accessors on a
        // live document, and the walk never leaves the document node.
        unsafe {
            let mut at = lxb_dom_node_first_child_noi(root);
            while !at.is_null() {
                if lxb_dom_node_type_noi(at) == ELEMENT {
                    n += 1;
                }
                let child = lxb_dom_node_first_child_noi(at);
                if !child.is_null() {
                    at = child;
                    continue;
                }
                loop {
                    let next = lxb_dom_node_next_noi(at);
                    if !next.is_null() {
                        at = next;
                        break;
                    }
                    at = lxb_dom_node_parent_noi(at);
                    if at.is_null() || at == root {
                        at = std::ptr::null_mut();
                        break;
                    }
                }
            }
        }
        n
    }
}

/// A parsed Lexbor document, destroyed on drop.
pub struct Document(*mut LxbHtmlDocument);

impl Document {
    pub fn parse(html: &[u8]) -> Document {
        // SAFETY: Lexbor reads `html.len()` bytes from `html`, and the
        // document is destroyed exactly once, in `drop`.
        unsafe {
            let doc = lxb_html_document_create();
            assert!(!doc.is_null(), "Lexbor couldn't create a document");
            let status = lxb_html_document_parse(doc, html.as_ptr(), html.len());
            assert_eq!(status, 0, "Lexbor's parse failed with status {status:#x}");
            Document(doc)
        }
    }
}

/// Lexbor's CSS parser and selector engine, set up as Lexbor's
/// `examples/lexbor/selectors/normal_way.c` does: every list parsed lives in
/// one shared memory, freed with the rest in `drop`.
pub struct Selectors {
    memory: *mut LxbCssMemory,
    css_selectors: *mut LxbCssSelectors,
    parser: *mut LxbCssParser,
    engine: *mut LxbSelectors,
}

/// A parsed selector list, living in its `Selectors`' memory.
pub struct SelectorList<'a>(
    *mut LxbCssSelectorList,
    std::marker::PhantomData<&'a Selectors>,
);

impl Selectors {
    pub fn new() -> Selectors {
        // SAFETY: each object is created, then initialised once; both are
        // destroyed in `drop`.
        unsafe {
            let memory = lxb_css_memory_create();
            assert_eq!(lxb_css_memory_init(memory, 128), 0);
            let parser = lxb_css_parser_create();
            assert_eq!(lxb_css_parser_init(parser, null_mut()), 0);
            lxb_css_parser_memory_set_noi(parser, memory);
            let css_selectors = lxb_css_selectors_create();
            assert_eq!(lxb_css_selectors_init(css_selectors), 0);
            lxb_css_parser_selectors_set_noi(parser, css_selectors);
            let engine = lxb_selectors_create();
            assert_eq!(lxb_selectors_init(engine), 0);
            lxb_selectors_opt_set_noi(engine, MATCH_FIRST);
            Selectors {
                memory,
                css_selectors,
                parser,
                engine,
            }
        }
    }

    pub fn parse(&self, css: &str) -> SelectorList<'_> {
        // SAFETY: Lexbor reads `css.len()` bytes.
        let list = unsafe { lxb_css_selectors_parse(self.parser, css.as_ptr(), css.len()) };
        assert!(!list.is_null(), "Lexbor couldn't parse {css:?}");
        SelectorList(list, std::marker::PhantomData)
    }

    /// How many elements under the document match, as Lexbor finds them.
    pub fn count(&self, doc: &Document, list: &SelectorList) -> usize {
        unsafe extern "C" fn found(_: *mut LxbDomNode, _: u32, ctx: *mut c_void) -> u32 {
            // SAFETY: `ctx` is the `usize` below, alive for the call.
            unsafe { *ctx.cast::<usize>() += 1 };
            0
        }
        let mut n = 0usize;
        // SAFETY: the document starts with its DOM node, and outlives the
        // call; `n` outlives it too.
        let status = unsafe {
            lxb_selectors_find(
                self.engine,
                doc.0.cast(),
                list.0,
                found,
                (&raw mut n).cast(),
            )
        };
        assert_eq!(status, 0, "lxb_selectors_find failed");
        n
    }
}

impl Drop for Selectors {
    fn drop(&mut self) {
        // SAFETY: created in `new` and not yet destroyed.
        unsafe {
            lxb_selectors_destroy(self.engine, true);
            lxb_css_parser_destroy(self.parser, true);
            lxb_css_memory_destroy(self.memory, true);
            lxb_css_selectors_destroy(self.css_selectors, true);
        }
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        // SAFETY: created by `lxb_html_document_create` and not yet destroyed.
        unsafe { lxb_html_document_destroy(self.0) };
    }
}

/// Room before each block for its size, which `free` needs and C doesn't
/// pass; 16 keeps the block aligned as `malloc`'s are.
const HEADER: usize = 16;

fn layout(size: usize) -> Option<Layout> {
    Layout::from_size_align(size.checked_add(HEADER)?, HEADER).ok()
}

/// Stores `size` in the header of a fresh block and returns what C sees.
unsafe fn finish(block: *mut u8, size: usize) -> *mut c_void {
    if block.is_null() {
        return null_mut();
    }
    // SAFETY: the block is at least HEADER bytes and aligned to 16.
    unsafe {
        block.cast::<usize>().write(size);
        block.add(HEADER).cast()
    }
}

/// The block and size behind a pointer this module handed out.
unsafe fn start(ptr: *mut c_void) -> (*mut u8, usize) {
    // SAFETY: `ptr` came from `finish`, HEADER bytes into its block.
    unsafe {
        let block = ptr.cast::<u8>().sub(HEADER);
        (block, block.cast::<usize>().read())
    }
}

unsafe extern "C" fn lx_malloc(size: usize) -> *mut c_void {
    match layout(size) {
        // SAFETY: the layout's size is at least HEADER, never zero.
        Some(l) => unsafe { finish(alloc(l), size) },
        None => null_mut(),
    }
}

unsafe extern "C" fn lx_calloc(n: usize, size: usize) -> *mut c_void {
    match n.checked_mul(size).and_then(|s| Some((s, layout(s)?))) {
        // SAFETY: as in `lx_malloc`.
        Some((s, l)) => unsafe { finish(alloc_zeroed(l), s) },
        None => null_mut(),
    }
}

unsafe extern "C" fn lx_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        // SAFETY: plain allocation.
        return unsafe { lx_malloc(size) };
    }
    let Some(new) = layout(size) else {
        return null_mut();
    };
    // SAFETY: `ptr` is live and its block was allocated with `layout(old)`.
    unsafe {
        let (block, old) = start(ptr);
        let grown = realloc(block, layout(old).unwrap(), new.size());
        finish(grown, size)
    }
}

unsafe extern "C" fn lx_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    // SAFETY: as in `lx_realloc`; C never frees a pointer twice.
    unsafe {
        let (block, old) = start(ptr);
        dealloc(block, layout(old).unwrap());
    }
}

/// Sends Lexbor's `malloc`, `realloc`, `calloc` and `free` through Rust's
/// global allocator. Call before Lexbor allocates anything.
pub fn count_allocations() {
    // SAFETY: the four functions behave as their C namesakes do.
    let status = unsafe { lexbor_memory_setup(lx_malloc, lx_realloc, lx_calloc, lx_free) };
    assert_eq!(status, 0, "lexbor_memory_setup failed");
}
