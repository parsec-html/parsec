#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// Turns web-platform-tests' media query tests (css/mediaqueries) into data
// for tests/wpt_media.rs. Run by scripts/fetch-wpt-cascade.sh:
//
//   node scripts/wpt-media-cases.js WPT_CHECKOUT tests/wpt-media
//
// Each file's scripts run in a Node sandbox with no DOM. What a test
// asks of the browser goes through a probe: a query applying (the
// `media` attribute of a style element, or `matchMedia().matches`), a
// query parsing (`mediaText` not being `not all`), or a query's
// serialization (`matchMedia().media`). A probe in a test answers
// "yes", and what the test then asserts says what it expects: a test
// that wants the query not to apply asserts false. The viewport and
// screen size the test set are recorded with each case. A test that
// probes more than once, or asserts anything else, is skipped and
// counted with why.
//
// Where a file asks the browser outside a test, to find out its colour
// depth or resolution before testing around it, the probe answers for
// the environment the Rust runner gives: 8 bits a colour, 96dpi.
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

const [src, out] = process.argv.slice(2);
if (!src || !out) {
  console.error('usage: wpt-media-cases.js WPT_CHECKOUT OUT_DIR');
  process.exit(2);
}

const SCREEN = { width: 1280, height: 720 };

class Unsupported extends Error {}
const unsupported = (what) => {
  throw new Unsupported(what);
};

// Answers for a query asked outside any test, for the environment the
// runner gives (Environment::default in src/media.rs).
function answer(q) {
  let m;
  if ((m = /^(?:all and )?\((min|max)-(color|monochrome): *(\d+)\)$/.exec(q))) {
    const have = m[2] === 'color' ? 8 : 0;
    return m[1] === 'min' ? have >= +m[3] : have <= +m[3];
  }
  if ((m = /^\((min|max)-resolution: *(\d+)dpi\)$/.exec(q))) {
    return m[1] === 'min' ? 96 >= +m[2] : 96 <= +m[2];
  }
  unsupported(`an answer for ${q} outside a test`);
}

