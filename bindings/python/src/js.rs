// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `parsec_html.js`: JavaScript for Python. `parse()` gives the ESTree of
//! a script or a module as dicts and lists, as acorn gives it to
//! JavaScript; `page_state()` reads the state a page keeps in a script
//! (`window.__NUXT__ = …` and its kin) without running anything, and
//! `HTMLParser.page_state()` does that for each script on a page.
//!
//! Both build the result as JSON in Rust, detached from the interpreter,
//! and hand it to `json.loads`, which makes the objects faster than
//! building them one call at a time.

use parsec::css::SelectorList;
use parsec::{AttrNs, Document};
use parsec_js::data::{Found, page_state_in, script_type};
use parsec_js::{
    DEFAULT_MAX_DEPTH, Diagnostic, EstreeOptions, Limits, LineIndex, Offsets, Options,
    RenderOptions, SourceType,
};
use pyo3::exceptions::{PyTimeoutError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyString;

pyo3::create_exception!(
    parsec_html.js,
    JSSyntaxError,
    PyValueError,
    "JavaScript that doesn't parse, or that nests deeper than max_depth. \
     `code`, `message`, `line`, `column` (both from 1, the column in \
     characters), `start` and `end` (indexes into the source) say where, \
     and `frame` shows it as a code frame."
);

/// The ESTree of `source`, a script or, with `module`, a module, as
/// dicts and lists in acorn's shape. `start` and `end` are indexes into
/// `source`. A syntax error raises `JSSyntaxError`; past `max_steps`
/// tokens it raises `TimeoutError`.
#[pyfunction]
#[pyo3(signature = (source, *, module = false, max_depth = None, max_steps = None))]
fn parse<'py>(
    py: Python<'py>,
    source: &Bound<'py, PyString>,
    module: bool,
    max_depth: Option<u32>,
    max_steps: Option<u64>,
) -> PyResult<Bound<'py, PyAny>> {
    let src = source.to_cow()?;
    let options = options(module, max_depth, max_steps)?;
    let estree = py.detach(|| {
        parsec_js::parse(&src, &options).map(|ast| {
            ast.to_estree_with(
                &src,
                EstreeOptions {
                    offsets: Offsets::Chars,
                    ..EstreeOptions::default()
                },
            )
        })
    });
    match estree {
        Ok(json) => loads(py, &json),
        Err(d) => Err(js_error(py, &d, &src, max_steps)),
    }
}

/// The page state in `source`, the text of one `<script>`: a list of
/// `{"name": …, "value": …}`, or `{"name": …, "error": {"code": …,
/// "source": …}}` for one whose value isn't data. A syntax error raises
/// `JSSyntaxError`.
#[pyfunction]
fn page_state<'py>(py: Python<'py>, source: &Bound<'py, PyString>) -> PyResult<Bound<'py, PyAny>> {
    let src = source.to_cow()?;
    let found = py.detach(|| parsec_js::data::page_state(&src).map(|f| found_json(&f, &src)));
    match found {
        Ok(json) => loads(py, &json),
        Err(d) => Err(js_error(py, &d, &src, None)),
    }
}

fn options(module: bool, max_depth: Option<u32>, max_steps: Option<u64>) -> PyResult<Options> {
    // A deeper tree needs more stack than a thread is sure to have.
    if max_depth.is_some_and(|d| d > DEFAULT_MAX_DEPTH) {
        return Err(PyValueError::new_err(format!(
            "max_depth can be at most {DEFAULT_MAX_DEPTH}"
        )));
    }
    Ok(Options {
        source_type: if module {
            SourceType::Module
        } else {
            SourceType::Script
        },
        limits: Limits {
            max_depth,
            max_steps,
            ..Limits::default()
        },
    })
}

pub fn loads<'py>(py: Python<'py>, json: &str) -> PyResult<Bound<'py, PyAny>> {
    py.import("json")?.call_method1("loads", (json,))
}

fn js_error(py: Python<'_>, d: &Diagnostic, src: &str, max_steps: Option<u64>) -> PyErr {
    if d.code == "step-budget" {
        return PyTimeoutError::new_err(format!(
            "max_steps: the parse took more than {} steps",
            max_steps.unwrap_or(0)
        ));
    }
    let lines = LineIndex::ecmascript(src);
    let at = lines.line_col(d.span.start);
    let err = JSSyntaxError::new_err(format!("{}:{}: {}", at.line, at.column, d.message));
    let value = err.value(py);
    let set = || -> PyResult<()> {
        value.setattr("code", d.code)?;
        value.setattr("message", &*d.message)?;
        value.setattr("line", at.line)?;
        value.setattr("column", at.column)?;
        value.setattr("start", lines.char_offset(d.span.start))?;
        value.setattr("end", lines.char_offset(d.span.end))?;
        value.setattr("frame", d.render(src, &lines, &RenderOptions::default()))
    };
    match set() {
        Ok(()) => err,
        Err(e) => e,
    }
}

/// `found` as a JSON array.
fn found_json(found: &[Found], src: &str) -> String {
    let mut out = String::from("[");
    push_found(&mut out, found, src);
    out.push(']');
    out
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
/// document order, as a JSON array. A script with `src`, one that isn't
/// JavaScript (`application/json`), and one that doesn't parse are
/// passed over.
pub fn document_state(doc: &Document) -> String {
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

/// Adds `parsec_html.js` to `parent`, importable as a module too.
pub fn add(parent: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = parent.py();
    let m = PyModule::new(py, "parsec_html.js")?;
    m.add_function(wrap_pyfunction!(parse, &m)?)?;
    m.add_function(wrap_pyfunction!(page_state, &m)?)?;
    m.add("JSSyntaxError", py.get_type::<JSSyntaxError>())?;
    parent.add("js", &m)?;
    py.import("sys")?
        .getattr("modules")?
        .set_item("parsec_html.js", &m)
}
