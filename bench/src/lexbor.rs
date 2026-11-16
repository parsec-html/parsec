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
