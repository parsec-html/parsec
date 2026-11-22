# Roadmap

Parsec started with one aim: to be the HTML parser you'd pick over Lexbor
on every count that matters to you. Milestones M0 to M8 got it there, and
the benchmarks and conformance counts that say so are on the wiki.

What we're building now is bigger: the parsing foundation for the web,
one crate per language. HTML, CSS, JavaScript and XML, each with its own tree
and its own error handling, over shared plumbing for input, positions,
diagnostics and limits. If you only parse HTML, you never compile the
JavaScript parser. If you parse XML, you get XML's rules, not HTML's
recovery.

Here's the order we're working in. Each milestone has an issue list on
GitHub and a place on the [Road to the parsing
platform](https://github.com/orgs/parsec-html/projects/2) board, and
nothing moves from "planned" to "done" in the README until it ships with
its tests.

## When

Targets, not promises: a milestone ships when it's done and tested, and
the order can change when users show us something more urgent.

| Milestone | Target | Today |
|---|---|---|
| M0 to M8. HTML, selectors, every way in, encoding, URL, CSS, XPath | done, November 2026 | released |
| M15. One toolkit, in pieces | December 2026 | under way: `parsec-base` on `main` |
| M9. JavaScript tokens | December 2026 | under way: the lexer, test262 and fuzzing on `main` |
| M10. JavaScript syntax tree | early 2027, with 0.3.0 | under way: scripts on `main` |
| M11. JavaScript scope and modules | spring 2027 | planned |
| M12. Data from scripts | spring 2027 | planned |
| M14. XML | summer 2027 | under way: `parsec-xml`, the W3C suite and the bench on `main` |
| M16. Every language, every way in | alongside each parser | under way: the native Node build on `main` |
| M13. JavaScript diagnostics and incremental parsing | late 2027 | planned |

Nothing marked "on `main`" is in a release yet.

## M0. Foundations

The repository, CI on Linux, macOS and Windows, the shared parser core and
the check that keeps it in step with Netweir, fuzzing, the wiki, and this
roadmap.

## M1. Proof

Done. Before Parsec claimed to be faster than Lexbor, a harness had to
show it: `bench/` builds Lexbor from a pinned release and runs it, html5ever
and Parsec on the same pages in one process, then times Parsec's selectors
against Lexbor's over the same documents. The results, with method and
machine, are on the wiki and replaced the borrowed numbers in the README.
Every CI run publishes the conformance counts in its summary.

## M2. HTML, past Lexbor

Done.

- Done: bytes in, with the encoding sniffed from the BOM and `<meta>`, as
  browsers do, and the html5lib encoding tests passing
  (`parsec_encoding::parse_bytes`, all 82 cases that don't need
  JavaScript).
- Done: chunked input that builds exactly the tree whole input does.
- Done: parse errors with their spec names, line and column.
- Done: source positions on every node and attribute.
- Done: the pretty-printing and html5lib serializer formats, with the
  html5lib serializer tests (229 passed, 1 skipped).
- Done: a depth limit alongside the time budget.
- Done: documents that are `Send` and `Sync`, enforced at compile time.
- Done: node ids that know when their node was removed. `remove` is for
  good, and every id in what it took out reads as nothing from then on
  (`is_removed` says so); `detach` takes a subtree out to move it. In
  the C API that's `parsec_remove` and `parsec_detach`.

## M3. Selectors that match what a page can decide

- Done: `:lang()`, `:dir()`, `:default`, `:indeterminate`, `:valid`,
  `:invalid` (with `pattern=`, Unicode property escapes included),
  `:in-range`, `:out-of-range`, `:defined` and `:blank`.
- Done: `:target` and `:local-link`, given a URL.
- Done: `:nth-col()`, `:nth-last-col()` and the column combinator, by the
  HTML table model (`colspan`, `rowspan`, `colgroup`), linear on hostile
  tables.
- Done: namespace prefixes, declared through `SelectorList::parse_with`
  the way `@namespace` rules would declare them.
- Done: specificity, lazy queries that stop at the first match, and the
  web-platform-tests selector cases that don't need layout (every case
  that runs without a browser passes).

## M4. Every way in

