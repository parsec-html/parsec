/* SPDX-License-Identifier: Apache-2.0
 * Copyright 2026 James Conway
 *
 * Parsec's C API: an HTML parser that follows the WHATWG HTML Standard,
 * with CSS selectors, editing and serialization, and a JavaScript parser
 * that gives ESTree JSON.
 *
 * Generated from crates/parsec-capi/src/lib.rs by cbindgen; don't edit it.
 * The wiki's C-API page has a guide and examples.
 *
 * The rules every function keeps:
 *
 * - Handles are opaque. A parsec_document, parsec_selector, parsec_xpath,
 *   parsec_xpath_result or parsec_byte_parser is made by one function and
 *   freed by one (a parsec_js_error too, though its fields can be read), and NULL is accepted everywhere: a call given NULL
 *   fails with PARSEC_ERR_NULL, or returns NULL, PARSEC_NO_NODE, 0 or
 *   false, and the free functions do nothing.
 * - Nodes are numbers. A parsec_node is valid for as long as its document
 *   is, and a number that isn't a node of the document is refused, never
 *   read. Removing a node detaches it; nothing is freed until the
 *   document is.
 * - Strings in are a pointer and a length in bytes, UTF-8, with no NUL
 *   needed. NULL with a length of 0 is the empty string. Bytes that aren't
 *   UTF-8 become U+FFFD, though spans and error offsets still count the
 *   bytes you passed. The exception is a page given to
 *   parsec_parse_bytes or a parsec_byte_parser, which is decoded as a
 *   browser decodes it, in whatever encoding it's in, and JavaScript
 *   given to a parsec_js_ function, which must be UTF-8.
 * - Strings out come two ways. Borrowed ones (const char *, from
 *   parsec_node_name, parsec_node_value, parsec_attr_get and the like)
 *   point into the document, are *len bytes long with no NUL after them,
 *   and stay valid until the document is next changed or freed. Owned
 *   ones (char *, from parsec_inner_html, parsec_outer_html and
 *   parsec_text) are copies, NUL-terminated, freed with
 *   parsec_string_free, once, and nothing else. Freeing a borrowed
 *   string, or an owned one twice, is undefined behaviour.
 * - Nothing panics into C. A bug in Parsec comes back as
 *   PARSEC_ERR_PANIC instead. That covers panics only: running out of
 *   memory, or out of stack, ends the process as it would in C.
 * - Threads: any number of threads may read one document at once (every
 *   function taking a const parsec_document *, parsec_xpath_evaluate
 *   included), and a selector or a compiled XPath may be used from any
 *   number of threads. A call taking a non-const parsec_document *
 *   changes it, and needs the document to itself, as a
 *   parsec_byte_parser needs itself to itself for each call. A
 *   document or selector may be freed on a different thread from the one
 *   that made it.
 * - A parsec_select callback may read the document, not change it: a
 *   change from there fails with PARSEC_ERR_BUSY (a create function
 *   returns PARSEC_NO_NODE), and parsec_document_free from there frees
 *   the document once parsec_select returns.
 * - XML has handles and functions of its own: a parsec_xml_document from
 *   parsec_xml_parse, read with the parsec_xml_ functions by the rules
 *   above. Its input is bytes decoded as XML says (a byte order mark,
 *   else the encoding the XML declaration names, else UTF-8), strictly:
 *   bytes that aren't that encoding are an error, not U+FFFD. It can't
 *   be changed, so any number of threads may read it at once.
 */

#ifndef PARSEC_H
#define PARSEC_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#define PARSEC_VERSION "0.2.0"
#define PARSEC_VERSION_MAJOR 0
#define PARSEC_VERSION_MINOR 2
#define PARSEC_VERSION_PATCH 0

// Parse with scripting on, as a browser with JavaScript does: `<noscript>`
// holds raw text.
#define PARSEC_SCRIPTING 1

// Record where each node and attribute was written, for
// `parsec_node_span`, `parsec_end_span`, `parsec_attr_span` and
// `parsec_line_col`. It costs a few bytes per node.
#define PARSEC_SPANS 2

// Keep the parse errors the parser recovered from, for
// `parsec_errors_count` and `parsec_errors_get`.
#define PARSEC_ERRORS 4

// What `parsec_xpath_result_item` gives as the attribute index of an item
// that's a node rather than an attribute.
#define PARSEC_NO_ATTR UINT32_MAX

// What `parsec_xpath_result_item` gives as the attribute index of a
// namespace node, from `namespace::`. An HTML document's elements have
// one, the `xml` namespace, whose URI is
// `http://www.w3.org/XML/1998/namespace`.
#define PARSEC_NS_ATTR (UINT32_MAX - 1)

#define PARSEC_NODE_ELEMENT 1

#define PARSEC_NODE_TEXT 3

#define PARSEC_NODE_CDATA 4

#define PARSEC_NODE_PROCESSING_INSTRUCTION 7

#define PARSEC_NODE_COMMENT 8

#define PARSEC_NODE_DOCUMENT 9

#define PARSEC_NODE_DOCTYPE 10

#define PARSEC_NODE_FRAGMENT 11

// Parse a module, with `import` and `export`, rather than a script.
#define PARSEC_JS_MODULE 1

// What `parsec_xml_node_type` gives for a reference to an entity Parsec
// didn't read (one declared external): the DOM's
// `ENTITY_REFERENCE_NODE`.
#define PARSEC_NODE_ENTITY_REFERENCE 5

// Parse without Namespaces in XML: a name is any XML name, colons and
// all, in no namespace.
#define PARSEC_XML_NO_NAMESPACES 1

// A parse of bytes as they arrive. Made by `parsec_byte_parser_new`,
// freed by `parsec_byte_parser_free`.
typedef struct parsec_byte_parser parsec_byte_parser;

