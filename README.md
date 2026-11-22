<h1 align="center">
  <img src="https://raw.githubusercontent.com/parsec-html/parsec/main/assets/parsec-header.svg" alt="Parsec, the parsing foundation for the web, in Rust" width="100%">
</h1>

<p align="center">
  <a href="https://www.bestpractices.dev/projects/16340"><img src="https://www.bestpractices.dev/projects/16340/badge" alt="OpenSSF Best Practices"></a>
  <a href="https://scorecard.dev/viewer/?uri=github.com/parsec-html/parsec"><img src="https://api.scorecard.dev/projects/github.com/parsec-html/parsec/badge" alt="OpenSSF Scorecard"></a>
  <a href="https://github.com/parsec-html/parsec/actions/workflows/ci.yml"><img src="https://github.com/parsec-html/parsec/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

The parsing foundation for the web, in Rust: HTML, CSS, JavaScript and
XML, one crate at a time, so you build on exactly the parsers you need.
The HTML parser reads pages the way a browser does, finds what you're after with
CSS selectors, and lets you change the tree and write it back out. Beside
it sit CSS with the cascade, XPath, and the WHATWG Encoding and URL
standards. JavaScript is next, then XML, each with its own tree, so
nobody who only wants HTML compiles a JavaScript parser.

If you use Lexbor today, directly or through selectolax, PHP's `Dom\HTMLDocument`,
nokolexbor or lazy_html, the HTML parser was built for you. Lexbor is fast and
follows the spec. What we want to add is everything people keep asking it
for: positions on nodes, a worst case that stays linear, documents you can
share across threads, XPath, selectors for everything a static page can
decide, and no use-after-free bugs, because the parser is memory-safe
Rust.

Parsec is on crates.io, PyPI and npm as `parsec-html`. This
page says what works today and what's on the way, and the line between
the two is kept sharp.

## What works today

```rust
use parsec::css::SelectorList;

let doc = parsec::parse("<ul><li class=hit>one<li>two</ul>", false);
let hits = SelectorList::parse("li.hit").unwrap().select(&doc, doc.root());
assert_eq!(doc.text(hits[0]), "one");
```

- **The HTML Standard's tokenizer and tree builder.** It passes all 7,042
  html5lib tokenizer tests and every web-platform-tests tree-construction
  case that doesn't need JavaScript, run with scripting on and with it off.
  `cargo test` runs both suites; the WPT files are pinned in
  `crates/parsec/tests/tree-construction`.
- **Fragments** parsed in the context of any element, the way `innerHTML`
  is, SVG and MathML included.
- **CSS selectors, Level 4**: `:has()`, `:is()`, `:where()`, `:not()` with
  lists, `:nth-child(… of S)`, `:scope`, attribute flags, and the form states
  (`:checked`, `:disabled`, `:required`, `:valid`, `:in-range` and friends),
  with `pattern=` run by Parsec's own JavaScript regex engine, Unicode
  property escapes like `\p{L}` included. `:lang()` and `:dir()` match the
  way the document decides them, and `:target` and `:local-link` match once
  you pass the page's URL, where Lexbor parses all four and never matches
  them.
- **Declarative shadow DOM**: a `<template shadowrootmode>` attaches a
  shadow root as browsers do, with slots, the flat tree, `:host`,
  `:host()`, `:host-context()` and `::slotted()`, and `getHTML`-style
  serialization. All 7,590 of web-platform-tests' declarative shadow DOM
  cases pass. Lexbor has a shadow root node type, but its parser never
  attaches one.
- **A page as it arrives**: `Parser::new(options)`, then `feed(chunk)` as
  each piece comes in and `finish()`, builds exactly the tree the whole
  page would: the tests cut every html5lib and tree-construction input at
  every character, 205,000 splits, and compare.
- **Parse errors, named and located**: with `Options { errors: true, .. }`,
  `doc.errors()` lists every parse error with the HTML Standard's name
  (`unexpected-null-character`, `eof-in-tag` and the rest), its byte
  offset in your input, and its line and column. Lexbor reports them as
  pointers into its buffer, which PHP had to turn into lines itself.
