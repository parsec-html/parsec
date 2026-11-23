// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times Parsec, Lexbor's own C and html5ever parsing the same pages in one
//! process: `cargo run --release --manifest-path bench/Cargo.toml`.
//!
//! Each parser builds a whole document from a string already in memory, with
//! scripting off, which is Lexbor's default. A time is the median of at
//! least 30 runs after three warm-up runs; freeing the document isn't timed.
//! Peak memory is the most the heap grew while one document was built and
//! held, counted by this program's allocator, which Lexbor's `malloc` goes
//! through too.
//!
//! Real pages come from `bench/fetch-pages.sh`; the generated and hostile
//! pages are built here. Name pages on the command line to run only those.
//!
//! Each hostile input runs in a child process, so a parser that takes
//! minutes or crashes on one is reported as such instead of stopping the
//! table. A parse that takes over a second is timed five times, not 30.
//!
//! After the pages comes a table of selectors over the real pages, Parsec
//! against Lexbor (html5ever has no selector engine of its own). Name
//! `selectors` to run only that table, or `encoding` for decoding
//! throughput against Lexbor's encoding module and streamed bytes against
//! whole ones (not part of a full run).
//!
//! Last comes URL parsing, Parsec against Lexbor and Ada, over the links on
//! the real pages and WPT's URL test inputs. Name `urls` to run only that.
//!
//! Then CSS: tokenizing and parsing real stylesheets, Parsec against
//! Lexbor's CSS module and Servo's cssparser. Name `css` to run only that.
//!
//! `xml` times XML parsing instead (not part of a full run): Parsec
//! against libxml2's C, roxmltree and quick-xml, on the documents from
//! `bench/fetch-xml.sh`. See src/xml.rs.
//!
//! `boss` runs the final boss instead (not part of a full run): a seeded
//! batch of broken fragments, real pages, encoding quirks and giants, as
//! bytes, each parser in its own process, timed per page with its peak
//! memory. See src/boss.rs.
//!
//! `realworld` runs Netweir's real-world corpus the same way, page by
//! page (not part of a full run). See src/realworld.rs.

mod boss;
mod css;
mod encoding;
mod js;
mod lexbor;
mod libxml2;
mod pages;
mod realworld;
mod selectors;
mod urls;
mod xml;

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use html5ever::tendril::TendrilSink;
use markup5ever_rcdom::RcDom;

/// The system allocator, counting the bytes in use while `COUNTING` is set.
struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static IN_USE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

fn grew(n: usize) {
    if COUNTING.load(Relaxed) {
        let now = IN_USE.fetch_add(n, Relaxed) + n;
        PEAK.fetch_max(now, Relaxed);
    }
}

fn shrank(n: usize) {
    if COUNTING.load(Relaxed) {
        IN_USE.fetch_sub(n, Relaxed);
    }
}

// SAFETY: every call goes straight to `System`; the counters only observe.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        shrank(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            shrank(layout.size());
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const PARSERS: [&str; 3] = ["Parsec", "Lexbor", "html5ever"];
const MIN_RUNS: usize = 30;
/// Runs for a parse that takes longer than a second.
const SLOW_RUNS: usize = 5;
/// How long a child process gets for one hostile input.
const CHILD_LIMIT: Duration = Duration::from_secs(120);
/// Small pages get more runs, up to this much time per parser and page.
const TIME_PER_PAGE: Duration = Duration::from_millis(500);

/// Builds one document with parser `which` and returns it, to be dropped
/// outside the timed part.
fn parse(which: usize, html: &str) -> Box<dyn std::any::Any> {
    match which {
        0 => Box::new(parsec::parse(html, false)),
        1 => Box::new(lexbor::Document::parse(html.as_bytes())),
        _ => {
            let mut opts = html5ever::ParseOpts::default();
            opts.tree_builder.scripting_enabled = false;
            Box::new(html5ever::parse_document(RcDom::default(), opts).one(html))
        }
    }
}