// A parsed document. Made by `parsec_parse` and friends, freed by
// `parsec_document_free`.
typedef struct parsec_document parsec_document;

// A compiled selector list. Made by `parsec_selector_compile`, freed by
// `parsec_selector_free`; compile once and run it on as many documents as
// you like.
typedef struct parsec_selector parsec_selector;

// A parsed XML document. Made by `parsec_xml_parse`, freed by
// `parsec_xml_document_free`. It can't be changed, so any number of
// threads may read it at once.
typedef struct parsec_xml_document parsec_xml_document;

// A compiled XPath 1.0 expression. Made by `parsec_xpath_compile`, freed
// by `parsec_xpath_free`; like a selector, it can be shared.
typedef struct parsec_xpath parsec_xpath;

// What an XPath expression evaluated to, copied out of the document, so
// it stays valid after the document changes or is freed. Made by
// `parsec_xpath_evaluate`, freed by `parsec_xpath_result_free`.
typedef struct parsec_xpath_result parsec_xpath_result;

// What a function that can fail returns. `parsec_status_message` names it.
typedef int32_t parsec_status;

// A namespace, for elements and fragment contexts. 0 is no namespace:
// what `parsec_element_ns` gives for a node that isn't an element.
typedef uint32_t parsec_ns;

// A node, numbered within its document. The document itself is node 0.
// A number stays valid, and keeps meaning the same node, for as long as
// the document lives: removing a node only detaches it.
typedef uint32_t parsec_node;

// Called by `parsec_select` with each match, in document order, and the
// `ctx` you passed. Return 0 to go on, anything else to stop there.
typedef int (*parsec_match_fn)(parsec_node node, void *ctx);

// A parse error the parser recovered from.
typedef struct {
  // Its name, `code_len` bytes with no NUL after them, static: the HTML
  // Standard's (`eof-in-tag`), or for an error tree construction finds,
  // html5lib's (`unexpected-end-tag`).
  const char *code;
  size_t code_len;
  // A byte offset into the input.
  size_t offset;
  // From 1.
  size_t line;
  // From 1, in characters.
  size_t column;
} parsec_syntax_error;

// What `parsec_xpath_result_type` returns; 0 is a NULL result.
typedef uint32_t parsec_xpath_type;

// How `parsec_js_parse` parses. NULL in its place is all zeros.
typedef struct {
  // 0, or `PARSEC_JS_MODULE`.
  uint32_t flags;
  // How deep the tree may nest, at most 256; 0 is 256. Deeper fails
  // with `PARSEC_ERR_JS` and the code `too-deep`. A parse at 256 can
  // use about 256 KiB of stack, so lower it on a thread with less:
  // musl's default thread stack, on Alpine for one, is 128 KiB.
  uint32_t max_depth;
  // The most tokens to read before giving up with
  // `PARSEC_ERR_TIMEOUT`; 0 is no limit.
  uint64_t max_steps;
} parsec_js_options;

// Why JavaScript didn't parse, and where. Made by `parsec_js_parse` and
// `parsec_js_page_state`, freed by `parsec_js_error_free`, which frees
// its strings too.
typedef struct {
  // Its stable name, `code_len` bytes with no NUL after them, static:
  // `unexpected-token`, `too-deep`, `step-budget` and the rest, as the
  // wiki's JavaScript page lists them.
  const char *code;
  size_t code_len;
  // A sentence saying what's wrong, NUL-terminated.
  char *message;
  size_t message_len;
  // Byte offsets into the source.
  size_t start;
  size_t end;
  // From 1.
  size_t line;
  // From 1, in characters.
  size_t column;
  // The lines it's on, with carets under the place, as a compiler
  // prints it; NUL-terminated.
  char *frame;
  size_t frame_len;
} parsec_js_error;

// How `parsec_xml_parse` parses. NULL in its place is all zeros, and a
// 0 field is its default.
typedef struct {
  // 0, or `PARSEC_XML_NO_NAMESPACES`.
  uint32_t flags;
  // How deep elements may nest; 0 is 2,048. Deeper fails with
  // `PARSEC_ERR_XML` and the code `too-deep`.
  uint32_t max_depth;
  // How deep entity references may nest in the entities they refer
  // to; 0 is 16, and more than 64 is 64.
  uint32_t max_entity_depth;
  // Entity expansion may add this many bytes to any document; 0 is
  // 1 MiB...
  uint64_t entity_allowance;
  // ...plus this many for each byte of the input; 0 is 8. Past either,
  // the parse fails with `PARSEC_ERR_XML` and the code
  // `entity-expansion-limit`, so a billion laughs is an error, not a
  // hang.
  uint64_t entity_ratio;
} parsec_xml_options;

// Why XML didn't parse, and where. Made by `parsec_xml_parse`, freed by
// `parsec_xml_error_free`, which frees its message too.
typedef struct {
  // Its stable name, `code_len` bytes with no NUL after them, static:
  // `mismatched-end-tag`, `too-deep`, `entity-expansion-limit` and the
  // rest, as the wiki's XML page lists them.
  const char *code;
  size_t code_len;
  // A sentence saying what's wrong, NUL-terminated.
  char *message;
  size_t message_len;
  // From 1.
  size_t line;
  // From 1, in characters.
  size_t column;
} parsec_xml_error;

