// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! JavaScript parsed in WebAssembly: `parseJs` gives a script's or a
//! module's ESTree as plain objects, as acorn does; `pageState` reads the
//! state a page keeps in a script (`window.__NUXT__ = …` and its kin)
//! without running anything, and `Document.pageState` does that for each
//! script on a page.
//!
//! Each builds its result as JSON in Rust and hands it to `JSON.parse`,
//! which makes the objects faster than building them a call at a time.
//! The same `document_state` is in bindings/python/src/js.rs.

use parsec::AttrNs;
use parsec::css::SelectorList;
use parsec_js::data::{Found, page_state_in, script_type};
use parsec_js::{
    DEFAULT_MAX_DEPTH, Diagnostic, Limits, LineIndex, Options, RenderOptions, SourceType,
};
use wasm_bindgen::prelude::*;

use crate::Document;

#[wasm_bindgen(typescript_custom_section)]
const JS_OPTIONS: &str = r#"
/** How `parseJs` parses. */
export interface JsParseOptions {
    /** Parse a module, with `import` and `export`, rather than a script. */
    module?: boolean;
    /** How deep the tree may nest, at most and by default 256. Deeper
     * input throws a SyntaxError with the code `"too-deep"`. */
    maxDepth?: number;
    /** Give up after this many tokens, by throwing an Error that names
     * `maxSteps`. */
    maxSteps?: number;
}

/**
 * What `parseJs` throws for JavaScript that doesn't parse: a SyntaxError
 * with Parsec's code for the error, where it is (`start` and `end` index
 * the source; `line` and `column` count from 1, the column in
 * characters), and the code frame that shows it.
 */
export interface JsSyntaxError extends SyntaxError {
    code: string;
    line: number;
    column: number;
    start: number;
    end: number;
    frame: string;
}

/**
 * A piece of page state: what the script assigned, and its value, or
 * why the value isn't data (`code`) and the start of the text that
 * stopped it (`source`).
 */
export type PageState =
    | { name: string; value: unknown }
    | { name: string; error: { code: string; source: string } };
"#;

#[wasm_bindgen]
extern "C" {
    /// A plain object, read with the getters below.
    #[wasm_bindgen(typescript_type = "JsParseOptions")]
    pub type JsParseOptions;
    #[wasm_bindgen(method, getter)]
    fn module(this: &JsParseOptions) -> Option<bool>;
    #[wasm_bindgen(method, getter, js_name = maxDepth)]
    fn max_depth(this: &JsParseOptions) -> Option<f64>;
    #[wasm_bindgen(method, getter, js_name = maxSteps)]
    fn max_steps(this: &JsParseOptions) -> Option<f64>;

    #[wasm_bindgen(js_namespace = JSON, js_name = parse)]
    fn json_parse(text: &str) -> JsValue;

    #[wasm_bindgen(js_name = SyntaxError)]
    type SyntaxError;
    #[wasm_bindgen(constructor, js_class = "SyntaxError")]
    fn new(message: &str) -> SyntaxError;
    #[wasm_bindgen(method, setter = code)]
    fn set_code(this: &SyntaxError, code: &str);
    #[wasm_bindgen(method, setter = line)]
    fn set_line(this: &SyntaxError, line: u32);
    #[wasm_bindgen(method, setter = column)]
    fn set_column(this: &SyntaxError, column: u32);
    #[wasm_bindgen(method, setter = start)]
    fn set_start(this: &SyntaxError, start: u32);
    #[wasm_bindgen(method, setter = end)]
    fn set_end(this: &SyntaxError, end: u32);
    #[wasm_bindgen(method, setter = frame)]
    fn set_frame(this: &SyntaxError, frame: &str);
}

/// The ESTree of `source`, a script or, with `options.module`, a module,
/// as plain objects in acorn's shape. Throws a `JsSyntaxError` for a
/// syntax error, and an Error naming `maxSteps` past that many tokens.
#[wasm_bindgen(js_name = parseJs, unchecked_return_type = "any")]
pub fn parse_js(source: &str, options: Option<JsParseOptions>) -> Result<JsValue, JsValue> {
    let options = self::options(options.as_ref())?;
    match parsec_js::parse(source, &options) {
        Ok(ast) => Ok(json_parse(&ast.to_estree(source))),
        Err(d) => Err(js_error(&d, source, options.limits.max_steps)),
    }
}

