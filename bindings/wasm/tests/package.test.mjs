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

test("spans", () => {
  const src = "<p class=x>h\u00e9llo</p>\n<div>x</div>";
  const doc = parse(src, { spans: true });
  const p = doc.selectFirst("p");
  const text = (s) => src.slice(s.start, s.end);
  assert.equal(text(p.span), "<p class=x>");
  assert.equal(text(p.endSpan), "</p>");
  assert.equal(text(p.attributeSpan("CLASS")), "class=x");
  assert.equal(text(p.firstChild.span), "h\u00e9llo");
  const div = doc.selectFirst("div").span;
  assert.deepEqual([div.line, div.column], [2, 1]);
  assert.equal(doc.body.span, undefined);
  assert.equal(parse(src).selectFirst("p").span, undefined);
});

test("spans slice the string that was parsed, past the BMP too", () => {
  const src = "<p>caf\u00e9</p><b>\u65e5\u672c</b><i title='\u{1F600}'>\u{1F600}x</i>";
  const doc = parse(src, { spans: true });
  const slice = (s) => src.slice(s.start, s.end);
  for (const [tag, start, end] of [["p", "<p>", "</p>"], ["b", "<b>", "</b>"], ["i", "<i title='\u{1F600}'>", "</i>"]]) {
    const n = doc.selectFirst(tag);
    assert.equal(slice(n.span), start);
    assert.equal(slice(n.endSpan), end);
  }
  const i = doc.selectFirst("i");
  assert.equal(slice(i.attributeSpan("title")), "title='\u{1F600}'");
  assert.equal(slice(i.firstChild.span), "\u{1F600}x");
  // The column counts each of the two emoji once, where the index counts
  // each twice.
  assert.equal(i.endSpan.column, i.endSpan.start + 1 - 2);
  const errors = parse("\u{1F600}</b>", { errors: true }).errors;
  assert.equal("\u{1F600}</b>".slice(errors[1].offset), "</b>");
});

test("errors", () => {
  const doc = parse("<p>a\n</b>", { errors: true });
  assert.deepEqual(
    doc.errors.map((e) => [e.code, e.offset, e.line, e.column]),
    [["expected-doctype-but-got-start-tag", 0, 1, 1], ["unexpected-end-tag", 5, 2, 1]],
  );
  assert.deepEqual(parse("</b>").errors, []);
});

test("xpath", () => {
  const doc = parse("<ul id=l><li class=a>one<li>two</ul>");
  const items = doc.xpath("//li");
  assert.deepEqual(items.map((n) => n.textContent), ["one", "two"]);
  assert.deepEqual(doc.xpath("//@id | //@class"), ["l", "a"]);
  assert.equal(doc.xpath("//li/text()")[1].nodeType, 3);
  assert.equal(doc.xpath("count(//li)"), 2);
  assert.equal(doc.xpath("name(//*[@class])"), "li");
  assert.equal(doc.xpath("boolean(//ul)"), true);
  assert.equal(items[1].xpath("string(preceding-sibling::li)"), "one");
  assert.throws(() => doc.xpath("//li["), /XPath/);
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
  assert.match(types, /xpath\(expr: string\): XPathResult;/);
  assert.match(types, /readonly span: Span \| undefined;/);
});
