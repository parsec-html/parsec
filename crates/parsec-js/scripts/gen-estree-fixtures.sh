#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
#
# Writes tests/estree/*.json: acorn's tree for each tests/estree/*.js,
# parsed as a script with ecmaVersion 2025, and each *.mjs, parsed as a
# module, which tests/estree.rs checks parsec-js's ESTree output against.
# locations.js gets `loc` too.
#
# Needs Node and npm. acorn is pinned here; a new version is a new
# fixture set, reviewed as one.
#
#   crates/parsec-js/scripts/gen-estree-fixtures.sh

set -eu
ACORN=8.19.0
here=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
(cd "$tmp" && npm init -y > /dev/null && npm install --silent --no-audit --no-fund "acorn@$ACORN")
ACORN_DIR="$tmp/node_modules/acorn" node - "$here/tests/estree" <<'JS'
const acorn = require(process.env.ACORN_DIR);
const fs = require("fs");
const path = require("path");
const dir = process.argv[2];
// JSON has no BigInt or RegExp; acorn's values for them become null, and
// their parts are in `bigint` and `regex`.
const json = (k, v) => (typeof v === "bigint" || v instanceof RegExp ? null : v);
for (const name of fs.readdirSync(dir).filter((n) => /\.m?js$/.test(n)).sort()) {
  const src = fs.readFileSync(path.join(dir, name), "utf8");
  const ast = acorn.parse(src, {
    ecmaVersion: 2025,
    sourceType: name.endsWith(".mjs") ? "module" : "script",
    locations: name === "locations.js",
  });
  const out = path.join(dir, name.replace(/\.m?js$/, ".json"));
  fs.writeFileSync(out, JSON.stringify(ast, json, 1) + "\n");
  console.log(`${name}: acorn ${acorn.version}`);
}
JS
