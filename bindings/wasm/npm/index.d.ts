// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

export * from "./node/parsec_html.js";
import type { Document, ParseOptions } from "./node/parsec_html.js";

/**
 * Parses a whole document as `parse` does, on one of Node's worker threads
 * when the native build loaded, so the main thread carries on meanwhile.
 * With the WebAssembly build it parses before returning.
 */
export function parseAsync(html: string, options?: ParseOptions | null): Promise<Document>;

/** Which build loaded: the one for this machine, or WebAssembly. */
export const backend: "native" | "wasm";