function sandboxFor(file) {
  const state = { cases: [], skipped: {}, test: null, probes: [], asserts: [] };
  const skip = (why) => {
    state.skipped[why] = (state.skipped[why] || 0) + 1;
  };
  const env = { width: null, height: null };
  const px = (v) => {
    const m = /^(\d+(?:\.\d+)?)(px)?$/.exec(String(v).trim());
    if (!m) unsupported(`a size of ${v}`);
    return +m[1];
  };
  const probe = (kind, query) => {
    if (state.test === null) return answer(query);
    state.probes.push({ kind, query, env: { ...env } });
    return true;
  };

  let media = '';
  let parseText = '';
  const styleEl = { setAttribute: (n, v) => { if (n === 'media') media = v; } };
  const bodyCs = {
    getPropertyValue: (p) => {
      if (p !== 'text-decoration-line') unsupported(`computed ${p}`);
      return probe('applies', media) ? 'underline' : 'none';
    },
  };
  const subdoc = {
    getElementById: () => styleEl,
    body: {},
    defaultView: { getComputedStyle: () => bodyCs },
  };
  const iframeStyle = {
    set width(v) { env.width = px(v); },
    set height(v) { env.height = px(v); },
  };
  const textNode = { set data(v) { parseText = v; } };
  const parseSheet = {
    cssRules: [
      {
        type: 4,
        media: {
          get mediaText() {
            const q = parseText.replace(/^@media screen, /, '').replace(/ \{\}$/, '');
            return probe('parseable', q) ? 'screen, ' + q : 'screen, not all';
          },
        },
      },
    ],
  };
  const document = {
    getElementById: (id) => {
      if (id === 'subdoc') return { contentDocument: subdoc, contentWindow: {}, style: iframeStyle };
      if (id === 'content') return { style: {} };
      unsupported(`#${id}`);
    },
    createElement: (t) => {
      if (t !== 'style') unsupported(`a <${t}>`);
      return { appendChild() {}, sheet: parseSheet };
    },
    createTextNode: () => textNode,
    getElementsByTagName: () => [{ appendChild() {} }],
  };
  const sandbox = {
    document,
    window: null,
    CSSRule: { MEDIA_RULE: 4 },
    getComputedStyle: () => ({ fontSize: '16px' }),
    matchMedia: (q) => ({
      get media() {
        if (state.test === null) unsupported('matchMedia().media outside a test');
        state.probes.push({ kind: 'serialize', query: q });
        return '\u0000';
      },
      get matches() {
        if (state.test === null) unsupported('matchMedia().matches outside a test');
        state.probes.push({ kind: 'applies', query: q, env: { width: 800, height: 600 } });
        return true;
      },
    }),
    setup() {},
    done() {},
    test(fn, name) {
      state.test = name;
      state.probes = [];
      state.asserts = [];
      try {
        fn();
        const [p] = state.probes;
        const [a] = state.asserts;
        if (state.probes.length !== 1 || state.asserts.length !== 1) {
          skip('probes the browser more than once in a test');
        } else if (p.kind === 'serialize' && a.kind === 'equals' && a.actual === '\u0000') {
          state.cases.push({ file, kind: 'serialize', query: p.query, expect: a.expected });
        } else if (p.kind !== 'serialize' && a.kind === 'bool') {
          state.cases.push({ file, kind: p.kind, query: p.query, env: p.env, expect: a.value });
        } else {
          skip('asserts something other than a probe');
        }
      } catch (e) {
        skip(e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`);
      }
      state.test = null;
    },
    promise_test: () => skip('runs asynchronously'),
    async_test: () => skip('runs asynchronously'),
    assert_true: (v) => state.asserts.push({ kind: 'bool', value: v === true }),
    assert_false: (v) => state.asserts.push({ kind: 'bool', value: v !== true }),
    assert_equals: (actual, expected) => state.asserts.push({ kind: 'equals', actual, expected }),
    assert_not_equals: () => state.asserts.push({ kind: 'other' }),
    assert_unreached: (m) => {
      throw new Error('assert_unreached: ' + m);
    },
    console: { log() {} },
  };
  sandbox.window = sandbox;
  sandbox.screen = SCREEN;
  return { sandbox, state };
}

function extract(rel) {
  const html = fs.readFileSync(path.join(src, rel), 'utf8');
  const entry = { file: rel };
  if (!/testharness\.js/.test(html)) return Object.assign(entry, { skipped_because: 'not a testharness test' });
  const { sandbox, state } = sandboxFor(rel);
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
    try {
      vm.runInContext(code, context, { timeout: 10000, filename: rel });
    } catch (e) {
      error = e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`;
      break;
    }
  }
  // The Mozilla suite runs its tests from onload.
  if (!error && /onload="run\(\)"/.test(html)) {
    try {
      vm.runInContext('run()', context, { timeout: 10000 });
    } catch (e) {
      error = e instanceof Unsupported ? `needs ${e.message}` : `${e.constructor.name}: ${e.message}`;
    }
  }
  if (state.cases.length === 0) {
    return Object.assign(entry, { skipped_because: error || Object.keys(state.skipped)[0] || 'no static cases' });
  }
  if (error) state.skipped[`the rest of the file ${error}`] = 1;
  return Object.assign(entry, { cases: state.cases, skipped: state.skipped, screen: SCREEN });
}

const dir = path.join(src, 'css/mediaqueries');
const files = fs
  .readdirSync(dir)
  .filter((f) => f.endsWith('.html'))
  .sort()
  .map((f) => path.join('css/mediaqueries', f));
const results = files.map(extract);
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(
  path.join(out, 'cases.json'),
  JSON.stringify({ commit: fs.readFileSync(path.join(src, 'COMMIT'), 'utf8').trim(), files: results }, null, 1) + '\n'
);
const ran = results.filter((f) => f.cases);
console.error(`${ran.length} files with ${ran.reduce((n, f) => n + f.cases.length, 0)} cases, ${results.length - ran.length} files with none`);
