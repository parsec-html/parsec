# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway

"""bytes in: the encoding sniffed as a browser sniffs it, whole and
streamed, the html5lib-tests encoding cases through both, and offsets
that stay byte offsets into the bytes given, whatever their encoding."""

import os
from pathlib import Path

import pytest

from parsec_html import HTMLParser, StreamParser, encoding_for_locale

NIHON = b"\x93\xfa\x96\x7b"  # 日本 in Shift_JIS


def test_a_meta_decides():
    p = HTMLParser(b"<meta charset=shift_jis><p>" + NIHON)
    assert p.css_first("p").text() == "日本"
    assert (p.encoding, p.encoding_confidence) == ("Shift_JIS", "certain")


def test_nothing_named_is_windows_1252_and_tentative():
    p = HTMLParser(b"<p>caf\xe9")
    assert p.css_first("p").text() == "café"
    assert (p.encoding, p.encoding_confidence) == ("windows-1252", "tentative")


def test_valid_utf8_with_nothing_named_is_utf8():
    p = HTMLParser("<p>café 日本".encode())
    assert p.css_first("p").text() == "café 日本"
    assert (p.encoding, p.encoding_confidence) == ("UTF-8", "tentative")
    # ASCII says nothing either way.
    assert HTMLParser(b"<p>cafe").encoding == "windows-1252"
    # Past the first 1,024 bytes too, streamed or not.
    page = b"<p>" + b"x" * 2000 + "é".encode()
    s = StreamParser()
    for i in range(0, len(page), 100):
        s.feed(page[i : i + 100])
    assert s.close().encoding == HTMLParser(page).encoding == "UTF-8"


def test_transport_encoding_beats_meta_and_a_bom_beats_both():
    page = b"<meta charset=euc-jp><p>\xc3\xa9"
    p = HTMLParser(page, transport_encoding="utf-8")
    assert p.css_first("p").text() == "é"
    assert (p.encoding, p.encoding_confidence) == ("UTF-8", "certain")
    # Labels are the Encoding Standard's, any case, spaces trimmed.
    assert HTMLParser(page, transport_encoding=" SJIS ").encoding == "Shift_JIS"
    bom = HTMLParser(b"\xff\xfe<\0p\0>\0h\0i\0", transport_encoding="shift_jis")
    assert (bom.encoding, bom.text()) == ("UTF-16LE", "hi")


def test_default_encoding_and_locales():
    page = b"<p>" + NIHON
    assert HTMLParser(page).css_first("p").text() == "“ú–{"
    p = HTMLParser(page, default_encoding=encoding_for_locale("ja-JP"))
    assert p.css_first("p").text() == "日本"
    assert (p.encoding, p.encoding_confidence) == ("Shift_JIS", "tentative")
    assert encoding_for_locale("ru") == "windows-1251"
    assert encoding_for_locale("zh-Hant") == "Big5"
    assert encoding_for_locale("en-GB") == "windows-1252"


def test_a_late_meta_parses_again():
    page = b"<title>" + b"x" * 1100 + b"</title><meta charset=shift_jis><p>" + NIHON
    p = HTMLParser(page)
    assert (p.encoding, p.encoding_confidence) == ("Shift_JIS", "certain")
    assert p.css_first("p").text() == "日本"


def test_mistakes():
    # A transport layer's label that names no encoding is passed over, as
    # browsers pass over a bad charset; a default that's no label is a bug.
    assert HTMLParser(b"", transport_encoding="utf8mb4").encoding == "windows-1252"
    with pytest.raises(ValueError, match="default_encoding"):
        HTMLParser(b"", default_encoding="latin-9000")
    # As html5lib: a str is decoded already.
    with pytest.raises(TypeError):
        HTMLParser("<p>", transport_encoding="utf-8")
    s = HTMLParser("<p>")
    assert s.encoding is None and s.encoding_confidence is None


def test_fragments_sniff_too():
    f = HTMLParser(b"<li>" + NIHON, is_fragment=True, transport_encoding="sjis")
    assert f.root.text() == "日本" and f.encoding == "Shift_JIS"
    assert HTMLParser(b"<li>\xe9", is_fragment=True).root.text() == "é"


def test_stream_parser():
    page = b"<!doctype html><meta charset=sjis><p class=a>" + NIHON + b"</p>"
    s = StreamParser()
    # Cut inside the meta and inside the first kanji.
    for piece in (page[:20], page[20:47], page[47:]):
        s.feed(piece)
    p = s.close()
    assert isinstance(p, HTMLParser)
    assert p.html == HTMLParser(page).html
    assert p.css_first("p.a").text() == "日本"
    assert (p.encoding, p.encoding_confidence) == ("Shift_JIS", "certain")
    with pytest.raises(ValueError, match="closed"):
        s.feed(b"x")
    with pytest.raises(ValueError, match="closed"):
        s.close()
    t = StreamParser(transport_encoding="utf-8", url="https://e.com/#a", scripting=True)
    t.feed(b"<noscript><p>x</p></noscript><a href=#a id=a>\xc3")
    t.feed(b"\xa9</a>")
    q = t.close()
    assert q.css_first(":target").text() == "é"
    assert q.css_first("p") is None
    assert StreamParser().close().encoding == "windows-1252"
    with pytest.raises(TimeoutError):
        u = StreamParser(budget=0.0)
        u.feed(b"<b>" * 200_000)
        u.close()