- Done: a C API with a generated header, pkg-config and CMake files, and
  prebuilt libraries for Linux, macOS and Windows on x86-64 and ARM,
  attached to each release.
- Done: a guide for moving from Lexbor, call by call, on the wiki.
- Done: a Python package with selectolax's API, in `bindings/python`, on
  free-threaded Python too, with XPath, source positions and parse
  errors ([#23](https://github.com/parsec-html/parsec/issues/23)). Its
  wheels go to PyPI with 0.1.0.
- Done: WebAssembly for npm, in `bindings/wasm`. It and the C API have
  XPath, source positions and parse errors too.
- Done: bytes in, for Python, WebAssembly and C, with the encoding
  sniffed as `parse_bytes` sniffs it, whole or streamed, and positions
  as offsets into those bytes in any encoding
  ([#116](https://github.com/parsec-html/parsec/issues/116)).

## M5. Encoding

Done. The WHATWG Encoding Standard's 40 encodings, decoding and
encoding, in `crates/parsec-encoding` on top of encoding_rs, with the
web-platform-tests encoding data in CI: every label, every single-byte
table, and 62,352 multi-byte characters, all passing. Bytes stream
straight into the parser too: `ByteParser` sniffs the encoding from the
first bytes, decodes each piece through encoding_rs's `Decoder` and feeds
it to the chunked parser, with the same result as `parse_bytes` however
the input is cut.

## M6. URL

Done. WHATWG URL, `URLSearchParams`, IDNA, Punycode and Unicode
normalization in `crates/parsec-url`, a parser of Parsec's own written
from the URL Standard's current text, with the web-platform-tests URL
pass counts published and a benchmark against Lexbor's URL parser and
Ada.

- Done: `crates/parsec-url`, with URL, `URLSearchParams`, IDNA and
  Punycode, and the WPT URL suite in `cargo test` with its counts in
  every CI run's summary.
- Done: the benchmark against Lexbor's URL parser and Ada, in `bench/`.
- Done: Unicode normalization, NFC and NFD from the Unicode 18.0
  character data, as `parsec_url::normalize`, passing all of Unicode's
  NormalizationTest in `cargo test`.
- Done: a URL parser of Parsec's own behind the same API, in place of
  the `url` crate ([#82](https://github.com/parsec-html/parsec/issues/82)).
  It passes all 4,565 WPT cases, where the `url` crate passed 3,690, and
  on the links in the benchmark pages it's 1.33 times as fast as Ada.

## M7. CSS

The CSS Syntax parser, passing `css-parsing-tests` with positions on every
rule. Then the CSS Object Model, every property in the spec's index,
custom properties and `var()`, `calc()`, media queries evaluated against an
environment you give it, and computed values with inheritance.

- Done: `crates/parsec-css`, CSS Syntax Level 3 as the current draft has
  it, with spans on every rule, declaration and token, and
  css-parsing-tests and the CSS tokenizer tests in `cargo test`. 293 of
  the suite's 305 syntax cases pass; the other 12 are where it predates
  the draft. Benchmarked against Lexbor's CSS module and Servo's
  cssparser in `bench/`.
- Done: `parse_stylesheet_deep`, a whole stylesheet with every block's
  contents in one pass, straight from the tokenizer. It parses at 211
  MB/s to cssparser's 196 and Lexbor's 112
  ([#88](https://github.com/parsec-html/parsec/issues/88)).
- Done: typed values
  ([#29](https://github.com/parsec-html/parsec/issues/29)). Every
  property in the specs, generated from w3c/webref, its value parsed
  against its grammar; Values 4's units and math functions; CSS Color 4
  and most of 5;
  shorthands into longhands; custom properties with cycles and `var()`.
  css-parsing-tests' color files run (4,803 of 8,061 pass, the rest
  checked against how CSS Color changed).
- Done: each value in its canonical form, CSSOM's shortest, property by
  property, the specs' prose restrictions as checks, and typed values
  ahead of Lexbor on every sheet in the bench, 145 MB/s to 114
  ([#92](https://github.com/parsec-html/parsec/issues/92)). 14,758 of
  WPT's 14,893 static CSS parsing cases pass.
- Done: the cascade, media queries and computed values
  ([#30](https://github.com/parsec-html/parsec/issues/30)). Origins,
  layers, specificity, `!important` and `style=""`; `@media` and
  `@supports` evaluated against an environment you give; inheritance,
  `revert`, `var()` and computed values as `getComputedStyle()` writes
  them. WPT: 1,370 of 1,370 media query cases, 11,287 of 12,041
  computed-value cases, 104 of 123 cascade cases.
- Done: the cascade's other at-rules
  ([#95](https://github.com/parsec-html/parsec/issues/95)). `@scope` with
  scoping proximity, `@container` against sizes the caller gives,
  `@import` through the caller's fetch with `layer()`, `supports()` and
  media, `@property` registrations, and `@keyframes` and
  `@counter-style` parsed and given back. WPT: 1,141 of 1,345 static
  cascade and at-rule cases.
- Done: range syntax in `style()` queries and the container query units
  (`cqw`, `cqh`, `cqi`, `cqb`, `cqmin`, `cqmax`), each axis from its
  nearest container and the small viewport without one
  ([#100](https://github.com/parsec-html/parsec/issues/100)). WPT: 1,198
  of 1,345 static cascade and at-rule cases.
- Done: pseudo-elements' computed values
  ([#111](https://github.com/parsec-html/parsec/issues/111)). `::before`,
  `::after`, `::marker`, `::placeholder`, `::first-line`,
  `::first-letter`, `::selection`, `::highlight()`, `::backdrop` and
  `::file-selector-button`, inherited from the originating element (and
  highlights from the parent's highlight), each with the properties that
  apply to it, `content` computed, and whether `::before`, `::after` and
  `::marker` exist. WPT: 1,343 of 1,474 static cascade, at-rule and
  pseudo-element cases, 11,356 of 12,111 computed-value cases.
- Still to come: shadow DOM for `@scope` and container lookups, now that
  the tree builder attaches declarative shadow roots (#110); and the
  values that need layout (`width: 50%` as pixels).

## M8. XPath

Done. XPath 1.0 over Parsec's tree, shared with Netweir: `crates/parsec-xpath`,
with libxml2's expression tests in `cargo test` and queries that would be
quadratic stopped by a work limit.

## M9. JavaScript tokens

The start of `parsec-js`, a JavaScript parser of our own, written from
scratch. A parser, not an engine: nothing in it runs code. First the
whole ECMAScript 2025 lexical grammar, with the caller choosing between
division and a regular expression the way the spec's goal symbols do, so
the lexer never guesses. test262 in CI, a fuzz target, and a bench
against oxc and swc ([#146](https://github.com/parsec-html/parsec/issues/146)).

- Done: `crates/parsec-base`, what the parsers share and nothing they
  don't: spans, a line index with UTF-16 columns, diagnostics, limits
  and a step budget, with no dependencies
  ([#143](https://github.com/parsec-html/parsec/issues/143)).
- Done: `crates/parsec-js`'s lexer, ECMAScript 2025's lexical grammar
  with Annex B's HTML-like comments in scripts and legacy octal flagged.
  The caller passes the goal, so `/` and `}` are never guessed; every
  token has a span and none allocates
  ([#147](https://github.com/parsec-html/parsec/issues/147)).
- Done: a fuzz target for the lexer in the nightly fuzz run, its corpus
  seeded from test262
  ([#150](https://github.com/parsec-html/parsec/issues/150)).
- Done: test262, pinned as a submodule, in `cargo test`, with the counts
  in every CI run's summary
  ([#148](https://github.com/parsec-html/parsec/issues/148)). All 49,655
  files that should parse tokenize. Of the 4,676 that shouldn't, the
  lexer catches the 267 whose error is in the tokens; the parser will
  check the rest.

## M10. JavaScript syntax tree

Every script and module ECMAScript 2025 allows, parsed to an arena tree
with a span on every node and written out as ESTree JSON, so tools built
for acorn can read it. test262's syntax tests in CI, a depth limit that
returns an error where other parsers overflow the stack, and a bench on
real bundles ([#151](https://github.com/parsec-html/parsec/issues/151)).

- Done: scripts
  ([#153](https://github.com/parsec-html/parsec/issues/153)). Every
  statement and declaration, the expression grammar with its precedence,
  destructuring, classes with private names and static blocks, async
  functions and generators, optional chaining, automatic semicolon
  insertion, and Annex B's additions for sloppy code. The parser picks
  each `/` and `}` goal, and reads the cover grammars (arrow parameters,
  assignment patterns) once, turning them into patterns in place. In
  test262, all 47,895 script files that should parse do. Of the 4,174
  that shouldn't, 3,639 fail to; the other 535 are early errors that
  need scope analysis (a name declared twice) or the regular expression
  pattern grammar, which `cargo test` skips and counts until M11.
- Done: the tree and its ESTree JSON
  ([#152](https://github.com/parsec-html/parsec/issues/152)). Every node
  sits in one arena with a span beside it, and comments are kept on the
  side. `Ast::to_estree` writes the shape acorn gives, offsets in UTF-16.
  `cargo test` checks it against acorn 8.19.0's trees for the files in
  `crates/parsec-js/tests/estree`, and `scripts/compare-acorn.sh` finds
  it the same as acorn's on all 47,542 test262 scripts both parse, and
  on typescript.js and react-dom.
- Done: modules
  ([#154](https://github.com/parsec-html/parsec/issues/154)): every
  `import` and `export` form, import attributes (`with { type: "json" }`),
  string names (`export { a as "a-b" }`), `import.meta`, top-level
  `await`, strict mode throughout, and exported names checked for
  duplicates. In test262, all 513 module files that should parse do,
  and 185 of the 200 that shouldn't fail to; the other 15 declare a name
  twice or export one never declared, which waits for scope analysis.
  `scripts/compare-acorn.sh` finds the tree the same as acorn's on the
  509 modules both parse, and on three.js.
- Done: limits
  ([#155](https://github.com/parsec-html/parsec/issues/155)). The parser
  counts its depth against `Limits::max_depth`, 256 by default, which
  real code stays far below (typescript.js needs 53), and its tokens
  against the step budget. A 1,000,000-deep `((((` fails with
  `too-deep` on a 1 MB thread, and so does nesting of each of 20 other
  kinds, in `cargo test`; 23 kinds of wide input parse in time that
  grows linearly, and a million-deep member chain writes out as ESTree
  on a 256 KB thread. A `parse` fuzz target joins the lexer's in the
  nightly run.
- Done: a bench against oxc and swc on real bundles
  ([#156](https://github.com/parsec-html/parsec/issues/156)):
  `parsec-html-bench js`, on react-dom, three.js and typescript.js
  pinned in `bench/scripts.txt` and the real pages' inline scripts.
  parsec-js parses 202 MB/s, 1.26 times swc_ecma_parser's speed and
  0.57 times oxc_parser's, and holds less memory than either; the gap
  to oxc is [#190](https://github.com/parsec-html/parsec/issues/190).
  `scripts/compare-acorn.sh --bench` finds every tree in the corpus the
  same as acorn's, which finishes #152.

## M11. JavaScript scope and modules

What a name refers to, which programs the spec rejects before they run,
and what each module imports and exports
([#157](https://github.com/parsec-html/parsec/issues/157)).

## M12. Data from scripts

The reason JavaScript matters for scraping. Pages ship their data inside
`<script>`: `__NEXT_DATA__`, `window.__NUXT__`, a Redux store assigned to
a variable. Parsec will read those straight from the source as JSON,
without executing a line, where scrapers today run a browser or a regex
that breaks on the first nested brace. Netweir is the first user
([#161](https://github.com/parsec-html/parsec/issues/161)).

## M13. JavaScript diagnostics and incremental parsing

Errors with codes, labels and code frames, comments kept for tools, and
reparsing only what an edit touched
([#165](https://github.com/parsec-html/parsec/issues/165)).

## M14. XML

`parsec-xml`: XML 1.0 and Namespaces with XML's own rules. Non-validating,
entity expansion capped so billion laughs stops with an error, nothing
ever fetched. The W3C conformance suite in CI, XPath over XML, and a bench
against libxml2 ([#168](https://github.com/parsec-html/parsec/issues/168)).

- Done: `crates/parsec-xml`, XML 1.0 (Fifth Edition) and Namespaces in
  XML 1.0, not validating
  ([#169](https://github.com/parsec-html/parsec/issues/169)). Names
  resolved to their namespaces, a span on every node and attribute, the
  internal subset's entities and attribute defaults, and nothing ever
  fetched: the external subset and external entities are reported as not
  read. Entity expansion is capped in depth and in size, so billion
  laughs and quadratic blowup stop with an error, and a million nested
  elements cost no recursion. Bytes in, with the encoding from the byte
  order mark or the declaration; a serializer that writes every prefix
  and declaration back as it was; and a fuzz target in the nightly run.
- Done: the W3C XML Conformance Test Suite in `cargo test`, with its
  counts in every CI run's summary
  ([#170](https://github.com/parsec-html/parsec/issues/170)). Of its
  2,585 cases, 1,915 run and pass, 227 of them matching their canonical
  output byte for byte, and none fail. The 670 skipped are XML 1.1 and
  Namespaces 1.1 (274), editions before the Fifth (310), errors the spec
  lets a processor report or not (27), and documents whose fault is in
  an external entity, which Parsec never reads (59).
- Done: the bench against libxml2, roxmltree and quick-xml, in `bench/`
  ([#172](https://github.com/parsec-html/parsec/issues/172)). libxml2
  comes in as a submodule at v2.15.4. On 102 MB of real XML (sitemaps,
  RSS and Atom feeds, SVGs, the XML spec and a Wikipedia dump slice),
  Parsec parses 1,314 MB/s to libxml2's 495 and roxmltree's 327, ahead
  of both on every document, and holds no more memory than libxml2 on any.
- Still to come: XPath over XML
  ([#171](https://github.com/parsec-html/parsec/issues/171)), which
  needs `parsec-xpath` to read a tree through a trait; that change
  starts in Netweir.

## M15. One toolkit, in pieces

`parsec-base` for what every parser shares (spans, line indexes,
diagnostics, limits), a CI check that each crate depends only on what it
should, and the architecture written down on the wiki
([#140](https://github.com/parsec-html/parsec/issues/140)).

## M16. Every language, every way in

JavaScript and XML through the C API, Python and WebAssembly, the way
HTML is today, and a native Node.js package beside the WebAssembly one
([#173](https://github.com/parsec-html/parsec/issues/173)).

- Done: the native Node.js build
  ([#177](https://github.com/parsec-html/parsec/issues/177)). The npm
  package loads it on Linux (glibc and musl), macOS and Windows, x86-64
  and ARM64, with the WebAssembly's API and `parseAsync`, and falls back
  to the WebAssembly anywhere else. It ships in the next release.
- Still to come: JavaScript and XML in the C API, Python and
  WebAssembly ([#174](https://github.com/parsec-html/parsec/issues/174),
  [#175](https://github.com/parsec-html/parsec/issues/175),
  [#176](https://github.com/parsec-html/parsec/issues/176)).

## Releases

0.1.0 shipped in November 2026 once M1 to M4 were done and the benchmarks
showed Parsec ahead on every page in the corpus; 0.2.0 followed with
security fixes. 0.2.1 brings selector step limits to the bindings. 0.3.0
is the first release with `parsec-js`.

## Not planned

Some things we've decided not to do, so nobody waits for them:

- **Running JavaScript.** `parsec-js` parses scripts; it never runs
  them. The HTML parser builds the tree the HTML Standard describes for a
  page without scripts running. Pages that need scripts run need a
  browser; Netweir drives one.
- **Layout and rendering.** No boxes, fonts or painting. Computed CSS
  values (M7) stop short of layout.
- **A "relaxed" mode that breaks the spec.** Parsec parses the way
  browsers do. If you need a different tree, the tokenizer's `Sink` lets
  you build your own.
- **Lexbor's `lxb_*` symbols.** The C API (M4) is Parsec's own. Copying
  Lexbor's would also copy its node lifetimes, which are where its
  use-after-free bugs come from. The migration guide maps one to the
  other instead.
- **Fetching.** Parsec parses what you give it. Downloading pages is
  someone else's job (Netweir's, for one).
- **DOM events and mutation observers.** Nothing runs in response to a
  change, because nothing runs at all.
