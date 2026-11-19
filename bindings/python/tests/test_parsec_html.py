# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway

import sys
import sysconfig
import threading

import pytest

from parsec_html import HTMLParser, Node, ParseError, SelectorError, XPathError

PAGE = (
    "<!doctype html><title>T</title>"
    "<div id=a class='x y' hidden>one <b>two</b> three<!--c--></div><p>p1</p>"
)


def test_document_parts():
    p = HTMLParser(PAGE)
    assert p.root.tag == "html"
    assert p.head.tag == "head"
    assert p.body.tag == "body"
    assert p.root.parent.tag == "-document"
    assert p.html.startswith("<!DOCTYPE html><html><head><title>T</title>")


def test_text_options():
    p = HTMLParser(PAGE)
    assert p.text() == "Tone two threep1"
    assert p.text(separator="|") == "T|one |two| three|p1"
    d = p.css_first("div")
    assert d.text(deep=False, separator="|") == "one | three"
    assert d.text(strip=True, separator="|") == "one|two|three"
    q = HTMLParser("<div> a <span>  </span> b</div>").css_first("div")
    # strip keeps the emptied piece; skip_empty drops whitespace-only text.
    assert q.text(strip=True, separator="|") == "a||b"
    assert q.text(skip_empty=True, separator="|") == " a | b"


def test_css_and_css_first():
    p = HTMLParser(PAGE)
    assert [n.tag for n in p.css("div, p")] == ["div", "p"]
    assert p.css_first("nope") is None
    assert p.css_first("nope", default="D") == "D"
    with pytest.raises(ValueError, match="Expected 1 match, but found 2 matches"):
        p.css_first("div, p", strict=True)
    d = p.css_first("#a")
    # A node's query counts the node itself, as selectolax's does.
    assert d.css("div") == [d]
    assert d.css_first("b").tag == "b"
    assert d.css_matches("b") and not d.css_matches("p")
    assert p.any_css_matches(("nope", "p"))
    assert [n.tag for n in p.tags("P")] == ["p"]
    with pytest.raises(SelectorError):
        p.css("p[")
    assert issubclass(SelectorError, ValueError)


def test_attributes():
    d = HTMLParser(PAGE).css_first("div")
    assert d.attributes == {"id": "a", "class": "x y", "hidden": ""}
    assert d.id == "a"
    a = d.attrs
    assert len(a) == 3 and "class" in a and a["class"] == "x y"
    assert list(a) == ["id", "class", "hidden"]
    assert a.get("nope") is None and a.get("nope", 1) == 1 and a.sget("nope") == ""
    a["data-x"] = "1"
    a["ID"] = "b"
    del a["hidden"]
    assert d.html.startswith('<div id="b" class="x y" data-x="1">')
    assert a.items() == [("id", "b"), ("class", "x y"), ("data-x", "1")]
    with pytest.raises(KeyError):
        del a["hidden"]
    with pytest.raises(KeyError):
        a["hidden"]
    svg = HTMLParser("<svg viewBox=0><a xlink:href=x></a></svg>").css_first("svg")
    assert svg.attributes == {"viewBox": "0"}
    assert svg.css_first("a").attributes == {"xlink:href": "x"}


def test_navigation_and_iteration():
    d = HTMLParser(PAGE).css_first("div")
    assert [(n.tag, n.text_content) for n in d.iter(include_text=True)] == [
        ("-text", "one "),
        ("b", None),
        ("-text", " three"),
        ("-comment", None),
    ]
    assert [n.tag for n in d.iter()] == ["b"]
    assert [n.tag for n in d.traverse(include_text=True)] == [
        "div", "-text", "b", "-text", "-text", "-comment",
    ]
    assert d.next.tag == "p" and d.prev is None
    assert d.first_child.is_text_node and d.child == d.first_child
    assert d.last_child.is_comment_node and d.last_child.comment_content == "c"
    assert d.first_child.next.parent == d
    assert d == d.parser.css_first("#a") and hash(d) == hash(d.parser.css_first("#a"))
    assert d != HTMLParser(PAGE).css_first("#a")
    assert repr(d) == "<Node div>"
    ws = HTMLParser("<div> <b></b></div>").css_first("div")
    assert ws.first_child.is_empty_text_node
    assert [n.tag for n in ws.iter(include_text=True, skip_empty=True)] == ["b"]


