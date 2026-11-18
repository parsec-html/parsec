// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Bytes into Parsec. [`parse_bytes`] finds a page's encoding the way the
//! HTML Standard says a browser does, decodes it and parses it.
//!
//! The decoding is [`encoding_rs`], Firefox's implementation of the WHATWG
//! Encoding Standard, re-exported here. It has every encoding the web
//! uses, decoding and encoding, whole or streamed:
//!
//! ```
//! use parsec_encoding::Encoding;
//!
//! let sjis = Encoding::for_label(b"sjis").unwrap();
//! assert_eq!(sjis.name(), "Shift_JIS");
//! let (text, _, had_errors) = sjis.decode(b"\x93\xfa\x96\x7b");
//! assert_eq!((&*text, had_errors), ("日本", false));
//! let (bytes, _, _) = sjis.encode("日本");
//! assert_eq!(&*bytes, b"\x93\xfa\x96\x7b");
//! ```

use std::time::Instant;

pub use encoding_rs::{self, Encoding};
use encoding_rs::{UTF_16BE, UTF_16LE};
use parsec::{AttrNs, Document, Kind, NodeId, Ns, ParseError};

mod prescan;
pub use prescan::prescan;

/// How sure the parser is of an encoding, in the HTML Standard's terms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confidence {
    /// A guess: from the prescan or the fallback. A `<meta>` the parser
    /// meets can still change it.
    Tentative,
    /// From a byte order mark, the transport layer, or a `<meta>` that
    /// agreed with the guess or replaced it.
    Certain,
}

/// How to turn bytes into a document.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// The parse itself.
    pub html: parsec::Options,
    /// The encoding the transport layer gave, such as the `charset` in an
    /// HTTP `Content-Type`: look it up with [`Encoding::for_label`]. A byte
    /// order mark still wins over it, as in browsers.
    pub transport: Option<&'static Encoding>,
    /// The encoding when nothing in or around the page names one:
    /// windows-1252 by default, as in browsers set to English.
    /// [`locale_default`] gives the one a browser in another language
    /// would use.
    pub fallback: &'static Encoding,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            html: parsec::Options::default(),
            transport: None,
            fallback: encoding_rs::WINDOWS_1252,
        }
    }
}

/// A document parsed from bytes, and how its encoding was decided.
pub struct Parsed {
    pub document: Document,
    /// The encoding the document was decoded with: what JavaScript's
    /// `document.characterSet` would say.
    pub encoding: &'static Encoding,
    pub confidence: Confidence,
}

/// The HTML Standard's encoding sniffing algorithm, for a parser that has
/// every byte: the byte order mark, then `transport`, then the prescan of
/// the first 1,024 bytes, then `fallback`.
///
/// ```
/// use parsec_encoding::{Confidence, sniff};
/// let page = b"<meta charset=windows-1251><p>\xcf\xf0\xe8\xe2\xe5\xf2";
/// let found = sniff(page, None, encoding_rs::WINDOWS_1252);
/// assert_eq!(found, (encoding_rs::WINDOWS_1251, Confidence::Tentative));
/// ```
pub fn sniff(
    bytes: &[u8],
    transport: Option<&'static Encoding>,
    fallback: &'static Encoding,
) -> (&'static Encoding, Confidence) {
    if let Some((bom, _)) = Encoding::for_bom(bytes) {
        return (bom, Confidence::Certain);
    }
    if let Some(given) = transport {
        return (given, Confidence::Certain);
    }
    match prescan(&bytes[..bytes.len().min(1024)]) {
        Some(declared) => (declared, Confidence::Tentative),
        None => (fallback, Confidence::Tentative),
    }
}

