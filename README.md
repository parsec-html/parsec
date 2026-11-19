<h1 align="center">
  <img src="assets/parsec-header.svg" alt="Parsec" width="100%">
</h1>

<p align="center">
  <a href="https://www.bestpractices.dev/projects/16340"><img src="https://www.bestpractices.dev/projects/16340/badge" alt="OpenSSF Best Practices"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/parsec-html/parsec"><img src="https://api.scorecard.dev/projects/github.com/parsec-html/parsec/badge" alt="OpenSSF Scorecard"></a>
  <a href="https://github.com/parsec-html/parsec/actions/workflows/ci.yml"><img src="https://github.com/parsec-html/parsec/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

An HTML parser in Rust that reads pages the way a browser does, finds what
you're after with CSS selectors, and lets you change the tree and write it
back out.

If you use Lexbor today, directly or through selectolax, PHP's `Dom\HTMLDocument`,
nokolexbor or lazy_html, Parsec is being built for you. Lexbor is fast and
follows the spec. What we want to add is everything people keep asking it
for: positions on nodes, a worst case that stays linear, documents you can
share across threads, XPath, selectors that match everything a static page
can decide, and no use-after-free bugs, because the parser is memory-safe
Rust.

Parsec isn't released yet. This page says what works today and what's on
the way, and the line between the two is kept sharp.

## What works today

```rust
use parsec::css::SelectorList;

let doc = parsec::parse("<ul><li class=hit>one<li>two</ul>", false);
let hits = SelectorList::parse("li.hit").unwrap().select(&doc, doc.root());
assert_eq!(doc.text(hits[0]), "one");
```

- **The HTML Standard's tokenizer and tree builder.** It passes all 7,045
  html5lib tokenizer tests and every web-platform-tests tree-construction
  case that doesn't need JavaScript, run with scripting on and with it off.
  `cargo test` runs both suites; the WPT files are pinned in
  `crates/parsec/tests/tree-construction`.
- **Fragments** parsed in the context of any element, the way `innerHTML`
  is, SVG and MathML included.
- **CSS selectors, Level 4**: `:has()`, `:is()`, `:where()`, `:not()` with
  lists, `:nth-child(… of S)`, `:scope`, attribute flags, and the form states
  (`:checked`, `:disabled`, `:required`, `:valid`, `:in-range` and friends). `:lang()` and `:dir()`
  match the way the document decides them, and `:target` and `:local-link`
  match once you pass the page's URL, where Lexbor parses all four and never
  matches them.
- **A tree you can edit**: create, insert, move and remove nodes, set and
  rename attributes, then serialize as `innerHTML`, `outerHTML` or text.
- **Bytes in, with the encoding found the way browsers find it.**
  `parsec_encoding::parse_bytes` checks for a byte order mark, then the
  `charset` you got from HTTP if you pass one, then the HTML Standard's
  `<meta>` prescan of the first 1,024 bytes, then falls back to
  windows-1252 (or the default for a language you name). If the parser
  later meets a `<meta>` that disagrees with a guess, it decodes and parses
  once more in the declared encoding, as a browser reloads. You get the
  document, the encoding and how sure it is. It passes all 82 html5lib
  `encoding/` cases that don't need JavaScript.
