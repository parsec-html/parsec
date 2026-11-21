# Changelog

Every change people using Parsec would notice, newest first. Version
numbers follow [Semantic Versioning](https://semver.org/); until 1.0, a
minor release can break the API and a patch release won't.

## Unreleased

### Added

- Every binding can now put a limit on selector matching. Most
  selectors cost one walk of the tree, but `div:has(span div)` over a
  few thousand nested divs costs millions of steps, and a crawler
  running selectors or pages it doesn't control needs a way to stop
  that. A limit that runs out is an error, never half the matches
  ([#135](https://github.com/parsec-html/parsec/issues/135)).
  - C: `parsec_select_with` and `parsec_matches_with` take `max_steps`
    and `budget_us` (0 for none) and give `PARSEC_ERR_TIMEOUT` past
    either. `parsec_select` is unchanged.
  - Python: `css()`, `css_first()`, `css_matches()` and
    `any_css_matches()` take keyword-only `max_steps=` and `budget=`
    (seconds), and raise `TimeoutError`, as a parse `budget=` does.
  - WebAssembly: `select`, `selectFirst` and `matches` take
    `{ maxSteps }`, and throw an Error naming `maxSteps`. There's no
    time limit there, since wasm32 has no clock.

## 0.2.0

A security release. A page could crash any program using Parsec
0.1.0: a few thousand nested `<template>`s overflowed the stack, which
nothing can catch, and ended the process in every binding. That's
fixed, along with inputs built to take quadratic time, a buffer that
could overflow on huge input, panics in XPath, edit calls that wrote
names as markup, positions in the C API, wheels 0.1.0 didn't ship, and
docs that said more than the code did. Upgrade if Parsec reads pages
you don't control.

It's 0.2.0, not 0.1.1, because the edit API changed shape: see
[Changed](#changed). Until 1.0, a minor release can break the API and
a patch release won't.

### Changed

- `Document::create_element` returns `Result<NodeId, EditError>`, and
  `set_attr`, `add_attr`, `set_attrs`, `rename` and `rename_attr`
  return `Result<(), EditError>`. A name `createElement` or
  `setAttribute` would refuse (empty, or with whitespace, `/`, `>`,
  `=`, a quote or a control character in it) is the new
  `EditError::InvalidName`, instead of being written out as markup.
  `create_element_as_parsed` and `add_attr_as_parsed` take a name as
  given, for copying a parsed node into another document, as
  `importNode` does: the parser makes names like `a"b` that the checks
  refuse ([#136](https://github.com/parsec-html/parsec/pull/136)).
- `MatchOptions` and `MatchTimeout` have a new field, `max_steps`: a
  limit on selector matching that needs no clock, so it works on wasm32
  too (#136). Code that builds either with a struct literal needs
  `..Default::default()`.
- In C, `parsec_create_element` and `parsec_attr_set` refuse those
  names with `PARSEC_NO_NODE` and `PARSEC_ERR_INVALID`; in Python,
  `create_node` and setting an attribute raise `ValueError`; in
  JavaScript, `setAttribute` throws (#136).

### Parser

- Nested `<template>`s at the end of the input no longer recurse, so no
  page can overflow the stack: a hundred thousand of them parse on a
  256 KB thread. Nothing else in the parser recurses on what a page
  holds either (#136).
- A page full of NUL bytes, each of which becomes three, can no longer
  push the parse buffer past 2^30 bytes, where it panicked or read
  another string (#136).
- Checking a tag's attributes for duplicates is linear, and the time
  budget fires inside a long run of them: 100,000 attributes on one tag
  took 12.8 s and take 10 ms. Attributes merged onto `<body>` and
  `<html>` were quadratic the same way (#136).
- `</body>`, `</html>` and heading end tags under very deep nesting,
  `:has()` with more than one compound, and nested `:is()` no longer
  take quadratic time, and `line_col` keeps a line index instead of
  counting from the start each call (#136).
- `pretty` stops indenting past 100 levels, so a deep tree can't make a
  string of gigabytes, and writes the newline the parser drops after
  `<pre>`, `<textarea>` and `<listing>` (#136).
- `max_depth: Some(0)` keeps the `<html>` element's children at depth 1,
  as `Some(1)` does, instead of dropping them (#136).

### XPath

- A scope past the end of the document, a node made after the index,
  an empty index, a subtree index used from outside it, or another
  document's index is an `XPathError` instead of a panic, or, for the
  last, an answer from the wrong document (#136).
- The work limit counts `lang()`'s climbs and string values' text, and
  is lower, so no single query runs for tens of seconds (#136).

### CSS

- A stylesheet or `style` attribute nested tens of thousands deep no
  longer overflows the stack. Functions, blocks and rules nest at most
  `MAX_NESTING` (256) deep, counted together; anything deeper is dropped
  as it's read, and a rule dropped that way is an `InvalidRule` error.
  The cascade looks 128 rules deep
  ([#128](https://github.com/parsec-html/parsec/pull/128)).
- `var()` substitution copies in at most 1 MiB, so custom properties
  that double each other can't take gigabytes and tens of seconds. Past
  that the value is invalid at computed-value time (#128).
- Nested style rules' selectors, with their parents written in, stop at
  64 KiB, and a lone parent is written in without `:is()`, which matched
  in time that doubled per level of nesting (#128).
- `@layer` names are looked up in a map and may have 64 parts, so a long
  dotted name or deep `@layer` nesting no longer takes minutes (#128).
- `parse_block_contents` on rules with no `;` between them, `a{}a{}…`,
  is linear instead of quadratic, and so is `parse_stylesheet_deep` on
  the same inside a block (#128).
- `cargo doc` builds every crate with warnings denied, in CI too (#128).

### C API

- With `PARSEC_SPANS` or `PARSEC_ERRORS`, spans and error offsets index
  the bytes you passed even when they aren't valid UTF-8; before, they
  counted the U+FFFD copy and could point past your buffer
  ([#129](https://github.com/parsec-html/parsec/pull/129)).
- `parsec_line_col` with an offset near `SIZE_MAX` gives an answer
  instead of a panic or a wrong one (#129).
- A `parsec_select` callback that changes the document gets
  `PARSEC_ERR_BUSY`; one that frees it has it freed when `parsec_select`
  returns (#129).
- `parsec.h` defines `PARSEC_VERSION` and its major, minor and patch
  parts, and says what's undefined about `parsec_string_free` and what
  ends the process (#129).

### URL

- A host label over 4,000 code points (`punycode::MAX_LABEL`) fails
  instead of taking seconds of Punycode work. All 4,565 WPT URL cases
  still pass (#129).

### Python

- Wheels for CPython 3.14 and 3.15 with the GIL on macOS and Windows,
  which 0.1.0 didn't have, and the release job fails if any expected
  wheel is missing ([#132](https://github.com/parsec-html/parsec/pull/132)).
- Wheels and the sdist carry `LICENSE` and `NOTICE`, and PyPI lists each
  supported Python version (#132).
- `Node.strip_tags()` and `Node.unwrap_tags()` include the node itself
  when it matches, as selectolax does (#132).
- The docs say what `decompose()` refuses, how `line_col()` counts
  without positions, and how XPath numbers become strings next to lxml
  (#132).

### WebAssembly

- The TypeScript types compile without `lib: esnext.disposable`
  ([#130](https://github.com/parsec-html/parsec/pull/130)).
- `package.json` exports `./package.json` and has a `main` (#130).
- The README says when to call `free()` or use `using`: in a loop that
  doesn't yield, the garbage collector never gets to run (#130).

### Packaging and docs

- crates.io pages show the header image, and their links work
  ([#131](https://github.com/parsec-html/parsec/pull/131)).
- The crates no longer carry their test suites and data (#131).
- The README's claims are corrected: two dependencies, not one; what
  can still end a C program; which inputs are still quadratic; the
  WebAssembly size; and WPT totals with their skipped cases (#130).

Fixes to the parser core itself arrive with the next sync from Netweir,
which adds them to this entry.

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
