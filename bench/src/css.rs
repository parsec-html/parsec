// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times CSS: Parsec's `parsec-css` against Lexbor's CSS module (through
//! `src/lexbor_css.c`) and Servo's `cssparser`, on real stylesheets: the
//! `<style>` contents of each real page, all of a page's elements as one
//! sheet, and the stylesheets in `bench/stylesheets.txt`.
//!
//! Two jobs. Tokenizing is every token in the sheet, comments skipped.
//! Parsing is the whole sheet into rules, with every block's contents
//! parsed into declarations and nested rules, all of it kept until the
//! clock stops. Lexbor's parser does more than that: it also parses each
//! selector and the values of the properties it knows, which a syntax
//! parser leaves for later. `cssparser` builds nothing of its own, so here
//! it's given the least a caller would: each prelude and value kept as its
//! tokens, and each block parsed as rules and declarations.

use std::ffi::c_void;
use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use cssparser::{
    AtRuleParser, CowRcStr, DeclarationParser, ParseError, Parser, ParserState,
    QualifiedRuleParser, RuleBodyItemParser, RuleBodyParser, StyleSheetParser, Token,
};

use crate::pages::Page;

unsafe extern "C" {
    fn bench_css_create() -> *mut c_void;
    fn bench_css_destroy(b: *mut c_void);
    fn bench_css_tokens(b: *mut c_void, css: *const u8, len: usize) -> usize;
    fn bench_css_parse(b: *mut c_void, css: *const u8, len: usize) -> *mut c_void;
    fn bench_css_free(sheet: *mut c_void);
}

/// Lexbor's tokenizer and parser, made once.
struct Lexbor(*mut c_void);

impl Lexbor {
    fn new() -> Lexbor {
        // SAFETY: no arguments; NULL is checked.
        let b = unsafe { bench_css_create() };
        assert!(!b.is_null(), "Lexbor's CSS parser didn't initialise");
        Lexbor(b)
    }

    fn tokens(&self, css: &str) -> usize {
        // SAFETY: Lexbor reads `css.len()` bytes from `css`.
        unsafe { bench_css_tokens(self.0, css.as_ptr(), css.len()) }
    }

    fn parse(&self, css: &str) -> LexborSheet {
        // SAFETY: as for `tokens`; the sheet is freed once, in its `drop`.
        let sheet = unsafe { bench_css_parse(self.0, css.as_ptr(), css.len()) };
        assert!(!sheet.is_null(), "Lexbor couldn't parse a stylesheet");
        LexborSheet(sheet)
    }
}

impl Drop for Lexbor {
    fn drop(&mut self) {
        // SAFETY: made in `new`, destroyed only here.
        unsafe { bench_css_destroy(self.0) }
    }
}

struct LexborSheet(*mut c_void);

impl Drop for LexborSheet {
    fn drop(&mut self) {
        // SAFETY: made by `bench_css_parse`, freed only here.
        unsafe { bench_css_free(self.0) }
    }
}

/// What Parsec's parse keeps: every prelude and every declaration.
type Parsed<'a> = (
    Vec<Vec<parsec_css::ComponentValue<'a>>>,
    Vec<parsec_css::Declaration<'a>>,
);

/// Parsec: the stylesheet, then every block's contents, depth first.
fn parsec_parse<'a>(css: &'a str) -> Parsed<'a> {
    use parsec_css::{Item, Rule};
    let (mut preludes, mut declarations) = (Vec::new(), Vec::new());
    let mut blocks = Vec::new();
    let take = |rule: Rule<'a>, preludes: &mut Vec<_>, blocks: &mut Vec<_>| match rule {
        Rule::Qualified(r) => {
            preludes.push(r.prelude);
            blocks.push(r.block);
        }
        Rule::At(r) => {
            preludes.push(r.prelude);
            blocks.extend(r.block);
        }
    };
    for rule in parsec_css::parse_stylesheet(css).rules {
        take(rule, &mut preludes, &mut blocks);
    }
    while let Some(block) = blocks.pop() {
        for item in block.contents(css).items {
            match item {
                Item::Declaration(d) => declarations.push(d),
                Item::Rule(r) => take(r, &mut preludes, &mut blocks),
            }
        }
    }
    (preludes, declarations)
}