- **Where everything came from**: with `Options { spans: true, .. }`,
  every element's start and end tag, every attribute, text run, comment
  and doctype has its byte range in the input you passed, CRLFs and a byte
  order mark accounted for, and `line_col` turns one into a line and
  column. Nodes the parser invents, like an implied `<tbody>`, have none.
- **XPath 1.0** in `crates/parsec-xpath`, which Lexbor doesn't have at all
  (nokolexbor ported libxml2's to get it): every axis and the core
  function library, steps from many nodes worked out once for the whole
  set, and a work limit for the queries that stay quadratic. It passes
  libxml2's 231 expression tests, skipping only its own extensions.

  ```rust
  use parsec::xpath::{Doc, Index, NoExtensions, XPath};

  let doc = parsec::parse("<ul><li>a<li class=x>b<li>c</ul>", false);
  let index = Index::build(&doc, &[doc.root()]);
  let query = XPath::new("count(//li[@class='x']/following-sibling::li)").unwrap();
  let n = query.evaluate(Doc::html(&doc, &index), doc.root(), &[], &[], &NoExtensions);
  ```

  Build the `Index` once per document and keep it. To see inside
  `<template>`, call `doc.adopt_template_contents()` first, as lxml has it.
- **A tree you can edit**: create, insert, move and remove nodes, set and
  rename attributes, then serialize as `innerHTML`, `outerHTML` or text,
  pretty-printed without changing the tree, in the html5lib test format,
  or with html5lib's serializer options (it passes their 229 tests).
- **Bytes in, with the encoding found the way browsers find it.**
  `parsec::encoding::parse_bytes` checks for a byte order mark, then the
  `charset` you got from HTTP if you pass one, then the HTML Standard's
  `<meta>` prescan of the first 1,024 bytes, then takes bytes that are
  valid UTF-8 for UTF-8 (the spec's autodetection step, narrowed to what
  Firefox's and Chrome's detectors both do), then falls back to
  windows-1252 (or the default for a language you name). If the parser
  later meets a `<meta>` that disagrees with a guess, it decodes and parses
  once more in the declared encoding, as a browser reloads. You get the
  document, the encoding and how sure it is. It passes all 82 html5lib
  `encoding/` cases that don't need JavaScript.
- **Bytes as they arrive, too.** `parsec::encoding::ByteParser` takes the
  page a piece at a time, straight off the socket. It holds back the first
  bytes only until it knows the encoding (a byte order mark, your HTTP
  `charset`, a whole `<meta>`, or 1,024 bytes), then decodes each piece and
  parses it as it comes. However the bytes are cut, you get what
  `parse_bytes` gives for the whole: the tests cut every html5lib encoding
  input at every byte, and the WPT multi-byte pages at 49,000 places, and
  compare. While the encoding is only a guess it keeps the
  bytes, in case a late `<meta>` means parsing again, so pass the HTTP
  `charset` when you have it.
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
  time in proportion to their length, and so do tags with a hundred
  thousand attributes and end tags under very deep nesting, which used
  to grow with the square of it. The adoption agency algorithm stays
  quadratic, as the HTML Standard defines it, and the time budget bounds
  it.
- **A C API**: `libparsec`, shared and static, with a `parsec.h` that
  cbindgen generates, and pkg-config and CMake files. Handles are opaque,
  nodes are numbers that can't dangle, every call is NULL-safe, and a bad
  argument or a bug in Parsec comes back as an error code rather than a
  crash. The parser never recurses on what a page holds, so a hundred
  thousand nested `<template>`s parse on a 256 KB thread. Documents
  can be read from many threads at once. XPath, source positions and parse errors are there
  too, and bytes in any encoding: `parsec_parse_bytes` sniffs it as a
  browser does, and `parsec_byte_parser_feed` takes the bytes as they
  arrive. Each release comes with a package for Linux, macOS and
  Windows, on x86-64 and ARM; Lexbor's own packages are x86-64 only. The
  C test runs on all three in CI, and under AddressSanitizer on Linux.
- **Two dependencies**, `memchr` and `url` (for `:local-link` and
  `:target`, and 34 crates in all with what `url` brings), and one
  `unsafe` block in the parser, with its reasoning written beside it. The C API's `unsafe` is all at the
  boundary, where C's pointers come in.
- **WHATWG URL**, in `crates/parsec-url`: every getter and setter the
  `URL` interface has, `URLSearchParams`, IDNA, Punycode and Unicode
  normalization, in a parser of Parsec's own written from the standard's
  current text, June 2026 changes included. `cargo test` runs the
  web-platform-tests URL suite, and all 4,565 of its cases pass, as do
  all 20,171 lines of Unicode's NormalizationTest; Lexbor publishes no
  URL count at all. The wiki's
  [URL](https://github.com/parsec-html/parsec/wiki/URL) page has them
  suite by suite. The IDNA and normalization tables are Unicode 18.0,
  the version WPT's IDNA tests are generated from.
- **CSS Syntax Level 3**, in `crates/parsec-css`: the tokenizer and the
  parser with every entry point of the current draft, nesting included,
  plus `parse_stylesheet_deep` for a whole stylesheet, every block's
  declarations and nested rules in one pass,
  An+B, `<urange>`, `@charset` decoding, and serialization that parses
  back to the same values. Every rule, declaration and token carries its
  byte span, and you can ask for a line and column, which Lexbor's users
  asked it for in [lexbor#374](https://github.com/lexbor/lexbor/issues/374).
  `cargo test` runs css-parsing-tests, the suite Servo and tinycss2 use,
  and passes 293 of its 305 syntax cases; the 12 it doesn't are where the
  suite predates the draft, and the wiki's
  [CSS](https://github.com/parsec-html/parsec/wiki/CSS) page names each
  one. It passes all 287 of Romain Menke's tokenizer tests too, positions
  included. Lexbor runs its own CSS tests, not an outside suite.
- **CSS values**, in `parsec::stylesheet::values`: every property in the CSS
  specs, 822 of them (597 longhands, 161 shorthands, 64 legacy aliases),
  generated from w3c/webref's data with its grammar, initial value and
  inheritance, and each value parsed against its grammar by a matcher
  for the value definition syntax. Lexbor types 105. Underneath: every
  unit in Values 4, colors per CSS Color 4 and most of 5 (relative
  colors, `color-mix()`, `light-dark()`), every math function from
  `calc()` to `atan2()`, simplified and written back as the spec says,
  shorthands split into their longhands, layer by layer for the likes of
  `background` and `transition`, and each value written back in the
  shortest form CSSOM gives it (`scale: 100% 100%` as `1`, `circle(at
  left)` as `circle(at left center)`). Custom properties resolve against each other,
  with cycles made the guaranteed-invalid value and `var()` substituted
  anywhere, which Lexbor doesn't have
  ([lexbor#232](https://github.com/lexbor/lexbor/issues/232)).
  css-parsing-tests' 8,061 color cases now run: 4,803 pass, and each of
  the other 3,258 is where CSS Color has changed since the suite was
  written, which the runner checks case by case. Of web-platform-tests'
  15,022 static CSS parsing cases, 14,758 pass, 135 fail and 129 are
  skipped, as they need computed values, layout or a DOM; the wiki's
  [CSS values](https://github.com/parsec-html/parsec/wiki/CSS-values)
  page breaks down the other 135.
- **The cascade and computed values**, in `parsec::stylesheet::cascade`: style a
  Parsec document with the user agent's, the user's and the author's
  sheets, `@media` and `@supports` evaluated against an environment you
  give (`parsec::stylesheet::media` and `::supports`), `@layer`, nesting,
  specificity, `!important` and `style=""`, `@scope` with scoping
  proximity, `@container` against the container sizes you give (size and
  `style()` queries, ranges like `style(--gap > 1em)` included, and the
  `cqw`, `cqi`, `cqmin` units, each from its nearest container), `@import` through a fetch you give with `layer()`,
  `supports()` and media, and `@property` registrations, then read any
  element's computed value as `getComputedStyle()` gives it: inheritance,
  the CSS-wide keywords with `revert` and `revert-layer`, `var()`,
  absolute lengths, and colours worked out, mixes and relative colours
  included. Pseudo-elements get computed values too:
  `styled.get_pseudo(el, &Pseudo::Before, "color")` for `::before`,
  `::after`, `::marker`, `::placeholder`, `::first-line`,
  `::first-letter`, `::selection`, `::highlight()`, `::backdrop` and
  `::file-selector-button`, each with only the properties that apply to
  it, `content` worked out (`attr()` substituted), and
  `styled.pseudo_exists()` telling you whether a `::before`, `::after` or
  `::marker` box is there. They're worked out when you ask, so a page
  costs no more to style for having them. `@keyframes` and `@counter-style` are parsed and given back
  (`parsec::stylesheet::rules`), whichever wins for each name. Lexbor has no media
  query evaluation and no computed values
  ([discussion #265](https://github.com/lexbor/lexbor/discussions/265)).
  `cargo test` runs WPT's media query cases (1,370 of 1,384, with 14
  skipped that probe the browser more than once in a test), its
  computed-value cases (11,356 of 12,466, with 355 skipped that change
  the page from script or need layout) and its static cascade, at-rule
  and pseudo-element cases, `@scope`, `@container`, `@property`,
  `@counter-style` and css-pseudo among them (1,343 of 1,673, with 199
  skipped that change the page from script);
  [Cascade](https://github.com/parsec-html/parsec/wiki/Cascade) on the
  wiki has what fails and why.
- **A Python package** in `bindings/python`, with selectolax's API:
  `HTMLParser(html).css()`, `css_first`, `text()`, `attributes` and the
  edits, so moving over is mostly a change of import. One parsed page can
  be shared by many threads, and on free-threaded Python the GIL stays
  off. It has what selectolax lacks: XPath 1.0 that answers as lxml's
  does, positions on nodes and attributes, and parse errors. Give it
  `bytes` and it finds their encoding as a browser would (pass the HTTP
  `charset` as `transport_encoding=`), tells you which in
  `parser.encoding`, and keeps offsets in your bytes; `StreamParser`
  does the same as the bytes arrive. That's a small change: `bytes` that
  aren't valid UTF-8 and don't name their encoding used to come out with
  U+FFFD and now decode as windows-1252, as in a browser. From Python, it
  parses the 13 pages below in 66 ms to selectolax's 106, and runs the 32
  selectors in 283 ms to its 781.
- **WebAssembly for JavaScript** in `bindings/wasm`, the npm package
  `parsec-html` with TypeScript types: `parse`, `parseFragment`,
  `select`, `selectFirst`, `xpath`, the tree, attributes, `textContent`,
  `innerHTML` and `outerHTML`, spans and parse errors, in Node and in
  browsers. `parseBytes` and `ByteParser` take a `Uint8Array`, whole or in
  pieces, and find its encoding as a browser does. It's 1.1 MB, 526 KB
  gzipped; the decoders' tables are about 201 KB of that, 142 KB gzipped.

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

Streaming costs a little. Feeding those transcoded pages to `ByteParser`
4 KB at a time, with the encoding given as HTTP would give it, runs at 91
to 95% of `parse_bytes` on the whole page: 502 to 555 MB a second on
wikipedia-population and 1,284 to 1,451 on nytimes, against 533 to 604
and 1,381 to 1,597. Left to sniff, it keeps the bytes in case a late
`<meta>` changes the encoding, and that brings it to 83 to 91%.

URLs as well. Over the 83,113 `href` and `src` values on those 13 pages,
each resolved against its page, Parsec takes 107 ns a URL. Ada, the C++
parser in Node.js, takes 142 and Lexbor 243, so Parsec is 1.33 times as
fast as Ada and 2.28 times as fast as Lexbor, and all three give the same
`href` for every link.

And CSS. On ten real stylesheets, 1.87 MB of Bootstrap, Bulma, Font
Awesome, normalize.css and the `<style>` elements of five of the 13
pages, Parsec tokenizes 389 MB a second to Lexbor's 210, 1.85 times as
fast, and parses the whole sheet, every block's declarations included,
at 211 to Lexbor's 112, 1.9 times, ahead on every sheet. Lexbor's parse
does read selectors and the properties it knows as well. Servo's
cssparser, which builds no tree of its own, parses at 196, so Parsec is
1.08 times as fast while keeping every rule and declaration with its
span; a second run had it 1.16 times. Its tokenizer is 1.2 times as
fast as cssparser's.
With every declaration's value typed for its property as well, all 822
properties where Lexbor types 105, Parsec runs at 145 MB a second to
Lexbor's 114, 1.26 times as fast, and it's ahead on every sheet, from
1.04 times on the New York Times' `<style>` to 1.9 on Font Awesome. And
with every element's computed values worked out, the 13 pages take 2.3
seconds between them, 1.6 of those for the HTML spec's 337,200
elements; Lexbor computes no values to compare with.

Hostile inputs, the ones built to make a parser crawl, go Parsec's way
on all eight. 80,000 unclosed `<div>`s take Parsec 3 ms and Lexbor 5.2
seconds. The closest, a dozen formatting tags reopened in every
paragraph, is 1.1 times; with 300 open `<b>` tags before 3,000
paragraphs, Parsec is 2.3 times as fast. Those two used to be Lexbor's
([#68](https://github.com/parsec-html/parsec/issues/68)).

`bench/` builds Lexbor 3.0.1 from source and runs all three on the same
strings, on an Apple M2 Max here, then times the selectors over the
same documents, URLs against Lexbor and Ada, and stylesheets against
Lexbor and cssparser. The wiki's
[Benchmarks](https://github.com/parsec-html/parsec/wiki/Benchmarks) page
has the method, every number and how to run it yourself.

## The final boss

Clean pages, one at a time, tell you how fast a parser is. They don't
tell you what happens when a crawl comes back with everything at once.
So here's the batch I'd least like to get: 1,000 pages built from a
seed, the same bytes every time. 900 are tiny broken fragments:
unclosed and misnested tags, stray `</table>`s, duplicate attributes,
NUL bytes, `&amp` with no semicolon, `&#xD800;`, tags nested 5,000
deep, comments and `<script>`s left open at the end. 46 are real pages
as they were served. 44 are real pages re-encoded: Shift_JIS, GBK,
EUC-KR, windows-1251, UTF-16 with a BOM, a `<meta>` that lies, a BOM
that disagrees with the `<meta>`, a charset that only the HTTP header
gives, UTF-8 that never says so, and UTF-8 peppered with invalid bytes.
The last 10 are giants of 10 to 30 MB. That's 265 MB, and three
quarters of it is the giants.

Each parser gets the bytes, as a crawler does, in the same order, in a
process of its own, so the peak memory is that parser's alone. Parsec
finds the encoding itself. Lexbor gets its own BOM sniff, `<meta>`
prescan and decoders. html5ever only takes text, so encoding_rs decodes
for it. These are medians of three runs, each started with the load
under 2, on the same M2 Max:

| | Seconds per 1,000 pages | Slowest page | Peak RSS | Crashes | Timeouts (10 s) |
|---|---:|---:|---:|---:|---:|
| Parsec | 1.02 | 0.163 s | 516 MB | 0 | 0 |
| Lexbor 3.0.1 | 1.94 | 0.247 s | 952 MB | 0 | 0 |
| html5ever 0.40 | 5.99 | 0.804 s | 708 MB | 0 | 0 |

Nothing crashed, nothing hit the limit, and all three built the same
number of elements on every one of the 1,000 pages. So what's left is
speed and memory, and that depends on what kind of page it is:

| | Parsec | Lexbor | html5ever |
|---|---:|---:|---:|
| 900 fragments, 1.4 MB | 0.049 s | 0.407 s | 0.752 s |
| 46 real pages, 33 MB | 0.057 s | 0.132 s | 0.298 s |
| 44 re-encoded pages, 37 MB | 0.063 s | 0.179 s | 0.308 s |
| 10 giants, 194 MB | 0.854 s | 1.221 s | 4.636 s |

The fragments show what a page costs before there's anything to parse.
Lexbor spends 0.45 ms on each and Parsec 54 µs, eight times less. On
real pages Parsec is 2.3 times as fast, and 2.8 on the re-encoded ones.

The giants used to be Lexbor's. Three of them are one long text node of
character references, and Parsec took twice Lexbor's time on those,
looking each name up in a sorted table and handing the tree builder a
fresh run of text at every reference
([#124](https://github.com/parsec-html/parsec/issues/124)). Now the
names come from a hash table built at compile time and a stretch of
text arrives in one piece, and those three pages take 0.21 s to
Lexbor's 0.28. The huge tables go 0.36 s to 0.56 and the real pages run
together 0.26 to 0.37, so the giants are Parsec's by 1.4 times, and so
is the slowest page: 0.163 s to Lexbor's 0.247, the same 27.6 MB table
for both. Memory is Parsec's as well: just over half Lexbor's peak, and
less than html5ever's. All three include the 32 MB the harness holds for the
biggest page.

`cargo run --release --manifest-path bench/Cargo.toml -- boss` runs it,
and CI runs a 300-page version with nothing over 2 MB through Parsec on
every pull request, failing on a crash or a hang. The wiki's
[Benchmarks](https://github.com/parsec-html/parsec/wiki/Benchmarks#the-final-boss)
page has every quirk, the seed and the raw output.

### The real web

The final boss is built to hurt. The real web is just big: 1,731
production pages, 559 MB of home pages and inner pages in 8 encodings
and over 60 languages, fetched on 21 November 2026. Parsec parses them
at 933 MB a second to Lexbor's 523 and html5ever's 160, 1.78 times
Lexbor overall, with all three building the same number of elements on
every page and none crashing. The wiki's
[Real-world benchmarks](https://github.com/parsec-html/parsec/wiki/Real-world-benchmarks)
page has the method, the breakdowns by size and encoding, where Parsec
loses, and how to refetch the same pages; only their addresses and
hashes are published.

## What's coming

The [roadmap](https://github.com/parsec-html/parsec/blob/main/ROADMAP.md) has the order. In short, Parsec is set to cover
every module Lexbor has, and then go past it:

| | Lexbor | Parsec |
|---|---|---|
| HTML parsing, fragments | yes | yes |
| Bytes in, encoding sniffed | yes | yes, `parse_bytes` |
| Chunked input | yes | yes, the same tree from any split, checked at every cut of every test |
| Bytes streamed, encoding sniffed as they arrive | no, `lxb_engine_parse` takes the whole page | yes, `ByteParser`, the same as `parse_bytes` for any split |
| Parse errors with line and column | pointers only | yes, with the spec's names; all 7,042 html5lib tokenizer error cases match |
| Source positions on nodes and attributes | no ([lexbor#374](https://github.com/lexbor/lexbor/issues/374)) | yes, byte ranges into your input with line and column |
| Linear worst case | no ([#340](https://github.com/lexbor/lexbor/issues/340)) | planned |
| Thread-safe documents | no | yes, `Send` and `Sync`, checked at compile time |
| `:scope`, `:lang()`, `:dir()`, `:target`, `:local-link` | parsed, never match | yes |
| `:valid`, `:invalid`, `:in-range`, `:out-of-range`, `:default`, `:indeterminate` | parsed, never match | yes |
| `:blank`, `:defined` | parsed, never match | yes |
| Queries that stop at the first match | no ([#32](https://github.com/lexbor/lexbor/issues/32)) | yes, `select_first` and `select_iter` |
| web-platform-tests selector cases | not run | yes, every one that runs without a browser: 1,719 DOM and 461 CSS cases |
| A tree depth limit | no | yes |
| A node you hold after it's removed | a dangling pointer, read after free ([lexbor#132](https://github.com/lexbor/lexbor/issues/132)) | reads as nothing, and `is_removed` tells you; `detach` keeps a subtree to move |
| XPath | no | yes, XPath 1.0, all of libxml2's expression tests but its own extensions |
| WHATWG Encoding, 40 encodings | yes | yes, through encoding_rs, with WPT pass counts published |
| WHATWG URL | yes, no WPT count published | yes, all 4,565 WPT cases passing |
| Unicode normalization | yes | yes, all of NormalizationTest passing |
| CSS Syntax: tokenizer, parser, serializer | yes, no outside suite run | yes, css-parsing-tests in `cargo test`, positions on every rule |
| Typed property values | yes, 105 properties | yes, all 822 in the specs, with `var()`, `calc()` and shorthands; 99% of WPT's parsing cases |
| Media queries, cascade, computed values | no | yes, against an environment you give, with `@scope`, `@container`, `@import`, `@property` and pseudo-elements; WPT counts published |
| JavaScript syntax tree | no | yes, `crates/parsec-js`: ECMAScript 2025 scripts and modules with Annex B's additions, to a tree with a span on every node, and ESTree JSON that matches acorn's; all 48,408 test262 files that should parse do, and 3,824 of the 4,374 that shouldn't fail, in `cargo test` |
| XML | no | yes, `crates/parsec-xml`: XML 1.0 and Namespaces, not validating, with a span on every node and attribute; billion laughs and quadratic blowup stop with an error, and nothing is ever fetched; all 1,915 W3C conformance cases that apply pass; 2.7 times as fast as libxml2 on real feeds, sitemaps, SVGs and Wikipedia |
| C API | yes, x86-64 packages | yes, with packages for Linux, macOS and Windows on x86-64 and ARM; XPath, positions, parse errors and bytes in any encoding, whole or streamed, included |
| Python | through selectolax | yes, free-threaded too, with XPath, positions, parse errors, and `bytes` sniffed as browsers do, whole or streamed |
| WebAssembly | through bindings | yes, on npm with 0.1.0, with XPath, spans, parse errors, and bytes sniffed as browsers do, whole or streamed |
| Node.js, native | not from Lexbor | next release: the npm package picks a native build on Linux, macOS and Windows, x86-64 and ARM, with the same API, `parseAsync`, and 1.5 times the WebAssembly's speed |

Every "yes" in the Parsec column is something `cargo test` checks, or
for Python, pytest in CI, and for WebAssembly, wasm-bindgen-test in Node
and Firefox. A
"planned" becomes a "yes" only when it ships.

## Roadmap

What's done, what's under way, and what's after that. Each milestone links
to its issues, and the [Road to the parsing
platform](https://github.com/orgs/parsec-html/projects/2) board shows who
has what. [ROADMAP.md](https://github.com/parsec-html/parsec/blob/main/ROADMAP.md) has the detail and the target dates.
How the crates fit together is on the wiki's
[Architecture](https://github.com/parsec-html/parsec/wiki/Architecture#the-crates-and-how-they-fit)
page.

| | Milestone | State |
|---|---|---|
| M0 | [Foundations](https://github.com/parsec-html/parsec/milestone/1?closed=1) | done |
| M1 | [Proof: the bench against Lexbor's C](https://github.com/parsec-html/parsec/milestone/2?closed=1) | done |
| M2 | [HTML, past Lexbor](https://github.com/parsec-html/parsec/milestone/3?closed=1) | done |
| M3 | [Selectors that match what a page can decide](https://github.com/parsec-html/parsec/milestone/4?closed=1) | done |
| M4 | [Every way in: C, Python, WebAssembly](https://github.com/parsec-html/parsec/milestone/5?closed=1) | done |
| M5 | [Encoding](https://github.com/parsec-html/parsec/milestone/6?closed=1) | done |
| M6 | [URL](https://github.com/parsec-html/parsec/milestone/7?closed=1) | done |
| M7 | [CSS, through the cascade](https://github.com/parsec-html/parsec/milestone/8?closed=1) | done |
| M8 | [XPath](https://github.com/parsec-html/parsec/milestone/9?closed=1) | done |
| M15 | [One toolkit, in pieces](https://github.com/parsec-html/parsec/milestone/17) | under way |
| M9 | [JavaScript tokens](https://github.com/parsec-html/parsec/milestone/11) | under way |
| M10 | [JavaScript syntax tree](https://github.com/parsec-html/parsec/milestone/12) | planned |
| M11 | [JavaScript scope and modules](https://github.com/parsec-html/parsec/milestone/13) | planned |
| M12 | [Data from scripts, read without running them](https://github.com/parsec-html/parsec/milestone/14) | planned |
| M14 | [XML](https://github.com/parsec-html/parsec/milestone/16) | planned |
| M16 | [JavaScript and XML in every binding](https://github.com/parsec-html/parsec/milestone/18) | planned |
| M13 | [JavaScript diagnostics and incremental parsing](https://github.com/parsec-html/parsec/milestone/15) | planned |

## Using it

Parsec is on crates.io, PyPI and npm as `parsec-html`, and the C
library comes with each [GitHub release](https://github.com/parsec-html/parsec/releases).

**Rust.** The package is `parsec-html` and the crate you `use` is
`parsec`:

```sh
cargo add parsec-html                      # parser, selectors, encoding, URL, XPath
cargo add parsec-html --features css       # plus CSS Syntax, values and the cascade
cargo add parsec-html --no-default-features --features xpath   # just what you name
```

Each standard is a module: `parsec::encoding`, `parsec::url`,
`parsec::xpath`, and with `css`, `parsec::stylesheet` (`parsec::css` is
the selector engine, always there). Bytes in looks like this:

```rust
let page = b"<meta charset=shift_jis><title>\x93\xfa\x96\x7b</title>";
let parsed = parsec::encoding::parse_bytes(page, Default::default()).unwrap();
assert_eq!(parsed.encoding.name(), "Shift_JIS");
assert_eq!(parsed.document.text(parsed.document.root()), "日本");
```

**Python** 3.9 or newer, free-threaded too; then `import parsec_html`:

```sh
pip install parsec-html
```

Wheels cover Linux, macOS and Windows; anywhere else, pip builds the
sdist, which needs Rust 1.88 or newer.

**JavaScript**, in Node or a browser:

```sh
npm install parsec-html
```

**C.** Each release has a package per platform (Linux, macOS and Windows,
on x86-64 and ARM) with the shared and static library, `parsec.h`, and
pkg-config and CMake files. Unpack it anywhere, then `find_package(parsec)`
with `CMAKE_PREFIX_PATH` pointing there, or `pkg-config parsec` with
`PKG_CONFIG_PATH` set to its `lib/pkgconfig`:

```sh
gh release download v0.1.0 --repo parsec-html/parsec --pattern 'parsec-capi-0.1.0-x86_64-unknown-linux-gnu.tar.gz'
tar -xzf parsec-capi-0.1.0-x86_64-unknown-linux-gnu.tar.gz
```

Or build the same package yourself, into a folder of your choice:

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
there. [CONTRIBUTING.md](https://github.com/parsec-html/parsec/blob/main/CONTRIBUTING.md) explains how that works and how
to get a change in. Questions go to
[Discussions](https://github.com/parsec-html/parsec/discussions), bugs to
[Issues](https://github.com/parsec-html/parsec/issues).

## Licence

Apache-2.0. See [LICENSE](https://github.com/parsec-html/parsec/blob/main/LICENSE) and [NOTICE](https://github.com/parsec-html/parsec/blob/main/NOTICE).
