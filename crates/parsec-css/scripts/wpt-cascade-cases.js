#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// Turns web-platform-tests' cascade and at-rule tests into data for
// tests/wpt_cascade.rs, copying each test file that gives any beside its
// cases, with the stylesheets it imports. Run by
// scripts/fetch-wpt-cascade.sh:
//
//   node scripts/wpt-cascade-cases.js WPT_CHECKOUT tests/wpt-cascade
//
// The files are css/css-cascade (layers, `@scope`, `@import`),
// css/css-conditional/container-queries, css/css-properties-values-api
// (`@property`), css/css-counter-styles/counter-style-at-rule and
// css/css-pseudo (pseudo-elements' computed values).
//
// Most of these tests add a <style> from script, read an element's
// computed style or a rule of a sheet through CSSOM, and remove the
// <style> again. In a sandbox with no DOM, adding and removing <style>
// elements is recorded, a <template>'s contents put into an element is
// recorded, an element is remembered by the selector the test found it
// with (and the element it was looked for under) and its place in the
// matches, and `getComputedStyle(element, pseudo)[property]`, a sheet's
// `cssRules.length` and a rule's attributes give probes; an assertion on
// a probe is a case. Anything else a test does throws, and the test is
// counted as skipped with why. The tests' own helper scripts run in the
// sandbox, as they would in a browser.
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

const same = (a, b) => JSON.stringify(a) === JSON.stringify(b);

// A test given up on can still fail later; it's been counted as skipped.
process.on('unhandledRejection', () => {});

