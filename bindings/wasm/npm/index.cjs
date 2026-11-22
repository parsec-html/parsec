// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// What Node loads: Parsec built for this machine, from the platform's
// optional dependency, when npm installed one, and the WebAssembly build
// otherwise. Both have the same API. PARSEC_BACKEND=wasm skips the native
// build; PARSEC_BACKEND=native throws if it can't load.
"use strict";

const { napi } = require("./package.json");

// The platform as napi-rs names it: linux-x64-gnu, darwin-arm64,
// win32-x64-msvc.
function platform() {
  const { platform, arch } = process;
  if (platform === "linux") {
    const glibc = process.report?.getReport().header.glibcVersionRuntime;
    return `linux-${arch}-${glibc ? "gnu" : "musl"}`;
  }
  return platform === "win32" ? `win32-${arch}-msvc` : `${platform}-${arch}`;
}

const wanted = process.env.PARSEC_BACKEND;
let binding;
let backend = "wasm";
if (wanted !== "wasm") {
  try {
    binding = require(`${napi.packageName}-${platform()}`);
    backend = "native";
  } catch (e) {
    if (wanted === "native") throw e;
  }
}

if (binding) {
  // The WebAssembly build's objects are freed by hand or with `using`; the
  // native ones are the garbage collector's, so freeing is a no-op that
  // keeps code written for either working with both.
  for (const Class of [binding.Document, binding.Node, binding.ByteParser, binding.Span, binding.ParseError]) {
    Class.prototype.free = function free() {};
    if (Symbol.dispose) Class.prototype[Symbol.dispose] = Class.prototype.free;
  }
} else {
  binding = require("./node/parsec_html.js");
}

module.exports = {
  ...binding,
  // The WebAssembly build has no threads to hand the parse to: this one
  // parses before it returns, and only the result is a promise.
  parseAsync: binding.parseAsync ?? (async (html, options) => binding.parse(html, options)),
  backend,
};
