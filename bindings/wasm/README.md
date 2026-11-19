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

- `parse(html, { scripting, maxDepth })` for a whole document, and
  `parseFragment(html, context, options)` for markup inside an element,
  as `innerHTML` would parse it (`parseFragment("<td>1", "tr")`).
- On the document: `root`, `documentElement`, `head`, `body`, `html`,
  `text`, `select`, `selectFirst`.
- On a node, DOM names wherever the DOM has one: `nodeType`, `nodeName`,
  `localName`, `parentNode`, `firstChild`, `lastChild`, `nextSibling`,
  `previousSibling`, `childNodes`, `children`, `content` (a template's),
  `getAttribute`, `hasAttribute`, `getAttributeNames`, `setAttribute`,
  `removeAttribute`, `id`, `textContent`, `outerHTML`, `innerHTML`,
  `remove`, `appendChild`, `insertBefore`, `isSameNode`, and `select`,
  `selectFirst` and `matches` for selectors.

Selectors are Level 4: `:has()`, `:is()`, `:not()` with lists,
`:nth-child(… of S)`, `:lang()`, `:dir()`, the form states and more. A
selector that doesn't parse throws.

Nodes hold their document, so a node stays usable after edits. You don't
have to call `free()`: the package is built with weak references, so the
garbage collector frees what you drop. Call it, or use `using`, if you
want the memory back sooner.

Not there yet: setting `innerHTML`, creating nodes, and source positions,
which arrive with [#8](https://github.com/parsec-html/parsec/issues/8).

## Size

The WebAssembly is 662 KB, 279 KB gzipped. It's built for speed per byte
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
