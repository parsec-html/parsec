# parsec-html for Node.js, native

The npm package `parsec-html` is WebAssembly, and WebAssembly runs
anywhere. On the machines most people run Node on, though, you can do
better: this is Parsec built for the machine with
[napi-rs](https://napi.rs), and in Node the package loads it whenever
there's a build for your platform. It parses about 1.5 times as fast,
and it can parse on a worker thread while your code carries on.

You don't install it yourself. `npm install parsec-html` brings the
build for your platform along as an optional dependency, and the code
you wrote for the WebAssembly runs unchanged:

```js
import { parse, parseAsync, backend } from "parsec-html";

const doc = parse("<ul><li class=hit>one<li>two</ul>");
doc.selectFirst("li.hit").textContent; // "one"

backend; // "native", or "wasm" where there's no build for the platform
```

## Builds

| Platform | Package |
|---|---|
| Linux x86-64, glibc | `@parsec-web/binding-linux-x64-gnu` |
| Linux ARM64, glibc | `@parsec-web/binding-linux-arm64-gnu` |
| Linux x86-64, musl (Alpine) | `@parsec-web/binding-linux-x64-musl` |
| Linux ARM64, musl (Alpine) | `@parsec-web/binding-linux-arm64-musl` |
| macOS x86-64 | `@parsec-web/binding-darwin-x64` |
| macOS ARM64 | `@parsec-web/binding-darwin-arm64` |
| Windows x86-64 | `@parsec-web/binding-win32-x64-msvc` |
| Windows ARM64 | `@parsec-web/binding-win32-arm64-msvc` |

CI builds each one on its own platform and runs the package's tests
against it there. Anywhere else, and in browsers and bundlers, you get
the WebAssembly.

## The same API

Every function, class, getter and option is the WebAssembly package's,
with the same names and the same error messages: `parse`,
`parseFragment`, `parseBytes`, `ByteParser`, `select` and `selectFirst`
with `maxSteps`, `xpath`, spans and errors. The [WebAssembly
README](../wasm/README.md) documents them all. The few differences:

- **`parseAsync(html, options)`** gives a promise of what `parse` gives,
  and parses on one of Node's worker threads (the libuv pool), so a
  server keeps answering while a 15 MB page parses. With the
  WebAssembly build it parses before it returns, since that has no
  threads to hand the work to.
- **Memory.** A native document belongs to the garbage collector, like
  any JavaScript object. `free()` and `using` still work, so code
  written for the WebAssembly doesn't break, but they do nothing.
- **`backend`** says which build loaded.

To choose the build yourself, set `PARSEC_BACKEND` to `wasm` (skip the
native build) or `native` (throw if it can't load).

## Speed

Parsing the 13 pages in `bench/` (24 MB: Wikipedia, MDN, GitHub, the
HTML spec and others), in Node 22 on an Apple M2 Max, MB/s:

| Build | All pages | Range per page |
|---|---:|---:|
| Native | 249 | 211 to 639 |
| WebAssembly | 165 | 112 to 498 |

That's 1.51 times overall, and 1.28 to 1.90 times page by page. Both
builds take a JavaScript string, so both pay to turn it into UTF-8;
on the HTML spec that's about 12 of the 69 ms.

## Working on it

```sh
cd bindings/wasm && ./build.sh                 # the package, in pkg/
cd ../node && cargo build --release            # the addon
node npm.mjs platform darwin-arm64 target/release/libparsec_html_node.dylib out
cd ../wasm
mkdir -p node_modules/@parsec-web
ln -s ../pkg node_modules/parsec-html
ln -s ../../../node/out/darwin-arm64 node_modules/@parsec-web/binding-darwin-arm64
PARSEC_BACKEND=native node --test tests/package.test.mjs
PARSEC_BACKEND=wasm node --test tests/package.test.mjs
node ../node/bench.mjs ../../bench/corpus     # after bench/fetch-pages.sh
```

Use your platform's name and library (`libparsec_html_node.so`,
`parsec_html_node.dll`) in place of macOS's. `src/lib.rs` mirrors
`bindings/wasm/src/lib.rs`, so a change to the API goes in both.