- **Every encoding the web uses**, decoding and encoding, from
  [encoding_rs](https://github.com/hsivonen/encoding_rs), Firefox's own
  implementation of the WHATWG Encoding Standard. Against the
  web-platform-tests `encoding/` data it gets every one right: all 228
  labels, the 168 single-byte label tables, 395 ISO-2022-JP, GB18030 and
  GBK decoder cases, and 62,352 characters in the Shift_JIS, EUC-JP,
  ISO-2022-JP, EUC-KR and Big5 test pages. Lexbor, up to its latest
  release (3.0.1), gets 5,240 of Shift_JIS's 7,393 right: its decoder
  refuses lead bytes 0xE1 to 0xFB, so most of JIS level 2 kanji come out
  as U+FFFD (fixed on Lexbor's master in
  [lexbor#424](https://github.com/lexbor/lexbor/pull/424), not yet
  released).
- **Limits you choose**: a time budget per parse, a cap on input size, and
  a maximum tree depth, past which elements go beside the current one
  instead of inside it, as in Chromium.
  The slow-input cases in `examples/hostile.rs`, like thousands of nested
  formatting elements (the pattern that runs Lexbor out of memory), take
  time in proportion to their length.
- **A C API**: `libparsec`, shared and static, with a `parsec.h` that
  cbindgen generates, and pkg-config and CMake files. Handles are opaque,
  nodes are numbers that can't dangle, every call is NULL-safe and returns
  an error code instead of crashing, and documents can be read from many
  threads at once. Each release comes with a package for Linux, macOS and
  Windows, on x86-64 and ARM; Lexbor's own packages are x86-64 only. The
  C test runs on all three in CI, and under AddressSanitizer on Linux.
- **One dependency** (`memchr`) and one `unsafe` block in the parser, with
  its reasoning written beside it. The C API's `unsafe` is all at the
  boundary, where C's pointers come in.
- **WHATWG URL**, in `crates/parsec-url`: every getter and setter the
  `URL` interface has, `URLSearchParams`, IDNA and Punycode, built on
  Servo's `url` crate. `cargo test` runs the web-platform-tests URL suite,
  and 3,690 of its 4,565 cases pass; Lexbor publishes no count at all.
  Most of the 875 failures, 795 of them, are one IDNA rule the standard
  changed in June 2026 and the `url` crate hasn't caught up with yet. The
  wiki's [URL](https://github.com/parsec-html/parsec/wiki/URL) page names
  every failing case and why it fails.
- **A Python package** in `bindings/python`, with selectolax's API:
  `HTMLParser(html).css()`, `css_first`, `text()`, `attributes` and the
  edits, so moving over is mostly a change of import. One parsed page can
  be shared by many threads, and on free-threaded Python the GIL stays
  off. From Python, it parses the 13 pages below in 66 ms to
  selectolax's 106, and runs the 32 selectors in 283 ms to its 781.

Speed, timed against Lexbor's own C in one process: on 13 real pages,
24 MB of Wikipedia, MDN, GitHub, the BBC, Stack Overflow, the New York
Times, the HTML spec and others, Parsec parses 370 MB a second. Lexbor
manages 242 and html5ever 82. Parsec is ahead on every one of the 13, by
1.4 to 2.7 times, and holds less memory than Lexbor on each. On a
generated 10 MB shop listing it's twice as fast.

Selectors, too. Run 32 of them over those 13 pages and Parsec takes
226 ms in all; Lexbor takes 728, 3.2 times as long. Parsec is ahead on
every one, by 1.1 to 3.9 times, and by 31 times on `p:nth-child(2n+1)`,
which costs Lexbor 339 ms. Where the match counts differ, on `[href]`,
Lexbor is also counting SVG's `xlink:href`, which a selector with no
namespace shouldn't match.

Decoding, too. Bytes to UTF-8 text on the same machine, Parsec (encoding_rs)
turns Shift_JIS, windows-1251 and GBK versions of two real pages into text
at 8 to 10 GB a second. Lexbor's decoders plus its UTF-8 encoder manage 240
to 377 MB a second, so Parsec is 22 to 41 times faster. Those pages are
mostly ASCII markup. On a generated megabyte of dense kanji, Cyrillic or
hanzi the gap is 1.5 to 3.8 times. One caveat: Lexbor's decoders on their
own, writing code points that you still have to convert, reach 716 MB a
second on dense GBK, where Parsec reaches 501, so on that one Lexbor is
ahead until you count the conversion. A UTF-8 page Parsec only validates
and then parses where it is, at about 60 GB a second.

URLs as well. Over the 83,113 `href` and `src` values on those 13 pages,
each resolved against its page, Parsec takes 180 ns a URL and Lexbor 240,
so Parsec is 1.3 times as fast. Ada, the C++ parser in Node.js, takes
143 ns, 1.25 times as fast as Parsec. That gap is why a URL parser of
Parsec's own is next on the roadmap.

Hostile inputs, the ones built to make a parser crawl, go Parsec's way
on all eight. 80,000 unclosed `<div>`s take Parsec 3 ms and Lexbor 5.2
seconds. The closest, a dozen formatting tags reopened in every
paragraph, is 1.1 times; with 300 open `<b>` tags before 3,000
paragraphs, Parsec is 2.3 times as fast. Those two used to be Lexbor's
([#68](https://github.com/parsec-html/parsec/issues/68)).

`bench/` builds Lexbor 3.0.1 from source and runs all three on the same
strings, on an Apple M2 Max here, then times the selectors over the
same documents, and URLs against Lexbor and Ada. The wiki's
[Benchmarks](https://github.com/parsec-html/parsec/wiki/Benchmarks) page
has the method, every number and how to run it yourself.

## What's coming

The [roadmap](ROADMAP.md) has the order. In short, Parsec is set to cover
every module Lexbor has, and then go past it:

| | Lexbor | Parsec |
|---|---|---|
| HTML parsing, fragments | yes | yes |
| Bytes in, encoding sniffed | yes | yes, `parse_bytes` |
| Chunked input | yes | planned |
| Parse errors with line and column | pointers only | planned |
| Source positions on nodes and attributes | no | planned |
| Linear worst case | no ([#340](https://github.com/lexbor/lexbor/issues/340)) | planned |
| Thread-safe documents | no | planned, checked at compile time |
| `:scope`, `:lang()`, `:dir()`, `:target`, `:local-link` | parsed, never match | yes |
| `:valid`, `:invalid`, `:in-range`, `:out-of-range`, `:default`, `:indeterminate` | parsed, never match | yes |
| `:blank`, `:defined` | parsed, never match | planned |
| Queries that stop at the first match | no ([#32](https://github.com/lexbor/lexbor/issues/32)) | yes, `select_first` and `select_iter` |
| A tree depth limit | no | yes |
| XPath | no | planned |
| WHATWG Encoding, 40 encodings | yes | yes, through encoding_rs, with WPT pass counts published |
| WHATWG URL | yes, no WPT count published | yes, with WPT pass counts published |
| CSS parser and CSSOM | yes | planned, with `var()`, media queries and computed values |
| C API | yes, x86-64 packages | yes, with packages for Linux, macOS and Windows on x86-64 and ARM |
| Python | through selectolax | yes, free-threaded too |
| WebAssembly | through bindings | planned, built here |

Every "yes" in the Parsec column is something `cargo test` checks, or
for Python, pytest in CI. A
"planned" becomes a "yes" only when it ships.

## Using it

Parsec isn't on crates.io yet. To try it from git:

```toml
[dependencies]
parsec-html = { git = "https://github.com/parsec-html/parsec" }
```

The package is `parsec-html` and the crate you `use` is `parsec`. For
bytes in, add `parsec-html-encoding` the same way; its crate is
`parsec_encoding`:

```rust
let page = b"<meta charset=shift_jis><title>\x93\xfa\x96\x7b</title>";
let parsed = parsec_encoding::parse_bytes(page, Default::default()).unwrap();
assert_eq!(parsed.encoding.name(), "Shift_JIS");
assert_eq!(parsed.document.text(parsed.document.root()), "日本");
```

From Python, until the wheels go to PyPI with 0.1.0, build it from git
(Rust 1.88 or newer) and `import parsec_html`:

```sh
pip install "git+https://github.com/parsec-html/parsec#subdirectory=bindings/python"
```

From C, build the library and its header, pkg-config and CMake files
into a folder of your choice, then `find_package(parsec)` or
`pkg-config parsec`:

```sh
crates/parsec-capi/package.sh "$HOME/.local/parsec"
```

```c
#include <parsec.h>

parsec_document *doc;
parsec_parse(html, len, 0, &doc);
parsec_selector *sel;
parsec_selector_compile("li.hit", 6, &sel, NULL);
parsec_node hit = parsec_select_first(doc, sel, parsec_root());
char *text = parsec_text(doc, hit, NULL);
/* ... */
parsec_string_free(text);
parsec_selector_free(sel);
parsec_document_free(doc);
```

The wiki's [C API](https://github.com/parsec-html/parsec/wiki/C-API)
page has the rest.

The [wiki](https://github.com/parsec-html/parsec/wiki) is where the
documentation lives: how parsing works, every option, the selector list,
and a guide for moving from Lexbor, call by call, in C and Rust.

## Working on it

```sh
git clone --recurse-submodules https://github.com/parsec-html/parsec
cd parsec
cargo test
```

The parser core is shared with Netweir, so changes to `crates/parsec` start
there. [CONTRIBUTING.md](CONTRIBUTING.md) explains how that works and how
to get a change in. Questions go to
[Discussions](https://github.com/parsec-html/parsec/discussions), bugs to
[Issues](https://github.com/parsec-html/parsec/issues).

## Licence

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