struct Result {
    /// Median seconds per parse.
    secs: f64,
    /// Most bytes the heap grew by while building and holding the document.
    peak: usize,
}

fn measure(which: usize, html: &str) -> Result {
    for _ in 0..3 {
        drop(black_box(parse(which, black_box(html))));
    }
    let t = Instant::now();
    drop(black_box(parse(which, black_box(html))));
    let once = t.elapsed().max(Duration::from_micros(1));
    let runs = if once > Duration::from_secs(1) {
        SLOW_RUNS as f64
    } else {
        (TIME_PER_PAGE.as_secs_f64() / once.as_secs_f64()).clamp(MIN_RUNS as f64, 2000.0)
    };

    let mut times: Vec<f64> = (0..runs as usize)
        .map(|_| {
            let t = Instant::now();
            let doc = black_box(parse(which, black_box(html)));
            let secs = t.elapsed().as_secs_f64();
            drop(doc);
            secs
        })
        .collect();
    times.sort_by(f64::total_cmp);

    COUNTING.store(true, Relaxed);
    IN_USE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    let doc = black_box(parse(which, black_box(html)));
    let peak = PEAK.load(Relaxed);
    COUNTING.store(false, Relaxed);
    drop(doc);

    Result {
        secs: times[times.len() / 2],
        peak,
    }
}

/// The most bytes the heap grew by while `f` ran and its result was held.
fn peak<T>(f: impl FnOnce() -> T) -> usize {
    COUNTING.store(true, Relaxed);
    IN_USE.store(0, Relaxed);
    PEAK.store(0, Relaxed);
    let kept = black_box(f());
    let peak = PEAK.load(Relaxed);
    COUNTING.store(false, Relaxed);
    drop(kept);
    peak
}

fn mb_per_s(bytes: usize, secs: f64) -> f64 {
    bytes as f64 / 1e6 / secs
}

/// How one parser fared on one page.
enum Outcome {
    Done(Result),
    TooSlow,
    Crashed,
}