/// Every token, into nested blocks too.
fn cssparser_tokens(input: &mut Parser<'_>) -> usize {
    let mut n = 0;
    while let Ok(t) = input.next_including_whitespace() {
        n += 1;
        if matches!(
            t,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        ) {
            n += input
                .parse_nested_block(|p| Ok::<_, ParseError<()>>(cssparser_tokens(p)))
                .unwrap();
        }
    }
    n
}

/// What a value or prelude is kept as for `cssparser`: its tokens.
fn keep<'i>(input: &mut Parser<'i>, out: &mut Vec<Token<'i>>) {
    while let Ok(t) = input.next_including_whitespace() {
        let t = t.clone();
        let nested = matches!(
            t,
            Token::Function(_)
                | Token::ParenthesisBlock
                | Token::SquareBracketBlock
                | Token::CurlyBracketBlock
        );
        out.push(t);
        if nested {
            let _ = input.parse_nested_block(|p| {
                keep(p, out);
                Ok::<_, ParseError<()>>(())
            });
        }
    }
}

#[allow(dead_code)] // Built to be kept, not read.
enum Item<'i> {
    Declaration(CowRcStr<'i>, Vec<Token<'i>>),
    Rule(Vec<Token<'i>>, Option<Vec<Item<'i>>>),
}

struct Rules;

impl<'i> QualifiedRuleParser<'i> for Rules {
    type Prelude = Vec<Token<'i>>;
    type QualifiedRule = Item<'i>;
    type Error = ();

    fn parse_prelude(&mut self, input: &mut Parser<'i>) -> Result<Vec<Token<'i>>, ParseError<()>> {
        let mut out = Vec::new();
        keep(input, &mut out);
        Ok(out)
    }

    fn parse_block(
        &mut self,
        prelude: Vec<Token<'i>>,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Item<'i>, ParseError<()>> {
        Ok(Item::Rule(prelude, Some(body(input))))
    }
}

impl<'i> AtRuleParser<'i> for Rules {
    type Prelude = Vec<Token<'i>>;
    type AtRule = Item<'i>;
    type Error = ();

    fn parse_prelude(
        &mut self,
        _: CowRcStr<'i>,
        input: &mut Parser<'i>,
    ) -> Result<Vec<Token<'i>>, ParseError<()>> {
        let mut out = Vec::new();
        keep(input, &mut out);
        Ok(out)
    }

    fn rule_without_block(
        &mut self,
        prelude: Vec<Token<'i>>,
        _: &ParserState,
    ) -> Result<Item<'i>, ()> {
        Ok(Item::Rule(prelude, None))
    }

    fn parse_block(
        &mut self,
        prelude: Vec<Token<'i>>,
        _: &ParserState,
        input: &mut Parser<'i>,
    ) -> Result<Item<'i>, ParseError<()>> {
        Ok(Item::Rule(prelude, Some(body(input))))
    }
}

impl<'i> DeclarationParser<'i> for Rules {
    type Declaration = Item<'i>;
    type Error = ();

    fn parse_value(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i>,
        _: &ParserState,
    ) -> Result<Item<'i>, ParseError<()>> {
        let mut out = Vec::new();
        keep(input, &mut out);
        Ok(Item::Declaration(name, out))
    }
}

impl<'i> RuleBodyItemParser<'i, Item<'i>, ()> for Rules {
    fn parse_declarations(&self) -> bool {
        true
    }

    fn parse_qualified(&self) -> bool {
        true
    }
}

fn body<'i>(input: &mut Parser<'i>) -> Vec<Item<'i>> {
    RuleBodyParser::new(input, &mut Rules)
        .filter_map(Result::ok)
        .collect()
}