// An attribute, as `parsec_xml_attr_at` gives it. Every string is
// borrowed from the document, as `parsec_node_name` says, and is empty,
// not NULL, when the attribute has no prefix or no namespace.
typedef struct {
  // The name as written: `xlink:href`.
  const char *name;
  size_t name_len;
  // `href`.
  const char *local_name;
  size_t local_name_len;
  // `xlink`.
  const char *prefix;
  size_t prefix_len;
  // The namespace the prefix is bound to:
  // `http://www.w3.org/1999/xlink`.
  const char *namespace_uri;
  size_t namespace_uri_len;
  // The value, normalized as XML says: references replaced, and white
  // space characters made spaces.
  const char *value;
  size_t value_len;
  // Where it was written, as byte offsets into the input from its name
  // to its closing quote; the start tag's, for a default from the DTD.
  size_t start;
  size_t end;
  // False for a default the DTD supplied.
  bool specified;
} parsec_xml_attr;

// No node: what navigation returns at the end of a list, and what you pass
// to `parsec_insert_before` to insert last.
#define PARSEC_NO_NODE UINT32_MAX

#define PARSEC_OK 0

// A pointer that must not be NULL was NULL.
#define PARSEC_ERR_NULL 1

// An argument was out of range: an unknown flag or namespace, or a length
// over `PTRDIFF_MAX`.
#define PARSEC_ERR_INVALID 2

// The input is over the 512 MiB a document can hold, or JavaScript over
// the 4 GiB a source can be.
#define PARSEC_ERR_TOO_LARGE 3

// The parse ran past the budget given to `parsec_parse_with`, a match
// past the step limit or budget given to `parsec_select_with` or
// `parsec_matches_with`, or JavaScript past the `max_steps` given to
// `parsec_js_parse`.
#define PARSEC_ERR_TIMEOUT 4

// The selector doesn't parse, or uses something Parsec doesn't support.
#define PARSEC_ERR_SELECTOR 5

// The node number isn't a node of this document.
#define PARSEC_ERR_NO_SUCH_NODE 6

// The node has to be an element for this call.
#define PARSEC_ERR_NOT_AN_ELEMENT 7

// The node would end up inside itself.
#define PARSEC_ERR_CYCLE 8

// The parent can't have children: it's text, a comment, a doctype or a
// processing instruction.
#define PARSEC_ERR_NOT_A_PARENT 9

// The node to insert before isn't a child of the parent.
#define PARSEC_ERR_NOT_A_CHILD 10

// The node is a document or a fragment (the root, or a template's
// contents), which is never anyone's child.
#define PARSEC_ERR_NOT_INSERTABLE 11

// A bug in Parsec: it panicked, and the call was stopped before the panic
// reached C. A document that was being changed may be half-changed; free
// it. Please report it.
#define PARSEC_ERR_PANIC 12

// The XPath expression doesn't parse, or couldn't be evaluated: it uses a
// `$variable` (which the C API doesn't bind), or it ran past the work
// limit that stops a query that would take hours.
#define PARSEC_ERR_XPATH 13

// The node was taken out with `parsec_remove`, or was inside one that
// was, so it can't be used again: not edited, inserted, nor used as an
// XPath context. `parsec_detach` takes a node out to move it.
#define PARSEC_ERR_REMOVED 14

// The document can't be changed from inside a `parsec_select` callback.
#define PARSEC_ERR_BUSY 15

// The JavaScript doesn't parse, or nests deeper than its `max_depth`:
// the `parsec_js_error` says where.
#define PARSEC_ERR_JS 16

// A node-set: `parsec_xpath_result_count` items, in document order.
#define PARSEC_XPATH_NODES 1

#define PARSEC_XPATH_STRING 2

#define PARSEC_XPATH_NUMBER 3

#define PARSEC_XPATH_BOOLEAN 4

#define PARSEC_NS_HTML 1

#define PARSEC_NS_SVG 2

#define PARSEC_NS_MATHML 3

// The XML isn't well-formed, nests deeper than its `max_depth`, or
// expands entities past its limits: the `parsec_xml_error` says which,
// and where.
#define PARSEC_ERR_XML 17

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

// The library's version, such as "0.2.0", NUL-terminated and static.
const char *parsec_version(void);

// A sentence saying what `status` means, NUL-terminated and static.
const char *parsec_status_message(parsec_status status);

// Frees a string a `parsec_*_html` or `parsec_text` call returned. NULL
// is fine and does nothing. Anything else, a borrowed string or one
// already freed, is undefined behaviour: the size is read from the bytes
// just before the pointer.
void parsec_string_free(char *s);

// Parses `len` bytes of UTF-8 as a whole document and puts it in `*out`.
// Bad markup never fails: the parser repairs it as a browser does. Bytes
// that aren't UTF-8 become U+FFFD, and a leading byte order mark is
// dropped. `flags` is 0, or any of `PARSEC_SCRIPTING`, `PARSEC_SPANS` and
// `PARSEC_ERRORS` or'd together.
//
// Fails with `PARSEC_ERR_TOO_LARGE` over 512 MiB. On any error `*out` is
// NULL.
parsec_status parsec_parse(const char *html, size_t len, uint32_t flags, parsec_document **out);

// `parsec_parse`, with limits for input you don't control. Past
// `budget_us` microseconds the parse gives up with `PARSEC_ERR_TIMEOUT`
// (0 is no budget; the clock is read every 4,096 tokenizer steps, so a
// small input always finishes). Past `max_depth` open elements, a new node
// goes beside the current one instead of inside it, as Chromium does at
// 512 (0 is no limit).
parsec_status parsec_parse_with(const char *html,
                                size_t len,
                                uint32_t flags,
                                uint64_t budget_us,
                                uint32_t max_depth,
                                parsec_document **out);

