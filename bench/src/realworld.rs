// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The real web: Netweir's real-world corpus, fed to each parser as a
//! crawler gets it, bytes and the `Content-Type` charset, each parser in
//! its own processes as in the final boss (src/boss.rs), so a crash or a
//! timeout costs one page and the peak memory is that parser's alone.
//!
//! `cargo run --release --manifest-path bench/Cargo.toml -- realworld`
//! reads bench/realworld.tsv (URL, SHA-256, date fetched, Content-Type)
//! and the bodies in bench/corpus/realworld/, named by their SHA-256,
//! which bench/fetch-realworld.sh fetches. A listed page whose body isn't
//! there, because it changed since the list was made, is left out and
//! counted. `--manifest FILE` and `--pages DIR` read others, `--runs N`
//! changes the three runs, `--tools parsec,lexbor,...` the parsers, and
//! `--now` skips waiting for a quiet machine. It prints a Markdown
//! report: totals, the spread of per-page times, peak memory, the
//! slowest pages, failures and element counts that differ from Parsec's,
//! and the same by encoding, by `<html lang>` and by size.
//!
//! Each parser is given the page as in the final boss: Parsec sniffs the
//! bytes itself, Lexbor gets its own sniff and decoders, and html5ever
//! text decoded by encoding_rs. A page's time is the median of its runs;
//! a parser's total is the median of its runs' totals.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use parsec_encoding::{Encoding, Options};

use crate::boss::{Outcome, feed, load_average, measure, median, wait_for_quiet};

/// The one-minute load average to wait for before each run.
const QUIET: f64 = 3.0;
const MAX_WAIT: Duration = Duration::from_secs(2 * 60 * 60);
const TOOLS: [&str; 3] = ["Parsec", "Lexbor", "html5ever"];
const SIZES: [(usize, &str); 5] = [
    (10_000, "under 10 KB"),
    (100_000, "10-100 KB"),
    (1_000_000, "100 KB-1 MB"),
    (10_000_000, "1-10 MB"),
    (usize::MAX, "over 10 MB"),
];

/// One page of the manifest.
struct Page {
    url: String,
    sha256: String,
    /// The `charset` from the `Content-Type` header, if it had one.
    transport: Option<String>,
}

/// The `charset` parameter of a `Content-Type` value.
fn charset(content_type: &str) -> Option<String> {
    content_type.split(';').skip(1).find_map(|p| {
        let (name, value) = p.split_once('=')?;
        let value = value.trim().trim_matches(['"', '\'']);
        (name.trim().eq_ignore_ascii_case("charset")
            && !value.is_empty()
            && !value.contains(char::is_whitespace))
        .then(|| value.to_string())
    })
}

/// The manifest's pages whose bodies are in `dir`, and how many weren't.
fn load(manifest: &Path, dir: &Path) -> (Vec<Page>, usize) {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("couldn't read {}: {e}", manifest.display()));
    let mut missing = 0;
    let mut pages = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let f: Vec<&str> = line.split('\t').collect();
        assert!(f.len() >= 4, "not URL, SHA-256, date, Content-Type: {line}");
        if !body_path(dir, f[1]).exists() {
            missing += 1;
            continue;
        }
        pages.push(Page {
            url: f[0].to_string(),
            sha256: f[1].to_string(),
            transport: charset(f[3]),
        });
    }
    (pages, missing)
}

fn body_path(dir: &Path, sha256: &str) -> PathBuf {
    dir.join(format!("{sha256}.html"))
}

/// What a page is, from one parse by Parsec outside the timed runs.
struct About {
    bytes: usize,
    encoding: &'static str,
    lang: String,
}

/// The primary subtag of the root element's `lang`, lowercased, or "none".
fn lang(doc: &parsec::Document) -> String {
    let root = doc.root();
    let mut at = doc.first_child(root);
    while let Some(id) = at {
        if matches!(doc.kind(id), parsec::Kind::Element { .. }) {
            let lang = doc
                .attrs(id)
                .find(|(_, name, _)| name.eq_ignore_ascii_case("lang"))
                .map(|(_, _, v)| v.trim().to_ascii_lowercase())
                .unwrap_or_default();
            let primary = lang.split(['-', '_']).next().unwrap_or("");
            return if primary.is_empty() { "none" } else { primary }.to_string();
        }
        at = doc.next_sibling(id);
    }
    "none".to_string()
}