/// Parses a page given as bytes, as a browser would: sniffs its encoding
/// ([`sniff`]), decodes it and parses it.
///
/// When the encoding was only a guess and the parser meets a `<meta>`
/// declaring another one (past the first 1,024 bytes, say), the spec has
/// the browser start again in the declared encoding. Here that's a second
/// decode and parse of the same bytes, once at most, since the encoding
/// is certain after it. The time budget covers both parses together.
///
/// ```
/// use parsec_encoding::{Options, parse_bytes};
/// let page = b"<meta charset=shift_jis><title>\x93\xfa\x96\x7b</title>";
/// let parsed = parse_bytes(page, Options::default()).unwrap();
/// assert_eq!(parsed.encoding, encoding_rs::SHIFT_JIS);
/// assert_eq!(parsed.document.text(parsed.document.root()), "日本");
/// ```
pub fn parse_bytes(bytes: &[u8], options: Options) -> Result<Parsed, ParseError> {
    let start = Instant::now();
    let (mut encoding, mut confidence) = sniff(bytes, options.transport, options.fallback);
    let mut document = decode_and_parse(bytes, encoding, options.html)?;
    if confidence == Confidence::Tentative
        && let Some(declared) = declared(&document)
    {
        confidence = Confidence::Certain;
        if let Some(new) = changed(encoding, declared) {
            encoding = new;
            let mut html = options.html;
            html.budget = html.budget.map(|b| b.saturating_sub(start.elapsed()));
            document = decode_and_parse(bytes, encoding, html)?;
        }
    }
    Ok(Parsed {
        document,
        encoding,
        confidence,
    })
}

fn decode_and_parse(
    bytes: &[u8],
    encoding: &'static Encoding,
    options: parsec::Options,
) -> Result<Document, ParseError> {
    // Any byte order mark already chose `encoding`, so removing one for
    // `encoding` is the Encoding Standard's "decode".
    let (text, _) = encoding.decode_with_bom_removal(bytes);
    parsec::parse_with(&text, options)
}

/// The first encoding a `<meta>` declares as the tree builder sees it: by
/// `charset`, or by `http-equiv="Content-Type"` with a `content` naming
/// one. The parser makes elements in the order it meets their tags, so
/// node order is that order, `<template>` contents included.
fn declared(doc: &Document) -> Option<&'static Encoding> {
    (0..doc.len() as NodeId).find_map(|id| {
        if !matches!(doc.kind(id), Kind::Element { ns: Ns::Html, .. })
            || doc.lower_name(id) != Some("meta")
        {
            return None;
        }
        let attr = |want: &str| {
            doc.attrs(id)
                .find(|&(ns, name, _)| ns == AttrNs::None && name == want)
                .map(|(_, _, value)| value)
        };
        if let Some(found) = attr("charset").and_then(|v| Encoding::for_label(v.as_bytes())) {
            return Some(found);
        }
        attr("http-equiv")
            .filter(|v| v.eq_ignore_ascii_case("content-type"))
            .and(attr("content"))
            .and_then(|v| prescan::charset_in_content(v.as_bytes()))
    })
}

/// "Change the encoding": the encoding to parse again in, or `None` when
/// the one in use stands.
fn changed(current: &'static Encoding, declared: &'static Encoding) -> Option<&'static Encoding> {
    if current == UTF_16BE || current == UTF_16LE {
        return None;
    }
    let new = prescan::as_declared(declared);
    (new != current).then_some(new)
}

