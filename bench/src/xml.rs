// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times XML parsing: Parsec's `parsec-xml` against libxml2's C (built
//! from bench/libxml2 at a release tag), roxmltree and quick-xml, on the
//! documents `bench/fetch-xml.sh` downloads: sitemaps, RSS and Atom feeds,
//! SVGs, the XML spec in XML, and a slice of a Wikipedia dump.
//!
//! Parsec, libxml2 and roxmltree each build a whole tree with namespaces
//! resolved and keep it until the clock stops. quick-xml builds no tree:
//! it's a pull parser, timed reading every event with namespaces
//! resolved and every text and attribute value unescaped, which is the
//! least a program reading the document has to do with it. Parsec and
//! roxmltree are given the text as a `&str`; libxml2 gets the bytes and
//! checks the UTF-8 itself.
//!
//! A time is the median of at least 30 runs after three warm-up runs,
//! the same as the HTML table's, and peak memory the most the heap grew
//! while one document was built and held. Run it with
//! `cargo run --release --manifest-path bench/Cargo.toml -- xml`.

use std::hint::black_box;
use std::path::Path;

use crate::{MIN_RUNS, SLOW_RUNS, TIME_PER_PAGE, mb_per_s};

const PARSERS: [&str; 4] = ["Parsec", "libxml2", "roxmltree", "quick-xml"];

/// Anything a parse built, held until it's dropped outside the clock.
trait Held {}
impl<T> Held for T {}

/// What one parse built; None if the parser refused the document.
type Built<'a> = Option<Box<dyn Held + 'a>>;

fn held<'a, T: 'a>(t: T) -> Box<dyn Held + 'a> {
    Box::new(t)
}

fn parsec(src: &str) -> Built<'_> {
    parsec_xml::parse(src).ok().map(held)
}

fn libxml2(src: &str) -> Built<'_> {
    crate::libxml2::Document::parse(src.as_bytes()).map(held)
}

fn roxmltree(src: &str) -> Built<'_> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    roxmltree::Document::parse_with_options(src, options)
        .ok()
        .map(held)
}

fn quick_xml(src: &str) -> Built<'_> {
    use quick_xml::XmlVersion::Implicit1_0 as V;
    use quick_xml::escape::resolve_predefined_entity;
    use quick_xml::events::Event;
    let mut reader = quick_xml::NsReader::from_str(src);
    let mut seen = 0usize;
    loop {
        match reader.read_resolved_event() {
            Ok((ns, Event::Start(e) | Event::Empty(e))) => {
                black_box(ns);
                for a in e.attributes() {
                    seen += a.ok()?.normalized_value(V).ok()?.len();
                }
            }
            Ok((_, Event::Text(t))) => seen += t.xml_content(V).len(),
            // A reference comes as an event of its own.
            Ok((_, Event::GeneralRef(r))) => match r.resolve_char_ref().ok()? {
                Some(c) => seen += c.len_utf8(),
                None => seen += resolve_predefined_entity(&r.xml_content(V))?.len(),
            },
            Ok((_, Event::CData(c))) => seen += c.xml_content(V).len(),
            Ok((_, Event::Eof)) => break,
            Ok(_) => {}
            Err(_) => return None,
        }
    }
    Some(held(seen))
}

fn parse(which: usize, src: &str) -> Built<'_> {
    match which {
        0 => parsec(src),
        1 => libxml2(src),
        2 => roxmltree(src),
        _ => quick_xml(src),
    }
}

/// A document, or a set of them parsed one after another and all held.
struct Doc {
    name: String,
    pieces: Vec<String>,
}

impl Doc {
    fn bytes(&self) -> usize {
        self.pieces.iter().map(String::len).sum()
    }
}

/// The documents in bench/corpus/xml, by the order of bench/xml.txt: a
/// file each, or a directory of files for a set.
fn documents() -> Vec<Doc> {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(here.join("xml.txt")).unwrap();
    let mut docs = Vec::new();
    for line in list.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(source)) = (fields.next(), fields.next()) else {
            continue;
        };
        if name.starts_with('#') {
            continue;
        }
        let missing =
            |p: &Path| -> ! { panic!("{} is missing: run bench/fetch-xml.sh", p.display()) };
        let pieces = if source == "npm" {
            let dir = here.join("corpus/xml").join(name);
            let mut files: Vec<_> = std::fs::read_dir(&dir)
                .unwrap_or_else(|_| missing(&dir))
                .map(|e| e.unwrap().path())
                .collect();
            files.sort();
            files
                .iter()
                .map(|f| std::fs::read_to_string(f).unwrap())
                .collect()
        } else {
            let path = here.join("corpus/xml").join(format!("{name}.xml"));
            vec![std::fs::read_to_string(&path).unwrap_or_else(|_| missing(&path))]
        };
        docs.push(Doc {
            name: name.to_string(),
            pieces,
        });
    }
    docs
}

