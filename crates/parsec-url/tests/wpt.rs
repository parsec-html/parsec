// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The web-platform-tests URL suite, from the JSON files pinned in
//! `tests/wpt` (WPT's `url/resources`), run the way WPT's own `.any.js`
//! files run them through `new URL()` and its setters. Each WPT `test()`
//! counts once.
//!
//! A case listed in `tests/wpt/known-failures.txt` is expected to fail and
//! is counted as failed (the list is written by running with
//! `WPT_LIST_FAILURES=1`, which prints every failure's line); the run fails on any other failure, and on a listed
//! case that has started to pass, so the list can't go stale.

#[path = "../../parsec/tests/summary/mod.rs"]
mod summary;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use parsec_url::Url;
use serde_json::Value;

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/wpt")
}

fn load(file: &str) -> Value {
    let text = std::fs::read_to_string(data_dir().join(file)).unwrap();
    serde_json::from_str(&usv(&text)).unwrap_or_else(|e| panic!("{file}: {e}"))
}

/// Replaces each lone surrogate escape in JSON text with U+FFFD. `new URL()`
/// takes a USVString, so a browser makes the same swap before parsing, and
/// a Rust string can't hold a lone surrogate anyway.
fn usv(json: &str) -> String {
    let unit = |s: &str, at: usize| {
        s.get(at..at + 6)
            .filter(|e| e.starts_with("\\u"))
            .and_then(|e| u16::from_str_radix(&e[2..], 16).ok())
    };
    let mut out = String::with_capacity(json.len());
    let mut i = 0;
    while let Some(off) = json[i..].find('\\') {
        let at = i + off;
        out.push_str(&json[i..at]);
        match unit(json, at) {
            Some(0xD800..=0xDBFF)
                if unit(json, at + 6).is_some_and(|lo| (0xDC00..=0xDFFF).contains(&lo)) =>
            {
                out.push_str(&json[at..at + 12]);
                i = at + 12;
            }
            Some(0xD800..=0xDFFF) => {
                out.push_str("\\uFFFD");
                i = at + 6;
            }
            // Any other escape, `\\` included, is two characters at least.
            _ => {
                let end = at + 1 + json[at + 1..].chars().next().map_or(0, char::len_utf8);
                out.push_str(&json[at..end]);
                i = end;
            }
        }
    }
    out.push_str(&json[i..]);
    out
}