fn cssparser_parse(css: &str) -> Vec<Item<'_>> {
    let mut parser = Parser::new(css);
    StyleSheetParser::new(&mut parser, &mut Rules)
        .filter_map(Result::ok)
        .collect()
}

/// The median of `runs` timings of `f`, after a warm-up, in seconds; what
/// `f` returns is dropped outside the timing.
fn median<T>(runs: usize, mut f: impl FnMut() -> T) -> f64 {
    drop(black_box(f()));
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            let kept = black_box(f());
            let secs = t.elapsed().as_secs_f64();
            drop(kept);
            secs
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// The sheets: each real page's `<style>` elements together, then the
/// fetched stylesheets.
fn sheets(pages: &[Page]) -> Vec<(String, String)> {
    let style = parsec::css::SelectorList::parse("style").unwrap();
    let mut sheets: Vec<(String, String)> = pages
        .iter()
        .filter_map(|p| {
            let doc = parsec::parse(&p.html, false);
            let css: Vec<String> = style
                .select(&doc, doc.root())
                .into_iter()
                .map(|n| doc.text(n))
                .collect();
            let css = css.join("\n");
            (css.len() > 1000).then(|| (format!("{} <style>", p.name), css))
        })
        .collect();
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let list = std::fs::read_to_string(dir.join("stylesheets.txt")).expect("bench/stylesheets.txt");
    for name in list
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_whitespace().next())
    {
        let path = dir.join("corpus").join(format!("{name}.css"));
        let css = std::fs::read_to_string(&path)
            .unwrap_or_else(|_| panic!("{} is missing: run bench/fetch-pages.sh", path.display()));
        sheets.push((format!("{name}.css"), css));
    }
    sheets
}

pub fn run(pages: &[Page]) {
    let lexbor = Lexbor::new();
    let sheets = sheets(pages);
    let total: usize = sheets.iter().map(|(_, c)| c.len()).sum();
    println!(
        "\nCSS: {} stylesheets, {:.2} MB, MB/s (median of 30)\n{:<32} {:>8}  {:>8} {:>8} {:>9} {:>6} {:>6}  {:>8} {:>8} {:>9} {:>6} {:>6}",
        sheets.len(),
        total as f64 / 1e6,
        "",
        "KB",
        "Parsec",
        "Lexbor",
        "cssparser",
        "vs L",
        "vs c",
        "Parsec",
        "Lexbor",
        "cssparser",
        "vs L",
        "vs c",
    );
    println!("{:<42}{:^44}  {:^44}", "", "tokenize", "parse");
    let mut sums = [0.0f64; 6];
    for (name, css) in &sheets {
        let times = [
            median(30, || parsec_css::Tokenizer::new(css).count()),
            median(30, || lexbor.tokens(css)),
            median(30, || cssparser_tokens(&mut Parser::new(css))),
            median(30, || parsec_parse(css)),
            median(30, || lexbor.parse(css)),
            median(30, || cssparser_parse(css)),
        ];
        for (sum, t) in sums.iter_mut().zip(times) {
            *sum += t;
        }
        row(name, css.len(), &times);
    }
    row("all stylesheets", total, &sums);
}

fn row(name: &str, bytes: usize, t: &[f64; 6]) {
    let mbs = |s: f64| bytes as f64 / 1e6 / s;
    println!(
        "  {:<30} {:>8.0}  {:>8.0} {:>8.0} {:>9.0} {:>5.2}x {:>5.2}x  {:>8.0} {:>8.0} {:>9.0} {:>5.2}x {:>5.2}x",
        name,
        bytes as f64 / 1e3,
        mbs(t[0]),
        mbs(t[1]),
        mbs(t[2]),
        t[1] / t[0],
        t[2] / t[0],
        mbs(t[3]),
        mbs(t[4]),
        mbs(t[5]),
        t[4] / t[3],
        t[5] / t[3],
    );
}