/// Every piece parsed and held, or None if the parser refused one.
fn parse_all<'a>(which: usize, doc: &'a Doc) -> Option<Vec<Box<dyn Held + 'a>>> {
    doc.pieces
        .iter()
        .map(|p| parse(which, black_box(p)))
        .collect()
}

/// Median seconds and peak bytes, or None if the parser refused it.
fn measure(which: usize, doc: &Doc) -> Option<(f64, usize)> {
    use std::sync::atomic::Ordering::Relaxed;
    use std::time::{Duration, Instant};
    drop(black_box(parse_all(which, doc))?);
    for _ in 0..2 {
        drop(black_box(parse_all(which, doc)));
    }
    let t = Instant::now();
    drop(black_box(parse_all(which, doc)));
    let once = t.elapsed().max(Duration::from_micros(1));
    let runs = if once > Duration::from_secs(1) {
        SLOW_RUNS as f64
    } else {
        (TIME_PER_PAGE.as_secs_f64() / once.as_secs_f64()).clamp(MIN_RUNS as f64, 2000.0)
    };
    let mut times: Vec<f64> = (0..runs as usize)
        .map(|_| {
            let t = Instant::now();
            let built = black_box(parse_all(which, doc));
            let secs = t.elapsed().as_secs_f64();
            drop(built);
            secs
        })
        .collect();
    times.sort_by(f64::total_cmp);
    crate::COUNTING.store(true, Relaxed);
    crate::IN_USE.store(0, Relaxed);
    crate::PEAK.store(0, Relaxed);
    let built = black_box(parse_all(which, doc));
    let peak = crate::PEAK.load(Relaxed);
    crate::COUNTING.store(false, Relaxed);
    drop(built);
    Some((times[times.len() / 2], peak))
}

pub fn run() {
    crate::libxml2::init();
    let docs = documents();
    println!(
        "XML: parsec-xml, libxml2 {}, roxmltree {}, quick-xml {} (events only, no tree)\n",
        env!("LIBXML2_VERSION"),
        env!("ROXMLTREE_VERSION"),
        env!("QUICK_XML_VERSION"),
    );
    println!(
        "{:<30} {:>9}  {:>16}  {:>16}  {:>16}  {:>16}  {:>10}",
        "MB/s, peak MB", "size", PARSERS[0], PARSERS[1], PARSERS[2], PARSERS[3], "vs libxml2"
    );
    let mut total_bytes = 0;
    let mut total_secs = [0.0; 4];
    let mut all = [true; 4];
    for doc in &docs {
        let results: Vec<_> = (0..4).map(|i| measure(i, doc)).collect();
        let bytes = doc.bytes();
        total_bytes += bytes;
        for (i, r) in results.iter().enumerate() {
            match r {
                Some((secs, _)) => total_secs[i] += secs,
                None => all[i] = false,
            }
        }
        let cells: Vec<String> = results
            .iter()
            .map(|r| match r {
                Some((secs, peak)) => format!(
                    "{:>7.0} {:>8.1}",
                    mb_per_s(bytes, *secs),
                    *peak as f64 / 1e6
                ),
                None => format!("{:>16}", "refused"),
            })
            .collect();
        let ratio = match (&results[0], &results[1]) {
            (Some((p, _)), Some((l, _))) => format!("{:>9.2}x", l / p),
            _ => format!("{:>10}", "-"),
        };
        println!(
            "  {:<28} {:>6.2} MB  {}  {}  {}  {}  {ratio}",
            doc.name,
            bytes as f64 / 1e6,
            cells[0],
            cells[1],
            cells[2],
            cells[3],
        );
    }
    let cell = |i: usize| {
        if all[i] {
            format!("{:>7.0} {:>8}", mb_per_s(total_bytes, total_secs[i]), "")
        } else {
            format!("{:>16}", "-")
        }
    };
    println!(
        "  {:<28} {:>6.2} MB  {}  {}  {}  {}  {:>9.2}x",
        "all documents",
        total_bytes as f64 / 1e6,
        cell(0),
        cell(1),
        cell(2),
        cell(3),
        total_secs[1] / total_secs[0]
    );
}