// Parses `len` bytes as the contents of an element named `context` in
// namespace `ns`, as setting `innerHTML` on it does: "tbody" in
// `PARSEC_NS_HTML` reads `<tr>` as a row. The result is a document of its
// own whose root (node 0) is a fragment holding what was parsed.
parsec_status parsec_parse_fragment(const char *html,
                                    size_t len,
                                    parsec_ns ns,
                                    const char *context,
                                    size_t context_len,
                                    uint32_t flags,
                                    parsec_document **out);

// Parses `len` bytes as a whole document, decoding them first as a
// browser does: in the encoding of a byte order mark, else of
// `transport` (the `charset` of an HTTP `Content-Type`, say:
// `transport_len` bytes naming any of the Encoding Standard's labels,
// such as "shift_jis" or "latin1"; NULL with a length of 0 for none, and
// one that's no label is passed over, as browsers do), else of a
// `<meta>` in the page, else UTF-8 if the bytes are valid UTF-8, else
// windows-1252. `flags` are
// `parsec_parse`'s, and spans and error offsets are byte offsets into
// these bytes, whatever their encoding.
//
// If `encoding` isn't NULL, `*encoding` is the name of the encoding the
// bytes were decoded from, as `parsec_document_encoding` gives it.
// Fails as `parsec_parse` does. On any error `*out` is NULL.
parsec_status parsec_parse_bytes(const char *bytes,
                                 size_t len,
                                 const char *transport,
                                 size_t transport_len,
                                 uint32_t flags,
                                 parsec_document **out,
                                 const char **encoding);

// The name of the encoding a document's bytes were decoded from, in the
// Encoding Standard's spelling ("UTF-8", "Shift_JIS", "windows-1252"),
// NUL-terminated and valid for as long as the document. NULL for a
// document parsed from text, or a NULL handle.
const char *parsec_document_encoding(const parsec_document *doc);

// Starts parsing a document given as bytes a piece at a time, as a
// browser does while it downloads one, into `*out`: feed it each piece
// with `parsec_byte_parser_feed`, then get the document from
// `parsec_byte_parser_finish`, the one `parsec_parse_bytes` gives for all
// the pieces joined, however they were cut. `transport` and `flags` are
// `parsec_parse_bytes`'s. On any error `*out` is NULL.
parsec_status parsec_byte_parser_new(const char *transport,
                                     size_t transport_len,
                                     uint32_t flags,
                                     parsec_byte_parser **out);

// Gives the parser the next `len` bytes of the page.
// `PARSEC_ERR_INVALID` once it has finished.
parsec_status parsec_byte_parser_feed(parsec_byte_parser *parser, const char *bytes, size_t len);

// Ends the page and puts the document in `*out`. A parser finishes once:
// after that every call but `parsec_byte_parser_free` fails with
// `PARSEC_ERR_INVALID`, and it still has to be freed. On any error `*out`
// is NULL.
parsec_status parsec_byte_parser_finish(parsec_byte_parser *parser, parsec_document **out);

// Frees a byte parser, finished or not. NULL is fine and does nothing.
void parsec_byte_parser_free(parsec_byte_parser *parser);

// Frees a document and everything in it: its nodes, and every string
// borrowed from it. NULL is fine and does nothing. Called from a
// `parsec_select` callback, it frees the document when `parsec_select`
// returns.
void parsec_document_free(parsec_document *doc);

// How many nodes the document holds, removed ones included: every node
// number below this is valid.
size_t parsec_node_count(const parsec_document *doc);

// The document itself, which is always node 0.
parsec_node parsec_root(void);

// The node's parent, or `PARSEC_NO_NODE` for the root and for a node
// that's been removed, detached or not yet inserted.
parsec_node parsec_parent(const parsec_document *doc, parsec_node node);

// The node's first child, or `PARSEC_NO_NODE`.
parsec_node parsec_first_child(const parsec_document *doc, parsec_node node);

// The node's last child, or `PARSEC_NO_NODE`.
parsec_node parsec_last_child(const parsec_document *doc, parsec_node node);

// The next node with the same parent, or `PARSEC_NO_NODE`.
parsec_node parsec_next_sibling(const parsec_document *doc, parsec_node node);

// The previous node with the same parent, or `PARSEC_NO_NODE`.
parsec_node parsec_prev_sibling(const parsec_document *doc, parsec_node node);

// A `<template>` element's contents, a fragment whose children are what
// the template holds; `PARSEC_NO_NODE` for any other node. A
// template's children in the markup are here, not under the element.
parsec_node parsec_template_contents(const parsec_document *doc, parsec_node node);

// What the node is, as a `PARSEC_NODE_*` number; 0 if `node` isn't in the
// document.
uint32_t parsec_node_type(const parsec_document *doc, parsec_node node);

// An element's namespace as a `PARSEC_NS_*` number; 0 for any other node.
parsec_ns parsec_element_ns(const parsec_document *doc, parsec_node node);

// The node's name: an element's as the markup gives it (lowercase for
// HTML, `foreignObject` for SVG's), a doctype's, or a processing
// instruction's target. NULL for any other node.
//
// Like every string borrowed from a document, it's `*len` bytes of UTF-8
// with no NUL after them, and it's valid until the document is changed or
// freed.
const char *parsec_node_name(const parsec_document *doc, parsec_node node, size_t *len);

// A text, comment or CDATA node's text, or a processing instruction's
// data, borrowed as `parsec_node_name` says. NULL for any other node; for
// an element's text, `parsec_text`.
const char *parsec_node_value(const parsec_document *doc, parsec_node node, size_t *len);

// A doctype's public identifier, borrowed; empty when it has none, NULL
// for any other node.
const char *parsec_doctype_public_id(const parsec_document *doc, parsec_node node, size_t *len);

// A doctype's system identifier, borrowed; empty when it has none, NULL
// for any other node.
const char *parsec_doctype_system_id(const parsec_document *doc, parsec_node node, size_t *len);