# A Shift_JIS page, so every offset into the bytes differs from the same
# offset into the decoded text.
SJIS_PAGE = (
    "<!doctype html><meta charset=shift_jis><title>日本語</title>"
    "<p class='名前' id=x>こんにちは</p>\n<b>x</b></i>"
).encode("shift_jis")


@pytest.mark.parametrize("streamed", [False, True])
def test_shift_jis_positions(streamed):
    if streamed:
        s = StreamParser(positions=True, errors=True)
        for i in range(0, len(SJIS_PAGE), 5):
            s.feed(SJIS_PAGE[i : i + 5])
        p = s.close()
    else:
        p = HTMLParser(SJIS_PAGE, positions=True, errors=True)
    assert p.encoding == "Shift_JIS"
    page = SJIS_PAGE
    para = p.css_first("p")
    assert page[slice(*para.source_range)] == "<p class='名前' id=x>".encode("shift_jis")
    assert page[slice(*para.attribute_range("class"))] == "class='名前'".encode("shift_jis")
    assert page[slice(*para.attribute_range("id"))] == b"id=x"
    assert page[slice(*para.first_child.source_range)].decode("shift_jis") == "こんにちは"
    assert page[slice(*para.end_tag_range)] == b"</p>"
    title = p.css_first("title").first_child
    assert page[slice(*title.source_range)].decode("shift_jis") == "日本語"
    b = p.css_first("b")
    assert page[slice(*b.source_range)] == b"<b>"
    assert b.position == (2, 1)
    assert p.line_col(page.index(b"</i>")) == (2, 9)
    stray = [e for e in p.errors if e.code == "unexpected-end-tag"]
    assert len(stray) == 1 and page[stray[0].offset :].startswith(b"</i>")
    assert (stray[0].line, stray[0].column) == (2, 9)


def test_positions_behind_a_bom_and_in_utf16():
    utf8 = b"\xef\xbb\xbf<p>caf\xc3\xa9</p><b>x</b>"
    p = HTMLParser(utf8, positions=True)
    assert utf8[slice(*p.css_first("b").source_range)] == b"<b>"
    utf16 = "﻿<p>日本</p><b>x</b>".encode("utf-16-le")
    q = HTMLParser(utf16, positions=True)
    assert q.encoding == "UTF-16LE"
    assert utf16[slice(*q.css_first("b").source_range)].decode("utf-16-le") == "<b>"
    assert utf16[slice(*q.css_first("p").first_child.source_range)].decode("utf-16-le") == "日本"


HTML5LIB = Path(__file__).resolve().parents[3] / "crates/parsec/tests/html5lib-tests/encoding"


def html5lib_cases(name):
    data = (HTML5LIB / name).read_bytes()
    out = []
    for chunk in data.split(b"#data\n")[1:]:
        page, rest = chunk.split(b"\n#encoding\n", 1)
        out.append((page, rest.split(b"\n", 1)[0].decode()))
    return out


def test_html5lib_encoding_cases():
    if not HTML5LIB.is_dir():
        # CI checks the submodule out; from an unpacked sdist it's absent.
        if os.environ.get("CI"):
            pytest.fail(f"{HTML5LIB} is missing: git submodule update --init")
        pytest.skip("the html5lib-tests submodule isn't checked out")
    cases = [c for f in ("tests1.dat", "tests2.dat", "test-yahoo-jp.dat") for c in html5lib_cases(f)]
    # scripted/ writes its <meta> with document.write, which needs a script
    # engine: counted here, not run.
    scripted = len(html5lib_cases("scripted/tests1.dat"))
    failures = []
    for i, (page, want) in enumerate(cases):
        whole = HTMLParser(page).encoding
        s = StreamParser()
        # Pieces of 1 to 7 bytes, the cut moving with each case.
        at, size = 0, 1 + i % 7
        while at < len(page):
            s.feed(page[at : at + size])
            at, size = at + size, 1 + (size + i) % 7
        streamed = s.close().encoding
        if whole.lower() != want.lower() or streamed.lower() != want.lower():
            failures.append(f"case {i}: want {want}, got {whole} whole, {streamed} streamed")
    print(f"html5lib encoding: {len(cases) - len(failures)} passed, {len(failures)} failed, {scripted} skipped (scripted)")
    assert len(cases) == 82 and scripted == 1
    assert not failures, "\n".join(failures)
