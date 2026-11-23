# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway

import threading

import pytest

import parsec_html.xml as px
from parsec_html import ParseError
from parsec_html.xml import XMLDocument, XMLNode, XMLSyntaxError

ATOM = "http://www.w3.org/2005/Atom"
XLINK = "http://www.w3.org/1999/xlink"
FEED = (
    f'<feed xmlns="{ATOM}" xmlns:x="{XLINK}">'
    '<entry x:href="/one" n="1">Ships &amp; ports</entry>'
    "<?pi data?><!--c--><x:link><![CDATA[<raw>]]></x:link></feed>"
)

LAUGHS = (
    '<!DOCTYPE l [<!ENTITY a "lollollollollollollollollollol">'
    + "".join(
        f'<!ENTITY {n} "{("&" + p + ";") * 10}">' for p, n in zip("abcdefgh", "bcdefghi")
    )
    + "]><l>&i;</l>"
)


def test_a_namespaced_document_walks():
    doc = px.parse(FEED)
    assert isinstance(doc, XMLDocument)
    feed = doc.root
    assert isinstance(feed, XMLNode)
    assert (feed.kind, feed.tag, feed.local_name) == ("element", "feed", "feed")
    assert (feed.prefix, feed.namespace) == (None, ATOM)
    assert feed.parent == doc.document and doc.document.kind == "document"
    assert doc.document.parent is None
    entry, pi, comment, link = feed.children
    assert feed.first_child == entry and feed.last_child == link
    assert entry.next == pi and pi.prev == entry and link.next is None
    assert (pi.kind, pi.tag, pi.value) == ("processing-instruction", "pi", "data")
    assert (comment.kind, comment.value) == ("comment", "c")
    assert (link.tag, link.local_name, link.prefix, link.namespace) == (
        "x:link",
        "link",
        "x",
        XLINK,
    )
    (cdata,) = link.children
    assert (cdata.kind, cdata.value, cdata.tag) == ("cdata", "<raw>", None)
    assert entry.text() == "Ships & ports" and feed.text() == "Ships & ports<raw>"
    assert entry.first_child.kind == "text"
    assert repr(link) == "<XMLNode x:link>" and repr(doc) == "<XMLDocument feed>"
    assert len({entry, feed.first_child, pi}) == 2


def test_attributes_with_prefixes():
    feed = px.parse(FEED).root
    entry = feed.first_child
    assert entry.attributes == {"x:href": "/one", "n": "1"}
    assert feed.attributes == {"xmlns": ATOM, "xmlns:x": XLINK}
    href, n = entry.attribute_list
    assert (href.name, href.local_name, href.prefix, href.namespace, href.value) == (
        "x:href",
        "href",
        "x",
        XLINK,
        "/one",
    )
    assert href.specified
    assert (n.prefix, n.namespace) == (None, None)
    assert feed.attribute_list[1].namespace == "http://www.w3.org/2000/xmlns/"
    assert entry.attribute("href", XLINK) == "/one"
    assert entry.attribute("href") is None
    assert entry.attribute("n") == "1"
    assert entry.first_child.attributes == {}


def test_serializing_gives_back_what_was_parsed():
    doc = px.parse(FEED)
    assert doc.xml == FEED
    assert px.parse(doc.xml).xml == FEED
    # An element alone takes the declarations it's in the scope of.
    assert doc.root.last_child.xml == (
        f'<x:link xmlns="{ATOM}" xmlns:x="{XLINK}"><![CDATA[<raw>]]></x:link>'
    )
    decl = '<?xml version="1.0" standalone="yes"?>\n<a>é</a>'
    assert px.parse(decl).xml == decl


def test_a_well_formedness_error_has_its_code_and_place():
    with pytest.raises(XMLSyntaxError) as e:
        px.parse("<a>\n  <b></a>")
    err = e.value
    assert isinstance(err, ValueError)
    assert (err.code, err.line, err.column) == ("mismatched-end-tag", 2, 6)
    assert err.message == "`</a>` closes `<b>`"
    assert str(err) == "2:6: `</a>` closes `<b>`"
    with pytest.raises(XMLSyntaxError, match="unclosed|end"):
        px.parse(b"<a>")
    with pytest.raises(TypeError):
        px.parse(3)


def test_a_billion_laughs_is_an_error_not_a_hang():
    with pytest.raises(XMLSyntaxError) as e:
        px.parse(LAUGHS)
    assert e.value.code == "entity-expansion-limit"
    # The limits are the caller's.
    small = '<!DOCTYPE a [<!ENTITY e "12345">]><a>&e;&e;</a>'
    assert px.parse(small).root.text() == "1234512345"
    with pytest.raises(XMLSyntaxError) as e:
        px.parse(small, entity_allowance=0, entity_ratio=0)
    assert e.value.code == "entity-expansion-limit"
    with pytest.raises(XMLSyntaxError) as e:
        px.parse("<a><b><c/></b></a>", max_depth=2)
    assert e.value.code == "too-deep"
    nested = '<!DOCTYPE a [<!ENTITY x "1"><!ENTITY y "&x;">]><a>&y;</a>'
    with pytest.raises(XMLSyntaxError) as e:
        px.parse(nested, max_entity_depth=1)
    assert e.value.code == "entity-too-deep"


def test_bytes_in_the_encoding_they_declare():
    src = b"<?xml version='1.0' encoding='ISO-8859-1'?>\n<!DOCTYPE p SYSTEM 'p.dtd'><p>caf\xe9</p>"
    doc = px.parse(src)
    assert doc.encoding == "windows-1252"
    p = doc.root
    assert p.text() == "café"
    # Ranges are byte offsets into the bytes given.
    start, end = p.source_range
    assert src[start:end] == b"<p>caf\xe9</p>"
    assert p.position == (2, 28) == doc.line_col(start)
    assert p.prev.kind == "doctype" and p.prev.tag == "p"
    (w,) = doc.warnings
    assert isinstance(w, ParseError)
    assert (w.code, w.line) == ("external-dtd-not-loaded", 2)
    utf16 = "<?xml version='1.0' encoding='UTF-16'?><p>ü</p>".encode("utf-16")
    assert px.parse(utf16).root.text() == "ü"
    assert px.parse(utf16).encoding == "UTF-16LE"
    with pytest.raises(XMLSyntaxError) as e:
        px.parse(b"<p>\xff</p>")
    assert e.value.code == "invalid-byte-sequence"
    assert px.parse("<p/>").encoding is None


def test_ranges_in_a_str_count_characters():
    src = "<a>😀<b x='1'/></a>"
    b = px.parse(src).root.last_child
    start, end = b.source_range
    assert src[start:end] == "<b x='1'/>"
    assert b.position == (1, 5)


def test_namespaces_off():
    a = px.parse("<p:a xmlns:p='urn:p'/>", namespaces=False).root
    assert (a.tag, a.local_name, a.prefix, a.namespace) == ("p:a", "p:a", None, None)


def test_threads_read_one_document():
    doc = px.parse("<r>" + "<i n='1'>t</i>" * 2000 + "</r>")
    seen = []

    def read():
        seen.append(sum(1 for i in doc.root.children if i.attributes["n"] == "1"))

    threads = [threading.Thread(target=read) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert seen == [2000] * 8