def test_inserting_text_and_copies():
    p = HTMLParser(PAGE)
    d = p.css_first("div")
    d.css_first("b").replace_with("<i>x</i>")
    assert d.html == '<div id="a" class="x y" hidden="">one &lt;i&gt;x&lt;/i&gt; three<!--c--></div>'
    # Without move, a node goes in as a copy.
    d.insert_before(p.css_first("p"))
    assert p.body.html.count("<p>p1</p>") == 2
    other = HTMLParser("<span title=t>s<!--k--></span>").css_first("span")
    d.insert_child(other)
    assert d.html.endswith('<span title="t">s<!--k--></span></div>')
    assert other.parent.tag == "body"
    d.insert_after(b"tail")
    assert p.body.html.endswith("</div>tail<p>p1</p></body>")


def test_moving():
    p = HTMLParser("<p>a</p><p>b</p>")
    a, b = p.css("p")
    a.insert_before(b, move=True)
    assert p.body.html == "<body><p>b</p><p>a</p></body>"
    a.replace_with(a, move=True)
    assert p.body.html == "<body><p>b</p><p>a</p></body>"
    q = HTMLParser("<i>x</i>")
    a.insert_child(q.css_first("i"), move=True)
    assert a.html == "<p>a<i>x</i></p>" and q.css_first("i") is None
    with pytest.raises(ValueError):
        a.insert_child(p.body, move=True)


def test_inner_html_and_templates():
    p = HTMLParser("<div>old</div><table></table><template><b>t</b></template>")
    d = p.css_first("div")
    d.inner_html = "<u>new</u> & more"
    assert d.html == "<div><u>new</u> &amp; more</div>"
    t = p.css_first("table")
    t.inner_html = "<tr><td>1"
    assert t.html == "<table><tbody><tr><td>1</td></tr></tbody></table>"
    tpl = p.css_first("template")
    assert p.css("b") == []
    d.insert_child(tpl)
    assert d.css_first("template").html == "<template><b>t</b></template>"
    tpl.inner_html = "<i>j</i>"
    assert tpl.html == "<template><i>j</i></template>"


def test_removing_and_unwrapping():
    p = HTMLParser("<div><span>a<i>b</i></span><em></em></div><p>1</p><p>2</p>")
    p.css_first("span").unwrap()
    p.css_first("em").unwrap()
    assert p.css_first("div").html == "<div>a<i>b</i><em></em></div>"
    p.css_first("em").unwrap(delete_empty=True)
    assert p.css_first("div").html == "<div>a<i>b</i></div>"
    first = p.css_first("p")
    first.decompose()
    assert p.body.html == "<body><div>a<i>b</i></div><p>2</p></body>"
    # A removed node can still be read and put back.
    assert first.html == "<p>1</p>"
    p.body.insert_child(first, move=True)
    p.strip_tags(["i"])
    p.unwrap_tags(["div"])
    assert p.body.html == "<body>a<p>2</p><p>1</p></body>"


def test_merge_text_nodes():
    p = HTMLParser("<div>a</div>")
    d = p.css_first("div")
    d.insert_child("b")
    d.insert_child("c")
    assert len(list(d.iter(include_text=True))) == 3
    d.merge_text_nodes()
    assert [n.text_content for n in d.iter(include_text=True)] == ["abc"]


def test_fragments_and_bytes():
    f = HTMLParser("<li>a</li><li>b</li>", is_fragment=True)
    assert f.root.tag == "li" and f.root.parent is None and f.body is None
    assert f.html == "<li>a</li><li>b</li>"
    assert HTMLParser("", is_fragment=True).root is None
    rows = HTMLParser("<tr><td>x", is_fragment=True, fragment_tag="table")
    assert rows.html == "<tbody><tr><td>x</td></tr></tbody>"
    svg = HTMLParser("<circle/>", is_fragment=True, fragment_tag="svg", fragment_namespace="svg")
    assert svg.html == "<circle></circle>"
    assert HTMLParser(b"<p>\xff</p>").body.html == "<body><p>�</p></body>"
    with pytest.raises(TypeError):
        HTMLParser(1)
    with pytest.raises(ValueError):
        HTMLParser("", is_fragment=True, fragment_namespace="xml")