fn about(page: &Page, dir: &Path) -> About {
    let bytes = std::fs::read(body_path(dir, &page.sha256)).unwrap();
    let options = Options {
        transport: page
            .transport
            .as_deref()
            .and_then(|l| Encoding::for_label(l.as_bytes())),
        ..Options::default()
    };
    let (encoding, lang) = match parsec_encoding::parse_bytes(&bytes, options) {
        Ok(p) => (p.encoding.name(), lang(&p.document)),
        Err(_) => ("refused", "none".to_string()),
    };
    About {
        bytes: bytes.len(),
        encoding,
        lang,
    }
}

fn ms(secs: f64) -> String {
    format!("{:.2} ms", secs * 1e3)
}

fn mb_s(bytes: usize, secs: f64) -> String {
    format!("{:.0}", bytes as f64 / 1e6 / secs)
}

/// The `q` quantile of sorted `v`, nearest rank.
fn quantile(v: &[f64], q: f64) -> f64 {
    v[((v.len() as f64 * q).ceil() as usize).clamp(1, v.len()) - 1]
}

/// `realworld [--manifest FILE] [--pages DIR] [--runs N] [--tools a,b] [--now]`.
pub fn run(args: &[String]) {
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
    };
    self_check();
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    if args[0] == "realworld-tool" {
        // `realworld-tool <tool> <manifest> <dir>`: one parser's run.
        let dir = Path::new(&args[3]);
        let (pages, _) = load(Path::new(&args[2]), dir);
        feed(&args[1], pages.len(), &|i| {
            let bytes = std::fs::read(body_path(dir, &pages[i].sha256)).unwrap();
            (bytes, pages[i].transport.clone())
        });
        return;
    }
    let manifest = value("--manifest").map_or(here.join("realworld.tsv"), PathBuf::from);
    let dir = value("--pages").map_or(here.join("corpus/realworld"), PathBuf::from);
    let runs: usize = value("--runs").map_or(3, |v| v.parse().unwrap());
    let now = args.iter().any(|a| a == "--now");
    let tools: Vec<usize> = value("--tools").map_or(vec![0, 1, 2], |list| {
        list.split(',')
            .map(|t| {
                TOOLS
                    .iter()
                    .position(|n| n.eq_ignore_ascii_case(t))
                    .unwrap_or_else(|| panic!("no tool {t}"))
            })
            .collect()
    });

    let (pages, missing) = load(&manifest, &dir);
    assert!(!pages.is_empty(), "no pages in {}", dir.display());
    let n = pages.len();
    let about: Vec<About> = pages.iter().map(|p| about(p, &dir)).collect();
    let total_bytes: usize = about.iter().map(|a| a.bytes).sum();
    let mut bodies: Vec<&str> = pages.iter().map(|p| p.sha256.as_str()).collect();
    bodies.sort_unstable();
    bodies.dedup();

    let deadline = Instant::now() + MAX_WAIT;
    let mut results: Vec<Vec<(Vec<Outcome>, u64, f64)>> =
        tools.iter().map(|_| Vec::new()).collect();
    let mut events = Vec::new();
    for r in 0..runs {
        // A different parser goes first each run.
        for k in 0..tools.len() {
            let slot = (k + r) % tools.len();
            let tool = tools[slot];
            let load = if now {
                load_average()
            } else {
                wait_for_quiet(QUIET, deadline)
            };
            let args = [
                "realworld-tool".to_string(),
                tool.to_string(),
                manifest.display().to_string(),
                dir.display().to_string(),
            ];
            let (outcomes, rss, ev) = measure(&args, n);
            eprintln!(
                "run {} {:<10} load {load:.2} at start, {:.1} s",
                r + 1,
                TOOLS[tool],
                outcomes.iter().map(Outcome::secs).sum::<f64>()
            );
            for e in ev {
                let i: usize = e.split(' ').nth(1).unwrap().parse().unwrap_or(0);
                let url = pages.get(i).map_or("?", |p| p.url.as_str());
                events.push(format!("{} run {}: {e} ({url})", TOOLS[tool], r + 1));
            }
            results[slot].push((outcomes, rss, load));
        }
    }
    let page_secs =
        |slot: usize, i: usize| median(results[slot].iter().map(|(o, _, _)| o[i].secs()).collect());
    let per_page: Vec<Vec<f64>> = (0..tools.len())
        .map(|slot| (0..n).map(|i| page_secs(slot, i)).collect())
        .collect();
    let lexbor = tools.iter().position(|&t| t == 1);
    let parsec = tools.iter().position(|&t| t == 0);

    println!("## Results\n");
    crate::header();
    println!(
        "{n} pages ({} distinct bodies, {:.1} MB); listed pages left out because their bodies weren't in {}: {missing}. Runs per parser: {runs}.\n",
        bodies.len(),
        total_bytes as f64 / 1e6,
        dir.display()
    );
    println!(
        "| Parser | Total | MB/s | p50 | p90 | p99 | Slowest | Peak RSS | Crashes | Timeouts | Refused |"
    );
    println!("|---|--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|");
    for (slot, &tool) in tools.iter().enumerate() {
        let runs = &results[slot];
        let total = median(
            runs.iter()
                .map(|(o, _, _)| o.iter().map(Outcome::secs).sum())
                .collect(),
        );
        let mut sorted = per_page[slot].clone();
        sorted.sort_by(f64::total_cmp);
        let rss = median(runs.iter().map(|&(_, rss, _)| rss as f64).collect());
        let count = |f: &dyn Fn(&Outcome) -> bool| {
            runs.iter()
                .map(|(o, _, _)| o.iter().filter(|p| f(p)).count())
                .max()
                .unwrap_or(0)
        };
        println!(
            "| {} | {:.2} s | {} | {} | {} | {} | {} | {:.0} MB | {} | {} | {} |",
            TOOLS[tool],
            total,
            mb_s(total_bytes, total),
            ms(quantile(&sorted, 0.5)),
            ms(quantile(&sorted, 0.9)),
            ms(quantile(&sorted, 0.99)),
            ms(quantile(&sorted, 1.0)),
            rss / 1e6,
            count(&|p| matches!(p, Outcome::Crashed)),
            count(&|p| matches!(p, Outcome::TimedOut)),
            count(&|p| matches!(p, Outcome::Refused { .. })),
        );
    }
    println!();

    // A table of groups: pages, MB, then each parser's MB/s over the sum of
    // its per-page medians, and Parsec against Lexbor.
    let group_table = |title: &str, key: &dyn Fn(usize) -> String, order: Option<&[&str]>| {
        let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
        for i in 0..n {
            let k = key(i);
            match groups.iter_mut().find(|(g, _)| *g == k) {
                Some((_, v)) => v.push(i),
                None => groups.push((k, vec![i])),
            }
        }
        match order {
            Some(order) => groups.sort_by_key(|(g, _)| order.iter().position(|o| o == g)),
            None => groups.sort_by_key(|(_, v)| std::cmp::Reverse(v.len())),
        }
        println!("### {title}\n");
        print!("| | Pages | MB |");
        for &t in &tools {
            print!(" {} MB/s |", TOOLS[t]);
        }
        println!(" Parsec vs Lexbor |");
        println!("|---|--:|--:|{}--:|", "--:|".repeat(tools.len()));
        let (shown, rest) = groups.split_at(groups.len().min(25));
        let row = |name: &str, idx: &[usize]| {
            let bytes: usize = idx.iter().map(|&i| about[i].bytes).sum();
            print!("| {name} | {} | {:.1} |", idx.len(), bytes as f64 / 1e6);
            let secs: Vec<f64> = (0..tools.len())
                .map(|s| idx.iter().map(|&i| per_page[s][i]).sum())
                .collect();
            for s in &secs {
                print!(" {} |", mb_s(bytes, *s));
            }
            match (parsec, lexbor) {
                (Some(p), Some(l)) => println!(" {:.2}x |", secs[l] / secs[p]),
                _ => println!(" - |"),
            }
        };
        for (name, idx) in shown {
            row(name, idx);
        }
        if !rest.is_empty() {
            let idx: Vec<usize> = rest.iter().flat_map(|(_, v)| v.iter().copied()).collect();
            row(&format!("{} more", rest.len()), &idx);
        }
        println!();
    };
    group_table("By encoding", &|i| about[i].encoding.to_string(), None);
    group_table("By `<html lang>`", &|i| about[i].lang.clone(), None);
    let size_names: Vec<&str> = SIZES.iter().map(|s| s.1).collect();
    group_table(
        "By size",
        &|i| {
            SIZES
                .iter()
                .find(|s| about[i].bytes < s.0)
                .unwrap()
                .1
                .to_string()
        },
        Some(&size_names),
    );

    let page_row = |i: usize| {
        print!(
            "| {} | {:.0} KB | {} |",
            pages[i].url,
            about[i].bytes as f64 / 1e3,
            about[i].encoding
        );
        for secs in &per_page {
            print!(" {} |", ms(secs[i]));
        }
        println!();
    };
    let page_header = || {
        print!("| Page | Size | Encoding |");
        for &t in &tools {
            print!(" {} |", TOOLS[t]);
        }
        println!("\n|---|--:|---|{}", "--:|".repeat(tools.len()));
    };
    if let Some(p) = parsec {
        println!("### Parsec's slowest pages\n");
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| per_page[p][b].total_cmp(&per_page[p][a]));
        page_header();
        order.iter().take(10).for_each(|&i| page_row(i));
        println!();
        if let Some(l) = lexbor {
            let slower: Vec<usize> = (0..n)
                .filter(|&i| per_page[p][i] > per_page[l][i])
                .collect();
            let lost: f64 = slower
                .iter()
                .map(|&i| per_page[p][i] - per_page[l][i])
                .sum();
            println!("### Where Lexbor is faster\n");
            println!(
                "Lexbor parses {} of {n} pages faster than Parsec; Parsec spends {} more than Lexbor on them in all. Those that lose the most time:\n",
                slower.len(),
                ms(lost)
            );
            let mut order = slower;
            order.sort_by(|&a, &b| {
                (per_page[p][b] - per_page[l][b]).total_cmp(&(per_page[p][a] - per_page[l][a]))
            });
            if !order.is_empty() {
                page_header();
                order.iter().take(10).for_each(|&i| page_row(i));
                println!();
            }
        }
        if let Some(h) = tools.iter().position(|&t| t == 2) {
            let slower = (0..n).filter(|&i| per_page[p][i] > per_page[h][i]).count();
            println!("html5ever parses {slower} of {n} pages faster than Parsec.\n");
        }

        // Element counts against Parsec's, from the first run.
        println!("### Element counts that differ from Parsec's\n");
        for (slot, &tool) in tools.iter().enumerate().filter(|&(_, &t)| t != 0) {
            let differ: Vec<(usize, usize, usize)> = results[p][0]
                .0
                .iter()
                .zip(&results[slot][0].0)
                .enumerate()
                .filter_map(|(i, pair)| match pair {
                    (Outcome::Parsed { elements: a, .. }, Outcome::Parsed { elements: b, .. })
                        if a != b =>
                    {
                        Some((i, *a, *b))
                    }
                    _ => None,
                })
                .collect();
            println!("{}: {} pages.\n", TOOLS[tool], differ.len());
            if !differ.is_empty() {
                println!(
                    "| Page | Encoding | Parsec | {} |\n|---|---|--:|--:|",
                    TOOLS[tool]
                );
                for &(i, a, b) in differ.iter().take(20) {
                    println!("| {} | {} | {a} | {b} |", pages[i].url, about[i].encoding);
                }
                println!();
            }
        }
    }
    let loads: Vec<String> = results
        .iter()
        .flatten()
        .map(|r| format!("{:.2}", r.2))
        .collect();
    println!(
        "Load average at the start of each run, by parser: {}.\n",
        loads.join(", ")
    );
    if events.is_empty() {
        println!("No crashes or timeouts.");
    }
    for e in &events {
        println!("- {e}");
    }
}

/// Checks the parsing above; the bench has no test target.
fn self_check() {
    assert_eq!(
        charset("text/html; charset=UTF-8").as_deref(),
        Some("UTF-8")
    );
    assert_eq!(
        charset("text/html;Charset=\"Shift_JIS\"").as_deref(),
        Some("Shift_JIS")
    );
    assert_eq!(charset("text/html"), None);
    assert_eq!(charset("charset=utf-8"), None);
    assert_eq!(charset("text/html; charset="), None);
    assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 0.5), 2.0);
    assert_eq!(quantile(&[1.0, 2.0, 3.0, 4.0], 1.0), 4.0);
}