fn known_failures() -> BTreeSet<String> {
    std::fs::read_to_string(data_dir().join("known-failures.txt"))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

fn get(url: &Url, attr: &str) -> String {
    match attr {
        "href" => url.href().into(),
        "origin" => url.origin(),
        "protocol" => url.protocol().into(),
        "username" => url.username().into(),
        "password" => url.password().into(),
        "host" => url.host().into(),
        "hostname" => url.hostname().into(),
        "port" => url.port().into(),
        "pathname" => url.pathname().into(),
        "search" => url.search().into(),
        "hash" => url.hash().into(),
        "searchParams" => url.search_params().to_string(),
        _ => panic!("no getter for {attr}"),
    }
}

fn set(url: &mut Url, attr: &str, value: &str) {
    match attr {
        "href" => {
            let _ = url.set_href(value);
        }
        "protocol" => url.set_protocol(value),
        "username" => url.set_username(value),
        "password" => url.set_password(value),
        "host" => url.set_host(value),
        "hostname" => url.set_hostname(value),
        "port" => url.set_port(value),
        "pathname" => url.set_pathname(value),
        "search" => url.set_search(value),
        "hash" => url.set_hash(value),
        _ => panic!("no setter for {attr}"),
    }
}

/// The first attribute that differs from `expected`, or `None`.
fn mismatch(
    url: &Url,
    expected: &serde_json::Map<String, Value>,
    attrs: &[&str],
) -> Option<String> {
    attrs.iter().find_map(|&attr| {
        let want = expected.get(attr)?.as_str()?;
        let got = get(url, attr);
        (got != want).then(|| format!("{attr}: got {got:?}, want {want:?}"))
    })
}

/// One suite's results: each failure's name and what went wrong.
struct Suite {
    name: &'static str,
    file: &'static str,
    passed: usize,
    skipped: usize,
    skipped_because: &'static str,
    failed: Vec<(String, String)>,
}

impl Suite {
    fn new(name: &'static str, file: &'static str) -> Suite {
        Suite {
            name,
            file,
            passed: 0,
            skipped: 0,
            skipped_because: "none",
            failed: Vec::new(),
        }
    }

    /// Counts one case; a failure is kept under its name with control
    /// characters escaped, so it fits on one line of the known-failures list.
    fn check(&mut self, test: String, result: Result<(), String>) {
        match result {
            Ok(()) => self.passed += 1,
            Err(why) => {
                let name = test
                    .chars()
                    .map(|c| match c.is_control() {
                        true => c.escape_debug().to_string(),
                        false => c.to_string(),
                    })
                    .collect();
                self.failed.push((name, why));
            }
        }
    }

    /// Prints the counts, writes them for CI, and panics on any failure
    /// the known-failures list doesn't name or any listed case that passed.
    fn finish(self) {
        eprintln!(
            "{}: {} passed, {} failed, {} skipped{}",
            self.file,
            self.passed,
            self.failed.len(),
            self.skipped,
            if self.skipped > 0 {
                format!(" ({})", self.skipped_because)
            } else {
                String::new()
            }
        );
        summary::write(&summary::Counts {
            suite: self.name,
            passed: self.passed,
            failed: self.failed.len(),
            skipped: self.skipped,
            skipped_because: self.skipped_because,
            data: &data_dir(),
        });
        if std::env::var_os("WPT_LIST_FAILURES").is_some() {
            for (t, _) in &self.failed {
                println!("{}: {t}", self.file);
            }
        }
        let known = known_failures();
        let prefix = format!("{}: ", self.file);
        let unexpected: Vec<String> = self
            .failed
            .iter()
            .filter(|(t, _)| !known.contains(&format!("{prefix}{t}")))
            .map(|(t, why)| format!("{prefix}{t}\n    {why}"))
            .collect();
        let failed: BTreeSet<String> = self
            .failed
            .iter()
            .map(|(t, _)| format!("{prefix}{t}"))
            .collect();
        let fixed: Vec<&String> = known
            .iter()
            .filter(|k| k.starts_with(&prefix) && !failed.contains(*k))
            .collect();
        assert!(
            unexpected.is_empty() && fixed.is_empty(),
            "{} unexpected failures:\n{}\n{} known failures now pass, so take them off \
             tests/wpt/known-failures.txt:\n{}",
            unexpected.len(),
            unexpected.join("\n"),
            fixed.len(),
            fixed
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
}

/// `url-constructor.any.js` over `urltestdata.json`.
#[test]
fn url_parsing() {
    let mut suite = Suite::new("wpt-url-parsing", "urltestdata.json");
    for case in load("urltestdata.json").as_array().unwrap() {
        let Some(case) = case.as_object() else {
            continue;
        };
        let input = case["input"].as_str().unwrap();
        let base = case["base"].as_str();
        let name = match base {
            Some(b) => format!("Parsing: <{input}> against <{b}>"),
            None => format!("Parsing: <{input}> without base"),
        };
        let parsed = match base {
            Some(b) => Url::parse(b).and_then(|b| Url::parse_with_base(input, &b)),
            None => Url::parse(input),
        };
        let result = match (parsed, case.contains_key("failure")) {
            (Err(_), true) => Ok(()),
            (Ok(url), true) => Err(format!("parsed as {:?}, should fail", url.href())),
            (Err(e), false) => Err(format!("failed: {e}")),
            (Ok(url), false) => mismatch(
                &url,
                case,
                &[
                    "href",
                    "protocol",
                    "username",
                    "password",
                    "host",
                    "hostname",
                    "port",
                    "pathname",
                    "search",
                    "searchParams",
                    "hash",
                ],
            )
            .map_or(Ok(()), Err),
        };
        suite.check(name, result);
    }
    suite.finish();
}

/// `url-origin.any.js`, which reads the same file's `origin` fields.
#[test]
fn url_origin() {
    let mut suite = Suite::new("wpt-url-origin", "urltestdata.json origin");
    for case in load("urltestdata.json").as_array().unwrap() {
        let Some(case) = case.as_object().filter(|c| c.contains_key("origin")) else {
            continue;
        };
        let input = case["input"].as_str().unwrap();
        let base = case["base"].as_str();
        let name = format!("Origin parsing: <{input}> against <{}>", base.unwrap_or(""));
        let parsed = match base {
            Some(b) => Url::parse(b).and_then(|b| Url::parse_with_base(input, &b)),
            None => Url::parse(input),
        };
        let result = match parsed {
            Ok(url) => mismatch(&url, case, &["origin"]).map_or(Ok(()), Err),
            Err(e) => Err(format!("failed: {e}")),
        };
        suite.check(name, result);
    }
    suite.finish();
}

/// `url-setters.any.js` over `setters_tests.json`.
#[test]
fn url_setters() {
    let mut suite = Suite::new("wpt-url-setters", "setters_tests.json");
    for (attr, cases) in load("setters_tests.json").as_object().unwrap() {
        if attr == "comment" {
            continue;
        }
        for case in cases.as_array().unwrap() {
            let href = case["href"].as_str().unwrap();
            let value = case["new_value"].as_str().unwrap();
            let name = format!("Setting <{href}>.{attr} = '{value}'");
            let expected = case["expected"].as_object().unwrap();
            let result = match Url::parse(href) {
                Ok(mut url) => {
                    set(&mut url, attr, value);
                    let attrs: Vec<&str> = expected.keys().map(String::as_str).collect();
                    mismatch(&url, expected, &attrs).map_or(Ok(()), Err)
                }
                Err(e) => Err(format!("{href} failed to parse: {e}")),
            };
            suite.check(name, result);
        }
    }
    suite.finish();
}

/// `new URL("https://" + input + "/x")` and its four checks, shared by the
/// IDNA and ToASCII files.
fn host_case(input: &str, output: Option<&str>) -> Result<(), String> {
    let parsed = Url::parse(&format!("https://{input}/x"));
    match (parsed, output) {
        (Err(_), None) => Ok(()),
        (Ok(url), None) => Err(format!("parsed as {:?}, should fail", url.href())),
        (Err(e), Some(_)) => Err(format!("failed: {e}")),
        (Ok(url), Some(out)) => {
            let want = serde_json::json!({
                "host": out,
                "hostname": out,
                "pathname": "/x",
                "href": format!("https://{out}/x"),
            });
            let want = want.as_object().unwrap();
            mismatch(&url, want, &["host", "hostname", "pathname", "href"]).map_or(Ok(()), Err)
        }
    }
}

/// `toascii.window.js` over `toascii.json`, the `URL` half: the
/// constructor and the `host` and `hostname` setters. The `<a>` and
/// `<area>` halves, the same three for each element, are counted as skipped.
#[test]
fn toascii() {
    let mut suite = Suite::new("wpt-url-toascii", "toascii.json");
    suite.skipped_because = "the <a> and <area> halves test HTML elements, not URL parsing";
    for case in load("toascii.json").as_array().unwrap() {
        let Some(case) = case.as_object() else {
            continue;
        };
        suite.skipped += 6;
        let input = case["input"].as_str().unwrap();
        let output = case["output"].as_str();
        suite.check(format!("{input} (using URL)"), host_case(input, output));
        for attr in ["host", "hostname"] {
            let mut url = Url::parse("https://x/x").unwrap();
            set(&mut url, attr, input);
            let want = output.unwrap_or("x");
            let got = get(&url, attr);
            suite.check(
                format!("{input} (using URL.{attr})"),
                if got == want {
                    Ok(())
                } else {
                    Err(format!("{attr}: got {got:?}, want {want:?}"))
                },
            );
        }
    }
    suite.finish();
}

fn idna(name: &'static str, file: &'static str) {
    let mut suite = Suite::new(name, file);
    suite.skipped_because = "an empty input can't go through new URL()";
    for case in load(file).as_array().unwrap() {
        let Some(case) = case.as_object() else {
            continue;
        };
        let input = case["input"].as_str().unwrap();
        // IdnaTestV2.any.js skips these too.
        if input.is_empty() {
            suite.skipped += 1;
            continue;
        }
        let output = case["output"].as_str();
        suite.check(format!("ToASCII(\"{input}\")"), host_case(input, output));
    }
    suite.finish();
}

/// `IdnaTestV2.any.js`: Unicode's IDNA tests, as WPT converted them.
#[test]
fn idna_v2() {
    idna("wpt-url-idna", "IdnaTestV2.json");
}

/// `IdnaTestV2-removed.any.js`: cases a newer IdnaTestV2 dropped.
#[test]
fn idna_v2_removed() {
    idna("wpt-url-idna-removed", "IdnaTestV2-removed.json");
}
