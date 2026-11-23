// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// XML from the npm package as someone installs it, on the native build
// or the WebAssembly one, as package.test.mjs says.

import assert from "node:assert/strict";
import test from "node:test";

import { XmlDocument, XmlNode, parseXml, parseXmlBytes } from "parsec-html";

const ATOM = "http://www.w3.org/2005/Atom";
const XLINK = "http://www.w3.org/1999/xlink";
const FEED =
  `<feed xmlns="${ATOM}" xmlns:x="${XLINK}">` +
  '<entry x:href="/one" n="1">Ships &amp; ports</entry>' +
  "<?pi data?><!--c--><x:link><![CDATA[<raw>]]></x:link></feed>";

const LAUGHS =
  '<!DOCTYPE l [<!ENTITY a "lollollollollollollollollollol">' +
  [..."abcdefgh"].map((p, i) => `<!ENTITY ${"bcdefghi"[i]} "${`&${p};`.repeat(10)}">`).join("") +
  "]><l>&i;</l>";

test("a namespaced document walks", () => {
  const doc = parseXml(FEED);
  assert.ok(doc instanceof XmlDocument);
  const feed = doc.documentElement;
  assert.ok(feed instanceof XmlNode);
  assert.equal(doc.root.nodeType, 9);
  assert.ok(feed.parentNode.isSameNode(doc.root));
  assert.equal(doc.root.parentNode, undefined);
  assert.deepEqual([feed.nodeName, feed.localName, feed.prefix, feed.namespaceURI], ["feed", "feed", undefined, ATOM]);
  const [entry, pi, comment, link] = feed.childNodes;
  assert.equal(feed.children.length, 2);
  assert.ok(entry.nextSibling.isSameNode(pi));
  assert.ok(link.previousSibling.isSameNode(comment));
  assert.equal(link.nextSibling, undefined);
  assert.deepEqual([pi.nodeType, pi.nodeName, pi.nodeValue], [7, "pi", "data"]);
  assert.deepEqual([comment.nodeType, comment.nodeName], [8, "#comment"]);
  assert.deepEqual([link.nodeName, link.localName, link.prefix, link.namespaceURI], ["x:link", "link", "x", XLINK]);
  assert.deepEqual([link.firstChild.nodeType, link.firstChild.nodeValue], [4, "<raw>"]);
  assert.equal(entry.textContent, "Ships & ports");
  assert.equal(doc.characterSet, "UTF-8");
});

test("attributes with prefixes", () => {
  const feed = parseXml(FEED).documentElement;
  const entry = feed.firstChild;
  const [href, n] = entry.attributes;
  assert.deepEqual(
    [href.name, href.localName, href.prefix, href.namespaceURI, href.value, href.specified],
    ["x:href", "href", "x", XLINK, "/one", true],
  );
  assert.equal(FEED.slice(href.span.start, href.span.end), 'x:href="/one"');
  assert.deepEqual([n.prefix, n.namespaceURI], [undefined, undefined]);
  assert.equal(feed.attributes[1].namespaceURI, "http://www.w3.org/2000/xmlns/");
  assert.equal(entry.getAttribute("x:href"), "/one");
  assert.equal(entry.getAttribute("href"), undefined);
  assert.equal(entry.getAttributeNS(XLINK, "href"), "/one");
  assert.equal(entry.getAttributeNS(null, "n"), "1");
  assert.ok(entry.hasAttribute("n"));
  assert.deepEqual(entry.getAttributeNames(), ["x:href", "n"]);
});

test("serializing gives back what was parsed", () => {
  const doc = parseXml(FEED);
  assert.equal(doc.xml, FEED);
  assert.equal(parseXml(doc.xml).xml, FEED);
  assert.equal(
    doc.documentElement.lastChild.xml,
    `<x:link xmlns="${ATOM}" xmlns:x="${XLINK}"><![CDATA[<raw>]]></x:link>`,
  );
});

test("a well-formedness error has its code and place", () => {
  assert.throws(
    () => parseXml("<a>\n  <b></a>"),
    (e) =>
      e instanceof SyntaxError &&
      e.code === "mismatched-end-tag" &&
      e.line === 2 &&
      e.column === 6 &&
      e.message === "`</a>` closes `<b>`",
  );
});

test("a billion laughs is an error, not a hang", () => {
  assert.throws(() => parseXml(LAUGHS), { code: "entity-expansion-limit" });
  assert.throws(() => parseXml("<a><b><c/></b></a>", { maxDepth: 2 }), { code: "too-deep" });
  const small = '<!DOCTYPE a [<!ENTITY e "12345">]><a>&e;&e;</a>';
  assert.equal(parseXml(small).documentElement.textContent, "1234512345");
  assert.throws(() => parseXml(small, { entityAllowance: 0, entityRatio: 0 }), { code: "entity-expansion-limit" });
  assert.throws(() => parseXml("<a/>", { maxDepth: -1 }), /maxDepth/);
  assert.equal(parseXml("<p:a/>", { namespaces: false }).documentElement.localName, "p:a");
});

test("bytes in the encoding they declare", () => {
  const src = Buffer.from("<?xml version='1.0' encoding='ISO-8859-1'?>\n<!DOCTYPE p SYSTEM 'p.dtd'><p>café</p>", "latin1");
  const doc = parseXmlBytes(src);
  assert.equal(doc.characterSet, "windows-1252");
  const p = doc.documentElement;
  assert.equal(p.textContent, "café");
  const { start, end, line, column } = p.span;
  assert.equal(src.subarray(start, end).toString("latin1"), "<p>café</p>");
  assert.deepEqual([line, column], [2, 28]);
  assert.equal(p.previousSibling.nodeType, 10);
  assert.deepEqual(doc.warnings.map((w) => w.code), ["external-dtd-not-loaded"]);
  assert.throws(() => parseXmlBytes(new Uint8Array([0x3c, 0x70, 0x3e, 0xff])), { code: "invalid-byte-sequence" });
});

test("spans in a string count UTF-16", () => {
  const src = "<a>\u{1F600}<b x='1'/></a>";
  const { start, end, column } = parseXml(src).documentElement.lastChild.span;
  assert.equal(src.slice(start, end), "<b x='1'/>");
  assert.equal(column, 5);
});

test("free and Symbol.dispose are there on either build", () => {
  const doc = parseXml("<a b='1'/>");
  for (const o of [doc, doc.documentElement, doc.documentElement.attributes[0]]) {
    assert.equal(typeof o.free, "function");
    assert.equal(typeof o[Symbol.dispose], "function");
  }
});
