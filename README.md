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
- **One dependency** (`memchr`) and one `unsafe` block, with its reasoning
  written beside it.

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
| WHATWG URL | yes | planned, with WPT pass counts published |
| CSS parser and CSSOM | yes | planned, with `var()`, media queries and computed values |
| C API | yes | planned, with prebuilt libraries for Linux, macOS and Windows |
| Python, WebAssembly | through bindings | planned, built here |

Every "yes" in the Parsec column is something `cargo test` checks. A
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

The [wiki](https://github.com/parsec-html/parsec/wiki) is where the
documentation lives: how parsing works, every option, the selector list,
and a guide for moving from Lexbor.

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
