// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway
//
// index.cjs for `import`, with its names spelled out: Node finds a
// CommonJS module's exports by reading its source, and index.cjs builds
// its exports at run time.
import parsec from "./index.cjs";

export const {
  ByteParser,
  Document,
  Node,
  ParseError,
  Span,
  backend,
  parse,
  parseAsync,
  parseBytes,
  parseFragment,
} = parsec;

export default parsec;