function sandboxFor(file, html) {
  const state = {
    cases: [],
    skipped: {},
    test: null,
    probes: new Map(),
    styles: [],
    inserts: [],
    asyncTests: [],
    script: null,
  };
  const skip = (why) => {
    state.skipped[why] = (state.skipped[why] || 0) + 1;
  };
  let nextProbe = 0;
  const probe = (what) => {
    const token = `\u0000probe${nextProbe++}`;
    state.probes.set(token, what);
    return token;
  };
  // Each <template>'s place, for `script.previousElementSibling`.
  const templates = [];
  for (const m of html.matchAll(/<template\b[^>]*>[\s\S]*?<\/template\s*>/gi)) {
    templates.push({ start: m.index, end: m.index + m[0].length });
  }
  // The page's own <style>s, as document.styleSheets has them, and the
  // declarations a test sets on their rules through `rule.style`.
  const pageStyles = [...html.matchAll(/<style\b[^>]*>([\s\S]*?)<\/style\s*>/gi)].map((m) => m[1]);
  const edits = new Map();
  const declarations = (sheetIndex, index) =>
    new Proxy(
      {},
      {
        get(_, p) {
          unsupported(`reading rule.style.${String(p)}`);
        },
        set(_, p, value) {
          const key = `${sheetIndex}/${index}`;
          if (!edits.has(key)) edits.set(key, new Map());
          const name = p === 'cssFloat' ? 'float' : String(p).replace(/[A-Z]/g, (c) => '-' + c.toLowerCase());
          if (value === '') edits.get(key).delete(name);
          else edits.get(key).set(name, String(value));
          return true;
        },
      }
    );
  // A page <style>'s text with the declarations set on its rules put at
  // the end of each rule's block.
  const restyled = (sheetIndex) => {
    const text = pageStyles[sheetIndex];
    const at = [];
    let depth = 0;
    let rules = 0;
    for (let i = 0; i < text.length; i++) {
      const c = text[i];
      if (c === '/' && text[i + 1] === '*') {
        const end = text.indexOf('*/', i + 2);
        i = end < 0 ? text.length : end + 1;
      } else if (c === '"' || c === "'") {
        while (++i < text.length && text[i] !== c) if (text[i] === '\\') i++;
      } else if (c === '{') {
        depth++;
      } else if (c === '}' && depth > 0 && --depth === 0) {
        at[rules++] = i;
      } else if (c === ';' && depth === 0) {
        rules++;
      }
    }
    let out = text;
    const changed = [...edits.keys()]
      .map((k) => k.split('/').map(Number))
      .filter(([s, r]) => s === sheetIndex && at[r] !== undefined && edits.get(`${s}/${r}`).size)
      .sort((a, b) => b[1] - a[1]);
    for (const [s, r] of changed) {
      const decls = [...edits.get(`${s}/${r}`)].map(([n, v]) => `; ${n}: ${v}`).join('');
      out = out.slice(0, at[r]) + decls + out.slice(at[r]);
    }
    return out;
  };
  const rule = (sheet, index, sheetIndex) =>
    new Proxy(
      {},
      {
        get(_, p) {
          if (typeof p !== 'string') return undefined;
          if (p === 'style') {
            if (sheetIndex === undefined) unsupported('a rule\'s style in a sheet added from script');
            return declarations(sheetIndex, index);
          }
          if (p === 'constructor') return { name: probe({ kind: 'rule', sheet, index, attr: 'constructor.name' }) };
          if (p === 'cssText') {
            return {
              indexOf: (text) => probe({ kind: 'contains', sheet, index, text }),
              toString: () => unsupported('rule.cssText as text'),
            };
          }
          if (p === 'cssRules') unsupported('a rule\'s rules');
          return probe({ kind: 'rule', sheet, index, attr: p });
        },
      }
    );
  const sheetOf = (el) => {
    const rules = () => {
      const text = el.__text;
      return new Proxy([], {
        get(_, p) {
          if (p === 'length') return probe({ kind: 'rules', sheet: text });
          if (typeof p === 'string' && /^\d+$/.test(p)) return rule(text, Number(p));
          if (p === 'item') return (i) => rule(text, i);
          unsupported(`sheet.cssRules.${String(p)}`);
        },
      });
    };
    return {
      get cssRules() { return rules(); },
      get rules() { return rules(); },
      insertRule: () => unsupported('insertRule'),
    };
  };
  const addStyle = (el) => {
    state.styles.push(el);
    if (typeof el.onload === 'function') Promise.resolve().then(() => el.onload());
  };
  const insertedInto = (desc) => state.inserts.filter((i) => same(i.into, desc));
  const element = (desc) =>
    new Proxy(
      { __el: desc },
      {
        get(t, p) {
          if (p === '__el') return t.__el;
          if (p === 'classList') return [];
          if (p === 'id') return '';
          if (p === 'querySelector') return (s) => element({ selector: s, index: 0, within: desc });
          if (p === 'querySelectorAll') {
            return (s) => {
              const list = [];
              for (let i = 0; i < 8; i++) list.push(element({ selector: s, index: i, within: desc }));
              return list;
            };
          }
          if (p === 'append' || p === 'appendChild') {
            return (...nodes) => {
              for (const n of nodes) {
                if (n && n.__fragment !== undefined) state.inserts.push({ into: desc, template: n.__fragment });
                else if (n && n.__style) {
                  n.__parent = desc;
                  addStyle(n);
                }
                else unsupported('appending something other than a <template>\'s contents or a <style>');
              }
              return nodes[0];
            };
          }
          const clear = () => {
            state.inserts = state.inserts.filter((i) => !same(i.into, desc));
            state.styles = state.styles.filter((s) => !same(s.__parent, desc));
          };
          if (p === 'replaceChildren') return clear;
          if (p === 'firstChild') {
            const any = insertedInto(desc).length || state.styles.some((s) => same(s.__parent, desc));
            return any ? { remove: clear } : null;
          }
          unsupported(`element.${String(p)}`);
        },
        set(_, p) {
          unsupported(`changing element.${String(p)}`);
        },
      }
    );
  const styleElement = () => {
    const el = {
      __style: true,
      __text: '',
      onload: null,
      onerror: null,
      set textContent(v) { el.__text = String(v); },
      get textContent() { return el.__text; },
      set innerText(v) { el.__text = String(v); },
      set innerHTML(v) { el.__text = String(v); },
      appendChild(n) { el.__text += n.__text || ''; },
      remove() { state.styles = state.styles.filter((s) => s !== el); },
      get sheet() { return sheetOf(el); },
      addEventListener(type, f) {
        if (type === 'load') el.onload = f;
      },
      setAttribute(n) { unsupported(`style.setAttribute(${n})`); },
    };
    return el;
  };
  const pageSheet = (i) => {
    if (i >= pageStyles.length) unsupported('a sheet that isn\'t a <style>');
    const rules = () =>
      new Proxy([], {
        get(_, p) {
          if (p === 'length') return probe({ kind: 'rules', sheet: pageStyles[i] });
          if (typeof p === 'string' && /^\d+$/.test(p)) return rule(pageStyles[i], Number(p), i);
          unsupported(`sheet.cssRules.${String(p)}`);
        },
      });
    return {
      get cssRules() { return rules(); },
      insertRule: () => unsupported('insertRule'),
    };
  };
  const document = {
    get styleSheets() {
      if (state.styles.length) unsupported('document.styleSheets with sheets added');
      return new Proxy([], {
        get(_, p) {
          if (p === 'length') return pageStyles.length;
          if (typeof p === 'string' && /^\d+$/.test(p)) return pageSheet(Number(p));
          unsupported(`document.styleSheets.${String(p)}`);
        },
      });
    },
    head: element({ selector: 'head', index: 0 }),
    createElement: (t) => (t === 'style' ? styleElement() : unsupported(`a <${t}>`)),
    createTextNode: (t) => ({ __text: String(t) }),
    querySelectorAll: (s) => {
      // As many as the runner finds; the test only iterates them.
      const list = [];
      for (let i = 0; i < 8; i++) list.push(element({ selector: s, index: i }));
      list.__selector = s;
      return list;
    },
    querySelector: (s) => element({ selector: s, index: 0 }),
    getElementById: (id) => element({ selector: `[id="${id}"]`, index: 0 }),
    get body() { return element({ selector: 'body', index: 0 }); },
    get documentElement() { return element({ selector: 'html', index: 0 }); },
    get currentScript() {
      const at = state.script;
      // The <template> right before this <script>, if there is one.
      const t = templates.findIndex((x) => x.end <= at && /^\s*$/.test(html.slice(x.end, at)));
      return {
        get previousElementSibling() {
          if (t < 0) unsupported('a script\'s sibling that isn\'t a <template>');
          return { tagName: 'TEMPLATE', content: { cloneNode: () => ({ __fragment: t }) } };
        },
      };
    },
  };
  const computed = (el, pseudo) => {
    if (!el || !el.__el) unsupported('getComputedStyle of something not found by a selector');
    if (pseudo !== undefined && pseudo !== null && typeof pseudo !== 'string') unsupported('a pseudo-element that isn\'t a string');
    const read = (prop) =>
      probe(Object.assign({ kind: 'computed', element: el.__el }, pseudo ? { pseudo } : {}, { property: prop }));
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
  const record = (pr, expect, like) => {
    const base = { file, sheets: state.styles.map((s) => s.__text) };
    if (state.inserts.length) base.inserts = state.inserts.slice();
    const restyle = pageStyles.map((_, i) => ({ index: i, text: restyled(i) })).filter((r) => r.text !== pageStyles[r.index]);
    if (restyle.length) base.restyle = restyle;
    if (pr.kind === 'computed') {
      const read = { element: pr.element };
      if (pr.pseudo) read.pseudo = pr.pseudo;
      read.property = pr.property;
      // What another read gives, where the test expects that.
      if (like) read.like = { element: like.element, pseudo: like.pseudo, property: like.property };
      state.cases.push(Object.assign(base, read, { expect }));
    } else {
      state.cases.push(Object.assign(base, pr, { expect }));
    }
  };
  const harness = (cleanups) => ({
    add_cleanup(f) { cleanups.push(f); },
    step_func: (f) => f,
    step_func_done: (f) => f,
  });
  const runCleanups = (cleanups) => {
    for (const f of cleanups.reverse()) {
      try { f(); } catch (e) { /* as a browser would, carry on */ }
    }
  };
  const sandbox = {
    document,
    getComputedStyle: computed,
    CSS: {
      supports: () => true,
      escape: (s) => s,
      registerProperty: () => unsupported('CSS.registerProperty'),
    },
    setTimeout: (f) => Promise.resolve().then(f),
    requestAnimationFrame: (f) => Promise.resolve().then(f),
    test(fn, name) {
      state.test = name;
      const before = state.cases.length;
      const cleanups = [];
      try {
        fn(harness(cleanups));
        if (state.cases.length === before) skip('asserts nothing about a computed style or a rule');
      } catch (e) {
        state.cases.length = before;
        skip(e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`);
      }
      runCleanups(cleanups);
      state.test = null;
    },
    promise_test(fn, name) {
      state.asyncTests.push({ fn, name });
    },
    async_test: () => skip('runs asynchronously, step by step'),
    setup() {},
    done() {},
    assert_implements() {},
    assert_equals(actual, expected) {
      const a = state.probes.get(actual);
      if (!a) {
        if (state.probes.has(expected)) unsupported('a probe on the right');
        if (!same(actual, expected)) throw new Error(`${actual} != ${expected}`);
        return;
      }
      if (state.probes.has(expected)) {
        const e = state.probes.get(expected);
        if (a.kind !== 'computed' || e.kind !== 'computed') unsupported('comparing two probes');
        return record(a, null, e);
      }
      if (a.kind === 'contains') {
        if (expected !== -1) unsupported('where in a rule\'s text something is');
        return record(a, false);
      }
      record(a, expected);
    },
    assert_not_equals(actual, expected) {
      const a = state.probes.get(actual);
      if (a && a.kind === 'contains' && expected === -1) return record(a, true);
      unsupported('assert_not_equals');
    },
    assert_true: () => unsupported('assert_true'),
    assert_false: () => unsupported('assert_false'),
    console: { log() {} },
  };
  sandbox.window = sandbox;
  return { sandbox, state, skip, runCleanups, harness };
}

async function extract(rel) {
  const html = fs.readFileSync(path.join(src, rel), 'utf8');
  const entry = { file: rel };
  if (!/testharness\.js/.test(html)) return Object.assign(entry, { skipped_because: 'not a testharness test' });
  const { sandbox, state, skip, runCleanups, harness } = sandboxFor(rel, html);
  // Elements with ids are globals, as in a browser.
  for (const [, id] of html.matchAll(/\sid\s*=\s*["']?([A-Za-z_$][\w$-]*)/g)) {
    if (/^[A-Za-z_$][\w$]*$/.test(id) && !(id in sandbox)) sandbox[id] = sandbox.document.getElementById(id);
  }
  const context = vm.createContext(sandbox);
  let error = null;
  for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script[^>]*>/gi)) {
    const [, attrs, code] = m;
    if (/\bsrc\s*=/.test(attrs)) {
      if (/testharness/.test(attrs)) continue;
      // The test's own helpers, from the checkout.
      const ref = /src\s*=\s*["']?([^"'\s>]+)/.exec(attrs)[1];
      const file = ref.startsWith('/') ? path.join(src, ref) : path.join(src, path.dirname(rel), ref);
      if (!fs.existsSync(file)) {
        error = `needs ${ref}`;
        break;
      }
      try {
        vm.runInContext(fs.readFileSync(file, 'utf8'), context, { timeout: 5000, filename: ref });
      } catch (e) {
        error = `needs ${ref} to run: ${e.message}`;
        break;
      }
      continue;
    }
    if (/type\s*=\s*["']?module/.test(attrs)) {
      error = 'needs a module script';
      break;
    }
    state.script = m.index;
    try {
      vm.runInContext(code, context, { timeout: 5000, filename: rel });
    } catch (e) {
      error = e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`;
      break;
    }
  }
  // Asynchronous tests, one after another, each given a second.
  for (const { fn, name } of state.asyncTests) {
    state.test = name;
    const before = state.cases.length;
    const cleanups = [];
    try {
      const run = Promise.resolve().then(() => fn(harness(cleanups)));
      let timer;
      const late = new Promise((_, no) => {
        timer = setTimeout(() => no(new Unsupported('waiting on more than a load')), 1000);
      });
      await Promise.race([run, late]).finally(() => clearTimeout(timer));
      if (state.cases.length === before) skip('asserts nothing about a computed style or a rule');
    } catch (e) {
      state.cases.length = before;
      skip(e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`);
    }
    runCleanups(cleanups);
  }
  if (state.cases.length === 0) {
    return Object.assign(entry, { skipped_because: error || Object.keys(state.skipped)[0] || 'no static cases' });
  }
  if (error) state.skipped[`the rest of the file ${error}`] = 1;
  return Object.assign(entry, { cases: state.cases, skipped: state.skipped });
}

const dirs = [
  'css/css-cascade',
  'css/css-conditional/container-queries',
  'css/css-properties-values-api',
  'css/css-counter-styles/counter-style-at-rule',
  'css/css-pseudo',
];

(async () => {
  const files = [];
  for (const d of dirs) {
    for (const f of fs.readdirSync(path.join(src, d)).sort()) {
      if (f.endsWith('.html') && !/-(not)?ref\.html$/.test(f)) files.push(path.join(d, f));
    }
  }
  const results = [];
  for (const f of files) results.push(await extract(f));
  fs.mkdirSync(out, { recursive: true });
  const copy = (rel) => {
    fs.mkdirSync(path.join(out, path.dirname(rel)), { recursive: true });
    fs.copyFileSync(path.join(src, rel), path.join(out, rel));
  };
  for (const r of results) {
    if (!r.cases) continue;
    copy(r.file);
    // What its sheets import, beside it.
    const texts = [fs.readFileSync(path.join(src, r.file), 'utf8'), ...r.cases.flatMap((c) => c.sheets || [])];
    for (const text of texts) {
      for (const [, url] of text.matchAll(/@import\s+(?:url\(\s*)?["']?([^"')\s;]+)/g)) {
        if (/^(data:|https?:)/.test(url)) continue;
        const rel = path.join(path.dirname(r.file), url);
        if (fs.existsSync(path.join(src, rel))) copy(rel);
      }
    }
  }
  fs.writeFileSync(
    path.join(out, 'cases.json'),
    JSON.stringify({ commit: fs.readFileSync(path.join(src, 'COMMIT'), 'utf8').trim(), files: results }, null, 1) + '\n'
  );
  const ran = results.filter((f) => f.cases);
  console.error(`${ran.length} files with ${ran.reduce((n, f) => n + f.cases.length, 0)} cases, ${results.length - ran.length} files with none`);
})();
