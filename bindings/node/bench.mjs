// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// Parse throughput in Node, native against WebAssembly, on the pages
// bench/fetch-pages.sh downloads. Build the package and install the
// platform's as bindings/node/README.md says, then:
//
//   node bindings/node/bench.mjs [CORPUS_DIR]
//
// Each figure is MB/s of the page's UTF-8, the median of 30 parses of a
// string, after 5 to warm up.
import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";

const require = createRequire(import.meta.url);
const builds = {
  native: require("../wasm/pkg/index.cjs"),
  wasm: require("../wasm/pkg/node/parsec_html.js"),
};
if (builds.native.backend !== "native") throw new Error("the native build didn't load");

const dir = process.argv[2] ?? new URL("../../bench/corpus/", import.meta.url).pathname;
const median = (xs) => xs.sort((a, b) => a - b)[xs.length >> 1];

function rate(build, html, mb) {
  const times = [];
  for (let i = 0; i < 35; i++) {
    const start = process.hrtime.bigint();
    const doc = build.parse(html);
    const ns = Number(process.hrtime.bigint() - start);
    // The WebAssembly build's memory only comes back when freed.
    doc.free();
    if (i >= 5) times.push(ns);
  }
  return mb / (median(times) / 1e9);
}

const rows = [];
let total = 0;
const sums = { native: 0, wasm: 0 };
for (const file of readdirSync(dir).filter((f) => f.endsWith(".html")).sort()) {
  const html = readFileSync(join(dir, file), "utf8");
  const mb = Buffer.byteLength(html) / 1e6;
  const native = rate(builds.native, html, mb);
  const wasm = rate(builds.wasm, html, mb);
  total += mb;
  // Time per page, summed, for the total.
  sums.native += mb / native;
  sums.wasm += mb / wasm;
  rows.push([file.replace(/\.html$/, ""), mb.toFixed(2), native.toFixed(0), wasm.toFixed(0), (native / wasm).toFixed(2)]);
}
rows.push(["all pages", total.toFixed(2), (total / sums.native).toFixed(0), (total / sums.wasm).toFixed(0), (sums.wasm / sums.native).toFixed(2)]);

console.log(`Node ${process.version}, ${process.platform}-${process.arch}: parse, MB/s`);
console.log("| Page | MB | Native | WebAssembly | Native / WebAssembly |");
console.log("|---|---:|---:|---:|---:|");
for (const r of rows) console.log(`| ${r.join(" | ")} |`);