// How many attributes an element has; 0 for any other node.
size_t parsec_attr_count(const parsec_document *doc, parsec_node node);

// An element's `index`th attribute, in source order: its name as written
// (`xlink:href`, `viewBox`) and its value, both borrowed as
// `parsec_node_name` says. Any of the four out pointers may be NULL if you
// don't want that part. `PARSEC_ERR_INVALID` if `index` is past the last,
// `PARSEC_ERR_REMOVED` for a removed element.
parsec_status parsec_attr_at(const parsec_document *doc,
                             parsec_node node,
                             size_t index,
                             const char **name,
                             size_t *name_len,
                             const char **value,
                             size_t *value_len);

// The value of the element's attribute named `name`, as `getAttribute`
// finds it: by the name as written (`xlink:href`), in any case on an HTML
// element. Borrowed as `parsec_node_name` says; NULL if there's no such
// attribute, or the node isn't an element.
const char *parsec_attr_get(const parsec_document *doc,
                            parsec_node node,
                            const char *name,
                            size_t name_len,
                            size_t *value_len);

// Sets an attribute, as `setAttribute` does: it replaces the first one of
// that name or adds one at the end. HTML elements' attribute names are
// lowercased. A name `setAttribute` refuses (empty, or with whitespace,
// `/`, `>`, `=`, a quote or a control character in it) is
// `PARSEC_ERR_INVALID`.
parsec_status parsec_attr_set(parsec_document *doc,
                              parsec_node node,
                              const char *name,
                              size_t name_len,
                              const char *value,
                              size_t value_len);

// Removes the element's attribute named `name`, matched as
// `parsec_attr_get` matches it. `*removed`, if `removed` isn't NULL, says
// whether there was one; there not being one isn't an error.
parsec_status parsec_attr_remove(parsec_document *doc,
                                 parsec_node node,
                                 const char *name,
                                 size_t name_len,
                                 bool *removed);

// The markup for the node's children, as `innerHTML` gives it. The string
// is yours: NUL-terminated, its length without the NUL in `*len` (if `len`
// isn't NULL), and freed with `parsec_string_free`. NULL if `node` isn't
// in the document.
char *parsec_inner_html(const parsec_document *doc, parsec_node node, size_t *len);

// The markup for the node and its children, as `outerHTML` gives it; the
// whole page for the root. Yours to free, as `parsec_inner_html` says.
char *parsec_outer_html(const parsec_document *doc, parsec_node node, size_t *len);

// The text of the node and everything under it, as `textContent` gives
// it. Yours to free, as `parsec_inner_html` says.
char *parsec_text(const parsec_document *doc, parsec_node node, size_t *len);

// Compiles a selector list (`len` bytes, such as `"ul > li.hit, p"`) into
// `*out`. On `PARSEC_ERR_SELECTOR`, if `message` isn't NULL, `*message` is
// a sentence saying what's wrong, yours to free with `parsec_string_free`.
// On any error `*out` is NULL.
//
// A selector can be shared: run it on any number of documents, from any
// number of threads at once.
parsec_status parsec_selector_compile(const char *selector,
                                      size_t len,
                                      parsec_selector **out,
                                      char **message);

// Frees a compiled selector. NULL is fine and does nothing.
void parsec_selector_free(parsec_selector *selector);

// Calls `callback` with each element under `scope` that matches, in
// document order, and stops early if it returns nonzero. `scope` itself is
// never a match, and is what `:scope` matches. As in a browser,
// combinators look past it: `article .price` run on an article finds the
// prices inside it.
//
// The callback may read the document but not change it: a change from
// there fails with `PARSEC_ERR_BUSY`, so collect the nodes and change it
// afterwards. `parsec_document_free` from there frees it when this
// returns.
parsec_status parsec_select(const parsec_document *doc,
                            const parsec_selector *selector,
                            parsec_node scope,
                            parsec_match_fn callback,
                            void *ctx);

// `parsec_select`, with limits for a selector or a page you don't
// control: a `:has()` of more than one compound over a very deep tree can
// take seconds. Past `max_steps` compounds matched, or past `budget_us`
// microseconds (the clock is read every 1,024 compounds), it gives up
// with `PARSEC_ERR_TIMEOUT`; 0 is no limit for either. A walk of the tree
// is a compound or so an element.
//
// It finds every match before it calls `callback` with any, so a limit
// that runs out calls it with none, never with part of the answer; and
// stopping early from the callback saves no matching.
parsec_status parsec_select_with(const parsec_document *doc,
                                 const parsec_selector *selector,
                                 parsec_node scope,
                                 uint64_t max_steps,
                                 uint64_t budget_us,
                                 parsec_match_fn callback,
                                 void *ctx);

// The first element under `scope` that matches, without looking further;
// `PARSEC_NO_NODE` if none does, or on a NULL handle or a bad `scope`.
parsec_node parsec_select_first(const parsec_document *doc,
                                const parsec_selector *selector,
                                parsec_node scope);

// Whether `node` matches the selector, with the root element as `:scope`.
// False for a NULL handle, a node that isn't an element, or one that isn't
// in the document.
bool parsec_matches(const parsec_document *doc, const parsec_selector *selector, parsec_node node);

// `parsec_matches`, with the limits `parsec_select_with` takes: sets
// `*out` (when not NULL) to whether `node` matches, false for a node that
// isn't an element, or gives `PARSEC_ERR_TIMEOUT` past a limit.
parsec_status parsec_matches_with(const parsec_document *doc,
                                  const parsec_selector *selector,
                                  parsec_node node,
                                  uint64_t max_steps,
                                  uint64_t budget_us,
                                  bool *out);

