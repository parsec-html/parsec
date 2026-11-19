#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// Turns web-platform-tests' CSS parsing tests (css/*/parsing/*.html) into
// data for tests/wpt_values.rs. Run by scripts/fetch-wpt-values.sh:
//
//   node scripts/wpt-values-cases.js WPT_CHECKOUT tests/wpt-values
//
// The tests are JavaScript, so each file's inline scripts run here in a
// sandbox with no DOM. The helpers from css/support that set a value on
// an element's style and read it back (test_valid_value,
// test_invalid_value, test_shorthand_value, fuzzy_test_valid_color) are
// replaced by ones that record what the test expects, which is all a
// static parser can be checked against. Anything else a file does, from
// computed style to building elements, throws inside the sandbox, and
// what it would have tested is counted as skipped with the reason.
'use strict';

const fs = require('fs');
const path = require('path');
const vm = require('vm');

const [src, out] = process.argv.slice(2);
if (!src || !out) {
  console.error('usage: wpt-values-cases.js WPT_CHECKOUT OUT_DIR');
  process.exit(2);
}

class Unsupported extends Error {}
const unsupported = (what) => {
  throw new Unsupported(what);
};

// What a helper that needs a browser counts as.
const NEEDS = {
  test_computed_value: 'computed values (#30)',
  fuzzy_test_computed_color: 'computed values (#30)',
  fuzzy_test_computed_color_property: 'computed values (#30)',
  fuzzy_test_computed_color_using_currentcolor: 'computed values (#30)',
  test_computed_value_combinations: 'computed values (#30)',
  test_pseudo_computed_value: 'computed values (#30)',
  test_valid_and_computed_value: 'computed values (#30)',
  test_math_computed: 'computed values (#30)',
  test_math_used: 'layout',
  test_valid_selector: 'selectors, not values',
  test_invalid_selector: 'selectors, not values',
  test_valid_forgiving_selector: 'selectors, not values',
  test_valid_rule: 'whole rules through CSSOM',
  test_invalid_rule: 'whole rules through CSSOM',
  test_is_legacy_name_alias: 'getComputedStyle',
  test_is_legacy_shorthand: 'getComputedStyle',
  test_keyframes_name_valid: 'whole rules through CSSOM',
  test_keyframes_name_invalid: 'whole rules through CSSOM',
};

function sandboxFor(file) {
  const state = { cases: [], skipped: {} };
  const skip = (why) => {
    state.skipped[why] = (state.skipped[why] || 0) + 1;
  };
  const record = (c) => state.cases.push(Object.assign({ file }, c));
  const valid = (property, value, serialized, options) => {
    if (typeof property !== 'string' || typeof value !== 'string') unsupported('a value that is not a string');
    const expect = serialized === undefined ? value : serialized;
    record({
      kind: 'valid',
      property,
      value,
      expect,
      fuzzy: !!(options && options.comparisonFunction),
      ...(options && options.epsilon ? { epsilon: options.epsilon } : {}),
    });
  };
  const element = new Proxy(
    {},
    {
      get(_, p) {
        unsupported(`element.${String(p)}`);
      },
    }
  );
  const sandbox = {
    test_valid_value(property, value, serialized, options) {
      if (arguments.length < 3) serialized = value;
      valid(property, value, serialized, options);
    },
    test_invalid_value(property, value) {
      if (typeof property !== 'string' || typeof value !== 'string') unsupported('a value that is not a string');
      record({ kind: 'invalid', property, value });
    },
    test_shorthand_value(property, value, longhands) {
      record({ kind: 'shorthand', property, value, longhands });
    },
    fuzzy_test_valid_color(specified, parsed, epsilon) {
      valid('color', specified, parsed === undefined ? specified : parsed, { comparisonFunction: true, epsilon });
    },
    fuzzy_test_valid_color_property(property, specified, parsed, epsilon) {
      valid(property, specified, parsed === undefined ? specified : parsed, { comparisonFunction: true, epsilon });
    },
    test(fn, name) {
      // A test written out in full: run it, and see what it touches.
      try {
        if (typeof fn === 'function') fn({ add_cleanup() {}, step_func: (f) => f });
      } catch (e) {
        skip(e instanceof Unsupported ? `needs ${e.message}` : 'a test written by hand');
      }
    },
    setup() {},
    done() {},
    promise_test: () => skip('runs asynchronously'),
    async_test: () => skip('runs asynchronously'),
    assert_equals() {},
    assert_true() {},
    assert_false() {},
    assert_not_equals() {},
    assert_in_array() {},
    assert_array_equals() {},
    document: new Proxy(
      {},
      {
        get(_, p) {
          if (p === 'getElementById' || p === 'querySelector' || p === 'createElement') {
            return () => element;
          }
          unsupported(`document.${String(p)}`);
        },
      }
    ),
    console: { log() {} },
    CSS: new Proxy({}, { get: (_, p) => unsupported(`CSS.${String(p)}`) }),
    getComputedStyle: () => unsupported('getComputedStyle'),
  };
  for (const [name, why] of Object.entries(NEEDS)) {
    sandbox[name] = () => skip(`needs ${why}`);
  }
  sandbox.window = sandbox;
  sandbox.self = sandbox;
  return { sandbox, state };
}

function extract(rel) {
  const html = fs.readFileSync(path.join(src, rel), 'utf8');
  const entry = { file: rel };
  if (!/testharness\.js/.test(html)) return Object.assign(entry, { skipped_because: 'not a testharness test' });
  const { sandbox, state } = sandboxFor(rel);
  const context = vm.createContext(sandbox);
  const scripts = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script[^>]*>/gi)];
  let error = null;
  for (const [, attrs, code] of scripts) {
    if (attrs && /\bsrc\s*=/.test(attrs)) {
      const s = /src\s*=\s*["']?([^"'\s>]+)/.exec(attrs)[1];
      if (/testharness|testharnessreport|parsing-testcommon|shorthand-testcommon|color-testcommon|computed-testcommon/.test(s)) {
        continue;
      }
      error = `needs the script ${s}`;
      break;
    }
    if (attrs && /type\s*=\s*["']?module/.test(attrs)) {
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

function walk(dir, rel = '') {
  const found = [];
  for (const name of fs.readdirSync(path.join(dir, rel)).sort()) {
    const r = path.join(rel, name);
    if (fs.statSync(path.join(dir, r)).isDirectory()) found.push(...walk(dir, r));
    else found.push(r);
  }
  return found;
}

const files = walk(src, 'css')
  .filter((f) => /\/parsing\/[^/]+\.html$/.test(f))
  .filter((f) => !/-ref\.html$|\/reference\/|\/resources\/|\/support\//.test(f));

const results = files.map(extract);
const result = {
  commit: fs.readFileSync(path.join(src, 'COMMIT'), 'utf8').trim(),
  files: results,
};
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, 'cases.json'), JSON.stringify(result, null, 0).replace(/\},\{"file"/g, '},\n{"file"') + '\n');
const ran = results.filter((f) => f.cases);
console.error(
  `${ran.length} files with ${ran.reduce((n, f) => n + f.cases.length, 0)} cases, ` +
    `${results.length - ran.length} files with none`
);