/// The page state in `source`, the text of one `<script>`, in source
/// order. Throws a `JsSyntaxError` if it doesn't parse.
#[wasm_bindgen(js_name = pageState, unchecked_return_type = "PageState[]")]
pub fn page_state(source: &str) -> Result<JsValue, JsValue> {
    match parsec_js::data::page_state(source) {
        Ok(found) => {
            let mut out = String::from("[");
            push_found(&mut out, &found, source);
            out.push(']');
            Ok(json_parse(&out))
        }
        Err(d) => Err(js_error(&d, source, None)),
    }
}

fn options(options: Option<&JsParseOptions>) -> Result<Options, JsValue> {
    let whole = |n: f64, max: f64| (0.0..=max).contains(&n) && n.fract() == 0.0;
    let max_depth = options.and_then(JsParseOptions::max_depth);
    if max_depth.is_some_and(|n| !whole(n, DEFAULT_MAX_DEPTH.into())) {
        // A deeper tree needs more stack than wasm32 is sure to have.
        return Err(JsError::new(&format!(
            "maxDepth must be a whole number from 0 to {DEFAULT_MAX_DEPTH}"
        ))
        .into());
    }
    let max_steps = options.and_then(JsParseOptions::max_steps);
    if max_steps.is_some_and(|n| !whole(n, u64::MAX as f64)) {
        return Err(JsError::new("maxSteps must be a whole number, 0 or more").into());
    }
    Ok(Options {
        source_type: if options.and_then(JsParseOptions::module).unwrap_or(false) {
            SourceType::Module
        } else {
            SourceType::Script
        },
        limits: Limits {
            max_depth: max_depth.map(|n| n as u32),
            max_steps: max_steps.map(|n| n as u64),
            ..Limits::default()
        },
    })
}

fn js_error(d: &Diagnostic, src: &str, max_steps: Option<u64>) -> JsValue {
    if d.code == "step-budget" {
        let steps = max_steps.unwrap_or(0);
        return JsError::new(&format!("maxSteps: the parse took more than {steps} steps")).into();
    }
    let lines = LineIndex::ecmascript(src);
    let at = lines.line_col(d.span.start);
    let e = SyntaxError::new(&d.message);
    e.set_code(d.code);
    e.set_line(at.line);
    e.set_column(at.column);
    e.set_start(lines.utf16_offset(d.span.start));
    e.set_end(lines.utf16_offset(d.span.end));
    e.set_frame(&d.render(src, &lines, &RenderOptions::default()));
    e.into()
}

fn push_found(out: &mut String, found: &[Found], src: &str) {
    for f in found {
        if out.len() > 1 {
            out.push(',');
        }
        f.write_json(src, out);
    }
}

#[wasm_bindgen]
impl Document {
    /// The state the page keeps in its scripts, as `pageState` reads it
    /// from each inline script a browser would run, in document order, in
    /// one array. A script that doesn't parse is passed over.
    #[wasm_bindgen(js_name = pageState, unchecked_return_type = "PageState[]")]
    pub fn page_state(&self) -> JsValue {
        document_state(&self.doc.borrow())
    }
}

/// The page state in every script on the page a browser would run, in
/// document order, as plain objects. A script with `src`, one that isn't
/// JavaScript (`application/json`), and one that doesn't parse are
/// passed over.
fn document_state(doc: &parsec::Document) -> JsValue {
    let scripts = SelectorList::parse("script").expect("a valid selector");
    let mut out = String::from("[");
    for el in scripts.select(doc, doc.root()) {
        let attr = |name: &str| {
            doc.attrs(el)
                .find(|&(ns, n, _)| ns == AttrNs::None && n == name)
                .map(|(_, _, v)| v)
        };
        if attr("src").is_some() {
            continue;
        }
        let Some(source_type) = script_type(attr("type"), attr("language")) else {
            continue;
        };
        let src = doc.text(el);
        let options = Options {
            source_type,
            ..Options::default()
        };
        if let Ok(ast) = parsec_js::parse(&src, &options) {
            push_found(&mut out, &page_state_in(&ast, &src), &src);
        }
    }
    out.push(']');
    json_parse(&out)
}
