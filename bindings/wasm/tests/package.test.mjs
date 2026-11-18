// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// The npm package as someone installs it: run `./build.sh`, then
// `npm install --no-save ./pkg` and `node --test tests/package.test.mjs`.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import test from "node:test";

import { parse, parseFragment } from "parsec-html";

test("Node gets the build that loads itself", () => {
  const doc = parse("<ul><li class=hit>one<li>two</ul>");
  const hits = doc.select("li.hit");
  assert.equal(hits.length, 1);
  assert.equal(hits[0].textContent, "one");
  assert.equal(doc.selectFirst("li + li").outerHTML, "<li>two</li>");
  assert.equal(doc.body.firstChild.nodeName, "UL");
  assert.throws(() => doc.select("li["), /expected/);
});

test("options and fragments", () => {
  const doc = parse("<noscript><p>x</p></noscript>", { scripting: true });
  assert.equal(doc.selectFirst("p"), undefined);
  const deep = parse("<div>".repeat(50), { maxDepth: 5 });
  assert.equal(deep.select("div").length, 50);
  assert.equal(parseFragment("<td>1", "tr").html, "<td>1</td>");
});

test("CommonJS require works too", () => {
  const require = createRequire(import.meta.url);
  const { parse } = require("parsec-html");
  assert.equal(parse("<p>hi").text, "hi");
});

test("the web build runs from bytes", async () => {
  // What a browser or bundler gets, loaded here by hand.
  const url = new URL("../node_modules/parsec-html/parsec_html.js", import.meta.url);
  const web = await import(url.href);
  web.initSync({ module: await readFile(new URL("parsec_html_bg.wasm", url)) });
  assert.equal(web.parse("<b>x</b>").selectFirst("b").innerHTML, "x");
});

test("the types ship with it", async () => {
  const types = await readFile(new URL("../node_modules/parsec-html/parsec_html.d.ts", import.meta.url), "utf8");
  assert.match(types, /export function parse\(html: string, options\?: ParseOptions \| null\): Document;/);
  assert.match(types, /selectFirst\(selector: string\): Node \| undefined;/);
});
