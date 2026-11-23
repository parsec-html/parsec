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

Node doesn't need that step: it gets a build that loads itself. On
Linux, macOS and Windows that's a native build with the same API, which
parses about 1.5 times as fast and adds `parseAsync`, parsing on a
worker thread; see [bindings/node](../node/README.md).

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

Most selectors cost one walk of the tree. A few cost far more on a page
built to make them: `div:has(span div)` over a few thousand nested divs
is millions of steps. If you run selectors or pages you don't control,
give `select`, `selectFirst` and `matches` a `maxSteps`. Past it they
throw an Error that names `maxSteps`, and you never get half the
matches. There's no time limit, because wasm32 has no clock to read; a
step count gives the same answer on every machine anyway.

```js
doc.select(selector, { maxSteps: 10_000_000 });
```

XPath is 1.0. `doc.xpath("//li")` gives an array of nodes, with
attributes as their values (`doc.xpath("//a/@href")` is an array of
strings) and namespace nodes as their URIs, and `count()`, `string()` or `boolean()` give a number, a
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

Nodes hold their document, so a node stays usable after edits, and a
document stays in memory for as long as any node from it does. The
package is built with weak references, so the garbage collector frees
what you drop. It gets round to that between turns of the event loop,
though, and a loop that never yields gives it no turn. Parse sixty 2 MB
pages in one, reading a node from each, and WebAssembly memory reaches
1.4 GB; at 4 GB it traps. Free each node and its document when you're
done with them, or declare them with `using`, and the same loop stays at
40 MB:

```js
for (const page of pages) {
  using doc = parse(page);
  using title = doc.selectFirst("title");
  titles.push(title?.textContent);
}
```

Freeing the document alone isn't enough while a node from it is still
alive. And WebAssembly memory only grows: what a free gives back is room
for the next page, not memory for the system, so the most you ever had
parsed at once stays reserved until the program ends.

## JavaScript

`parseJs(source, { module, maxDepth, maxSteps })` gives a script's
ESTree as plain objects, the same tree acorn gives, with `start` and
`end` as indexes into the string. JavaScript that doesn't parse throws a
`SyntaxError` with `code`, `line`, `column`, `start`, `end` and a
`frame` to print:

```js
import { parseJs, pageState } from "parsec-html";

const tree = parseJs("export const a = [1, 2];", { module: true });
try {
  parseJs("let x = ;");
} catch (e) {
  console.log(e.code, e.line, e.column); // "unexpected-token" 1 9
}
```

`maxDepth` (at most and by default 256) and `maxSteps` bound a script
you don't trust: too deep throws a `SyntaxError` with the code
`too-deep`, and past the steps, an Error that names `maxSteps`.

`pageState(source)` reads the state a script keeps, without running
it: `window.__NUXT__ = …`, a top-level `var ytInitialData = …`,
`JSON.parse("…")`, Nuxt 2's function form and Next.js's `self.__next_f`
pushes. `doc.pageState()` does it for every inline script on a parsed
page. Each piece is `{ name, value }`, or `{ name, error: { code, source
} }` when the value would need code to run.

Not there yet: setting `innerHTML`, and creating nodes.

## XML

`parseXml(text, options)` and `parseXmlBytes(bytes, options)` parse XML
1.0 with namespaces into an `XmlDocument`. Bytes are decoded from a byte
order mark, else the encoding the XML declaration names, else UTF-8, and
bytes that aren't that encoding are an error rather than U+FFFD.

```js
import { parseXml } from "parsec-html";

const doc = parseXml(atom);
const feed = doc.documentElement;
feed.namespaceURI;                                        // "http://www.w3.org/2005/Atom"
for (const entry of feed.children) {
  entry.getAttributeNS("http://www.w3.org/1999/xlink", "href");
}
doc.xml;                                                  // the document, written back out
```

A node has the DOM's names: `nodeType` (5 is a reference to an entity
Parsec didn't read), `nodeName` as written (`atom:link`), `localName`,
`prefix`, `namespaceURI`, `nodeValue`, `textContent`, `parentNode`,
`firstChild`, `lastChild`, `nextSibling`, `previousSibling`,
`childNodes`, `children`, `getAttribute`, `getAttributeNS`,
`hasAttribute`, `getAttributeNames` and `isSameNode`. `attributes` is
an array of `XmlAttribute`s with `name`, `localName`, `prefix`,
`namespaceURI`, `value`, `specified` and `span`; `xml` writes a node
back out, and an element written alone brings the namespace
declarations it needs. On the document: `root` (the document node),
`documentElement`, `characterSet`, `xml`, and `warnings`, which lists
what the parser didn't read, such as an external DTD. Spans count as
they do for HTML.

Nothing is fetched, ever. XML that isn't well-formed throws a
`SyntaxError` with `code`, `line` and `column`:

```js
try {
  parseXml("<a>\n  <b></a>");
} catch (e) {
  `${e.line}:${e.column}: ${e.code}`;                     // "2:6: mismatched-end-tag"
}
```

So does XML past its limits, which you can tighten: `maxDepth` (2,048
unless you give one), `maxEntityDepth` (16), and entity expansion of
`entityAllowance` bytes (1 MiB) plus `entityRatio` (8) per byte of
input. A billion laughs throws `entity-expansion-limit` straight away.
`{ namespaces: false }` reads names as they are, colons and all.

## Size

The WebAssembly is 1,325,774 bytes, 597 KB gzipped, as `main` builds it
(each CI run's summary has the sizes of its own build); XPath and spans
are about 110 KB of that, parse errors 21 KB, and the decoders for bytes 201 KB (142 KB
gzipped), most of it the tables of the Chinese, Japanese and Korean
encodings. It's built for speed per byte
(`opt-level = "s"`, then `wasm-opt -Os`): building for size alone (`"z"`)
saved 59 KB but parsed the 13 pages in `bench/` 40% slower.

The JavaScript parser is 193 KB of the package's `.wasm` (79 KB gzipped)
and XML is 113 KB (44 KB gzipped). Built with `--no-default-features`,
the `.wasm` is the HTML parser alone, and CI fails if that build grows
past its ceiling:

```sh
wasm-pack build --release --target web -- --no-default-features
```

## Working on it

```sh
cd bindings/wasm
wasm-pack test --node
wasm-pack test --headless --firefox --test browser
./build.sh                      # the package, in pkg/
mkdir -p node_modules && ln -s ../pkg node_modules/parsec-html
node --test tests/package.test.mjs tests/xml.test.mjs
```

The [WASM page](https://github.com/parsec-html/parsec/wiki/WASM) on the
wiki has more.
