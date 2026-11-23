// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Every page that broke Parsec on the real web, reduced to the smallest
//! input that still did (by `parsec-reduce`), in `tests/regressions/`.
//!
//! Each `<name>.html` starts with a comment saying where it came from:
//!
//! ```text
//! <!-- site: https://example.com/page; date: 2026-11-21; failed: a panic in tree.rs; issue: #203 -->
//! ```
//!
//! Each one has to parse without a panic and inside `LIMIT`. If
//! `<name>.tree` sits beside it, the tree has to match it too: the
//! html5lib-tests tree dump, as `SerializeOptions::html5test` writes it.
//! `BLESS_REGRESSIONS=1 cargo test -p parsec-html --test regressions`
//! writes each fixture's `.tree` from what Parsec builds now.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use parsec::{Options, SerializeOptions};

/// How long a fixture may take, generously: they're a few KB at most.
const LIMIT: Duration = Duration::from_secs(10);
/// The most a fixture may be: reduced inputs only, never whole pages.
const MAX_SIZE: usize = 64 * 1024;

/// Checks the first line is the header, with every field in order.
fn header(html: &str) -> Result<(), String> {
    let first = html.lines().next().unwrap_or("");
    let fields = first
        .strip_prefix("<!-- ")
        .and_then(|s| s.strip_suffix(" -->"))
        .ok_or(
            "doesn't start with a <!-- site: ...; date: ...; failed: ...; issue: ... --> line",
        )?;
    let mut parts = fields.split("; ");
    let mut field = |key: &str| {
        parts
            .next()
            .and_then(|p| p.strip_prefix(key))
            .and_then(|p| p.strip_prefix(": "))
            .filter(|v| !v.is_empty())
            .ok_or(format!("the header has no {key}"))
    };
    let site = field("site")?;
    let date = field("date")?;
    field("failed")?;
    let issue = field("issue")?;
    if !site.starts_with("https://") && !site.starts_with("http://") {
        return Err(format!("the site {site:?} isn't a URL"));
    }
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let ymd: Vec<&str> = date.split('-').collect();
    if !(ymd.len() == 3 && ymd[0].len() == 4 && ymd[1].len() == 2 && ymd[2].len() == 2)
        || !ymd.iter().all(|p| digits(p))
    {
        return Err(format!("the date {date:?} isn't YYYY-MM-DD"));
    }
    if !issue.strip_prefix('#').is_some_and(digits) {
        return Err(format!("the issue {issue:?} isn't #N"));
    }
    Ok(())
}

/// The tree dump, from a parse on its own thread, so a hang fails the
/// test instead of stopping it.
fn parse(html: String) -> Result<String, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let start = Instant::now();
        let parsed = parsec::parse_with(
            &html,
            Options {
                budget: Some(LIMIT),
                ..Options::default()
            },
        );
        let _ = tx.send(parsed.map(|doc| {
            let dump = doc.serialize_with(
                doc.root(),
                &SerializeOptions {
                    html5test: true,
                    ..SerializeOptions::default()
                },
            );
            (dump, start.elapsed())
        }));
    });
    match rx.recv_timeout(LIMIT + Duration::from_secs(5)) {
        Ok(Ok((dump, took))) if took <= LIMIT => Ok(dump),
        Ok(Ok((_, took))) => Err(format!("took {took:?}, over {LIMIT:?}")),
        Ok(Err(e)) => Err(e.to_string()),
        Err(mpsc::RecvTimeoutError::Timeout) => Err(format!("still parsing after {LIMIT:?}")),
        Err(mpsc::RecvTimeoutError::Disconnected) => Err("panicked".to_string()),
    }
}