// A new element named `name` in namespace `ns`, not yet in the tree;
// insert it with `parsec_append` or `parsec_insert_before`. HTML names are
// lowercased, as `createElement` does. `PARSEC_NO_NODE` on a NULL
// document, an unknown namespace, or a name `createElement` refuses: an
// empty one, or one with whitespace, `/`, `>`, `=`, a quote or a control
// character in it.
parsec_node parsec_create_element(parsec_document *doc, parsec_ns ns, const char *name, size_t len);

// A new text node, not yet in the tree.
parsec_node parsec_create_text(parsec_document *doc, const char *text, size_t len);

// A new comment, not yet in the tree.
parsec_node parsec_create_comment(parsec_document *doc, const char *text, size_t len);

// Moves `child` to be the last child of `parent`, taking it from wherever
// it was.
parsec_status parsec_append(parsec_document *doc, parsec_node parent, parsec_node child);

// Moves `child` into `parent`, just before `before`, which must be one of
// `parent`'s children; `PARSEC_NO_NODE` puts it last. Refuses, with the
// tree unchanged, to put a node inside itself, to give children to a node
// that can't have them, or to insert a document or fragment.
parsec_status parsec_insert_before(parsec_document *doc,
                                   parsec_node parent,
                                   parsec_node child,
                                   parsec_node before);

// Takes the node, and everything in it, out of the document for good, as
// `lxb_dom_node_destroy_deep` would. Their numbers stay valid but read
// as nothing: each keeps its node type, and an element its name, but
// has no parent, children, siblings, text, attributes, spans or markup,
// matches no selector, and editing or inserting it is
// `PARSEC_ERR_REMOVED`. So a node you still hold after something else
// removed it can't quietly read stale data. A template's contents and a
// shadow host's shadow tree go with it. Memory goes when the document is
// freed. Removing the root, a template's contents or a node already
// removed does nothing. To move a node, use `parsec_detach`.
parsec_status parsec_remove(parsec_document *doc, parsec_node node);

// Takes the node out of the tree, as `lxb_dom_node_remove` does. It and
// everything in it stay usable: insert it again with `parsec_append` or
// `parsec_insert_before` to move it. `PARSEC_ERR_REMOVED` for a node
// `parsec_remove` took out.
parsec_status parsec_detach(parsec_document *doc, parsec_node node);

// Whether `parsec_remove` took the node out, or one it was inside. False
// for a detached node, a NULL handle or a bad node.
bool parsec_node_is_removed(const parsec_document *doc, parsec_node node);

// Where `node` was written, as byte offsets into the input from `*start`
// up to `*end`: an element's start tag from `<` to `>`, or the whole of a
// text node, comment, doctype or processing instruction. False, leaving
// both alone, for a document parsed without `PARSEC_SPANS`, a node the
// parser implied (`<html>`, `<head>`, `<body>`, `<tbody>`) or an edit
// made, a NULL handle or a bad node.
bool parsec_node_span(const parsec_document *doc, parsec_node node, size_t *start, size_t *end);

// Where an element's end tag was written, as `parsec_node_span` gives a
// start tag: the end tag that closed it (`</p>`, or a `</body>` that
// closed it too). False when it had none.
bool parsec_end_span(const parsec_document *doc, parsec_node node, size_t *start, size_t *end);

// Where an element's `index`th attribute (as `parsec_attr_at` counts
// them) was written: from its name to the end of its value, past a
// closing quote. False for one an edit set, or as `parsec_node_span`.
bool parsec_attr_span(const parsec_document *doc,
                      parsec_node node,
                      size_t index,
                      size_t *start,
                      size_t *end);

// The line and column of a byte offset into the input, as an editor shows
// them: both from 1, a line ending at an LF, a CRLF or a lone CR, and the
// column counted in characters, not bytes. It reads the input up to
// `offset` each time.
parsec_status parsec_line_col(const parsec_document *doc,
                              size_t offset,
                              size_t *line,
                              size_t *column);

// How many parse errors the input had: 0 unless it was parsed with
// `PARSEC_ERRORS`.
size_t parsec_errors_count(const parsec_document *doc);

// The `index`th parse error, in source order, into `*out`.
// `PARSEC_ERR_INVALID` if `index` is past the last.
parsec_status parsec_errors_get(const parsec_document *doc, size_t index, parsec_syntax_error *out);

// Compiles an XPath 1.0 expression (`len` bytes, such as
// `"//a[@href]/@href"`) into `*out`. On `PARSEC_ERR_XPATH`, if `message`
// isn't NULL, `*message` is a sentence saying what's wrong, yours to free
// with `parsec_string_free`. On any error `*out` is NULL.
//
// Names match as lxml matches them in HTML: lowercase, in no namespace,
// so `//div` finds every div and `//DIV` none.
parsec_status parsec_xpath_compile(const char *expr,
                                   size_t len,
                                   parsec_xpath **out,
                                   char **message);

// Frees a compiled XPath expression. NULL is fine and does nothing.
void parsec_xpath_free(parsec_xpath *xpath);

// Evaluates `xpath` with `context` as the context node (`parsec_root()`
// for the whole document) and puts the result in `*out`. Errors as
// `parsec_xpath_compile`; on any error `*out` is NULL.
//
// The first query after a change indexes the document, which takes about
// as long as a pass over it; later ones reuse the index. Like any read,
// it may run on many threads at once.
parsec_status parsec_xpath_evaluate(const parsec_document *doc,
                                    const parsec_xpath *xpath,
                                    parsec_node context,
                                    parsec_xpath_result **out,
                                    char **message);

// Frees an XPath result. NULL is fine and does nothing.
void parsec_xpath_result_free(parsec_xpath_result *result);

