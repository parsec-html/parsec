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
| M2. HTML, past Lexbor | early 2027 |
| M3. Selectors | early 2027, alongside M2 |
| M4. Every way in, then 0.1.0 | spring 2027 |
| M5. Encoding | done, November 2026 |
| M6. URL | done, November 2026 |
| M8. XPath | summer 2027 |
| M7. CSS | late 2027 into 2028, in parts; the syntax parser done, November 2026 |

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

- Done: bytes in, with the encoding sniffed from the BOM and `<meta>`, as
  browsers do, and the html5lib encoding tests passing
  (`parsec_encoding::parse_bytes`, all 82 cases that don't need
  JavaScript).
- Chunked input that builds exactly the tree whole input does.
- Parse errors with their spec names, line and column.
- Source positions on every node and attribute.
- The pretty-printing and html5lib serializer formats, with the html5lib
  serializer tests.
- A depth limit alongside the time budget.
- Documents that are `Send` and `Sync`, enforced at compile time.
- Node handles that can tell when the node they named was removed.

## M3. Selectors that match what a page can decide

- Done: `:lang()`, `:dir()`, `:default`, `:indeterminate`, `:valid`,
  `:invalid` (with `pattern=`, Unicode property escapes included),
  `:in-range`, `:out-of-range`, `:defined` and `:blank`.
- Done: `:target` and `:local-link`, given a URL.
- `:nth-col()`, `:nth-last-col()` and the column combinator.
- Namespace prefixes.
- Specificity, lazy queries that stop at the first match, and the
  web-platform-tests selector cases that don't need layout.

## M4. Every way in

- Done: a C API with a generated header, pkg-config and CMake files, and
  prebuilt libraries for Linux, macOS and Windows on x86-64 and ARM,
  attached to each release.
- Done: a guide for moving from Lexbor, call by call, on the wiki.
- Python wheels, including free-threaded builds. The package is done, in
  `bindings/python`; its wheels go to PyPI with 0.1.0.
- WebAssembly for npm.

## M5. Encoding

Done. The WHATWG Encoding Standard's 40 encodings, decoding and
encoding, in `crates/parsec-encoding` on top of encoding_rs, with the
web-platform-tests encoding data in CI: every label, every single-byte
table, and 62,352 multi-byte characters, all passing. Decoding a stream
works now, through encoding_rs's `Decoder`; feeding it into the parser
chunk by chunk arrives with chunked input in M2.

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
- Still to come: every longhand, shorthands, custom properties, `var()`
  and `calc()`; then the cascade, media queries and computed values.

## M8. XPath

XPath 1.0 over Parsec's tree, shared with Netweir.

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
