// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! JavaScript for Node.js: bindings/wasm/src/js.rs with Node-API in place
//! of wasm-bindgen, the same names, options, results and errors.
//!
//! Each builds its result as JSON in Rust and hands it to `JSON.parse`,
//! as the WebAssembly build does. The same `document_state` is in
//! bindings/wasm/src/js.rs and bindings/python/src/js.rs.

use napi::bindgen_prelude::*;
use napi_derive::napi;
use parsec::AttrNs;
use parsec::css::SelectorList;
use parsec_js::data::{Found, page_state_in, script_type};
use parsec_js::{
    DEFAULT_MAX_DEPTH, Diagnostic, Limits, LineIndex, Options, RenderOptions, SourceType,
};

use crate::{Document, error};

/// How `parseJs` parses; the WebAssembly build's `JsParseOptions`.
#[napi(object)]
pub struct JsParseOptions {
    pub module: Option<bool>,
    pub max_depth: Option<f64>,
    pub max_steps: Option<f64>,
}

/// The ESTree of `source`, a script or, with `options.module`, a module,
/// as plain objects in acorn's shape. Throws a SyntaxError for a syntax
/// error, and an Error naming `maxSteps` past that many tokens.
#[napi(js_name = "parseJs", catch_unwind)]
pub fn parse_js<'env>(
    env: &'env Env,
    source: String,
    options: Option<JsParseOptions>,
) -> Result<Unknown<'env>> {
    let options = self::options(options)?;
    match parsec_js::parse(&source, &options) {
        Ok(ast) => json_parse(env, ast.to_estree(&source)),
        Err(d) => Err(js_error(env, &d, &source, options.limits.max_steps)),
    }
}

/// The page state in `source`, the text of one `<script>`, in source
/// order. Throws a SyntaxError if it doesn't parse.
#[napi(js_name = "pageState", catch_unwind)]
pub fn page_state<'env>(env: &'env Env, source: String) -> Result<Unknown<'env>> {
    match parsec_js::data::page_state(&source) {
        Ok(found) => {
            let mut out = String::from("[");
            push_found(&mut out, &found, &source);
            out.push(']');
            json_parse(env, out)
        }
        Err(d) => Err(js_error(env, &d, &source, None)),
    }
}

#[napi]
impl Document {
    /// The state the page keeps in its scripts, as `pageState` reads it
    /// from each inline script a browser would run, in document order, in
    /// one array. A script that doesn't parse is passed over.
    #[napi(js_name = "pageState", catch_unwind)]
    pub fn page_state<'env>(&self, env: &'env Env) -> Result<Unknown<'env>> {
        json_parse(env, document_state(&self.doc.borrow()))
    }
}

fn json_parse(env: &Env, text: String) -> Result<Unknown<'_>> {
    let json: Object = env.get_global()?.get_named_property("JSON")?;
    let parse: Function<String, Unknown> = json.get_named_property("parse")?;
    parse.call(text)
}

fn options(options: Option<JsParseOptions>) -> Result<Options> {
    let whole = |n: f64, max: f64| (0.0..=max).contains(&n) && n.fract() == 0.0;
    let (module, max_depth, max_steps) = options
        .map(|o| (o.module, o.max_depth, o.max_steps))
        .unwrap_or_default();
    if max_depth.is_some_and(|n| !whole(n, DEFAULT_MAX_DEPTH.into())) {
        // The WebAssembly build's limit, so code can't tell which it got.
        return Err(error(format!(
            "maxDepth must be a whole number from 0 to {DEFAULT_MAX_DEPTH}"
        )));
    }
    if max_steps.is_some_and(|n| !whole(n, u64::MAX as f64)) {
        return Err(error("maxSteps must be a whole number, 0 or more"));
    }
    Ok(Options {
        source_type: if module.unwrap_or(false) {
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

/// A SyntaxError with the error's `code`, `line`, `column`, `start` and
/// `end` (UTF-16 indexes of the source) and its code `frame`, as the
/// WebAssembly build throws; past `maxSteps`, an Error saying so.
fn js_error(env: &Env, d: &Diagnostic, src: &str, max_steps: Option<u64>) -> Error {
    if d.code == "step-budget" {
        let steps = max_steps.unwrap_or(0);
        return error(format!("maxSteps: the parse took more than {steps} steps"));
    }
    let build = || -> Result<Error> {
        let lines = LineIndex::ecmascript(src);
        let at = lines.line_col(d.span.start);
        let ctor: Function<String, Unknown> =
            env.get_global()?.get_named_property("SyntaxError")?;
        let thrown = ctor.new_instance(d.message.to_string())?;
        let mut object = Object::from_unknown(thrown)?;
        object.set_named_property("code", d.code)?;
        object.set_named_property("line", at.line)?;
        object.set_named_property("column", at.column)?;
        object.set_named_property("start", lines.utf16_offset(d.span.start))?;
        object.set_named_property("end", lines.utf16_offset(d.span.end))?;
        object.set_named_property("frame", d.render(src, &lines, &RenderOptions::default()))?;
        Ok(Error::from(object.to_unknown()))
    };
    build().unwrap_or_else(|failed| failed)
}

fn push_found(out: &mut String, found: &[Found], src: &str) {
    for f in found {
        if out.len() > 1 {
            out.push(',');
        }
        f.write_json(src, out);
    }
}

/// The page state in every script on the page a browser would run, in
/// document order, as JSON. A script with `src`, one that isn't
/// JavaScript (`application/json`), and one that doesn't parse are
/// passed over.
fn document_state(doc: &parsec::Document) -> String {
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
    out
}
