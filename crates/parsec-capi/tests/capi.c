/* SPDX-License-Identifier: Apache-2.0
 * Copyright 2026 James Conway
 *
 * The C API from C: parse, select, read, edit, serialize, bytes in any
 * encoding, and every error path a binding could hit. Built with
 * HTML5LIB_ENCODING defined as the html5lib-tests encoding/ folder, it
 * runs those cases too. Built and run by tests/c.rs, by CI on Linux, macOS and Windows
 * through CMake, and on Linux under AddressSanitizer, LeakSanitizer and
 * UndefinedBehaviorSanitizer. Exits 0 when every check passes; the first
 * failure prints its line.
 */

#include <parsec.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(cond)                                                       \
    do {                                                                  \
        if (!(cond)) {                                                    \
            fprintf(stderr, "%s:%d: failed: %s\n", __FILE__, __LINE__, #cond); \
            failures++;                                                   \
        }                                                                 \
    } while (0)

/* A borrowed string equals a C string. The length is read through a
 * pointer, after the call that sets it: C doesn't say in which order a
 * call's arguments are worked out. */
static int is(const char *s, const size_t *len, const char *want) {
    return s != NULL && *len == strlen(want) && memcmp(s, want, *len) == 0;
}

/* An owned string equals a C string; it's freed either way. */
static int owned_is(char *s, const size_t *len, const char *want) {
    int ok = s != NULL && *len == strlen(want) && strcmp(s, want) == 0;
    if (!ok && s != NULL) fprintf(stderr, "  got: %s\n", s);
    parsec_string_free(s);
    return ok;
}

static parsec_selector *compile(const char *s) {
    parsec_selector *sel = NULL;
    CHECK(parsec_selector_compile(s, strlen(s), &sel, NULL) == PARSEC_OK);
    return sel;
}

struct collected {
    parsec_node nodes[16];
    size_t count;
    size_t stop_after;
};

static int collect(parsec_node node, void *ctx) {
    struct collected *c = ctx;
    if (c->count < 16) c->nodes[c->count] = node;
    c->count++;
    return c->stop_after != 0 && c->count >= c->stop_after;
}

static void parse_select_read(void) {
    const char *html =
        "<!DOCTYPE html><title>T</title>"
        "<ul id=list><li class=hit>one<li>two<li class=hit>three</ul>"
        "<svg viewBox='0 0 1 1'><foreignObject/></svg><!--note-->";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    CHECK(doc != NULL);
    CHECK(parsec_node_count(doc) > 10);

    parsec_node root = parsec_root();
    CHECK(parsec_node_type(doc, root) == PARSEC_NODE_DOCUMENT);
    parsec_node doctype = parsec_first_child(doc, root);
    CHECK(parsec_node_type(doc, doctype) == PARSEC_NODE_DOCTYPE);
    size_t len = 99;
    CHECK(is(parsec_node_name(doc, doctype, &len), &len, "html"));
    CHECK(is(parsec_doctype_public_id(doc, doctype, &len), &len, ""));

    /* Every match, in document order. */
    parsec_selector *hits = compile("li.hit");
    struct collected c = {{0}, 0, 0};
    CHECK(parsec_select(doc, hits, root, collect, &c) == PARSEC_OK);
    CHECK(c.count == 2);
    CHECK(owned_is(parsec_text(doc, c.nodes[0], &len), &len, "one"));
    CHECK(owned_is(parsec_text(doc, c.nodes[1], &len), &len, "three"));
    CHECK(parsec_matches(doc, hits, c.nodes[0]));

    /* Stopping early, and the first match alone. */
    struct collected one = {{0}, 0, 1};
    CHECK(parsec_select(doc, hits, root, collect, &one) == PARSEC_OK);
    CHECK(one.count == 1 && one.nodes[0] == c.nodes[0]);
    CHECK(parsec_select_first(doc, hits, root) == c.nodes[0]);

    /* Navigation and names. */
    parsec_node ul = parsec_parent(doc, c.nodes[0]);
    CHECK(is(parsec_node_name(doc, ul, &len), &len, "ul"));
    CHECK(parsec_element_ns(doc, ul) == PARSEC_NS_HTML);
    CHECK(parsec_first_child(doc, ul) == c.nodes[0]);
    CHECK(parsec_last_child(doc, ul) == c.nodes[1]);
    parsec_node two = parsec_next_sibling(doc, c.nodes[0]);
    CHECK(parsec_prev_sibling(doc, two) == c.nodes[0]);
    CHECK(parsec_next_sibling(doc, c.nodes[1]) == PARSEC_NO_NODE);
    parsec_node text = parsec_first_child(doc, two);
    CHECK(parsec_node_type(doc, text) == PARSEC_NODE_TEXT);
    CHECK(is(parsec_node_value(doc, text, &len), &len, "two"));
    CHECK(parsec_node_value(doc, ul, &len) == NULL && len == 0);

    /* Attributes, by name and by index. */
    CHECK(is(parsec_attr_get(doc, ul, "ID", 2, &len), &len, "list"));
    CHECK(parsec_attr_get(doc, ul, "class", 5, &len) == NULL);
    CHECK(parsec_attr_count(doc, ul) == 1);
    const char *name, *value;
    size_t name_len, value_len;
    CHECK(parsec_attr_at(doc, ul, 0, &name, &name_len, &value, &value_len) == PARSEC_OK);
    CHECK(is(name, &name_len, "id") && is(value, &value_len, "list"));
    CHECK(parsec_attr_at(doc, ul, 1, &name, NULL, NULL, NULL) == PARSEC_ERR_INVALID);
    CHECK(parsec_attr_at(doc, text, 0, NULL, NULL, NULL, NULL) == PARSEC_ERR_NOT_AN_ELEMENT);

    /* SVG keeps its case, and its namespace. */
    parsec_selector *svg = compile("svg");
    parsec_node s = parsec_select_first(doc, svg, root);
    CHECK(parsec_element_ns(doc, s) == PARSEC_NS_SVG);
    CHECK(is(parsec_attr_get(doc, s, "viewBox", 7, &len), &len, "0 0 1 1"));
    CHECK(is(parsec_node_name(doc, parsec_first_child(doc, s), &len), &len, "foreignObject"));

    parsec_selector_free(svg);
    parsec_selector_free(hits);
    parsec_document_free(doc);
}

static void edit_and_serialize(void) {
    const char *html = "<ul><li id=b>b</li></ul><p>old <b>bold</b></p>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    parsec_selector *sel = compile("#b");
    parsec_node b = parsec_select_first(doc, sel, parsec_root());
    parsec_node ul = parsec_parent(doc, b);

    parsec_node a = parsec_create_element(doc, PARSEC_NS_HTML, "LI", 2);
    CHECK(parsec_append(doc, a, parsec_create_text(doc, "a", 1)) == PARSEC_OK);
    CHECK(parsec_insert_before(doc, ul, a, b) == PARSEC_OK);
    parsec_node c = parsec_create_element(doc, PARSEC_NS_HTML, "li", 2);
    CHECK(parsec_append(doc, c, parsec_create_text(doc, "c & d", 5)) == PARSEC_OK);
    CHECK(parsec_insert_before(doc, ul, c, PARSEC_NO_NODE) == PARSEC_OK);
    CHECK(parsec_append(doc, ul, parsec_create_comment(doc, "end", 3)) == PARSEC_OK);
    CHECK(parsec_attr_set(doc, a, "Class", 5, "first", 5) == PARSEC_OK);
    bool removed = false;
    CHECK(parsec_attr_remove(doc, b, "id", 2, &removed) == PARSEC_OK && removed);
    CHECK(parsec_attr_remove(doc, b, "id", 2, &removed) == PARSEC_OK && !removed);

    size_t len;
    CHECK(owned_is(parsec_inner_html(doc, ul, &len), &len,
                   "<li class=\"first\">a</li><li>b</li><li>c &amp; d</li><!--end-->"));

    /* Detaching takes it out; the number still works, and can go back in. */
    CHECK(parsec_detach(doc, b) == PARSEC_OK);
    CHECK(parsec_parent(doc, b) == PARSEC_NO_NODE);
    CHECK(owned_is(parsec_outer_html(doc, b, &len), &len, "<li>b</li>"));
    CHECK(parsec_append(doc, ul, b) == PARSEC_OK);

    /* Edits that would break the tree are refused, and change nothing. */
    CHECK(parsec_append(doc, b, ul) == PARSEC_ERR_CYCLE);
    CHECK(parsec_append(doc, parsec_first_child(doc, a), c) == PARSEC_ERR_NOT_A_PARENT);
    CHECK(parsec_insert_before(doc, ul, c, parsec_root()) == PARSEC_ERR_NOT_A_CHILD);
    CHECK(parsec_append(doc, ul, parsec_root()) == PARSEC_ERR_NOT_INSERTABLE);
    CHECK(parsec_attr_set(doc, parsec_first_child(doc, a), "x", 1, "y", 1) == PARSEC_ERR_NOT_AN_ELEMENT);

    /* The whole page, with the length given and the NUL after it. */
    char *page = parsec_outer_html(doc, parsec_root(), &len);
    CHECK(page != NULL && strlen(page) == len);
    CHECK(strstr(page, "<li>c &amp; d</li><!--end--><li>b</li></ul><p>old <b>bold</b></p>") != NULL);
    parsec_string_free(page);

    parsec_selector_free(sel);
    parsec_document_free(doc);
}

/* parsec_remove is for good: a number held for the node, or for anything
 * in it, reads as nothing afterwards, which is what Lexbor's held nodes
 * don't do. parsec_detach keeps it, to move. */
static void removed_and_detached(void) {
    const char *html =
        "<div><p id=a class=x>one <b>two</b><!--c--></p><p id=b>three</p>"
        "<template><i>t</i></template></div>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), PARSEC_SPANS, &doc) == PARSEC_OK);
    parsec_selector *sel_a = compile("#a");
    parsec_selector *sel_p = compile("p, b, i");
    parsec_node p = parsec_select_first(doc, sel_a, parsec_root());
    parsec_node div = parsec_parent(doc, p);
    parsec_node text = parsec_first_child(doc, p);
    parsec_node b = parsec_next_sibling(doc, text);
    parsec_node comment = parsec_next_sibling(doc, b);
    parsec_node other = parsec_next_sibling(doc, p);
    parsec_node template = parsec_next_sibling(doc, other);
    parsec_node contents = parsec_template_contents(doc, template);
    parsec_node i = parsec_first_child(doc, contents);
    size_t len = 0, start = 0, end = 0;
    const char *name = NULL, *value = NULL;
    size_t name_len = 0, value_len = 0;
    bool gone = true;

    /* Detached: out of the tree, still itself, and it can go back. */
    CHECK(parsec_detach(doc, p) == PARSEC_OK);
    CHECK(!parsec_node_is_removed(doc, p) && !parsec_node_is_removed(doc, b));
    CHECK(parsec_parent(doc, p) == PARSEC_NO_NODE && parsec_parent(doc, b) == p);
    CHECK(owned_is(parsec_outer_html(doc, p, &len), &len,
                   "<p id=\"a\" class=\"x\">one <b>two</b><!--c--></p>"));
    CHECK(parsec_matches(doc, sel_a, p));
    CHECK(parsec_insert_before(doc, div, p, other) == PARSEC_OK);
    CHECK(parsec_parent(doc, p) == div);

    /* Removed: it and all it holds read as nothing. */
    CHECK(parsec_remove(doc, p) == PARSEC_OK);
    parsec_node held[] = {p, text, b, comment};
    for (size_t k = 0; k < sizeof held / sizeof held[0]; k++) {
        parsec_node n = held[k];
        CHECK(parsec_node_is_removed(doc, n));
        CHECK(parsec_parent(doc, n) == PARSEC_NO_NODE);
        CHECK(parsec_first_child(doc, n) == PARSEC_NO_NODE);
        CHECK(parsec_last_child(doc, n) == PARSEC_NO_NODE);
        CHECK(parsec_next_sibling(doc, n) == PARSEC_NO_NODE);
        CHECK(parsec_prev_sibling(doc, n) == PARSEC_NO_NODE);
        CHECK(parsec_attr_count(doc, n) == 0);
        CHECK(owned_is(parsec_outer_html(doc, n, &len), &len, ""));
        CHECK(owned_is(parsec_inner_html(doc, n, &len), &len, ""));
        CHECK(owned_is(parsec_text(doc, n, &len), &len, ""));
        CHECK(!parsec_node_span(doc, n, &start, &end));
        CHECK(!parsec_end_span(doc, n, &start, &end));
        CHECK(!parsec_matches(doc, sel_p, n));
        CHECK(parsec_select_first(doc, sel_p, n) == PARSEC_NO_NODE);
        struct collected found = {{0}, 0, 0};
        CHECK(parsec_select(doc, sel_p, n, collect, &found) == PARSEC_OK && found.count == 0);
        CHECK(parsec_append(doc, div, n) == PARSEC_ERR_REMOVED);
        CHECK(parsec_detach(doc, n) == PARSEC_ERR_REMOVED);
    }
    /* Each keeps its type, and an element its name, so you can tell what
     * it was; a text's or comment's text is empty. */
    CHECK(parsec_node_type(doc, p) == PARSEC_NODE_ELEMENT);
    CHECK(parsec_element_ns(doc, p) == PARSEC_NS_HTML);
    CHECK(is(parsec_node_name(doc, p, &len), &len, "p"));
    CHECK(parsec_node_type(doc, text) == PARSEC_NODE_TEXT);
    CHECK(is(parsec_node_value(doc, text, &len), &len, ""));
    CHECK(is(parsec_node_value(doc, comment, &len), &len, ""));
    CHECK(parsec_attr_get(doc, p, "id", 2, &len) == NULL && len == 0);
    CHECK(parsec_attr_at(doc, p, 0, &name, &name_len, &value, &value_len) == PARSEC_ERR_REMOVED);
    CHECK(!parsec_attr_span(doc, p, 0, &start, &end));
    CHECK(parsec_attr_set(doc, p, "id", 2, "z", 1) == PARSEC_ERR_REMOVED);
    CHECK(parsec_attr_remove(doc, p, "id", 2, &gone) == PARSEC_ERR_REMOVED && gone);
    parsec_node fresh = parsec_create_element(doc, PARSEC_NS_HTML, "span", 4);
    CHECK(parsec_append(doc, p, fresh) == PARSEC_ERR_REMOVED);
    CHECK(parsec_insert_before(doc, div, fresh, p) == PARSEC_ERR_REMOVED);
    CHECK(strcmp(parsec_status_message(PARSEC_ERR_REMOVED), "unknown status") != 0);
    /* Removing again, or the root, does nothing. */
    CHECK(parsec_remove(doc, p) == PARSEC_OK);
    CHECK(parsec_remove(doc, parsec_root()) == PARSEC_OK);
    CHECK(!parsec_node_is_removed(doc, parsec_root()));

    /* The rest of the document is as it was. */
    CHECK(parsec_append(doc, div, fresh) == PARSEC_OK);
    CHECK(owned_is(parsec_inner_html(doc, div, &len), &len,
                   "<p id=\"b\">three</p><template><i>t</i></template><span></span>"));
    struct collected left = {{0}, 0, 0};
    CHECK(parsec_select(doc, sel_p, parsec_root(), collect, &left) == PARSEC_OK);
    CHECK(left.count == 1 && left.nodes[0] == other);

    /* A template's contents go with it. */
    CHECK(parsec_remove(doc, template) == PARSEC_OK);
    CHECK(parsec_node_is_removed(doc, contents) && parsec_node_is_removed(doc, i));
    CHECK(parsec_template_contents(doc, template) == PARSEC_NO_NODE);
    CHECK(owned_is(parsec_inner_html(doc, div, &len), &len,
                   "<p id=\"b\">three</p><span></span>"));

    parsec_selector_free(sel_a);
    parsec_selector_free(sel_p);
    parsec_document_free(doc);
}

static void fragments_and_options(void) {
    parsec_document *frag = NULL;
    const char *rows = "<tr><td>1<td>2";
    CHECK(parsec_parse_fragment(rows, strlen(rows), PARSEC_NS_HTML, "tbody", 5, 0, &frag) == PARSEC_OK);
    CHECK(parsec_node_type(frag, parsec_root()) == PARSEC_NODE_FRAGMENT);
    size_t len;
    CHECK(owned_is(parsec_inner_html(frag, parsec_root(), &len), &len, "<tr><td>1</td><td>2</td></tr>"));
    parsec_document_free(frag);

    /* Scripting decides what <noscript> holds. */
    const char *ns = "<body><noscript><b>x</b></noscript>";
    parsec_document *on = NULL, *off = NULL;
    CHECK(parsec_parse(ns, strlen(ns), PARSEC_SCRIPTING, &on) == PARSEC_OK);
    CHECK(parsec_parse(ns, strlen(ns), 0, &off) == PARSEC_OK);
    parsec_selector *b = compile("noscript b");
    CHECK(parsec_select_first(on, b, parsec_root()) == PARSEC_NO_NODE);
    CHECK(parsec_select_first(off, b, parsec_root()) != PARSEC_NO_NODE);
    parsec_selector_free(b);
    parsec_document_free(on);
    parsec_document_free(off);

    /* A template's markup is in its contents. */
    const char *t = "<template><p>in</p></template>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(t, strlen(t), 0, &doc) == PARSEC_OK);
    parsec_selector *tpl = compile("template");
    parsec_node el = parsec_select_first(doc, tpl, parsec_root());
    parsec_node contents = parsec_template_contents(doc, el);
    CHECK(owned_is(parsec_inner_html(doc, contents, &len), &len, "<p>in</p>"));
    CHECK(parsec_template_contents(doc, contents) == PARSEC_NO_NODE);
    parsec_selector_free(tpl);
    parsec_document_free(doc);

    /* A depth limit flattens; a budget gives up. */
    char deep[3 * 600 + 1];
    for (int i = 0; i < 600; i++) memcpy(deep + 3 * i, "<b>", 3);
    CHECK(parsec_parse_with(deep, 3 * 600, 0, 0, 16, &doc) == PARSEC_OK);
    parsec_document_free(doc);
    size_t big_len = 4 << 20;
    char *big = malloc(big_len);
    CHECK(big != NULL);
    for (size_t i = 0; i < big_len; i += 4) memcpy(big + i, "<a> ", 4);
    doc = (parsec_document *)1;
    CHECK(parsec_parse_with(big, big_len, 0, 1, 0, &doc) == PARSEC_ERR_TIMEOUT);
    CHECK(doc == NULL);
    free(big);

    /* Bytes that aren't UTF-8 become U+FFFD. */
    CHECK(parsec_parse("<p>\xff</p>", 8, 0, &doc) == PARSEC_OK);
    CHECK(owned_is(parsec_text(doc, parsec_root(), &len), &len, "\xef\xbf\xbd"));
    parsec_document_free(doc);
}

static void errors_and_null(void) {
    parsec_document *doc = NULL;
    parsec_selector *sel = NULL;
    size_t len = 7;

    CHECK(parsec_parse(NULL, 0, 0, &doc) == PARSEC_OK);
    /* An empty page still has html, head and body, as in a browser. */
    CHECK(parsec_node_count(doc) == 4);
    CHECK(is(parsec_node_name(doc, parsec_first_child(doc, parsec_root()), &len), &len, "html"));
    parsec_document_free(doc);
    CHECK(parsec_parse(NULL, 1, 0, &doc) == PARSEC_ERR_NULL && doc == NULL);
    CHECK(parsec_parse("x", 1, 0, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_parse("x", 1, 0x100, &doc) == PARSEC_ERR_INVALID);
    CHECK(parsec_parse("x", (size_t)-1, 0, &doc) == PARSEC_ERR_INVALID);
    CHECK(parsec_parse_fragment("x", 1, 0, "div", 3, 0, &doc) == PARSEC_ERR_INVALID);

    char *message = NULL;
    CHECK(parsec_selector_compile("p:", 2, &sel, &message) == PARSEC_ERR_SELECTOR);
    CHECK(sel == NULL && message != NULL && strlen(message) > 0);
    parsec_string_free(message);
    CHECK(parsec_selector_compile(":lexbor-contains('x')", 21, &sel, NULL) == PARSEC_ERR_SELECTOR);

    CHECK(parsec_parse("<p>x", 4, 0, &doc) == PARSEC_OK);
    sel = compile("p");
    /* Node numbers that aren't in the document are refused, never read. */
    parsec_node bad = (parsec_node)parsec_node_count(doc);
    CHECK(parsec_node_type(doc, bad) == 0);
    CHECK(parsec_parent(doc, bad) == PARSEC_NO_NODE);
    CHECK(parsec_text(doc, bad, &len) == NULL);
    CHECK(parsec_node_name(doc, bad, &len) == NULL && len == 0);
    CHECK(parsec_remove(doc, bad) == PARSEC_ERR_NO_SUCH_NODE);
    CHECK(parsec_append(doc, parsec_root(), PARSEC_NO_NODE) == PARSEC_ERR_NO_SUCH_NODE);
    CHECK(parsec_select(doc, sel, bad, collect, NULL) == PARSEC_ERR_NO_SUCH_NODE);
    CHECK(parsec_select(doc, sel, parsec_root(), NULL, NULL) == PARSEC_ERR_NULL);
    CHECK(!parsec_matches(doc, sel, parsec_root()));
    CHECK(parsec_create_element(doc, 9, "p", 1) == PARSEC_NO_NODE);

    /* NULL handles. */
    CHECK(parsec_node_count(NULL) == 0);
    CHECK(parsec_first_child(NULL, 0) == PARSEC_NO_NODE);
    CHECK(parsec_text(NULL, 0, &len) == NULL);
    CHECK(parsec_select(NULL, sel, 0, collect, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_select(doc, NULL, 0, collect, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_select_first(doc, NULL, 0) == PARSEC_NO_NODE);
    CHECK(parsec_remove(NULL, 0) == PARSEC_ERR_NULL);
    CHECK(parsec_detach(NULL, 0) == PARSEC_ERR_NULL);
    CHECK(parsec_detach(doc, bad) == PARSEC_ERR_NO_SUCH_NODE);
    CHECK(!parsec_node_is_removed(NULL, 0) && !parsec_node_is_removed(doc, bad));
    CHECK(parsec_create_text(NULL, "x", 1) == PARSEC_NO_NODE);
    CHECK(parsec_node_name(doc, 0, NULL) == NULL);
    parsec_document_free(NULL);
    parsec_selector_free(NULL);
    parsec_string_free(NULL);

    CHECK(strcmp(parsec_status_message(PARSEC_ERR_CYCLE),
                 "a node can't be inserted into itself or its descendants") == 0);
    CHECK(strcmp(parsec_status_message(-1), "unknown status") == 0);
    CHECK(strlen(parsec_version()) >= 5);

    parsec_selector_free(sel);
    parsec_document_free(doc);
}

static void positions(void) {
    const char *html = "<p class=x>h\xc3\xa9llo</p>\n<div>x</div>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), PARSEC_SPANS, &doc) == PARSEC_OK);
    parsec_selector *sel = compile("p");
    parsec_node p = parsec_select_first(doc, sel, parsec_root());
    size_t start = 0, end = 0, line = 0, column = 0;
    CHECK(parsec_node_span(doc, p, &start, &end));
    CHECK(start == 0 && end == 11);
    CHECK(parsec_end_span(doc, p, &start, &end) && start == 17 && end == 21);
    CHECK(parsec_attr_span(doc, p, 0, &start, &end) && start == 3 && end == 10);
    CHECK(!parsec_attr_span(doc, p, 1, &start, &end));
    /* The text is 6 bytes, as the e with an accent takes two. */
    CHECK(parsec_node_span(doc, parsec_first_child(doc, p), &start, &end));
    CHECK(start == 11 && end == 17);
    CHECK(parsec_line_col(doc, 22, &line, &column) == PARSEC_OK && line == 2 && column == 1);
    CHECK(parsec_line_col(doc, 17, &line, &column) == PARSEC_OK && line == 1 && column == 17);
    /* The body was implied, so it was written nowhere. */
    CHECK(!parsec_node_span(doc, parsec_parent(doc, p), &start, &end));
    CHECK(parsec_node_span(doc, p, NULL, NULL));
    CHECK(!parsec_node_span(NULL, p, &start, &end));
    CHECK(!parsec_node_span(doc, PARSEC_NO_NODE, &start, &end));
    CHECK(parsec_line_col(NULL, 0, &line, &column) == PARSEC_ERR_NULL);
    parsec_document_free(doc);

    /* Byte offsets, which slice the UTF-8 that was passed in, past the
     * Basic Multilingual Plane too. */
    const char *wide = "<p>caf\xc3\xa9</p><b>\xe6\x97\xa5\xe6\x9c\xac</b>"
                       "<i title='\xf0\x9f\x98\x80'>\xf0\x9f\x98\x80x</i>";
    CHECK(parsec_parse(wide, strlen(wide), PARSEC_SPANS, &doc) == PARSEC_OK);
    const char *tags[] = {"p", "<p>", "</p>", "b", "<b>", "</b>",
                          "i", "<i title='\xf0\x9f\x98\x80'>", "</i>"};
    for (int t = 0; t < 9; t += 3) {
        parsec_selector *s = compile(tags[t]);
        parsec_node n = parsec_select_first(doc, s, parsec_root());
        CHECK(parsec_node_span(doc, n, &start, &end));
        CHECK(end - start == strlen(tags[t + 1]) && memcmp(wide + start, tags[t + 1], end - start) == 0);
        CHECK(parsec_end_span(doc, n, &start, &end));
        CHECK(end - start == strlen(tags[t + 2]) && memcmp(wide + start, tags[t + 2], end - start) == 0);
        if (t == 6) {
            CHECK(parsec_node_span(doc, parsec_first_child(doc, n), &start, &end));
            CHECK(end - start == 5 && memcmp(wide + start, "\xf0\x9f\x98\x80x", 5) == 0);
            /* The column counts characters: 35 of them come before </i>. */
            CHECK(parsec_end_span(doc, n, &start, &end));
            CHECK(parsec_line_col(doc, start, &line, &column) == PARSEC_OK && column == 36);
        }
        parsec_selector_free(s);
    }
    parsec_document_free(doc);

    /* Off unless asked for. */
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    CHECK(!parsec_node_span(doc, parsec_select_first(doc, sel, parsec_root()), &start, &end));
    parsec_selector_free(sel);
    parsec_document_free(doc);
}

/* Text that isn't UTF-8 is read with U+FFFD for the bad bytes, which
 * take more room than they did; spans and errors still index the bytes
 * that were passed in. */
static void positions_in_bytes_that_arent_utf8(void) {
    const char html[] = "\xff\xff\xff\xff<b x=1>t</b></p>";
    size_t len = sizeof html - 1, start = 0, end = 0, line = 0, column = 0;
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, len, PARSEC_SPANS | PARSEC_ERRORS, &doc) == PARSEC_OK);
    parsec_selector *sel = compile("b");
    parsec_node b = parsec_select_first(doc, sel, parsec_root());
    CHECK(parsec_node_span(doc, b, &start, &end) && start == 4 && end == 11);
    CHECK(memcmp(html + start, "<b x=1>", 7) == 0);
    CHECK(parsec_attr_span(doc, b, 0, &start, &end) && start == 7 && end == 10);
    CHECK(parsec_end_span(doc, b, &start, &end) && start == 12 && end == 16);
    CHECK(parsec_errors_count(doc) > 0);
    for (size_t i = 0; i < parsec_errors_count(doc); i++) {
        parsec_syntax_error e;
        CHECK(parsec_errors_get(doc, i, &e) == PARSEC_OK && e.offset <= len);
    }
    /* The stray </p> is the last error, at its own bytes. */
    parsec_syntax_error last;
    CHECK(parsec_errors_get(doc, parsec_errors_count(doc) - 1, &last) == PARSEC_OK);
    CHECK(last.offset == 16 && last.line == 1);
    CHECK(parsec_line_col(doc, 4, &line, &column) == PARSEC_OK && column == 5);
    /* An offset past the end is past the end, not an overflow. */
    CHECK(parsec_line_col(doc, (size_t)-1, &line, &column) == PARSEC_OK);
    parsec_document_free(doc);

    /* A fragment keeps a byte order mark as text, and its bytes count. */
    const char frag[] = "\xef\xbb\xbf\xfe<i>x</i>";
    CHECK(parsec_parse_fragment(frag, sizeof frag - 1, PARSEC_NS_HTML, "div", 3, PARSEC_SPANS,
                                &doc) == PARSEC_OK);
    parsec_selector *i = compile("i");
    CHECK(parsec_node_span(doc, parsec_select_first(doc, i, parsec_root()), &start, &end));
    CHECK(start == 4 && end == 7);
    parsec_selector_free(i);
    parsec_document_free(doc);

    /* And a document drops it, but its bytes still count. */
    const char page[] = "\xef\xbb\xbf\xfe<b>x</b>";
    CHECK(parsec_parse(page, sizeof page - 1, PARSEC_SPANS, &doc) == PARSEC_OK);
    CHECK(parsec_node_span(doc, parsec_select_first(doc, sel, parsec_root()), &start, &end));
    CHECK(start == 4 && end == 7);
    parsec_document_free(doc);

    /* So does a page decoded from bytes. */
    const char *enc = NULL;
    CHECK(parsec_parse_bytes(page, sizeof page - 1, NULL, 0, PARSEC_SPANS, &doc, &enc) == PARSEC_OK);
    CHECK(parsec_line_col(doc, (size_t)-1, &line, &column) == PARSEC_OK);
    parsec_selector_free(sel);
    parsec_document_free(doc);
}

struct in_callback {
    parsec_document *doc;
    int calls;
    parsec_status edit;
    parsec_node created;
    int free_it;
};

static int edit_from_callback(parsec_node node, void *ctx) {
    struct in_callback *c = ctx;
    c->calls++;
    c->edit = parsec_remove(c->doc, node);
    c->created = parsec_create_text(c->doc, "x", 1);
    size_t len = 0;
    /* Reading is fine. */
    CHECK(is(parsec_node_name(c->doc, node, &len), &len, "p"));
    if (c->free_it) {
        parsec_document_free(c->doc);
        return 1;
    }
    return 0;
}

static void select_callbacks_cant_edit(void) {
    const char *html = "<p>a<p>b";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    parsec_selector *p = compile("p");
    struct in_callback c = {doc, 0, PARSEC_OK, 0, 0};
    CHECK(parsec_select(doc, p, parsec_root(), edit_from_callback, &c) == PARSEC_OK);
    CHECK(c.calls == 2 && c.edit == PARSEC_ERR_BUSY && c.created == PARSEC_NO_NODE);
    /* Once it's returned, edits work again. */
    parsec_node first = parsec_select_first(doc, p, parsec_root());
    CHECK(parsec_remove(doc, first) == PARSEC_OK);
    /* Freed from the callback, it's freed when parsec_select returns:
     * under the sanitizers, a use after free or a leak fails here. */
    c = (struct in_callback){doc, 0, PARSEC_OK, 0, 1};
    CHECK(parsec_select(doc, p, parsec_root(), edit_from_callback, &c) == PARSEC_OK);
    CHECK(c.calls == 1);
    CHECK(strcmp(parsec_status_message(PARSEC_ERR_BUSY), "unknown status") != 0);
    parsec_selector_free(p);
}

#if PARSEC_VERSION_MAJOR != 0 || !defined(PARSEC_VERSION_MINOR) || !defined(PARSEC_VERSION_PATCH)
#error "parsec.h should define PARSEC_VERSION_MAJOR, _MINOR and _PATCH"
#endif

static void syntax_errors(void) {
    const char *html = "<p>a\n</b>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), PARSEC_ERRORS, &doc) == PARSEC_OK);
    CHECK(parsec_errors_count(doc) == 2);
    parsec_syntax_error e;
    CHECK(parsec_errors_get(doc, 1, &e) == PARSEC_OK);
    CHECK(is(e.code, &e.code_len, "unexpected-end-tag"));
    CHECK(e.offset == 5 && e.line == 2 && e.column == 1);
    CHECK(parsec_errors_get(doc, 2, &e) == PARSEC_ERR_INVALID);
    CHECK(parsec_errors_get(doc, 0, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_errors_get(NULL, 0, &e) == PARSEC_ERR_NULL);
    CHECK(parsec_errors_count(NULL) == 0);
    parsec_document_free(doc);
    /* Kept only when asked for. */
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    CHECK(parsec_errors_count(doc) == 0);
    parsec_document_free(doc);
}

static parsec_xpath_result *eval(const parsec_document *doc, parsec_node context,
                                 const char *expr) {
    parsec_xpath *x = NULL;
    parsec_xpath_result *r = NULL;
    CHECK(parsec_xpath_compile(expr, strlen(expr), &x, NULL) == PARSEC_OK);
    CHECK(parsec_xpath_evaluate(doc, x, context, &r, NULL) == PARSEC_OK);
    parsec_xpath_free(x);
    return r;
}

static void xpath(void) {
    const char *html = "<ul id=list><li class=a>one<li>two<li class=a>three</ul>";
    parsec_document *doc = NULL;
    CHECK(parsec_parse(html, strlen(html), 0, &doc) == PARSEC_OK);
    size_t len = 0;
    parsec_node node = 0;
    uint32_t attr = 0;

    parsec_xpath_result *r = eval(doc, parsec_root(), "//li[@class='a']");
    CHECK(parsec_xpath_result_type(r) == PARSEC_XPATH_NODES);
    CHECK(parsec_xpath_result_count(r) == 2);
    CHECK(parsec_xpath_result_item(r, 1, &node, &attr) == PARSEC_OK && attr == PARSEC_NO_ATTR);
    CHECK(owned_is(parsec_text(doc, node, &len), &len, "three"));
    CHECK(parsec_xpath_result_item(r, 2, &node, &attr) == PARSEC_ERR_INVALID);
    CHECK(parsec_xpath_result_string(r, &len) == NULL && len == 0);
    parsec_xpath_result_free(r);

    /* An attribute is its element and its place there. */
    r = eval(doc, parsec_root(), "//@id");
    CHECK(parsec_xpath_result_item(r, 0, &node, &attr) == PARSEC_OK && attr == 0);
    const char *value = NULL;
    size_t value_len = 0;
    CHECK(parsec_attr_at(doc, node, attr, NULL, NULL, &value, &value_len) == PARSEC_OK);
    CHECK(is(value, &value_len, "list"));
    parsec_xpath_result_free(r);

    r = eval(doc, parsec_root(), "count(//li)");
    CHECK(parsec_xpath_result_type(r) == PARSEC_XPATH_NUMBER);
    CHECK(parsec_xpath_result_number(r) == 3.0);
    CHECK(parsec_xpath_result_count(r) == 0 && !parsec_xpath_result_boolean(r));
    parsec_xpath_result_free(r);
    r = eval(doc, parsec_root(), "concat(//li[2], '!')");
    CHECK(parsec_xpath_result_type(r) == PARSEC_XPATH_STRING);
    CHECK(is(parsec_xpath_result_string(r, &len), &len, "two!"));
    CHECK(parsec_xpath_result_number(r) != parsec_xpath_result_number(r)); /* NaN */
    parsec_xpath_result_free(r);

    /* From a node, and again after a change, which the next query sees. */
    parsec_selector *sel = compile("ul");
    parsec_node ul = parsec_select_first(doc, sel, parsec_root());
    r = eval(doc, ul, "boolean(li[3])");
    CHECK(parsec_xpath_result_type(r) == PARSEC_XPATH_BOOLEAN && parsec_xpath_result_boolean(r));
    parsec_xpath_result_free(r);
    parsec_node li = parsec_create_element(doc, PARSEC_NS_HTML, "li", 2);
    CHECK(parsec_append(doc, ul, li) == PARSEC_OK);
    r = eval(doc, parsec_root(), "count(//li)");
    CHECK(parsec_xpath_result_number(r) == 4.0);
    parsec_xpath_result_free(r);
    /* A detached subtree is a tree of its own. */
    CHECK(parsec_detach(doc, ul) == PARSEC_OK);
    r = eval(doc, parsec_root(), "count(//li)");
    CHECK(parsec_xpath_result_number(r) == 0.0);
    parsec_xpath_result_free(r);
    r = eval(doc, ul, "count(//li)");
    CHECK(parsec_xpath_result_number(r) == 4.0);
    parsec_xpath_result_free(r);
    /* A removed one can't be queried at all. */
    li = parsec_first_child(doc, ul);
    CHECK(parsec_remove(doc, ul) == PARSEC_OK);
    parsec_xpath *all = NULL;
    CHECK(parsec_xpath_compile("count(//li)", 11, &all, NULL) == PARSEC_OK);
    CHECK(parsec_xpath_evaluate(doc, all, ul, &r, NULL) == PARSEC_ERR_REMOVED && r == NULL);
    CHECK(parsec_xpath_evaluate(doc, all, li, &r, NULL) == PARSEC_ERR_REMOVED && r == NULL);
    parsec_xpath_free(all);
    /* The result is a copy, so it outlives the document. */
    r = eval(doc, parsec_root(), "string(//nothing | /)");
    parsec_selector_free(sel);
    parsec_document_free(doc);
    CHECK(parsec_xpath_result_type(r) == PARSEC_XPATH_STRING);
    parsec_xpath_result_free(r);

    /* Errors. */
    parsec_xpath *x = NULL;
    char *message = NULL;
    CHECK(parsec_xpath_compile("//li[", 5, &x, &message) == PARSEC_ERR_XPATH);
    CHECK(x == NULL && message != NULL && strlen(message) > 0);
    parsec_string_free(message);
    CHECK(parsec_xpath_compile("$v", 2, &x, NULL) == PARSEC_OK);
    CHECK(parsec_parse("<p>", 3, 0, &doc) == PARSEC_OK);
    CHECK(parsec_xpath_evaluate(doc, x, parsec_root(), &r, &message) == PARSEC_ERR_XPATH);
    CHECK(r == NULL && message != NULL && strstr(message, "$v") != NULL);
    parsec_string_free(message);
    CHECK(parsec_xpath_evaluate(doc, x, PARSEC_NO_NODE, &r, NULL) == PARSEC_ERR_NO_SUCH_NODE);
    CHECK(parsec_xpath_evaluate(NULL, x, 0, &r, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_xpath_evaluate(doc, NULL, 0, &r, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_xpath_compile("x", 1, NULL, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_xpath_result_type(NULL) == 0 && parsec_xpath_result_count(NULL) == 0);
    CHECK(parsec_xpath_result_item(NULL, 0, &node, &attr) == PARSEC_ERR_NULL);
    CHECK(parsec_xpath_result_string(NULL, &len) == NULL);
    parsec_xpath_free(x);
    parsec_xpath_free(NULL);
    parsec_xpath_result_free(NULL);
    parsec_document_free(doc);
}

/* ASCII letters equal, whatever their case: an encoding label as
 * html5lib-tests writes it against Parsec's name for the encoding. */
static int same_label(const char *a, const char *b) {
    for (; *a && *b; a++, b++) {
        char x = *a >= 'A' && *a <= 'Z' ? *a + 32 : *a;
        char y = *b >= 'A' && *b <= 'Z' ? *b + 32 : *b;
        if (x != y) return 0;
    }
    return *a == *b;
}

static void bytes_and_encodings(void) {
    /* 日本 in Shift_JIS, named by a <meta>: 4 bytes for 6 decoded. */
    const char page[] = "<meta charset=shift_jis><p class=a>\x93\xfa\x96\x7b</p>\n<b>x</b></i>";
    const size_t n = sizeof page - 1;
    parsec_document *doc = NULL;
    const char *enc = NULL;
    size_t len = 0, start = 0, end = 0, line = 0, column = 0;
    CHECK(parsec_parse_bytes(page, n, NULL, 0, PARSEC_SPANS | PARSEC_ERRORS, &doc, &enc) == PARSEC_OK);
    CHECK(enc != NULL && strcmp(enc, "Shift_JIS") == 0);
    CHECK(parsec_document_encoding(doc) == enc);
    CHECK(owned_is(parsec_text(doc, parsec_root(), &len), &len, "\xe6\x97\xa5\xe6\x9c\xac\nx"));
    /* Spans and errors are offsets into the Shift_JIS bytes. */
    parsec_selector *sel = compile("p");
    parsec_node p = parsec_select_first(doc, sel, parsec_root());
    parsec_selector_free(sel);
    CHECK(parsec_node_span(doc, p, &start, &end) && start == 24 && end == 35);
    CHECK(parsec_attr_span(doc, p, 0, &start, &end) && start == 27 && end == 34);
    CHECK(parsec_node_span(doc, parsec_first_child(doc, p), &start, &end) && start == 35 && end == 39);
    CHECK(parsec_end_span(doc, p, &start, &end) && start == 39 && end == 43);
    sel = compile("b");
    parsec_node b = parsec_select_first(doc, sel, parsec_root());
    parsec_selector_free(sel);
    CHECK(parsec_node_span(doc, b, &start, &end) && start == 44 && end == 47);
    CHECK(parsec_line_col(doc, 52, &line, &column) == PARSEC_OK && line == 2 && column == 9);
    size_t stray = 0;
    for (size_t i = 0; i < parsec_errors_count(doc); i++) {
        parsec_syntax_error e;
        CHECK(parsec_errors_get(doc, i, &e) == PARSEC_OK);
        if (e.code_len == 18 && memcmp(e.code, "unexpected-end-tag", 18) == 0) {
            CHECK(e.offset == 52 && e.line == 2 && e.column == 9);
            stray++;
        }
    }
    CHECK(stray == 1);
    parsec_document_free(doc);

    /* The transport layer's encoding beats the guess, and a label is any
     * of the Encoding Standard's. */
    CHECK(parsec_parse_bytes("<p>\xc3\xa9", 5, "UTF8", 4, 0, &doc, &enc) == PARSEC_OK);
    CHECK(strcmp(enc, "UTF-8") == 0);
    CHECK(owned_is(parsec_text(doc, parsec_root(), &len), &len, "\xc3\xa9"));
    parsec_document_free(doc);
    /* With nothing to go on, valid UTF-8 is taken for UTF-8, and other
     * bytes for windows-1252, as in browsers. */
    CHECK(parsec_parse_bytes("<p>\xc3\xa9", 5, NULL, 0, 0, &doc, NULL) == PARSEC_OK);
    CHECK(strcmp(parsec_document_encoding(doc), "UTF-8") == 0);
    parsec_document_free(doc);
    CHECK(parsec_parse_bytes("<p>\xe9", 4, NULL, 0, 0, &doc, NULL) == PARSEC_OK);
    CHECK(strcmp(parsec_document_encoding(doc), "windows-1252") == 0);
    CHECK(owned_is(parsec_text(doc, parsec_root(), &len), &len, "\xc3\xa9"));
    parsec_document_free(doc);
    /* A label that names no encoding is passed over, as browsers do. */
    CHECK(parsec_parse_bytes("<p>\xe9", 4, "nope", 4, 0, &doc, &enc) == PARSEC_OK);
    CHECK(strcmp(enc, "windows-1252") == 0);
    parsec_document_free(doc);
    CHECK(parsec_parse_bytes(NULL, 1, NULL, 0, 0, &doc, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_parse_bytes("<p>", 3, NULL, 0, 99, &doc, NULL) == PARSEC_ERR_INVALID);
    CHECK(parsec_parse_bytes("<p>", 3, NULL, 0, 0, NULL, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_parse("<p>", 3, 0, &doc) == PARSEC_OK);
    CHECK(parsec_document_encoding(doc) == NULL);
    CHECK(parsec_document_encoding(NULL) == NULL);
    parsec_document_free(doc);

    /* In pieces of 3 bytes, cutting the kanji: what the whole gives. */
    parsec_byte_parser *bp = NULL;
    CHECK(parsec_byte_parser_new(NULL, 0, PARSEC_SPANS, &bp) == PARSEC_OK && bp != NULL);
    for (size_t at = 0; at < n; at += 3)
        CHECK(parsec_byte_parser_feed(bp, page + at, n - at < 3 ? n - at : 3) == PARSEC_OK);
    CHECK(parsec_byte_parser_finish(bp, &doc) == PARSEC_OK);
    CHECK(strcmp(parsec_document_encoding(doc), "Shift_JIS") == 0);
    sel = compile("b");
    b = parsec_select_first(doc, sel, parsec_root());
    parsec_selector_free(sel);
    CHECK(parsec_node_span(doc, b, &start, &end) && start == 44 && end == 47);
    parsec_document_free(doc);
    /* Finished once: then only freeing it works. */
    CHECK(parsec_byte_parser_feed(bp, "x", 1) == PARSEC_ERR_INVALID);
    CHECK(parsec_byte_parser_finish(bp, &doc) == PARSEC_ERR_INVALID && doc == NULL);
    parsec_byte_parser_free(bp);
    /* One left unfinished frees what it held. */
    CHECK(parsec_byte_parser_new("sjis", 4, 0, &bp) == PARSEC_OK);
    CHECK(parsec_byte_parser_feed(bp, page, n) == PARSEC_OK);
    parsec_byte_parser_free(bp);
    CHECK(parsec_byte_parser_new(NULL, 0, 99, &bp) == PARSEC_ERR_INVALID && bp == NULL);
    CHECK(parsec_byte_parser_new(NULL, 0, 0, NULL) == PARSEC_ERR_NULL);
    CHECK(parsec_byte_parser_feed(NULL, "x", 1) == PARSEC_ERR_NULL);
    CHECK(parsec_byte_parser_finish(NULL, &doc) == PARSEC_ERR_NULL);
    parsec_byte_parser_free(NULL);
}

/* A whole file, NUL-terminated, or NULL. */
static char *slurp(const char *dir, const char *name, size_t *len) {
    char path[4096];
    snprintf(path, sizeof path, "%s/%s", dir, name);
    FILE *f = fopen(path, "rb");
    if (f == NULL) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *data = malloc((size_t)size + 1);
    *len = data != NULL ? fread(data, 1, (size_t)size, f) : 0;
    if (data != NULL) data[*len] = '\0';
    fclose(f);
    return data;
}

/* Where `needle` first is in `n` bytes at `s`, or NULL. */
static const char *find(const char *s, size_t n, const char *needle) {
    size_t m = strlen(needle);
    for (size_t i = 0; i + m <= n; i++)
        if (memcmp(s + i, needle, m) == 0) return s + i;
    return NULL;
}

/* html5lib-tests' encoding cases, whole and in pieces of 1 to 7 bytes,
 * from `dir`, the suite's encoding/ folder. The scripted case needs
 * document.write, so it isn't here. */
static void html5lib_encoding(const char *dir) {
    const char *files[] = {"tests1.dat", "tests2.dat", "test-yahoo-jp.dat"};
    size_t cases = 0, passed = 0;
    for (int f = 0; f < 3; f++) {
        size_t size = 0;
        char *data = slurp(dir, files[f], &size);
        CHECK(data != NULL);
        if (data == NULL) continue;
        const char *rest = data, *stop = data + size;
        const char *at;
        while ((at = find(rest, (size_t)(stop - rest), "#data\n")) != NULL) {
            const char *page = at + 6;
            const char *tail = find(page, (size_t)(stop - page), "\n#encoding\n");
            CHECK(tail != NULL);
            if (tail == NULL) break;
            const char *label = tail + 11;
            char want[64] = {0};
            size_t w = 0;
            while (w < sizeof want - 1 && label + w < stop && label[w] != '\n') {
                want[w] = label[w];
                w++;
            }
            size_t n = (size_t)(tail - page);
            parsec_document *doc = NULL;
            const char *whole = NULL;
            CHECK(parsec_parse_bytes(page, n, NULL, 0, 0, &doc, &whole) == PARSEC_OK);
            int ok = whole != NULL && same_label(whole, want);
            parsec_document_free(doc);
            parsec_byte_parser *bp = NULL;
            CHECK(parsec_byte_parser_new(NULL, 0, 0, &bp) == PARSEC_OK);
            for (size_t i = 0, piece = 1 + cases % 7; i < n; i += piece, piece = 1 + (piece + cases) % 7)
                CHECK(parsec_byte_parser_feed(bp, page + i, n - i < piece ? n - i : piece) == PARSEC_OK);
            CHECK(parsec_byte_parser_finish(bp, &doc) == PARSEC_OK);
            const char *streamed = parsec_document_encoding(doc);
            ok = ok && streamed != NULL && same_label(streamed, want);
            if (!ok) fprintf(stderr, "html5lib %s case %zu: want %s\n", files[f], cases, want);
            parsec_document_free(doc);
            parsec_byte_parser_free(bp);
            passed += (size_t)ok;
            cases++;
            rest = tail;
        }
        free(data);
    }
    CHECK(cases == 82 && passed == cases);
    printf("html5lib encoding: %zu of %zu passed, whole and in pieces\n", passed, cases);
}

int main(void) {
    parse_select_read();
    edit_and_serialize();
    removed_and_detached();
    fragments_and_options();
    errors_and_null();
    positions();
    positions_in_bytes_that_arent_utf8();
    select_callbacks_cant_edit();
    syntax_errors();
    xpath();
    bytes_and_encodings();
    /* tests/c.rs and the sanitizer job build this with the html5lib-tests
     * encoding folder defined; the CMake runs, from a package, have none. */
#ifdef HTML5LIB_ENCODING
    html5lib_encoding(HTML5LIB_ENCODING);
#else
    printf("html5lib encoding: not run, as no folder was given\n");
#endif
    if (failures) {
        fprintf(stderr, "%d check(s) failed\n", failures);
        return 1;
    }
    CHECK(strcmp(parsec_version(), PARSEC_VERSION) == 0);
    printf("parsec %s: every C API check passed\n", parsec_version());
    return 0;
}