// Which of `PARSEC_XPATH_NODES`, `_STRING`, `_NUMBER` and `_BOOLEAN` the
// result is; 0 for NULL.
parsec_xpath_type parsec_xpath_result_type(const parsec_xpath_result *result);

// How many items a node-set holds; 0 for any other result.
size_t parsec_xpath_result_count(const parsec_xpath_result *result);

// The node-set's `index`th item, in document order. A node comes back as
// `*node` with `*attr` set to `PARSEC_NO_ATTR`; an attribute as its
// element in `*node` and its place among the element's attributes in
// `*attr`, for `parsec_attr_at`; a namespace node as its element and
// `PARSEC_NS_ATTR`. Either out pointer may be NULL.
// `PARSEC_ERR_INVALID` if `index` is past the last item or the result
// isn't a node-set.
parsec_status parsec_xpath_result_item(const parsec_xpath_result *result,
                                       size_t index,
                                       parsec_node *node,
                                       uint32_t *attr);

// A string result, borrowed from the result (valid until it's freed),
// `*len` bytes with no NUL after them; NULL for any other result.
const char *parsec_xpath_result_string(const parsec_xpath_result *result, size_t *len);

// A number result; NaN for any other result.
double parsec_xpath_result_number(const parsec_xpath_result *result);

// A boolean result; false for any other result.
bool parsec_xpath_result_boolean(const parsec_xpath_result *result);

// Parses `len` bytes of UTF-8 as JavaScript, a script or, with
// `PARSEC_JS_MODULE`, a module, and puts its ESTree in `*out`: JSON in
// the shape acorn gives, `start` and `end` as byte offsets into the
// source. `*out_len` (if not NULL) is its length; free it with
// `parsec_string_free`. `opts` may be NULL.
//
// Fails with `PARSEC_ERR_JS` for a syntax error or nesting past
// `max_depth`, `PARSEC_ERR_TIMEOUT` past `max_steps`, and
// `PARSEC_ERR_TOO_LARGE` over 4 GiB; each puts a `parsec_js_error` in
// `*err`, if `err` isn't NULL, to free with `parsec_js_error_free`.
// Source that isn't UTF-8, or a `max_depth` over 256 (deeper needs more
// stack than a thread is sure to have), fails with `PARSEC_ERR_INVALID`.
// On any error `*out` is NULL, and `*err` is NULL unless set as above.
parsec_status parsec_js_parse(const char *src,
                              size_t len,
                              const parsec_js_options *opts,
                              char **out,
                              size_t *out_len,
                              parsec_js_error **err);

// Reads the state a page keeps in a script (`window.__NUXT__ = …`, a
// top-level `var`, `JSON.parse("…")`, Next.js's `self.__next_f.push`)
// from `len` bytes of UTF-8, the text of one `<script>`, without running
// anything. `*out` is a JSON array in source order of
// `{"name": …, "value": …}`, or `{"name": …, "error": {"code": …,
// "source": …}}` for a value that isn't data; free it with
// `parsec_string_free`.
//
// Fails as `parsec_js_parse` does, with `*err` set the same way.
parsec_status parsec_js_page_state(const char *src,
                                   size_t len,
                                   char **out,
                                   size_t *out_len,
                                   parsec_js_error **err);

// `parsec_js_page_state` of each inline script in `doc` a browser would
// run, in document order, in one JSON array in `*out`; free it with
// `parsec_string_free`. A script with `src`, one that isn't JavaScript
// (`type="application/json"`), one inside a `<template>`, and one that
// doesn't parse are passed over.
parsec_status parsec_js_document_state(const parsec_document *doc, char **out, size_t *out_len);

// Frees a `parsec_js_error` and its strings. NULL is fine and does
// nothing.
void parsec_js_error_free(parsec_js_error *err);

// Parses `len` bytes as an XML document into `*out`, to free with
// `parsec_xml_document_free`. The bytes are decoded as XML says: from
// the byte order mark, else the first bytes' pattern and the encoding
// the XML declaration names (`<?xml version="1.0"
// encoding="ISO-8859-1"?>`), else UTF-8; bytes that aren't that
// encoding are an error, not U+FFFD. `opts` may be NULL. Nothing is
// ever fetched: the external subset and external entities are left
// unread, and the document's warnings say so.
//
// Fails with `PARSEC_ERR_XML` for XML that isn't well-formed or runs
// past a limit in `opts`, and `PARSEC_ERR_TOO_LARGE` over 4 GiB; each
// puts a `parsec_xml_error` in `*err`, if `err` isn't NULL, to free with
// `parsec_xml_error_free`. An unknown flag fails with
// `PARSEC_ERR_INVALID`. On any error `*out` is NULL, and `*err` is NULL
// unless set as above.
parsec_status parsec_xml_parse(const char *src,
                               size_t len,
                               const parsec_xml_options *opts,
                               parsec_xml_document **out,
                               parsec_xml_error **err);

// Frees a `parsec_xml_error` and its message. NULL is fine and does
// nothing.
void parsec_xml_error_free(parsec_xml_error *err);

// Frees a document. NULL is fine and does nothing. Every node number,
// and every string borrowed from it, is invalid afterwards.
void parsec_xml_document_free(parsec_xml_document *doc);

// The Encoding Standard's name for what the bytes were decoded from,
// such as "UTF-8" or "windows-1252", NUL-terminated and valid as long as
// the document; NULL for a NULL handle.
const char *parsec_xml_document_encoding(const parsec_xml_document *doc);

// How many nodes the document holds: every node number below this is
// valid. The document itself is node 0, as `parsec_root` says.
size_t parsec_xml_node_count(const parsec_xml_document *doc);

// The document element, the one element at the top.
parsec_node parsec_xml_document_element(const parsec_xml_document *doc);

