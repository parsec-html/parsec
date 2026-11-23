#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Builds the npm package into bindings/wasm/pkg: the web build at the top,
# which browsers and bundlers load with `await init()`, and the Node build
# in node/, which loads its .wasm itself. npm/package.json picks between them,
# and in Node, npm/index.cjs picks the native build (bindings/node) over
# node/ when the platform's package is installed.
set -eu
cd "$(dirname "$0")"
rm -rf pkg
wasm-pack build --release --target web --weak-refs --out-dir pkg --out-name parsec_html
wasm-pack build --release --target nodejs --weak-refs --out-dir pkg/node --out-name parsec_html
# wasm-pack writes a package.json and .gitignore into each; ours replaces
# them, and node/ is CommonJS inside an ES module package.
rm -f pkg/.gitignore pkg/node/.gitignore pkg/node/README.md
cp npm/package.json npm/index.cjs npm/index.mjs npm/index.d.ts README.md ../../LICENSE ../../NOTICE pkg/
printf '{ "type": "commonjs" }\n' > pkg/node/package.json
# The classes have [Symbol.dispose] for `using`, which TypeScript knows only
# with the esnext.disposable lib; this brings it in for a project that
# doesn't list it, rather than failing with TS2550.
for d in pkg/parsec_html.d.ts pkg/node/parsec_html.d.ts; do
  { printf '/// <reference lib="esnext.disposable" />\n'; cat "$d"; } > "$d.tmp"
  mv "$d.tmp" "$d"
done
