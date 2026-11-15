// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The three Lexbor calls the benchmark needs, and Lexbor's allocations
//! routed through Rust's global allocator so they're counted like the
//! others.

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

unsafe extern "C" {
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
