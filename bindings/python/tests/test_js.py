# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway

import threading
from pathlib import Path

import pytest

import parsec_html
from parsec_html import HTMLParser, js
from parsec_html.js import JSSyntaxError, page_state, parse

PAGES = Path(__file__).parents[3] / "crates" / "parsec-js" / "tests" / "pages"


def test_a_script_parses_to_estree_dicts():
    tree = parse("const total = items.reduce((sum, x) => sum + x, 0);")
    assert tree["type"] == "Program"
    assert tree["sourceType"] == "script"
    decl = tree["body"][0]
    assert decl["kind"] == "const"
    init = decl["declarations"][0]["init"]
    assert init["arguments"][0]["type"] == "ArrowFunctionExpression"
    assert js.parse is parse and parsec_html.js is js


def test_a_module_and_offsets_in_characters():
    src = 'import a from "é😀";\nexport default a;'
    tree = parse(src, module=True)
    assert tree["sourceType"] == "module"
    last = tree["body"][1]
    assert src[last["start"] : last["end"]] == "export default a;"
    with pytest.raises(JSSyntaxError):
        parse(src)


def test_a_syntax_error_says_where():
    with pytest.raises(JSSyntaxError) as info:
        parse("let ok = 1;\nlet x = ;")
    e = info.value
    assert isinstance(e, ValueError)
    assert (e.code, e.line, e.column, e.start, e.end) == ("unexpected-token", 2, 9, 20, 21)
    assert e.message == "`;` can't go here"
    assert str(e) == "2:9: `;` can't go here"
    assert e.frame.startswith("error[unexpected-token]: `;` can't go here\n --> 2:9\n")


def test_limits_raise_rather_than_crash():
    for deep in ["(" * 100_000, "[" * 100_000, "{" * 100_000, "-" * 100_000 + "x"]:
        with pytest.raises(JSSyntaxError) as info:
            parse(deep)
        assert info.value.code == "too-deep"
    with pytest.raises(JSSyntaxError):
        parse("((((((x))))))", max_depth=3)
    assert parse("((((((x))))))", max_depth=10)["body"]
    with pytest.raises(TimeoutError, match="max_steps"):
        parse("a + b;" * 1000, max_steps=100)
    assert parse("a + b;", max_steps=100)["body"]
    with pytest.raises(ValueError, match="max_depth"):
        parse("x", max_depth=100_000)


def test_page_state_of_a_script():
    found = page_state('window.__S = {user: {name: "Ada", admin: !1}, cart: [1, , 3]}; var q = f();')
    assert found == [
        {"name": "window.__S", "value": {"user": {"name": "Ada", "admin": False}, "cart": [1, None, 3]}},
        {"name": "q", "error": {"code": "not-a-literal", "source": "f()"}},
    ]
    with pytest.raises(JSSyntaxError):
        page_state("window.X = {")


def by_name(found, name):
    return next(f["value"] for f in found if f["name"] == name)


def test_page_state_of_real_pages():
    def read(name):
        return page_state((PAGES / name).read_text())

    nuxt2 = by_name(read("nuxt2-backmarket.js"), "window.__NUXT__")
    assert nuxt2["layout"] == "default"
    first = nuxt2["data"][0]["apiHome"]["top_categories"][0]
    assert first["title"] == "iPhone" == first["slug_title"]
    assert nuxt2["state"]["countryConfig"]["sales"][0]["indice"] == -10
    config = by_name(read("nuxt3-gitlab.js"), "window.__NUXT__.config")
    assert config["app"]["baseURL"] == "/"
    lowes = by_name(read("redux-lowes.js"), "window.__PRELOADED_STATE__")
    assert lowes["channel"] == "desktop"
    target = by_name(read("json-parse-target.js"), "window.__FLAGS__")
    assert target["GLOBAL_ADOBE_TAG_MANAGER_ENABLED"] is True
    youtube = by_name(read("var-youtube.js"), "ytInitialData")
    assert youtube["responseContext"]["serviceTrackingParams"][0]["service"] == "GFEEDBACK"
    pushes = [f["value"] for f in read("nextjs-app-router.js")]
    assert [p[0] for p in pushes] == [1, 1]
    assert pushes[0][1].startswith("1:\"$Sreact.fragment\"\n")


def test_page_state_of_a_page():
    scripts = "".join(
        f"<script>{(PAGES / name).read_text()}</script>"
        for name in ["nuxt3-gitlab.js", "var-youtube.js"]
    )
    page = (
        "<!doctype html><title>t</title>"
        + scripts
        + "<script type=application/json id=__NEXT_DATA__>{}</script>"
        + "<script src=a.js>window.ignored = 1</script>"
        + "<script>window.broken = {</script>"
        + "<script type=module>window.fromModule = [1]</script>"
        + "<template><script>window.inert = 1</script></template>"
    )
    found = HTMLParser(page).page_state()
    names = [f["name"] for f in found]
    assert names == ["window.__NUXT__", "window.__NUXT__.config", "ytInitialData", "window.fromModule"]
    assert by_name(found, "window.__NUXT__.config")["app"]["baseURL"] == "/"
    assert HTMLParser(b"<script>window.a = 1</script>").page_state() == [{"name": "window.a", "value": 1}]


def test_threads_parse_at_once():
    src = (PAGES / "nuxt2-backmarket.js").read_text()
    expected = parse(src)
    results = []

    def work():
        for _ in range(20):
            results.append(parse(src) == expected and page_state(src)[0]["name"] == "window.__NUXT__")

    threads = [threading.Thread(target=work) for _ in range(8)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(results) == 160 and all(results)
