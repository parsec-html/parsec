# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway

import sys
import sysconfig
import threading

import pytest

from parsec_html import HTMLParser, Node, SelectorError

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


@pytest.mark.skipif(
    not sysconfig.get_config_var("Py_GIL_DISABLED"), reason="needs a free-threaded Python"
)
def test_free_threaded_python_keeps_the_gil_off():
    assert not sys._is_gil_enabled()


def test_node_is_not_constructible():
    with pytest.raises(TypeError):
        Node()
