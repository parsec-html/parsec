# SPDX-License-Identifier: Apache-2.0
# Copyright 2026 James Conway
"""Times parsec_html against selectolax's Lexbor backend on bench/'s real
pages: parsing, the 32 selectors bench/ times in Rust, and the loop a
scraper runs (every link's href and text).

    bench/fetch-pages.sh
    pip install selectolax parsec-html   # or maturin develop --release
    python bindings/python/bench.py [bench/corpus] [--runs N]
"""

import argparse
import pathlib
import statistics
import time

import selectolax
from selectolax.lexbor import LexborHTMLParser

import parsec_html

# The list in bench/src/selectors.rs.
SELECTORS = [
    "*", "a", "div a", "ul > li", "li + li", "li ~ li", "[href]",
    "a[href^='/']", "a[href$='.html']", "a[href*='?']", "[class]", "img[alt]",
    "div:first-child", "li:last-child", "p:nth-child(2n+1)",
    "tr:nth-of-type(odd)", "span:only-child", ":root > body",
    "div:not([class])", ":is(h1, h2, h3)", "article:has(img)",
    "meta[name][content]", "script:empty", "td:nth-last-child(2)",
    "nav ul li a", "div > div > div", "[data-testid]", "input[name]",
    "a:first-of-type", "section h2 + p", "li:nth-child(-n+3)",
    "body *:last-child",
]

LIBRARIES = {"parsec_html": parsec_html.HTMLParser, "selectolax": LexborHTMLParser}


def median_ms(fn, runs):
    times = []
    for _ in range(runs):
        start = time.perf_counter()
        fn()
        times.append(time.perf_counter() - start)
    return statistics.median(times) * 1000


def scrape(doc):
    return [(a.attributes.get("href"), a.text()) for a in doc.css("a")]


def main():
    here = pathlib.Path(__file__).resolve().parent
    ap = argparse.ArgumentParser()
    ap.add_argument("corpus", nargs="?", default=here.parent.parent / "bench" / "corpus")
    ap.add_argument("--runs", type=int, default=20)
    args = ap.parse_args()
    pages = sorted(pathlib.Path(args.corpus).glob("*.html"))
    if not pages:
        raise SystemExit(f"no pages in {args.corpus}: run bench/fetch-pages.sh")
    html = {p.stem: p.read_text("utf-8") for p in pages}
    total_mb = sum(len(h.encode()) for h in html.values()) / 1e6
    print(f"parsec_html {parsec_html.__version__}, selectolax {selectolax.__version__}; "
          f"{len(html)} pages, {total_mb:.1f} MB, median ms of {args.runs} runs\n")

    print(f"{'parse':<24} {'parsec_html':>12} {'selectolax':>12} {'ratio':>8}")
    sums = dict.fromkeys(LIBRARIES, 0.0)
    for name, h in html.items():
        row = {lib: median_ms(lambda: make(h), args.runs) for lib, make in LIBRARIES.items()}
        for lib in LIBRARIES:
            sums[lib] += row[lib]
        print(f"  {name:<22} {row['parsec_html']:>12.2f} {row['selectolax']:>12.2f} "
              f"{row['selectolax'] / row['parsec_html']:>7.2f}x")
    print(f"  {'all pages':<22} {sums['parsec_html']:>12.2f} {sums['selectolax']:>12.2f} "
          f"{sums['selectolax'] / sums['parsec_html']:>7.2f}x")

    docs = {lib: [make(h) for h in html.values()] for lib, make in LIBRARIES.items()}
    print(f"\n{'css(), all pages':<24} {'parsec_html':>12} {'selectolax':>12} {'ratio':>8} {'matches':>16}")
    sums = dict.fromkeys(LIBRARIES, 0.0)
    for sel in SELECTORS:
        row, found = {}, {}
        for lib in LIBRARIES:
            row[lib] = median_ms(lambda: [d.css(sel) for d in docs[lib]], args.runs)
            found[lib] = sum(len(d.css(sel)) for d in docs[lib])
            sums[lib] += row[lib]
        counts = (str(found["parsec_html"]) if found["parsec_html"] == found["selectolax"]
                  else f"{found['parsec_html']} vs {found['selectolax']}")
        print(f"  {sel:<22} {row['parsec_html']:>12.2f} {row['selectolax']:>12.2f} "
              f"{row['selectolax'] / row['parsec_html']:>7.2f}x {counts:>16}")
    print(f"  {'all selectors':<22} {sums['parsec_html']:>12.2f} {sums['selectolax']:>12.2f} "
          f"{sums['selectolax'] / sums['parsec_html']:>7.2f}x")

    row = {lib: median_ms(lambda: [scrape(d) for d in docs[lib]], args.runs) for lib in LIBRARIES}
    print(f"\n{'links: href and text':<24} {row['parsec_html']:>12.2f} {row['selectolax']:>12.2f} "
          f"{row['selectolax'] / row['parsec_html']:>7.2f}x")


if __name__ == "__main__":
    main()
