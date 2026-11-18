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

It doesn't win everywhere. Of the eight hostile inputs, two go Lexbor's
way: 300 open `<b>` tags followed by 3,000 paragraphs, where Lexbor is
1.8 times faster, and a dozen formatting tags reopened in every
paragraph, where it's 1.2 times faster
([#68](https://github.com/parsec-html/parsec/issues/68)). The other way
round, 80,000 unclosed `<div>`s take Parsec 3 ms and Lexbor 5.2 seconds.

`bench/` builds Lexbor 3.0.1 from source and runs all three on the same
strings, on an Apple M2 Max here, then times the selectors over the
same documents. The wiki's
[Benchmarks](https://github.com/parsec-html/parsec/wiki/Benchmarks) page
has the method, every number and how to run it yourself.

## What's coming

The [roadmap](ROADMAP.md) has the order. In short, Parsec is set to cover
every module Lexbor has, and then go past it:

| | Lexbor | Parsec |
|---|---|---|
| HTML parsing, fragments | yes | yes |
| Bytes in, encoding sniffed | yes | planned |
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
| WHATWG Encoding, 40 encodings | yes | planned |
| WHATWG URL | yes | planned, with WPT pass counts published |
| CSS parser and CSSOM | yes | planned, with `var()`, media queries and computed values |
| C API | yes, x86-64 packages | yes, with packages for Linux, macOS and Windows on x86-64 and ARM |
| Python, WebAssembly | through bindings | planned, built here |

Every "yes" in the Parsec column is something `cargo test` checks. A
"planned" becomes a "yes" only when it ships.

## Using it

Parsec isn't on crates.io yet. To try it from git:

```toml
[dependencies]
parsec-html = { git = "https://github.com/parsec-html/parsec" }
```

The package is `parsec-html` and the crate you `use` is `parsec`.

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
