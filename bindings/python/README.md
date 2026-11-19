# parsec-html for Python

Parsec is an HTML parser in Rust that reads pages the way a browser does,
finds what you want with CSS selectors, and lets you change the tree and
write it back out. This is its Python package, and if you've used
selectolax you already know the API:

```python
from parsec_html import HTMLParser

page = HTMLParser("<ul><li class=hit>one<li>two</ul>")
assert page.css_first("li.hit").text() == "one"
for li in page.css("li"):
    print(li.tag, li.attributes, li.html)
```

What you get over selectolax:

- **Threads.** One parsed page can be read from as many threads as you
  like, and edited from any of them. On free-threaded Python (3.14t
  and later) the module runs with the GIL off, and on any Python, parsing and
  `css()` let go of it while they work.
- **Selectors selectolax 1.0.2 won't run**: `:lang()`, `:dir()`,
  `:valid`, `:in-range`, `:scope` and more, which it rejects as
  unparseable, and `:target` and `:local-link` once you pass the page's
  `url`.
- **Limits** for pages you don't trust: `budget=` (seconds) and
  `max_depth=`.
- **Nodes that outlive edits.** A removed node can still be read, and put
  back.

## Installing

Parsec isn't released yet. Wheels go to PyPI as `parsec-html` with 0.1.0.
Until then, build it from git (you need Rust 1.88 or newer):

```sh
pip install "git+https://github.com/parsec-html/parsec#subdirectory=bindings/python"
```

## Moving from selectolax

Change the import, and most code runs as it was:

```python
# from selectolax.lexbor import LexborHTMLParser as HTMLParser
from parsec_html import HTMLParser
```

`HTMLParser(html, is_fragment=False, fragment_tag="div",
fragment_namespace="html", scripting=False)` takes what
`LexborHTMLParser` does, and the parser and its nodes have selectolax's
names: `root`, `head`, `body`, `html`, `css`, `css_first` (with `default`
and `strict`), `css_matches`, `any_css_matches`, `tags`, `text(deep,
separator, strip, skip_empty)`, `merge_text_nodes`, `strip_tags`,
`unwrap_tags`, `create_node`; on a node also `tag`, `id`, `attributes`,
`attrs`, `inner_html` (which you can set), `text_content`,
`comment_content`, `parent`, `next`, `prev`, `first_child`, `last_child`,
`iter`, `traverse`, `decompose`, `remove`, `unwrap`, `replace_with`,
`insert_before`, `insert_after`, `insert_child` (all four taking a string
as text, or a node to copy, or to move with `move=True`), and the
`is_*_node` checks.

Where it behaves differently:

| | selectolax 1.0.2 | parsec_html |
|---|---|---|
| `<a href>`'s `attributes` | `{'href': None}` | `{'href': ''}`: Parsec doesn't yet record whether a value was written |
| A selector that doesn't parse | `SelectolaxError` | `SelectorError`, a `ValueError` |
| A node after `decompose()` | freed | still readable, and can be inserted again |
| `attrs.items()`, `keys()`, `values()` | iterators | lists |
| `bytes` in | UTF-8, or sniffed with `encoding=True` | UTF-8, bad bytes replaced; sniffing comes with [#5](https://github.com/parsec-html/parsec/issues/5) |
| Class names | `LexborHTMLParser`, `LexborNode` | `HTMLParser`, `Node` |

Not there yet: `clone()`, `raw_html`, `raw_value`, `html_pretty`,
`select()` and its filters, `scripts_contain`, `script_srcs_contain`,
`template_fragments`, `mem_id`, `tag_id`, the `:lexbor-contains()`
pseudo-class, and source positions, which arrive with
[#8](https://github.com/parsec-html/parsec/issues/8).

## Speed

`bench.py` times both on the 13 real pages `bench/` uses (24 MB in all):

| | parsec_html | selectolax | |
|---|---:|---:|---:|
| Parse all 13 pages | 66 ms | 106 ms | 1.6× |
| The 32 selectors over all 13 | 283 ms | 781 ms | 2.8× |
| Every link's `href` and text | 43 ms | 50 ms | 1.2× |

Parsec parses each of the 13 pages faster, by 1.5 to 2.8 times, and runs
each of the 32 selectors faster, by 1.04 to 29 times
(`p:nth-child(2n+1)`). The narrowest margins are the selectors that
match most of a page, such as `*` and `[class]`, where the time goes on
making a Python object per match. Medians of 20 runs on an Apple M2 Max,
CPython 3.14, at a load of 3; a second run gave the same to within 6%.

## Working on it

```sh
cd bindings/python
pip install maturin pytest
maturin develop --release
pytest tests
python bench.py   # after bench/fetch-pages.sh and pip install selectolax
```

The [Python page](https://github.com/parsec-html/parsec/wiki/Python) on
the wiki has the whole API.
