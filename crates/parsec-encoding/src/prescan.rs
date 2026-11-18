// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The HTML Standard's "prescan a byte stream to determine its encoding",
//! "get an attribute", "get an XML encoding" and "extracting a character
//! encoding from a meta element", step for step.

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252, X_USER_DEFINED};

/// The encoding a `<meta>` (or an XML declaration) in `bytes` declares, as
/// the HTML Standard's prescan finds it, reading `bytes` and nothing
/// past them. [`crate::sniff`] gives it the first 1,024 bytes, as the
/// spec advises; pass more or fewer to scan further or less.
///
/// ```
/// let found = parsec_encoding::prescan(b"<meta charset=shift_jis>");
/// assert_eq!(found, Some(encoding_rs::SHIFT_JIS));
/// ```
pub fn prescan(bytes: &[u8]) -> Option<&'static Encoding> {
    // Running out of bytes anywhere in the scan ends it, and then only an
    // XML declaration can still give an answer.
    scan(bytes).or_else(|| xml_encoding(bytes))
}

/// The prescan's steps. `None` is running out of bytes; the loop has no
/// other way to end.
fn scan(b: &[u8]) -> Option<&'static Encoding> {
    if b.starts_with(&[0x3C, 0, 0x3F, 0, 0x78, 0]) {
        return Some(UTF_16LE);
    }
    if b.starts_with(&[0, 0x3C, 0, 0x3F, 0, 0x78]) {
        return Some(UTF_16BE);
    }
    let mut p = 0;
    loop {
        let rest = b.get(p..).filter(|r| !r.is_empty())?;
        if rest.starts_with(b"<!--") {
            // The first `>` after two `-`, which may be the comment
            // opener's own: `<!-->` ends here.
            p += 2 + find(&rest[2..], b"-->")? + 2;
        } else if rest.len() >= 6
            && rest[0] == b'<'
            && rest[1..5].eq_ignore_ascii_case(b"meta")
            && (is_space(rest[5]) || rest[5] == b'/')
        {
            p += 5;
            if let Some(found) = meta(b, &mut p)? {
                return Some(found);
            }
        } else if rest[0] == b'<' && starts_tag(&rest[1..]) {
            p += rest
                .iter()
                .position(|&c| matches!(c, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ' | b'>'))?;
            while attribute(b, &mut p)?.is_some() {}
        } else if rest.starts_with(b"<!") || rest.starts_with(b"</") || rest.starts_with(b"<?") {
            p += 1 + rest[1..].iter().position(|&c| c == b'>')?;
        }
        p += 1;
    }
}

/// `<` is followed by an optional `/` and an ASCII letter.
fn starts_tag(after: &[u8]) -> bool {
    let after = after.strip_prefix(b"/").unwrap_or(after);
    after.first().is_some_and(u8::is_ascii_alphabetic)
}

/// A `<meta>`'s attributes, from `p` just after `<meta`: `Some(None)` when
/// it declares nothing usable, `None` when the bytes ran out.
fn meta(b: &[u8], p: &mut usize) -> Option<Option<&'static Encoding>> {
    let mut seen: Vec<Vec<u8>> = Vec::new();
    let mut got_pragma = false;
    let mut need_pragma: Option<bool> = None;
    // The spec's null (`None`) is distinct from failure (`Some(None)`): a
    // `charset` that names no encoding still stops `content` from counting.
    let mut charset: Option<Option<&'static Encoding>> = None;
    while let Some((name, value)) = attribute(b, p)? {
        if seen.contains(&name) {
            continue;
        }
        match name.as_slice() {
            b"http-equiv" if value == b"content-type" => got_pragma = true,
            b"content" if charset.is_none() => {
                if let Some(found) = charset_in_content(&value) {
                    charset = Some(Some(found));
                    need_pragma = Some(true);
                }
            }
            b"charset" => {
                charset = Some(Encoding::for_label(&value));
                need_pragma = Some(false);
            }
            _ => {}
        }
        seen.push(name);
    }
    Some(match (need_pragma, charset) {
        (Some(need), Some(Some(found))) if !need || got_pragma => Some(as_declared(found)),
        _ => None,
    })
}

