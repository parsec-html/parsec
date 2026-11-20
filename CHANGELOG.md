# Changelog

Every change people using Parsec would notice, newest first. Version
numbers follow [Semantic Versioning](https://semver.org/); until 1.0, a
minor release can break the API and a patch release won't.

## 0.1.0

The first release. It's on crates.io, PyPI and npm as `parsec-html`, and
the C library is attached to the
[GitHub release](https://github.com/parsec-html/parsec/releases/tag/v0.1.0).

### Packages

- **`parsec-html` on crates.io** is the one crate to depend on. The parser
  is at its top (`use parsec::...`), and each standard is a module behind
  a feature: `encoding`, `url` and `xpath` are on by default, `css` (as
  `parsec::stylesheet`) is off. Underneath, it's built from
  `parsec-html-core`, `parsec-html-encoding`, `parsec-url`, `parsec-xpath`
  and `parsec-html-css`, which are published so it can depend on them.
- Every crate is built twice from fresh checkouts and must come out the
  same byte for byte, then attested; the published `.crate` files are
  checked against those
  ([#76](https://github.com/parsec-html/parsec/pull/76)).

### HTML parser

- The HTML Standard's tokenizer and tree builder, with fragments, a tree
  you can edit, and serialization as `innerHTML`, `outerHTML`, text,
  pretty-printed or in the html5lib format
  ([#1](https://github.com/parsec-html/parsec/pull/1),
  [#93](https://github.com/parsec-html/parsec/pull/93)).
- A page can be parsed as it arrives, with `Parser::feed` and `finish`,
  and builds the same tree from any split
  ([#114](https://github.com/parsec-html/parsec/pull/114)).
- Parse errors are reported with the spec's names and their line and
  column ([#106](https://github.com/parsec-html/parsec/pull/106)), and
  every node and attribute can carry its byte range in the source
  ([#102](https://github.com/parsec-html/parsec/pull/102)).
- A `<template shadowrootmode>` attaches a declarative shadow root, with
  slots and the flat tree
  ([#119](https://github.com/parsec-html/parsec/pull/119)).
- A `NodeId` for a removed node reads as nothing, and `is_removed` says
  so, instead of reading the old node
  ([#121](https://github.com/parsec-html/parsec/pull/121)).
- The crafted pages that crashed the parser, hung serialization or made
  it crawl no longer do, and limits on time, input size and tree depth
  are yours to set
  ([#49](https://github.com/parsec-html/parsec/pull/49),
  [#84](https://github.com/parsec-html/parsec/pull/84),
  [#96](https://github.com/parsec-html/parsec/pull/96)).

### Selectors

- Selectors Level 4 in `parsec::css`, with `:has()`, `:nth-child(… of S)`,
  the form states, `:lang()`, `:dir()`, `:target`, `:local-link`,
  `:blank`, `:defined`, `pattern=` with `\p{…}`, `||` and `:nth-col()`,
  and namespace prefixes
  ([#62](https://github.com/parsec-html/parsec/pull/62),
  [#90](https://github.com/parsec-html/parsec/pull/90),
  [#103](https://github.com/parsec-html/parsec/pull/103),
  [#107](https://github.com/parsec-html/parsec/pull/107)).
- The web-platform-tests selector cases run in `cargo test`
  ([#105](https://github.com/parsec-html/parsec/pull/105)).

### Encoding (`parsec::encoding`)

- `parse_bytes` sniffs a page's encoding as a browser does and parses it;
  `ByteParser` does the same as the bytes arrive
  ([#79](https://github.com/parsec-html/parsec/pull/79),
  [#115](https://github.com/parsec-html/parsec/pull/115)).

### URL (`parsec::url`)

- The WHATWG URL Standard, `URLSearchParams`, IDNA, Punycode and Unicode
  normalization, passing all 4,565 web-platform-tests URL cases
  ([#77](https://github.com/parsec-html/parsec/pull/77),
  [#86](https://github.com/parsec-html/parsec/pull/86)).

### XPath (`parsec::xpath`)

- XPath 1.0 over the parsed document, passing libxml2's expression tests
  but its own extensions
  ([#104](https://github.com/parsec-html/parsec/pull/104)).

### CSS (`parsec::stylesheet`, feature `css`)

- CSS Syntax Level 3: the tokenizer, the parser and serialization, with a
  span on every rule ([#87](https://github.com/parsec-html/parsec/pull/87),
  [#89](https://github.com/parsec-html/parsec/pull/89)).
- Typed values for all 822 properties in the specs, written back in
  CSSOM's canonical forms
  ([#91](https://github.com/parsec-html/parsec/pull/91),
  [#98](https://github.com/parsec-html/parsec/pull/98)).
- The cascade and computed values, with media queries, `@layer`,
  `@scope`, `@container`, `@import`, `@property` and pseudo-elements
  ([#94](https://github.com/parsec-html/parsec/pull/94),
  [#99](https://github.com/parsec-html/parsec/pull/99),
  [#109](https://github.com/parsec-html/parsec/pull/109),
  [#112](https://github.com/parsec-html/parsec/pull/112)).

### C API

- `libparsec`, shared and static, with `parsec.h`, pkg-config and CMake
  files, packaged for Linux, macOS and Windows on x86-64 and ARM
  ([#78](https://github.com/parsec-html/parsec/pull/78)).
- XPath, positions, parse errors and bytes in any encoding, whole or
  streamed ([#108](https://github.com/parsec-html/parsec/pull/108),
  [#118](https://github.com/parsec-html/parsec/pull/118)).

### Python

- `pip install parsec-html`: selectolax's API, free-threaded Python
  included ([#80](https://github.com/parsec-html/parsec/pull/80)), with
  XPath, positions and parse errors
  ([#108](https://github.com/parsec-html/parsec/pull/108)), and `bytes`
  sniffed as browsers do, whole or streamed
  ([#118](https://github.com/parsec-html/parsec/pull/118)).

### WebAssembly

- `npm install parsec-html`: the parser and selectors for browsers and
  Node ([#97](https://github.com/parsec-html/parsec/pull/97)), with XPath,
  spans, parse errors and bytes sniffed as browsers do
  ([#108](https://github.com/parsec-html/parsec/pull/108),
  [#118](https://github.com/parsec-html/parsec/pull/118)).
