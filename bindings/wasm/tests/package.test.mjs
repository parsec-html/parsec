// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// The npm package as someone installs it: run `./build.sh`, then
// `npm install --no-save ./pkg` and `node --test tests/package.test.mjs`.

import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import test from "node:test";

import { ByteParser, parse, parseBytes, parseFragment } from "parsec-html";

test("Node gets the build that loads itself", () => {
  const doc = parse("<ul><li class=hit>one<li>two</ul>");
  const hits = doc.select("li.hit");
  assert.equal(hits.length, 1);
  assert.equal(hits[0].textContent, "one");
  assert.equal(doc.selectFirst("li + li").outerHTML, "<li>two</li>");
  assert.equal(doc.body.firstChild.nodeName, "UL");
  assert.throws(() => doc.select("li["), /expected/);
});

test("a removed node stays usable, as the DOM's remove() leaves it", () => {
  const doc = parse("<div><p>one <b>two</b></p></div><section></section>");
  const p = doc.selectFirst("p");
  const b = doc.selectFirst("b");
  p.remove();
  assert.equal(doc.selectFirst("div").outerHTML, "<div></div>");
  assert.equal(p.outerHTML, "<p>one <b>two</b></p>");
  assert.ok(b.parentNode.isSameNode(p));
  doc.selectFirst("section").appendChild(p);
  assert.equal(doc.selectFirst("section").textContent, "one two");
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
  assert.match(types, /export function parseBytes\(bytes: Uint8Array \| number\[\], options\?: ByteParseOptions \| null\): Document;/);
  assert.match(types, /export interface ByteParseOptions extends ParseOptions/);
  assert.match(types, /feed\(chunk: Uint8Array \| number\[\]\): void;/);
  assert.match(types, /readonly encodingConfidence: "certain" \| "tentative" \| undefined;/);
  // `[Symbol.dispose]` needs this lib, which a project may not list.
  for (const build of ["parsec_html.d.ts", "node/parsec_html.d.ts"]) {
    const d = await readFile(new URL(`../node_modules/parsec-html/${build}`, import.meta.url), "utf8");
    assert.ok(d.startsWith('/// <reference lib="esnext.disposable" />\n'), build);
  }
});

test("package.json can be imported, and main is the Node build", () => {
  const require = createRequire(import.meta.url);
  const pkg = require("parsec-html/package.json");
  assert.equal(pkg.name, "parsec-html");
  // Tools that predate "exports" go by main: the same file Node's require gets.
  assert.ok(require.resolve("parsec-html").endsWith(pkg.main.slice(2)), pkg.main);
});

test("bytes, sniffed as a browser sniffs them", () => {
  // 日本 in Shift_JIS, named by a <meta>.
  const sjis = new Uint8Array([...new TextEncoder().encode("<meta charset=sjis><p>"), 0x93, 0xfa, 0x96, 0x7b]);
  const doc = parseBytes(sjis);
  assert.equal(doc.text, "日本");
  assert.equal(doc.characterSet, "Shift_JIS");
  assert.equal(doc.encodingConfidence, "certain");
  const e9 = new Uint8Array([0x3c, 0x70, 0x3e, 0xc3, 0xa9]); // <p>é in UTF-8
  // Valid UTF-8 with no declaration is taken for UTF-8, tentatively.
  assert.deepEqual([parseBytes(e9).text, parseBytes(e9).characterSet], ["é", "UTF-8"]);
  // Bytes that aren't UTF-8 fall back to windows-1252.
  assert.equal(parseBytes(new Uint8Array([0x3c, 0x70, 0x3e, 0xe9])).text, "é");
  assert.equal(parseBytes(e9).encodingConfidence, "tentative");
  const utf8 = parseBytes(e9, { transportEncoding: "utf-8" });
  assert.deepEqual([utf8.text, utf8.characterSet, utf8.encodingConfidence], ["é", "UTF-8", "certain"]);
  assert.equal(parseBytes(sjis.slice(19), { defaultEncoding: "shift_jis" }).text, "日本");
  // A label that names no encoding is passed over, as browsers do.
  assert.equal(parseBytes(new Uint8Array([0xe9]), { transportEncoding: "nope" }).characterSet, "windows-1252");
  assert.throws(() => parseBytes(e9, { defaultEncoding: "nope" }), /defaultEncoding/);
  assert.equal(parse("<p>").characterSet, "UTF-8");
  assert.equal(parse("<p>").encodingConfidence, undefined);

  const parser = new ByteParser({ transportEncoding: "shift_jis" });
  for (const piece of [sjis.slice(0, 23), sjis.slice(23)]) parser.feed(piece);
  const streamed = parser.finish();
  assert.equal(streamed.text, "日本");
  assert.throws(() => parser.feed(e9), /finished/);
});

test("spans for bytes are byte offsets into them", () => {
  const enc = (s) => new TextEncoder().encode(s);
  // <p>日本</p><b>x</b> in Shift_JIS: 4 bytes for the kanji, 6 decoded.
  const page = new Uint8Array([...enc("<meta charset=shift_jis><p class=a>"), 0x93, 0xfa, 0x96, 0x7b, ...enc("</p>\n<b>x</b></i>")]);
  const ascii = (s) => String.fromCharCode(...page.slice(s.start, s.end));
  const whole = parseBytes(page, { spans: true, errors: true });
  const parser = new ByteParser({ spans: true, errors: true });
  for (let i = 0; i < page.length; i += 3) parser.feed(page.slice(i, i + 3));
  for (const doc of [whole, parser.finish()]) {
    const p = doc.selectFirst("p");
    assert.equal(ascii(p.span), "<p class=a>");
    assert.equal(ascii(p.endSpan), "</p>");
    assert.deepEqual([...page.slice(p.firstChild.span.start, p.firstChild.span.end)], [0x93, 0xfa, 0x96, 0x7b]);
    const b = doc.selectFirst("b").span;
    assert.equal(ascii(b), "<b>");
    assert.deepEqual([b.line, b.column], [2, 1]);
    const stray = doc.errors.find((e) => e.code === "unexpected-end-tag");
    assert.equal(ascii({ start: stray.offset, end: stray.offset + 4 }), "</i>");
    assert.deepEqual([stray.line, stray.column], [2, 9]);
  }
});
