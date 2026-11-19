#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// Turns web-platform-tests' cascade tests (css/css-cascade) into data for
// tests/wpt_cascade.rs, copying each test file that gives any beside its
// cases. Run by scripts/fetch-wpt-cascade.sh:
//
//   node scripts/wpt-cascade-cases.js WPT_CHECKOUT tests/wpt-cascade
//
// Most of these tests add a <style> from script, read an element's
// computed style, and remove the <style> again. In a sandbox with no
// DOM, adding and removing <style> elements is recorded, an element is
// remembered by the selector the test found it with and its place in the
// matches, and `getComputedStyle(element)[property]` gives a probe; an
// assert_equals on a probe is a case: these sheets on top of the file's
// own, this element, this property, this value. Anything else a test
// does throws, and the test is counted as skipped with why.
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

const [src, out] = process.argv.slice(2);
if (!src || !out) {
  console.error('usage: wpt-cascade-cases.js WPT_CHECKOUT OUT_DIR');
  process.exit(2);
}

class Unsupported extends Error {}
const unsupported = (what) => {
  throw new Unsupported(what);
};

function sandboxFor(file) {
  const state = { cases: [], skipped: {}, test: null, probes: new Map(), styles: [] };
  const skip = (why) => {
    state.skipped[why] = (state.skipped[why] || 0) + 1;
  };
  let nextProbe = 0;
  const element = (selector, index) =>
    new Proxy(
      { __el: { selector, index } },
      {
        get(t, p) {
          if (p === '__el') return t.__el;
          if (p === 'classList') return [];
          if (p === 'id') return '';
          unsupported(`element.${String(p)}`);
        },
        set(_, p) {
          unsupported(`changing element.${String(p)}`);
        },
      }
    );
  const styleElement = () => {
    const el = {
      __text: '',
      set textContent(v) { el.__text = String(v); },
      set innerText(v) { el.__text = String(v); },
      set innerHTML(v) { el.__text = String(v); },
      appendChild(n) { el.__text += n.__text || ''; },
      remove() { state.styles = state.styles.filter((s) => s !== el); },
      setAttribute(n) { unsupported(`style.setAttribute(${n})`); },
    };
    return el;
  };
  const head = {
    append(el) { state.styles.push(el); },
    appendChild(el) { state.styles.push(el); return el; },
  };
  const document = {
    head,
    createElement: (t) => (t === 'style' ? styleElement() : unsupported(`a <${t}>`)),
    createTextNode: (t) => ({ __text: String(t) }),
    querySelectorAll: (s) => {
      // As many as the runner finds; the test only iterates them.
      const list = [];
      for (let i = 0; i < 8; i++) list.push(element(s, i));
      list.__selector = s;
      return list;
    },
    querySelector: (s) => element(s, 0),
    getElementById: (id) => element(`[id="${id}"]`, 0),
    get body() { return element('body', 0); },
    get documentElement() { return element('html', 0); },
  };
  const computed = (el) => {
    if (!el || !el.__el) unsupported('getComputedStyle of something not found by a selector');
    const read = (prop) => {
      const token = `\u0000probe${nextProbe++}`;
      state.probes.set(token, { element: el.__el, property: prop });
      return token;
    };
    return new Proxy(
      {},
      {
        get(_, p) {
          if (p === 'getPropertyValue') return (prop) => read(prop);
          if (typeof p !== 'string') return undefined;
          return read(p.replace(/[A-Z]/g, (c) => '-' + c.toLowerCase()));
        },
      }
    );
  };
  const sandbox = {
    document,
    getComputedStyle: computed,
    test(fn, name) {
      state.test = name;
      const before = state.cases.length;
      try {
        fn({ add_cleanup() {} });
        if (state.cases.length === before) skip('asserts nothing about a computed style');
      } catch (e) {
        state.cases.length = before;
        skip(e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`);
      }
      state.test = null;
    },
    promise_test: () => skip('runs asynchronously'),
    async_test: () => skip('runs asynchronously'),
    setup() {},
    done() {},
    assert_equals(actual, expected) {
      const probe = state.probes.get(actual);
      if (!probe) unsupported('an assertion on something other than a computed style');
      if (state.probes.has(expected)) unsupported('comparing two computed styles');
      // Only elements found by a selector of the markup: the runner
      // takes the nth match.
      state.cases.push({
        file,
        sheets: state.styles.map((s) => s.__text),
        element: probe.element,
        property: probe.property,
        expect: expected,
      });
    },
    assert_not_equals: () => unsupported('assert_not_equals'),
    assert_true: () => unsupported('assert_true'),
    assert_false: () => unsupported('assert_false'),
    console: { log() {} },
  };
  sandbox.window = sandbox;
  return { sandbox, state };
}

function extract(rel) {
  const html = fs.readFileSync(path.join(src, rel), 'utf8');
  const entry = { file: rel };
  if (!/testharness\.js/.test(html)) return Object.assign(entry, { skipped_because: 'not a testharness test' });
  const { sandbox, state } = sandboxFor(rel);
  // Elements with ids are globals, as in a browser.
  for (const [, id] of html.matchAll(/\sid\s*=\s*["']?([A-Za-z_$][\w$-]*)/g)) {
    if (/^[A-Za-z_$][\w$]*$/.test(id) && !(id in sandbox)) sandbox[id] = sandbox.document.getElementById(id);
  }
  const context = vm.createContext(sandbox);
  let error = null;
  for (const [, attrs, code] of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script[^>]*>/gi)) {
    if (/\bsrc\s*=/.test(attrs)) {
      if (!/testharness/.test(attrs)) {
        error = `needs ${/src\s*=\s*["']?([^"'\s>]+)/.exec(attrs)[1]}`;
        break;
      }
      continue;
    }
    if (/type\s*=\s*["']?module/.test(attrs)) {
      error = 'needs a module script';
      break;
    }
    try {
      vm.runInContext(code, context, { timeout: 5000, filename: rel });
    } catch (e) {
      error = e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`;
      break;
    }
  }
  if (state.cases.length === 0) {
    return Object.assign(entry, { skipped_because: error || Object.keys(state.skipped)[0] || 'no static cases' });
  }
  if (error) state.skipped[`the rest of the file ${error}`] = 1;
  return Object.assign(entry, { cases: state.cases, skipped: state.skipped });
}

const dir = path.join(src, 'css/css-cascade');
const files = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith('.html') && !/-ref\.html$/.test(f))
  .sort()
  .map((f) => path.join('css/css-cascade', f));
const results = files.map(extract);
fs.mkdirSync(out, { recursive: true });
for (const r of results) {
  if (!r.cases) continue;
  fs.mkdirSync(path.join(out, path.dirname(r.file)), { recursive: true });
  fs.copyFileSync(path.join(src, r.file), path.join(out, r.file));
}
fs.writeFileSync(
  path.join(out, 'cases.json'),
  JSON.stringify({ commit: fs.readFileSync(path.join(src, 'COMMIT'), 'utf8').trim(), files: results }, null, 1) + '\n'
);
const ran = results.filter((f) => f.cases);
console.error(`${ran.length} files with ${ran.reduce((n, f) => n + f.cases.length, 0)} cases, ${results.length - ran.length} files with none`);
