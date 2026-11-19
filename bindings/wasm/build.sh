#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Builds the npm package into bindings/wasm/pkg: the web build at the top,
# which browsers and bundlers load with `await init()`, and the Node build
# in node/, which loads its .wasm itself. npm/package.json picks between them.
set -eu
cd "$(dirname "$0")"
rm -rf pkg
wasm-pack build --release --target web --weak-refs --out-dir pkg --out-name parsec_html
wasm-pack build --release --target nodejs --weak-refs --out-dir pkg/node --out-name parsec_html
# wasm-pack writes a package.json and .gitignore into each; ours replaces
# them, and node/ is CommonJS inside an ES module package.
rm -f pkg/.gitignore pkg/node/.gitignore pkg/node/README.md
cp npm/package.json README.md ../../LICENSE ../../NOTICE pkg/
printf '{ "type": "commonjs" }\n' > pkg/node/package.json