/// The fallback encoding a browser uses for a user whose language is
/// `locale` (a BCP 47 tag such as `ja`, `ru-RU` or `zh-Hant`), from the
/// HTML Standard's table of suggested defaults. Pass it as
/// [`Options::fallback`] when you know your users' language, or a site's.
///
/// ```
/// use parsec_encoding::locale_default;
/// assert_eq!(locale_default("ja-JP"), encoding_rs::SHIFT_JIS);
/// assert_eq!(locale_default("zh-TW"), encoding_rs::BIG5);
/// assert_eq!(locale_default("en-GB"), encoding_rs::WINDOWS_1252);
/// ```
pub fn locale_default(locale: &str) -> &'static Encoding {
    use encoding_rs::*;
    let locale = locale.to_ascii_lowercase();
    let mut tags = locale.split(['-', '_']);
    match tags.next().unwrap_or("") {
        "ar" | "fa" => WINDOWS_1256,
        "az" | "ku" | "tr" => WINDOWS_1254,
        "ba" | "be" | "bg" | "kk" | "ky" | "mk" | "ru" | "sah" | "sr" | "tg" | "tt" | "uk" => {
            WINDOWS_1251
        }
        "cs" | "hr" | "sk" => WINDOWS_1250,
        "el" => ISO_8859_7,
        "et" | "lt" | "lv" => WINDOWS_1257,
        "he" => WINDOWS_1255,
        "hu" | "pl" | "sl" => ISO_8859_2,
        "ja" => SHIFT_JIS,
        "ko" => EUC_KR,
        "th" => WINDOWS_874,
        "vi" => WINDOWS_1258,
        // The script subtag comes before the region, so it decides first.
        "zh" => tags
            .find_map(|t| match t {
                "hans" | "cn" | "sg" => Some(GBK),
                "hant" | "hk" | "mo" | "tw" => Some(BIG5),
                _ => None,
            })
            .unwrap_or(WINDOWS_1252),
        _ => WINDOWS_1252,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{EUC_JP, SHIFT_JIS, UTF_8, WINDOWS_1251, WINDOWS_1252};

    fn parse(bytes: &[u8]) -> Parsed {
        parse_bytes(bytes, Options::default()).unwrap()
    }

    #[test]
    fn bom_beats_transport_and_meta() {
        let page = b"\xef\xbb\xbf<meta charset=euc-jp>\xc3\xa9";
        let options = Options {
            transport: Some(SHIFT_JIS),
            ..Options::default()
        };
        let p = parse_bytes(page, options).unwrap();
        assert_eq!((p.encoding, p.confidence), (UTF_8, Confidence::Certain));
        assert_eq!(p.document.text(p.document.root()), "é");
    }

    #[test]
    fn utf16_bom_is_removed() {
        let p = parse(b"\xff\xfe<\0p\0>\0h\0i\0");
        assert_eq!(p.encoding, UTF_16LE);
        assert_eq!(p.document.text(p.document.root()), "hi");
    }

    #[test]
    fn transport_is_certain() {
        let options = Options {
            transport: Some(WINDOWS_1251),
            ..Options::default()
        };
        let p = parse_bytes(b"<meta charset=euc-jp>\xcf", options).unwrap();
        assert_eq!(
            (p.encoding, p.confidence),
            (WINDOWS_1251, Confidence::Certain)
        );
        assert_eq!(p.document.text(p.document.root()), "П");
    }

    #[test]
    fn late_meta_reparses() {
        let mut page = b"<!doctype html><title>".to_vec();
        page.extend(std::iter::repeat_n(b'x', 1100));
        page.extend(b"</title><meta charset=windows-1251><p>\xcf");
        let p = parse(&page);
        assert_eq!(
            (p.encoding, p.confidence),
            (WINDOWS_1251, Confidence::Certain)
        );
        assert!(p.document.text(p.document.root()).ends_with('П'));
    }

    #[test]
    fn agreeing_meta_makes_it_certain() {
        let p = parse(b"<meta charset=euc-jp>");
        assert_eq!((p.encoding, p.confidence), (EUC_JP, Confidence::Certain));
        let p = parse(b"<p>no declaration");
        assert_eq!(
            (p.encoding, p.confidence),
            (WINDOWS_1252, Confidence::Tentative)
        );
    }

    #[test]
    fn late_http_equiv_and_svg_meta() {
        let mut page = b"<!-- ".to_vec();
        page.extend(std::iter::repeat_n(b'x', 1100));
        page.extend(b" --><svg><meta charset=euc-jp></svg>");
        // An SVG <meta> breaks out of foreign content, so it's HTML's.
        assert_eq!(parse(&page).encoding, EUC_JP);
        let mut page = b"<!-- ".to_vec();
        page.extend(std::iter::repeat_n(b'x', 1100));
        page.extend(b" --><meta http-equiv=content-type content='text/html;charset=sjis'>");
        assert_eq!(parse(&page).encoding, SHIFT_JIS);
    }

    #[test]
    fn later_metas_and_utf16_declarations() {
        let mut page = b"<!-- ".to_vec();
        page.extend(std::iter::repeat_n(b'x', 1100));
        page.extend(b" --><meta charset=bogus><meta charset=utf-16><meta charset=sjis>");
        // The first that names an encoding decides; UTF-16 means UTF-8.
        assert_eq!(parse(&page).encoding, UTF_8);
    }

    #[test]
    fn budget_covers_both_parses() {
        let mut page = b"<title>".to_vec();
        page.extend(std::iter::repeat_n(b'x', 1100));
        page.extend(b"</title><meta charset=windows-1251>");
        let options = Options {
            html: parsec::Options {
                budget: Some(std::time::Duration::from_secs(60)),
                ..parsec::Options::default()
            },
            ..Options::default()
        };
        assert_eq!(parse_bytes(&page, options).unwrap().encoding, WINDOWS_1251);
    }

    #[test]
    fn locales() {
        assert_eq!(locale_default("RU"), WINDOWS_1251);
        assert_eq!(locale_default("zh-Hans-HK"), encoding_rs::GBK);
        assert_eq!(locale_default("zh"), WINDOWS_1252);
        assert_eq!(locale_default(""), WINDOWS_1252);
    }
}
