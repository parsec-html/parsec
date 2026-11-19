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
- On the document: `root`, `documentElement`, `head`, `body`, `html`,
  `text`, `select`, `selectFirst`, `xpath`, and `errors`.
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

Nodes hold their document, so a node stays usable after edits. You don't
have to call `free()`: the package is built with weak references, so the
garbage collector frees what you drop. Call it, or use `using`, if you
want the memory back sooner.

Not there yet: setting `innerHTML`, and creating nodes.

## Size

The WebAssembly is 869 KB, 361 KB gzipped; XPath and spans are 110 KB of that, and
parse errors 21 KB. It's built for speed per byte
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