// The node's parent, or `PARSEC_NO_NODE` for the document.
parsec_node parsec_xml_parent(const parsec_xml_document *doc, parsec_node node);

// The node's first child, or `PARSEC_NO_NODE`.
parsec_node parsec_xml_first_child(const parsec_xml_document *doc, parsec_node node);

// The node's last child, or `PARSEC_NO_NODE`.
parsec_node parsec_xml_last_child(const parsec_xml_document *doc, parsec_node node);

// The next node with the same parent, or `PARSEC_NO_NODE`.
parsec_node parsec_xml_next_sibling(const parsec_xml_document *doc, parsec_node node);

// The previous node with the same parent, or `PARSEC_NO_NODE`.
parsec_node parsec_xml_prev_sibling(const parsec_xml_document *doc, parsec_node node);

// What the node is, as a `PARSEC_NODE_*` number (`PARSEC_NODE_CDATA`
// for a CDATA section, `PARSEC_NODE_ENTITY_REFERENCE` for an entity
// Parsec didn't read); 0 if `node` isn't in the document.
uint32_t parsec_xml_node_type(const parsec_xml_document *doc, parsec_node node);

// The node's name: an element's as written, prefix and all
// (`atom:link`), the doctype's, a processing instruction's target, or
// the entity an entity reference names. NULL for any other node.
// Borrowed, as every string from a document is: `*len` bytes of UTF-8
// with no NUL after them, valid until the document is freed.
const char *parsec_xml_node_name(const parsec_xml_document *doc, parsec_node node, size_t *len);

// An element's local name, `link` for `atom:link`, borrowed; NULL for
// any other node.
const char *parsec_xml_local_name(const parsec_xml_document *doc, parsec_node node, size_t *len);

// An element's prefix, `atom` for `atom:link`, borrowed; empty for none,
// NULL for any other node.
const char *parsec_xml_prefix(const parsec_xml_document *doc, parsec_node node, size_t *len);

// The namespace URI an element's prefix (or the default namespace) is
// bound to, borrowed; empty for none, NULL for any other node.
const char *parsec_xml_namespace_uri(const parsec_xml_document *doc, parsec_node node, size_t *len);

// A text, CDATA or comment node's text, or a processing instruction's
// data, borrowed; NULL for any other node. For an element's text,
// `parsec_xml_text`.
const char *parsec_xml_node_value(const parsec_xml_document *doc, parsec_node node, size_t *len);

// How many attributes an element has, namespace declarations and the
// DTD's defaults included; 0 for any other node.
size_t parsec_xml_attr_count(const parsec_xml_document *doc, parsec_node node);

// An element's `index`th attribute into `*out`: in the order of its
// start tag, then the defaults the DTD supplied. `PARSEC_ERR_INVALID` if
// `index` is past the last, or the node isn't an element.
parsec_status parsec_xml_attr_at(const parsec_xml_document *doc,
                                 parsec_node node,
                                 size_t index,
                                 parsec_xml_attr *out);

// The value of the element's attribute named `name` as written, prefix
// and all (`xlink:href`), borrowed; NULL if there's no such attribute,
// or the node isn't an element.
const char *parsec_xml_attr_get(const parsec_xml_document *doc,
                                parsec_node node,
                                const char *name,
                                size_t name_len,
                                size_t *value_len);

// The value of the element's attribute in namespace `ns` (empty, or
// NULL with a length of 0, for none) with local name `local`, whatever
// its prefix, borrowed; NULL if there's no such attribute.
const char *parsec_xml_attr_get_ns(const parsec_xml_document *doc,
                                   parsec_node node,
                                   const char *ns,
                                   size_t ns_len,
                                   const char *local,
                                   size_t local_len,
                                   size_t *value_len);

// The node and everything under it as XML; the whole document, its XML
// declaration and `<!DOCTYPE>` included, for the root. An element on
// its own takes along the namespace declarations it's in the scope of,
// so it means the same. The string is yours: NUL-terminated, its length
// without the NUL in `*len` (if `len` isn't NULL), freed with
// `parsec_string_free`. NULL if `node` isn't in the document.
char *parsec_xml_serialize(const parsec_xml_document *doc, parsec_node node, size_t *len);

// The text of the node and everything under it, character data and
// CDATA in order, as XPath's `string()` reads it. Yours to free, as
// `parsec_xml_serialize` says.
char *parsec_xml_text(const parsec_xml_document *doc, parsec_node node, size_t *len);

// Where `node` was written, as byte offsets into the input from
// `*start` up to `*end`: an element from its start tag's `<` to its end
// tag's `>`. A node an entity reference produced has the reference's
// place. False, leaving both alone, for a NULL handle or a bad node.
bool parsec_xml_node_span(const parsec_xml_document *doc,
                          parsec_node node,
                          size_t *start,
                          size_t *end);

// The line and column of a byte offset into the input, as
// `parsec_line_col` gives them for HTML. It reads the input up to
// `offset` each time.
parsec_status parsec_xml_line_col(const parsec_xml_document *doc,
                                  size_t offset,
                                  size_t *line,
                                  size_t *column);

// How many warnings the parse gave: what it didn't read and says so, as
// the XML spec asks of a processor that doesn't validate (the external
// subset, an external entity). None of them is an error.
size_t parsec_xml_warnings_count(const parsec_xml_document *doc);

// The `index`th warning, in source order, into `*out`: its code (such
// as `external-dtd-not-loaded`), and where it is in the input.
// `PARSEC_ERR_INVALID` if `index` is past the last.
parsec_status parsec_xml_warnings_get(const parsec_xml_document *doc,
                                      size_t index,
                                      parsec_syntax_error *out);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* PARSEC_H */
