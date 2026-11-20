// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The pages to parse: the real ones `bench/fetch-pages.sh` downloads, two
//! generated shop listings, and the hostile inputs from
//! `crates/parsec/examples/hostile.rs`.

use std::path::Path;

pub struct Page {
    pub name: String,
    pub group: &'static str,
    pub html: String,
}

impl Page {
    pub fn group_title(&self) -> &'static str {
        match self.group {
            "real" => "Real pages",
            "generated" => "Generated shop listings",
            _ => "Hostile inputs",
        }
    }
}

/// Every page whose name contains one of `only`, or every page if `only`
/// is empty. Only the real pages chosen need to be in bench/corpus.
pub fn chosen(only: &[String]) -> Vec<Page> {
    let wanted = |name: &str| only.is_empty() || only.iter().any(|o| name.contains(o.as_str()));
    let mut pages = real(&wanted);
    for (name, size) in [("shop 1 MB", 1_000_000), ("shop 10 MB", 10_000_000)] {
        if wanted(name) {
            pages.push(Page {
                name: name.into(),
                group: "generated",
                html: shop(size),
            });
        }
    }
    pages.extend(hostile().into_iter().filter(|p| wanted(&p.name)));
    pages
}

/// The pages listed in pages.txt that `wanted` picks, read from
/// bench/corpus.
fn real(wanted: &dyn Fn(&str) -> bool) -> Vec<Page> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(dir.join("pages.txt")).expect("bench/pages.txt");
    list.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|line| line.split_whitespace().next().unwrap())
        .filter(|name| wanted(name))
        .map(|name| {
            let path = dir.join("corpus").join(format!("{name}.html"));
            let bytes = std::fs::read(&path).unwrap_or_else(|_| {
                panic!("{} is missing: run bench/fetch-pages.sh", path.display())
            });
            Page {
                name: name.into(),
                group: "real",
                // Every parser gets the same text; a page that isn't UTF-8
                // gets U+FFFD for the bad bytes, for all three.
                html: String::from_utf8_lossy(&bytes).into_owned(),
            }
        })
        .collect()
}

pub(crate) const WORDS: [&str; 22] = [
    "light", "bright", "hollow", "river", "stone", "paper", "glass", "winter", "amber", "copper",
    "quiet", "field", "garden", "harbour", "lantern", "meadow", "north", "orchard", "silver",
    "timber", "velvet", "willow",
];

/// A small, fixed random sequence (xorshift), so every run builds the same
/// page.
pub(crate) struct Rng(pub(crate) u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub(crate) fn words(&mut self, n: usize) -> String {
        (0..n)
            .map(|_| WORDS[self.below(WORDS.len())])
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// A shop's product listing of about `target` bytes, the shape of page most
/// scrapers parse: the same as Netweir's bench/pages.py.
pub fn shop(target: usize) -> String {
    let mut rng = Rng(7);
    let mut html =
        String::from("<!doctype html><html lang=en><head><meta charset=utf-8><title>Shop</title>");
    for i in 0..10 {
        html += &format!("<link rel=stylesheet href=\"/static/{i}.css\">");
    }
    html += "</head><body><header><nav><ul>";
    for w in WORDS {
        html += &format!("<li><a href=\"/c/{w}\">{w}</a></li>");
    }
    html += "</ul></nav></header><main><ol class=row>";
    let mut i = 0;
    while html.len() < target {
        let stock = if rng.below(5) > 0 {
            "In stock"
        } else {
            "Sold out"
        };
        let rating = ["One", "Two", "Three", "Four", "Five"][rng.below(5)];
        let price = 5.0 + rng.below(8500) as f64 / 100.0;
        let description = 20 + rng.below(41);
        html += &format!(
            r#"<li>
<article class="product_pod" data-id="{i}">
  <div class="image_container"><a href="/catalogue/item-{i}/index.html"><img src="/media/{i}.jpg" alt="{}" class="thumbnail"></a></div>
  <p class="star-rating {rating}"><i class="icon-star"></i><i class="icon-star"></i></p>
  <h3><a href="/catalogue/item-{i}/index.html" title="{}">{}</a></h3>
  <div class="product_price">
    <p class="price_color">£{price:.2}</p>
    <p class="instock availability"><i class="icon-ok"></i> {stock}</p>
    <form><button type="submit" class="btn btn-primary btn-block" data-loading-text="Adding...">Add to basket</button></form>
  </div>
  <p class="description">{}</p>
</article></li>"#,
            rng.words(3),
            rng.words(5),
            rng.words(4),
            rng.words(description),
        );
        i += 1;
    }
    html += r#"</ol><ul class=pager><li class=next><a href="page-2.html">next</a></li></ul></main></body></html>"#;
    html
}

/// The inputs crates/parsec/examples/hostile.rs times, built to make a
/// parser slow.
fn hostile() -> Vec<Page> {
    let fmt: String = (0..300).map(|i| format!("<b class=c{i}>")).collect();
    [
        ("unclosed divs", "<div>".repeat(80_000)),
        ("unclosed tables", "<table>".repeat(40_000)),
        ("nested inline", "<span>".repeat(80_000)),
        ("formatting then paragraphs", fmt + &"<p>x".repeat(3_000)),
        (
            "distinct formatting per p",
            "<p><b><i><u><s><em><strong><tt><code><big><small><strike>".repeat(5_000),
        ),
        ("misnested a", "<a><p><a>x</p>".repeat(20_000)),
        ("adoption", "<b><div><b><div>".repeat(10_000)),
        (
            "table foster",
            "<table>".to_string() + &"x<td>y".repeat(50_000),
        ),
    ]
    .into_iter()
    .map(|(name, html)| Page {
        name: name.into(),
        group: "hostile",
        html,
    })
    .collect()
}
