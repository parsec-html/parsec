/* SPDX-License-Identifier: Apache-2.0
 * Copyright 2026 James Conway
 *
 * Parsec's C API: an HTML parser that follows the WHATWG HTML Standard,
 * with CSS selectors, editing and serialization.
 *
 * Generated from crates/parsec-capi/src/lib.rs by cbindgen; don't edit it.
 * The wiki's C-API page has a guide and examples.
 *
 * The rules every function keeps:
 *
 * - Handles are opaque. A parsec_document, parsec_selector, parsec_xpath,
 *   parsec_xpath_result or parsec_byte_parser is made by one function and
 *   freed by one, and NULL is accepted everywhere: a call given NULL
 *   fails with PARSEC_ERR_NULL, or returns NULL, PARSEC_NO_NODE, 0 or
 *   false, and the free functions do nothing.
 * - Nodes are numbers. A parsec_node is valid for as long as its document
 *   is, and a number that isn't a node of the document is refused, never
 *   read. Removing a node detaches it; nothing is freed until the
 *   document is.
 * - Strings in are a pointer and a length in bytes, UTF-8, with no NUL
 *   needed. NULL with a length of 0 is the empty string. Bytes that aren't
 *   UTF-8 become U+FFFD. The exception is a page given to
 *   parsec_parse_bytes or a parsec_byte_parser, which is decoded as a
 *   browser decodes it, in whatever encoding it's in.
 * - Strings out come two ways. Borrowed ones (const char *, from
 *   parsec_node_name, parsec_node_value, parsec_attr_get and the like)
 *   point into the document, are *len bytes long with no NUL after them,
 *   and stay valid until the document is next changed or freed. Owned
 *   ones (char *, from parsec_inner_html, parsec_outer_html and
 *   parsec_text) are copies, NUL-terminated, freed with
 *   parsec_string_free and nothing else.
 * - Nothing panics into C. A bug in Parsec comes back as
 *   PARSEC_ERR_PANIC instead.
 * - Threads: any number of threads may read one document at once (every
 *   function taking a const parsec_document *, parsec_xpath_evaluate
 *   included), and a selector or a compiled XPath may be used from any
 *   number of threads. A call taking a non-const parsec_document *
 *   changes it, and needs the document to itself, as a
 *   parsec_byte_parser needs itself to itself for each call. A
 *   document or selector may be freed on a different thread from the one
 *   that made it.
 */

#ifndef PARSEC_H
#define PARSEC_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

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

#define PARSEC_NODE_ELEMENT 1

#define PARSEC_NODE_TEXT 3

#define PARSEC_NODE_CDATA 4

#define PARSEC_NODE_PROCESSING_INSTRUCTION 7

#define PARSEC_NODE_COMMENT 8

#define PARSEC_NODE_DOCUMENT 9

#define PARSEC_NODE_DOCTYPE 10

#define PARSEC_NODE_FRAGMENT 11

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

// No node: what navigation returns at the end of a list, and what you pass
// to `parsec_insert_before` to insert last.
#define PARSEC_NO_NODE UINT32_MAX

#define PARSEC_OK 0

// A pointer that must not be NULL was NULL.
#define PARSEC_ERR_NULL 1

// An argument was out of range: an unknown flag or namespace, or a length
// over `PTRDIFF_MAX`.
#define PARSEC_ERR_INVALID 2

// The input is over the 512 MiB a document can hold.
#define PARSEC_ERR_TOO_LARGE 3

// The parse ran past the budget given to `parsec_parse_with`.
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

// A node-set: `parsec_xpath_result_count` items, in document order.
#define PARSEC_XPATH_NODES 1

#define PARSEC_XPATH_STRING 2

#define PARSEC_XPATH_NUMBER 3

#define PARSEC_XPATH_BOOLEAN 4

#define PARSEC_NS_HTML 1

#define PARSEC_NS_SVG 2

#define PARSEC_NS_MATHML 3

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

// The library's version, such as "0.1.0", NUL-terminated and static.
const char *parsec_version(void);

// A sentence saying what `status` means, NUL-terminated and static.
const char *parsec_status_message(parsec_status status);

// Frees a string a `parsec_*_html` or `parsec_text` call returned. NULL
// is fine and does nothing.
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
// borrowed from it. NULL is fine and does nothing.
void parsec_document_free(parsec_document *doc);

// How many nodes the document holds, removed ones included: every node
// number below this is valid.
size_t parsec_node_count(const parsec_document *doc);

// The document itself, which is always node 0.
parsec_node parsec_root(void);

// The node's parent, or `PARSEC_NO_NODE` for the root and for a node
// that's been removed or not yet inserted.
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
// don't want that part. `PARSEC_ERR_INVALID` if `index` is past the last.
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
// lowercased.
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
// Don't change the document from the callback; collect the nodes and
// change it afterwards.
parsec_status parsec_select(const parsec_document *doc,
                            const parsec_selector *selector,
                            parsec_node scope,
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

// A new element named `name` in namespace `ns`, not yet in the tree;
// insert it with `parsec_append` or `parsec_insert_before`. HTML names are
// lowercased, as `createElement` does. `PARSEC_NO_NODE` on a NULL
// document or an unknown namespace.
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

// Takes the node out of the tree. Its number stays valid and it can be
// inserted again; its memory goes when the document is freed.
parsec_status parsec_remove(parsec_document *doc, parsec_node node);

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
// `*attr`, for `parsec_attr_at`. Either out pointer may be NULL.
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

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* PARSEC_H */