/// A declared encoding as HTML uses it: a page that says UTF-16 in ASCII
/// can't be UTF-16, and x-user-defined means windows-1252.
pub(crate) fn as_declared(e: &'static Encoding) -> &'static Encoding {
    if e == UTF_16BE || e == UTF_16LE {
        UTF_8
    } else if e == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        e
    }
}

fn is_space(c: u8) -> bool {
    matches!(c, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

/// "Get an attribute": `Some(None)` when there isn't one (a `>`), `None`
/// when the bytes ran out. Names and values come back lowercased.
#[allow(clippy::type_complexity)]
fn attribute(b: &[u8], p: &mut usize) -> Option<Option<(Vec<u8>, Vec<u8>)>> {
    while is_space(*b.get(*p)?) || b[*p] == b'/' {
        *p += 1;
    }
    if b[*p] == b'>' {
        return Some(None);
    }
    let mut name = Vec::new();
    let mut value = Vec::new();
    loop {
        match *b.get(*p)? {
            b'=' if !name.is_empty() => {
                *p += 1;
                break;
            }
            c if is_space(c) => {
                while is_space(*b.get(*p)?) {
                    *p += 1;
                }
                if b[*p] != b'=' {
                    return Some(Some((name, value)));
                }
                *p += 1;
                break;
            }
            b'/' | b'>' => return Some(Some((name, value))),
            c => name.push(c.to_ascii_lowercase()),
        }
        *p += 1;
    }
    while is_space(*b.get(*p)?) {
        *p += 1;
    }
    match b[*p] {
        quote @ (b'"' | b'\'') => loop {
            *p += 1;
            let c = *b.get(*p)?;
            if c == quote {
                *p += 1;
                return Some(Some((name, value)));
            }
            value.push(c.to_ascii_lowercase());
        },
        b'>' => return Some(Some((name, value))),
        c => {
            value.push(c.to_ascii_lowercase());
            *p += 1;
        }
    }
    loop {
        match *b.get(*p)? {
            c if is_space(c) || c == b'>' => return Some(Some((name, value))),
            c => value.push(c.to_ascii_lowercase()),
        }
        *p += 1;
    }
}

/// "Get an XML encoding": the encoding in a leading `<?xml ... ?>`.
fn xml_encoding(b: &[u8]) -> Option<&'static Encoding> {
    if !b.starts_with(b"<?xml") {
        return None;
    }
    let end = b.iter().position(|&c| c == b'>')?;
    let mut p = find(&b[..end], b"encoding")? + b"encoding".len();
    while *b.get(p)? <= 0x20 {
        p += 1;
    }
    if b[p] != b'=' {
        return None;
    }
    p += 1;
    while *b.get(p)? <= 0x20 {
        p += 1;
    }
    let quote = b[p];
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    p += 1;
    let len = b[p..].iter().position(|&c| c == quote)?;
    let name = &b[p..p + len];
    if name.iter().any(|&c| c <= 0x20) {
        return None;
    }
    let found = Encoding::for_label(name)?;
    Some(if found == UTF_16BE || found == UTF_16LE {
        UTF_8
    } else {
        found
    })
}

/// "The algorithm for extracting a character encoding from a meta
/// element", given a `content` attribute's value.
pub(crate) fn charset_in_content(s: &[u8]) -> Option<&'static Encoding> {
    let mut p = 0;
    loop {
        p += s[p..]
            .windows(7)
            .position(|w| w.eq_ignore_ascii_case(b"charset"))?
            + 7;
        while s.get(p).is_some_and(|c| c.is_ascii_whitespace()) {
            p += 1;
        }
        if s.get(p) != Some(&b'=') {
            continue;
        }
        p += 1;
        while s.get(p).is_some_and(|c| c.is_ascii_whitespace()) {
            p += 1;
        }
        let rest = &s[p..];
        return match *rest.first()? {
            quote @ (b'"' | b'\'') => {
                let len = rest[1..].iter().position(|&c| c == quote)?;
                Encoding::for_label(&rest[1..1 + len])
            }
            _ => {
                let len = rest
                    .iter()
                    .position(|&c| c.is_ascii_whitespace() || c == b';')
                    .unwrap_or(rest.len());
                Encoding::for_label(&rest[..len])
            }
        };
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{EUC_JP, ISO_8859_2, SHIFT_JIS, WINDOWS_1251};

    #[test]
    fn meta_charset_and_pragma() {
        assert_eq!(prescan(b"<meta charset='euc-jp'>"), Some(EUC_JP));
        assert_eq!(
            prescan(b"<meta http-equiv=Content-Type content='text/html; charset=windows-1251'>"),
            Some(WINDOWS_1251)
        );
        // A content charset without the pragma counts for nothing.
        assert_eq!(prescan(b"<meta content='charset=windows-1251'>"), None);
        // The pragma may come after the content.
        assert_eq!(
            prescan(b"<meta content='charset=iso-8859-2' http-equiv=content-type>"),
            Some(ISO_8859_2)
        );
    }

    #[test]
    fn charset_failure_blocks_content() {
        let b = b"<meta charset=nonsense http-equiv=content-type content='charset=utf-8'>";
        assert_eq!(prescan(b), None);
    }

    #[test]
    fn duplicates_and_comments() {
        assert_eq!(
            prescan(b"<meta charset=sjis charset=euc-jp>"),
            Some(SHIFT_JIS)
        );
        assert_eq!(prescan(b"<!-- <meta charset=euc-jp> --><p>"), None);
        // `<!-->` is a whole comment.
        assert_eq!(prescan(b"<!--><meta charset=euc-jp>"), Some(EUC_JP));
        // Inside another tag's attribute value, a meta isn't seen.
        assert_eq!(prescan(b"<a title='<meta charset=euc-jp>'>x"), None);
    }

    #[test]
    fn utf16_and_user_defined_are_mapped() {
        assert_eq!(prescan(b"<meta charset=utf-16le>"), Some(UTF_8));
        assert_eq!(
            prescan(b"<meta charset=x-user-defined>"),
            Some(WINDOWS_1252)
        );
        assert_eq!(prescan(b"<\0?\0x\0m\0l\0"), Some(UTF_16LE));
        assert_eq!(prescan(b"\0<\0?\0x\0m\0l"), Some(UTF_16BE));
    }

    #[test]
    fn running_out_falls_back_to_xml() {
        assert_eq!(prescan(b"<meta charset=euc-jp"), None);
        assert_eq!(
            prescan(b"<?xml version='1.0' encoding='Shift_JIS'?><p"),
            Some(SHIFT_JIS)
        );
        assert_eq!(
            prescan(b"<?xml version='1.0' encoding='utf-16'?><p"),
            Some(UTF_8)
        );
        assert_eq!(prescan(b"<?xml encoding = \"euc-jp\"?>"), Some(EUC_JP));
        // A scan that finds no `<meta>` always ends by running out.
        assert_eq!(prescan(b"<?xml encoding='euc-jp'?><p>"), Some(EUC_JP));
    }

    #[test]
    fn content_extraction() {
        assert_eq!(
            charset_in_content(b"text/html; charset=sjis"),
            Some(SHIFT_JIS)
        );
        assert_eq!(
            charset_in_content(b"charsetcharset = \"euc-jp\""),
            Some(EUC_JP)
        );
        assert_eq!(charset_in_content(b"charset='euc-jp"), None);
        assert_eq!(charset_in_content(b"charset="), None);
        assert_eq!(charset_in_content(b"CHARSET=sjis;x"), Some(SHIFT_JIS));
    }

    #[test]
    fn every_prefix_runs_out_cleanly() {
        let page: &[u8] = b"<?xml encoding = 'x'?><!-- c --><a b='c' d=e f><meta \
            http-equiv=content-type content=\"charset = 'sjis'\" charset>";
        for end in 0..=page.len() {
            prescan(&page[..end]);
        }
        // The bare `charset` at the end names nothing, and it overrides
        // the `content`.
        assert_eq!(prescan(page), None);
        let without = b"<meta http-equiv=content-type content=\"charset = 'sjis'\">";
        assert_eq!(prescan(without), Some(SHIFT_JIS));
    }
}
