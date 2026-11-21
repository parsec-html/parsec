# Roadmap

Parsec has one aim: to be the parser you'd pick over Lexbor on every count
that matters to you. That means faster on the same pages, measured in one
process against Lexbor's own C; at least as conformant on every public test
suite; memory-safe; and pleasant to use from Rust, C, Python and the
browser.

Here's the order we're working in. Each milestone has an issue list on
GitHub, and nothing moves from "planned" to "done" in the README until it
ships with its tests.

## When

Targets, not promises: a milestone ships when it's done and tested, and
the order can change when users show us something more urgent.

| Milestone | Target |
|---|---|
| M1. Proof | done, November 2026 |
| M2. HTML, past Lexbor | done, November 2026 |
| M3. Selectors | early 2027, alongside M2 |
| M4. Every way in, then 0.1.0 | spring 2027 |
| M5. Encoding | done, November 2026 |
| M6. URL | done, November 2026 |
| M8. XPath | summer 2027 |
| M7. CSS | late 2027 into 2028, in parts; syntax, values and the cascade done, November 2026 |

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

The first step towards `parsec-js`, a JavaScript parser of Parsec's own:
a parser, not an engine, so nothing runs.

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

Every program ECMAScript 2025 accepts, parsed to a tree, and every one it
rejects, rejected
([#151](https://github.com/parsec-html/parsec/issues/151)).

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
- Done: the tree and its ESTree JSON, apart from the bench check
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
- Planned: a bench against oxc and swc on real bundles
  ([#156](https://github.com/parsec-html/parsec/issues/156)).

## 0.1.0

The first release goes to crates.io, PyPI and npm as `parsec-html` once
M1 to M4 are done and the benchmarks show Parsec ahead on every page in the
corpus.

## Not planned

Some things we've decided not to do, so nobody waits for them:

- **Running JavaScript.** Parsec builds the tree the HTML Standard
  describes for a page without scripts running. Pages that need scripts
  need a browser; Netweir drives one.
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
