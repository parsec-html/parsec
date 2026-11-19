# parsec-html for JavaScript

Parsec is an HTML parser in Rust that reads pages the way a browser does,
finds what you want with CSS selectors, and lets you change the tree and
write it back out. This package is Parsec compiled to WebAssembly, with
TypeScript types, for Node and for browsers.

```js
import { parse } from "parsec-html";

const doc = parse("<ul><li class=hit>one<li>two</ul>");
doc.selectFirst("li.hit").textContent; // "one"
for (const li of doc.select("li")) console.log(li.outerHTML);
```

In a browser or a bundler, load the WebAssembly first:

```js
import init, { parse } from "parsec-html";
await init();
```

Node doesn't need that step: it gets a build that loads itself.

## What's in it

- `parse(html, { scripting, maxDepth, spans, errors })` for a whole document, and
  `parseFragment(html, context, options)` for markup inside an element,
  as `innerHTML` would parse it (`parseFragment("<td>1", "tr")`).
- `parseBytes(bytes, { transportEncoding, defaultEncoding, ...options })`
  for a page as a `Uint8Array`, and `new ByteParser(options)` with
  `feed(bytes)` and `finish()` for one that arrives in pieces: see
  [Bytes](#bytes).
- On the document: `root`, `documentElement`, `head`, `body`, `html`,
  `text`, `select`, `selectFirst`, `xpath`, `errors`, and
  `characterSet` and `encodingConfidence`.
- On a node, DOM names wherever the DOM has one: `nodeType`, `nodeName`,
  `localName`, `parentNode`, `firstChild`, `lastChild`, `nextSibling`,
  `previousSibling`, `childNodes`, `children`, `content` (a template's),
  `getAttribute`, `hasAttribute`, `getAttributeNames`, `setAttribute`,
  `removeAttribute`, `id`, `textContent`, `outerHTML`, `innerHTML`,
  `remove`, `appendChild`, `insertBefore`, `isSameNode`, `select`,
  `selectFirst` and `matches` for selectors, `xpath`, and `span`,
  `endSpan` and `attributeSpan(name)` for where it was written.

Selectors are Level 4: `:has()`, `:is()`, `:not()` with lists,
`:nth-child(… of S)`, `:lang()`, `:dir()`, the form states and more. A
selector that doesn't parse throws.

XPath is 1.0. `doc.xpath("//li")` gives an array of nodes, with
attributes as their values (`doc.xpath("//a/@href")` is an array of
strings), and `count()`, `string()` or `boolean()` give a number, a
string or a boolean.

```js
const doc = parse(html, { spans: true, errors: true });
const a = doc.selectFirst("a");
html.slice(a.span.start, a.span.end); // the <a ...> tag as it was written
doc.errors.map((e) => `${e.line}:${e.column}: ${e.code}`);
```

A span's `start` and `end` index the string you parsed, in UTF-16 units
as JavaScript strings do, so `html.slice(start, end)` is the source even
past the Basic Multilingual Plane. Its `line` and `column` count from 1,
and the column counts characters, so an emoji is one column but two
units. An error's `offset` indexes the string the same way.

## Bytes

Hand over what `fetch` gave you and Parsec finds the encoding the way a
browser does: a byte order mark first, then the `charset` from the
`Content-Type`, which you pass as `transportEncoding`, then a `<meta>`
in the page, then UTF-8 if the bytes are valid UTF-8, then
`defaultEncoding`, windows-1252 unless you say otherwise.

```js
const res = await fetch(url);
const charset = /charset=([^;]+)/i.exec(res.headers.get("content-type") ?? "")?.[1];
const doc = parseBytes(new Uint8Array(await res.arrayBuffer()), { transportEncoding: charset });
doc.characterSet;        // "Shift_JIS", as document.characterSet says
doc.encodingConfidence;  // "certain" or "tentative", as the HTML Standard says
```

Streamed, with the same result however the bytes are cut:

```js
const parser = new ByteParser({ transportEncoding: charset });
for await (const chunk of res.body) parser.feed(chunk);
const doc = parser.finish();
```

A page that's bytes gets spans and error offsets in bytes too: indexes
into the `Uint8Array`, in whatever encoding it was, so
`bytes.subarray(span.start, span.end)` is the tag as it was sent. For a
string, `characterSet` is `"UTF-8"`, as `DOMParser` says, and
`encodingConfidence` is undefined. A `transportEncoding` the Encoding
Standard doesn't know is passed over, as browsers pass over a bad
`charset`; a `defaultEncoding` it doesn't know throws.

Nodes hold their document, so a node stays usable after edits. You don't
have to call `free()`: the package is built with weak references, so the
garbage collector frees what you drop. Call it, or use `using`, if you
want the memory back sooner.

Not there yet: setting `innerHTML`, and creating nodes.

## Size

The WebAssembly is 1,071 KB, 504 KB gzipped; XPath and spans are 110 KB
of that, parse errors 21 KB, and the decoders for bytes 201 KB (142 KB
gzipped), most of it the tables of the Chinese, Japanese and Korean
encodings. It's built for speed per byte
(`opt-level = "s"`, then `wasm-opt -Os`): building for size alone (`"z"`)
saved 59 KB but parsed the 13 pages in `bench/` 40% slower.

## Working on it

```sh
cd bindings/wasm
wasm-pack test --node
wasm-pack test --headless --firefox --test browser
./build.sh                      # the package, in pkg/
mkdir -p node_modules && ln -s ../pkg node_modules/parsec-html
node --test tests/package.test.mjs
```

The [WASM page](https://github.com/parsec-html/parsec/wiki/WASM) on the
wiki has more.
