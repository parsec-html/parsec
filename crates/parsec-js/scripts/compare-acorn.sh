#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Parses JavaScript files with acorn and with parsec-js, and says which
# give a different ESTree tree; a `.mjs` file is a module, the rest are
# scripts. With no files, it takes every test262 script and module that
# should parse and that acorn parses too.
#
# Needs Node and npm; acorn is pinned to the version the fixtures in
# tests/estree come from.
#
#   crates/parsec-js/scripts/compare-acorn.sh
#   crates/parsec-js/scripts/compare-acorn.sh bundle.js other.js

set -eu
ACORN=8.19.0
here=$(cd "$(dirname "$0")/.." && pwd)
cargo build --quiet --release --example estree --manifest-path "$here/Cargo.toml"
bin=$(cargo metadata --format-version 1 --manifest-path "$here/Cargo.toml" | sed 's/.*"target_directory":"\([^"]*\)".*/\1/')/release/examples/estree
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && npm init -y > /dev/null && npm install --silent --no-audit --no-fund "acorn@$ACORN")
ACORN_DIR="$tmp/node_modules/acorn" BIN="$bin" TEST262="$here/tests/test262/test" node - "$@" <<'JS'
const acorn = require(process.env.ACORN_DIR);
const fs = require("fs");
const path = require("path");
const cp = require("child_process");
const json = (k, v) => (typeof v === "bigint" || v instanceof RegExp ? null : v);

let files = process.argv.slice(2).map((f) => [f, f.endsWith(".mjs")]);
if (files.length === 0) {
  const root = process.env.TEST262;
  const walk = (d) => fs.readdirSync(d, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(path.join(d, e.name)) : [path.join(d, e.name)]);
  const past2025 = /decorators|explicit-resource-management|source-phase-imports|import-defer|export-defer/;
  files = walk(root).flatMap((f) => {
    if (!f.endsWith(".js") || f.endsWith("_FIXTURE.js")) return [];
    const meta = (/\/\*---([\s\S]*?)---\*\//.exec(fs.readFileSync(f, "utf8")) || ["", ""])[1];
    if (/phase:\s*parse/.test(meta) || /flags:.*onlyStrict/.test(meta) || past2025.test(meta)) return [];
    return [[f, /flags:.*module/.test(meta)]];
  });
}

const theirs = { false: [], true: [] };
for (const [f, module] of files) {
  try {
    const src = fs.readFileSync(f, "utf8");
    const sourceType = module ? "module" : "script";
    theirs[module].push([f, JSON.parse(JSON.stringify(acorn.parse(src, { ecmaVersion: 2025, sourceType }), json))]);
  } catch (e) {
    // acorn rejects it: there's nothing to compare.
  }
}

function difference(a, b, at) {
  if (a === null || b === null || typeof a !== "object" || typeof b !== "object") {
    return a === b ? null : `${at}: ${JSON.stringify(b)}, not ${JSON.stringify(a)}`;
  }
  for (const k of new Set([...Object.keys(a), ...Object.keys(b)])) {
    const d = difference(a[k], b[k], `${at}.${k}`);
    if (d) return d;
  }
  return null;
}

let same = 0;
const different = [];
for (const module of [false, true]) for (let i = 0; i < theirs[module].length; i += 500) {
  const chunk = theirs[module].slice(i, i + 500);
  const args = (module ? ["--module"] : []).concat(chunk.map((c) => c[0]));
  const out = cp.spawnSync(process.env.BIN, args, { encoding: "utf8", maxBuffer: 1 << 30 });
  const lines = out.stdout.split("\n");
  chunk.forEach(([f, tree], j) => {
    const d = difference(tree, JSON.parse(lines[j]), "");
    if (d) different.push(`${f}: ${d}`);
    else same++;
  });
}
console.log(different.slice(0, 50).join("\n"));
console.log(`acorn ${acorn.version}: ${same} the same, ${different.length} different`);
process.exit(different.length ? 1 : 0);
JS