def test_limits_and_url():
    deep = HTMLParser("<div>" * 100, max_depth=10)
    n, depth = deep.css("div")[-1], 0
    while n.parent is not None:
        n, depth = n.parent, depth + 1
    # At most 10 elements above it, then the document.
    assert len(deep.css("div")) == 100 and depth == 11
    with pytest.raises(TimeoutError):
        HTMLParser("<b>" * 200_000, budget=0.0)
    page = HTMLParser("<a href='#s'>s</a><p id=s>x</p>", url="https://e.com/#s")
    assert page.css_first(":target").tag == "p"
    assert page.css_first(":local-link").tag == "a"


def test_one_document_shared_by_threads():
    p = HTMLParser("<ul>" + "<li class=i>x</li>" * 2000 + "</ul>")
    counts = []

    def read():
        for _ in range(50):
            counts.append(len(p.css("li.i")))

    def write():
        ul = p.css_first("ul")
        for i in range(200):
            ul.attrs["data-n"] = str(i)
            ul.insert_child("t")

    threads = [threading.Thread(target=read) for _ in range(8)] + [threading.Thread(target=write)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert counts == [2000] * 400
    assert p.css_first("ul").attrs["data-n"] == "199"


def test_positions():
    src = "<p class=x id=a>h\u00e9llo</p>\n<div>x<br></div>"
    p = HTMLParser(src, positions=True)
    para = p.css_first("p")
    start, end = para.source_range
    assert src[start:end] == "<p class=x id=a>"
    assert src[slice(*para.end_tag_range)] == "</p>"
    assert src[slice(*para.attribute_range("ID"))] == "id=a"
    assert para.attribute_range("nope") is None
    assert src[slice(*para.first_child.source_range)] == "h\u00e9llo"
    div = p.css_first("div")
    assert div.position == (2, 1)
    assert p.css_first("br").end_tag_range is None
    assert p.line_col(len(src)) == (2, 17)
    # Implied by the parser, so written nowhere.
    assert p.body.source_range is None and p.body.position is None
    # Off unless asked, and nothing an edit makes has one.
    assert HTMLParser(src).css_first("p").source_range is None
    para.insert_child("new")
    assert para.last_child.source_range is None


WIDE = "<p>caf\u00e9</p><b>\u65e5\u672c</b><i title='\U0001f600'>\U0001f600x</i>"


def test_positions_slice_what_was_passed_in():
    # A str gives code point indexes.
    p = HTMLParser(WIDE, positions=True)
    for tag, start, end in [("p", "<p>", "</p>"), ("b", "<b>", "</b>"), ("i", "<i title='\U0001f600'>", "</i>")]:
        n = p.css_first(tag)
        assert WIDE[slice(*n.source_range)] == start
        assert WIDE[slice(*n.end_tag_range)] == end
    i = p.css_first("i")
    assert WIDE[slice(*i.attribute_range("title"))] == "title='\U0001f600'"
    assert WIDE[slice(*i.first_child.source_range)] == "\U0001f600x"
    assert p.line_col(i.source_range[0]) == (1, i.source_range[0] + 1)
    # bytes give byte offsets into those bytes.
    data = WIDE.encode()
    q = HTMLParser(data, positions=True)
    assert data[slice(*q.css_first("b").source_range)] == b"<b>"
    assert data[slice(*q.css_first("i").first_child.source_range)] == "\U0001f600x".encode()
    # Even when they aren't all UTF-8: each bad byte is read as a U+FFFD,
    # three bytes long, but the offsets stay in the bytes given.
    bad = b"<p>\xff\xfe</p><b>x</b>"
    r = HTMLParser(bad, positions=True, errors=True)
    assert bad[slice(*r.css_first("b").source_range)] == b"<b>"
    assert bad[slice(*r.css_first("p").end_tag_range)] == b"</p>"
    assert r.line_col(bad.index(b"<b>")) == (1, 10)


def test_error_offsets_are_in_the_inputs_units():
    src = "\u65e5\u672c</b>"
    e = HTMLParser(src, errors=True).errors[1]
    assert e.code == "unexpected-end-tag" and src[e.offset:].startswith("</b>")
    assert e.column == 3


def test_errors():
    p = HTMLParser("<p>a\n</b><div x='1' x='2'>", errors=True)
    found = [(e.code, e.offset, e.line, e.column) for e in p.errors]
    assert found == [
        ("expected-doctype-but-got-start-tag", 0, 1, 1),
        ("unexpected-end-tag", 5, 2, 1),
        ("duplicate-attribute", 21, 2, 17),
        ("expected-closing-tag-but-got-eof", 26, 2, 22),
    ]
    assert isinstance(p.errors[0], ParseError)
    assert str(p.errors[1]) == "2:1: unexpected-end-tag"
    assert repr(p.errors[1]) == "<ParseError unexpected-end-tag at 2:1>"
    # Kept only when asked for.
    assert HTMLParser("</b>").errors == []
    assert HTMLParser("<!doctype html><p>ok</p>", errors=True).errors == []


def test_xpath_results_are_shaped_as_lxml_gives_them():
    p = HTMLParser(PAGE)
    assert [n.tag for n in p.xpath("//div | //p")] == ["div", "p"]
    assert p.xpath("//b/text()") == ["two"]
    assert p.xpath("//div/@class") == ["x y"]
    assert p.xpath("count(//*)") == 7.0
    assert p.xpath("string(//title)") == "T"
    assert p.xpath("boolean(//p)") is True
    assert p.xpath("//div/comment()")[0].comment_content == "c"
    assert p.xpath("//p[. = $want]", want="p1")[0].tag == "p"
    assert p.xpath("$n + 1", n=2) == 3.0 and p.xpath("$b", b=False) is False
    with pytest.raises(XPathError):
        p.xpath("//p[")
    with pytest.raises(XPathError, match=r"\$missing"):
        p.xpath("$missing")
    with pytest.raises(TypeError):
        p.xpath("$x", x=[1])


def test_xpath_from_a_node_and_after_edits():
    p = HTMLParser("<ul><li>a</li><li>b</li></ul><ol><li>c</li></ol>")
    ul = p.css_first("ul")
    assert ul.xpath("li/text()") == ["a", "b"]
    assert len(ul.xpath("//li")) == 3
    assert [li.text() for li in ul.xpath("following::li")] == ["c"]
    # Each edit drops the index, so the next query sees the change.
    ul.insert_child(p.create_node("li"))
    assert p.xpath("count(//ul/li)") == 3.0
    ul.decompose()
    assert p.xpath("count(//li)") == 1.0
    # A removed subtree can still be queried, as its own tree.
    assert ul.xpath("count(//li)") == 3.0
    assert p.css_first("li").xpath("ancestor::ol")[0].tag == "ol"


def test_xpath_from_many_threads_while_one_edits():
    p = HTMLParser("<ul>" + "<li class=i>x</li>" * 2000 + "</ul>")
    counts = []

    def read():
        for _ in range(30):
            counts.append(p.xpath("count(//li[@class='i'])"))

    def write():
        ul = p.css_first("ul")
        for _ in range(100):
            ul.insert_child("t")

    threads = [threading.Thread(target=read) for _ in range(8)] + [threading.Thread(target=write)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert counts == [2000.0] * 240
    assert len(p.xpath("//ul/text()")) == 100


@pytest.mark.skipif(
    not sysconfig.get_config_var("Py_GIL_DISABLED"), reason="needs a free-threaded Python"
)
def test_free_threaded_python_keeps_the_gil_off():
    assert not sys._is_gil_enabled()


def test_node_is_not_constructible():
    with pytest.raises(TypeError):
        Node()