/// Measures in a child process, `--one <parser> <page>`, killed after
/// CHILD_LIMIT.
fn measure_apart(which: usize, page: &str) -> Outcome {
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--one", &which.to_string(), page])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("couldn't start a child process");
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let mut out = String::new();
            std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut out).unwrap();
            let mut nums = out.split_whitespace();
            return match (status.success(), nums.next(), nums.next()) {
                (true, Some(secs), Some(peak)) => Outcome::Done(Result {
                    secs: secs.parse().unwrap(),
                    peak: peak.parse().unwrap(),
                }),
                _ => Outcome::Crashed,
            };
        }
        if start.elapsed() > CHILD_LIMIT {
            let _ = child.kill();
            let _ = child.wait();
            return Outcome::TooSlow;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn main() {
    lexbor::count_allocations();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, which, page] = args.as_slice()
        && flag == "--one"
    {
        let page = pages::chosen(std::slice::from_ref(page)).remove(0);
        let r = measure(which.parse().unwrap(), &page.html);
        println!("{} {}", r.secs, r.peak);
        return;
    }
    if args.first().is_some_and(|a| a.starts_with("boss")) {
        if args[0] == "boss" {
            header();
        }
        boss::run(&args);
        return;
    }
    if args.first().is_some_and(|a| a.starts_with("realworld")) {
        realworld::run(&args);
        return;
    }
    if args.iter().any(|a| a == "encoding") {
        header();
        encoding::run(&real_pages());
        return;
    }
    if args.iter().any(|a| a == "selectors") {
        header();
        selectors::run(&real_pages());
        return;
    }
    if args.iter().any(|a| a == "css") {
        header();
        css::run(&real_pages());
        return;
    }
    if args.iter().any(|a| a == "xml") {
        header();
        xml::run();
        return;
    }
    if let [flag, dir] = args.as_slice()
        && flag == "js-dump"
    {
        js::dump(&real_pages(), std::path::Path::new(dir));
        return;
    }
    if args.iter().any(|a| a == "js-scope") {
        header();
        js::scopes(&real_pages());
        return;
    }
    if args.iter().any(|a| a == "js") {
        header();
        js::run(&real_pages());
        return;
    }
    if args.iter().any(|a| a == "urls") {
        header();
        urls::run(&real_pages());
        return;
    }
    let pages = pages::chosen(&args);
    header();
    println!(
        "{:<28} {:>9}  {:>18}  {:>18}  {:>18}  {:>9}",
        "MB/s, peak MB", "size", PARSERS[0], PARSERS[1], PARSERS[2], "vs Lexbor"
    );

    let mut group = "";
    let mut total_bytes = 0;
    let mut total_secs = [0.0; 3];
    for page in &pages {
        if page.group != group {
            if group == "real" {
                total_row(total_bytes, &total_secs);
            }
            group = page.group;
            println!("{}", page.group_title());
        }
        let results: Vec<Outcome> = (0..3)
            .map(|i| match page.group {
                "hostile" => measure_apart(i, &page.name),
                _ => Outcome::Done(measure(i, &page.html)),
            })
            .collect();
        let bytes = page.html.len();
        if group == "real" {
            total_bytes += bytes;
            for (sum, r) in total_secs.iter_mut().zip(&results) {
                if let Outcome::Done(r) = r {
                    *sum += r.secs;
                }
            }
        }
        let cells: Vec<String> = results
            .iter()
            .map(|r| match r {
                Outcome::Done(r) if mb_per_s(bytes, r.secs) < 1.0 => {
                    format!("{:>6.1} s {:>9.1}", r.secs, r.peak as f64 / 1e6)
                }
                Outcome::Done(r) => format!(
                    "{:>8.0} {:>9.1}",
                    mb_per_s(bytes, r.secs),
                    r.peak as f64 / 1e6
                ),
                Outcome::TooSlow => format!("{:>18}", "over 2 min"),
                Outcome::Crashed => format!("{:>18}", "crashed"),
            })
            .collect();
        let ratio = match (&results[0], &results[1]) {
            (Outcome::Done(p), Outcome::Done(l)) => format!("{:>8.2}x", l.secs / p.secs),
            _ => format!("{:>9}", "-"),
        };
        println!(
            "  {:<26} {:>6.2} MB  {}  {}  {}  {ratio}",
            page.name,
            bytes as f64 / 1e6,
            cells[0],
            cells[1],
            cells[2],
        );
    }
    if group == "real" {
        total_row(total_bytes, &total_secs);
    }
    if args.is_empty() {
        selectors::run(&real_pages());
        urls::run(&real_pages());
        css::run(&real_pages());
    }
}

fn real_pages() -> Vec<pages::Page> {
    pages::chosen(&[])
        .into_iter()
        .filter(|p| p.group == "real")
        .collect()
}

/// The versions and the machine.
fn header() {
    println!(
        "Lexbor {}, html5ever {}, Ada {}, Parsec {}; {} {}, {} threads available\n",
        env!("LEXBOR_VERSION"),
        env!("HTML5EVER_VERSION"),
        env!("ADA_VERSION"),
        env!("PARSEC_COMMIT"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(1, |n| n.get()),
    );
}

/// The real pages as one corpus: all their bytes over all their medians.
fn total_row(bytes: usize, secs: &[f64; 3]) {
    println!(
        "  {:<26} {:>6.2} MB  {:>8.0} {:>9}  {:>8.0} {:>9}  {:>8.0} {:>9}  {:>8.2}x",
        "all real pages",
        bytes as f64 / 1e6,
        mb_per_s(bytes, secs[0]),
        "",
        mb_per_s(bytes, secs[1]),
        "",
        mb_per_s(bytes, secs[2]),
        "",
        secs[1] / secs[0]
    );
}
