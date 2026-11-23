// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! libxml2's C, built from the submodule at bench/libxml2, through the
//! few calls the XML benchmark makes: parse a buffer into a tree, free it.
//! Its allocations go through this program's allocator, so its peak
//! memory is counted as Parsec's is.

use std::alloc::{Layout, alloc, dealloc, realloc as grow};
use std::ffi::{c_char, c_int, c_void};

#[repr(C)]
struct XmlDoc {
    _private: [u8; 0],
}

type Free = unsafe extern "C" fn(*mut c_void);
type Malloc = unsafe extern "C" fn(usize) -> *mut c_void;
type Realloc = unsafe extern "C" fn(*mut c_void, usize) -> *mut c_void;
type Strdup = unsafe extern "C" fn(*const c_char) -> *mut c_char;

unsafe extern "C" {
    fn xmlInitParser();
    fn xmlMemSetup(free: Free, malloc: Malloc, realloc: Realloc, strdup: Strdup) -> c_int;
    fn xmlReadMemory(
        buffer: *const c_char,
        size: c_int,
        url: *const c_char,
        encoding: *const c_char,
        options: c_int,
    ) -> *mut XmlDoc;
    fn xmlFreeDoc(doc: *mut XmlDoc);
    fn xmlSetGenericErrorFunc(ctx: *mut c_void, handler: *const c_void);
    fn xmlSetStructuredErrorFunc(ctx: *mut c_void, handler: *const c_void);
}

/// Don't touch the network. Its other defaults, limits included, stay as
/// a caller gets them, as Parsec's do.
const XML_PARSE_NONET: c_int = 1 << 11;

/// Each block starts with its size, so `free` and `realloc` know it.
const HEADER: usize = 16;

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size + HEADER, HEADER).unwrap()
}

unsafe extern "C" fn x_malloc(size: usize) -> *mut c_void {
    unsafe {
        let p = alloc(layout(size));
        if p.is_null() {
            return p.cast();
        }
        p.cast::<usize>().write(size);
        p.add(HEADER).cast()
    }
}

unsafe extern "C" fn x_free(ptr: *mut c_void) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        let p = ptr.cast::<u8>().sub(HEADER);
        dealloc(p, layout(p.cast::<usize>().read()));
    }
}

unsafe extern "C" fn x_realloc(ptr: *mut c_void, size: usize) -> *mut c_void {
    if ptr.is_null() {
        return unsafe { x_malloc(size) };
    }
    unsafe {
        let p = ptr.cast::<u8>().sub(HEADER);
        let old = p.cast::<usize>().read();
        let q = grow(p, layout(old), size + HEADER);
        if q.is_null() {
            return q.cast();
        }
        q.cast::<usize>().write(size);
        q.add(HEADER).cast()
    }
}

unsafe extern "C" fn x_strdup(s: *const c_char) -> *mut c_char {
    unsafe {
        let len = std::ffi::CStr::from_ptr(s).to_bytes_with_nul().len();
        let p = x_malloc(len).cast::<c_char>();
        if !p.is_null() {
            std::ptr::copy_nonoverlapping(s, p, len);
        }
        p
    }
}

unsafe extern "C" fn quiet(_: *mut c_void, _: *const c_void) {}

/// Routes libxml2's allocations through ours and silences its error
/// printing. Call once, before any parse.
pub fn init() {
    unsafe {
        xmlMemSetup(x_free, x_malloc, x_realloc, x_strdup);
        xmlInitParser();
        let handler = quiet as unsafe extern "C" fn(*mut c_void, *const c_void);
        xmlSetGenericErrorFunc(std::ptr::null_mut(), handler as *const c_void);
        xmlSetStructuredErrorFunc(std::ptr::null_mut(), handler as *const c_void);
    }
}

/// A document libxml2 built, freed on drop.
pub struct Document(*mut XmlDoc);

impl Document {
    /// Parses `xml` into a tree; None if libxml2 calls it not well-formed.
    pub fn parse(xml: &[u8]) -> Option<Document> {
        let doc = unsafe {
            xmlReadMemory(
                xml.as_ptr().cast(),
                c_int::try_from(xml.len()).ok()?,
                c"bench.xml".as_ptr(),
                std::ptr::null(),
                XML_PARSE_NONET,
            )
        };
        (!doc.is_null()).then_some(Document(doc))
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        unsafe { xmlFreeDoc(self.0) };
    }
}