/// Checks one fixture, writing its `.tree` instead of comparing with
/// `bless`.
fn check(path: &Path, bless: bool) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() > MAX_SIZE {
        return Err(format!(
            "{} bytes, over {MAX_SIZE}: reduce it with parsec-reduce",
            bytes.len()
        ));
    }
    let html = String::from_utf8(bytes).map_err(|_| "isn't UTF-8".to_string())?;
    header(&html)?;
    let dump = parse(html)?;
    let tree = path.with_extension("tree");
    if bless {
        return std::fs::write(&tree, format!("{dump}\n")).map_err(|e| e.to_string());
    }
    match std::fs::read_to_string(&tree) {
        Ok(want) if want.trim_end_matches('\n') == dump => Ok(()),
        Ok(want) => Err(format!(
            "the tree differs from {}:\nwant:\n{want}\ngot:\n{dump}",
            tree.display()
        )),
        Err(_) => Ok(()),
    }
}

fn fixtures(dir: &Path) -> Vec<PathBuf> {
    let mut list: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|d| d.map(|e| e.unwrap().path()).collect())
        .unwrap_or_default();
    list.retain(|p| p.extension().is_some_and(|e| e == "html"));
    list.sort();
    list
}

#[test]
fn every_fixture() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/regressions");
    let bless = std::env::var_os("BLESS_REGRESSIONS").is_some();
    let list = fixtures(&dir);
    let failures: Vec<String> = list
        .iter()
        .filter_map(|p| {
            check(p, bless)
                .err()
                .map(|e| format!("{}: {e}", p.display()))
        })
        .collect();
    println!(
        "{} regression fixtures, {} failed",
        list.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn headers() {
    let ok = "<!-- site: https://example.com/a?b; date: 2026-11-21; failed: a panic; issue: #203 -->\n<p>";
    assert_eq!(header(ok), Ok(()));
    for (bad, why) in [
        ("<p>", "doesn't start"),
        (
            "<!-- site: https://x; date: 2026-11-21; failed: x -->",
            "no issue",
        ),
        (
            "<!-- site: example.com; date: 2026-11-21; failed: x; issue: #1 -->",
            "isn't a URL",
        ),
        (
            "<!-- site: https://x; date: 21/11/2026; failed: x; issue: #1 -->",
            "YYYY-MM-DD",
        ),
        (
            "<!-- site: https://x; date: 2026-11-21; failed: ; issue: #1 -->",
            "no failed",
        ),
        (
            "<!-- site: https://x; date: 2026-11-21; failed: x; issue: 1 -->",
            "isn't #N",
        ),
        (
            "<!-- date: 2026-11-21; site: https://x; failed: x; issue: #1 -->",
            "no site",
        ),
    ] {
        let err = header(bad).unwrap_err();
        assert!(err.contains(why), "{bad}: {err}");
    }
}

#[test]
fn a_fixture_is_held_to_its_tree() {
    let dir = std::env::temp_dir().join(format!("parsec-regressions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("p.html");
    std::fs::write(
        &page,
        "<!-- site: https://example.com/; date: 2026-11-21; failed: a test; issue: #203 -->\n<p>one",
    )
    .unwrap();
    assert_eq!(check(&page, false), Ok(()), "no .tree is fine");
    assert_eq!(check(&page, true), Ok(()));
    let tree = std::fs::read_to_string(dir.join("p.tree")).unwrap();
    assert!(tree.contains("|     <p>\n|       \"one\""), "{tree}");
    assert_eq!(check(&page, false), Ok(()));
    std::fs::write(dir.join("p.tree"), tree.replace("one", "two")).unwrap();
    assert!(
        check(&page, false)
            .unwrap_err()
            .contains("the tree differs")
    );

    std::fs::write(&page, "<p>no header").unwrap();
    assert!(check(&page, false).unwrap_err().contains("doesn't start"));
    std::fs::write(&page, vec![b'x'; MAX_SIZE + 1]).unwrap();
    assert!(check(&page, false).unwrap_err().contains("reduce it"));
    assert_eq!(fixtures(&dir), [page]);
    std::fs::remove_dir_all(dir).unwrap();
}
