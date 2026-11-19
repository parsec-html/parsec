// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Offsets into the bytes a page came as. The parser counts UTF-8 bytes
//! into the text it was given, the decoded page; a caller with the
//! original bytes wants offsets into those.

use encoding_rs::{CoderResult, Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// Turns offsets into a decoded page into offsets into its bytes, and
/// back, for the spans and parse errors of a document parsed from bytes.
///
/// ```
/// use parsec_encoding::{Offsets, Options, parse_bytes};
///
/// let page = b"<meta charset=shift_jis><p>\x93\xfa\x96\x7b<b>!</b>";
/// let parsed = parse_bytes(page, Options {
///     html: parsec::Options { spans: true, ..Default::default() },
///     ..Options::default()
/// }).unwrap();
/// let offsets = Offsets::new(page, parsed.encoding);
/// let doc = &parsed.document;
/// let b = (0..doc.len() as u32).find(|&n| doc.lower_name(n) == Some("b")).unwrap();
/// let span = doc.span(b).unwrap();
/// // 日本 is six bytes decoded, four in Shift_JIS.
/// assert_eq!(span, 33..36);
/// let (start, end) = (offsets.to_source(span.start), offsets.to_source(span.end));
/// assert_eq!(&page[start..end], b"<b>");
/// ```
///
/// It's built by decoding the bytes again, noting where each character's
/// bytes started (UTF-8 and the single-byte encodings in one pass, the
/// others a byte at a time, which takes two or three times as long as
/// the parse), and keeps only the places where the two stop moving
/// together: after the byte order mark, at each character whose length
/// differs between the two (each Shift_JIS kanji, say), and after each
/// run of bytes the decoding replaced with U+FFFD. ASCII costs nothing;
/// text that's all non-ASCII costs 8 bytes a character.
pub struct Offsets {
    /// (decoded, source) at each place where an offset can't be worked out
    /// from the one before by counting on, in order of both.
    points: Vec<(u32, u32)>,
}

impl Offsets {
    /// The offsets for `bytes` decoded as [`parse_bytes`](crate::parse_bytes)
    /// and [`ByteParser`](crate::ByteParser) decode them in `encoding`:
    /// with the encoding's own byte order mark removed.
    pub fn new(bytes: &[u8], encoding: &'static Encoding) -> Offsets {
        let mut map = Offsets { points: Vec::new() };
        let bom = match Encoding::for_bom(bytes) {
            Some((found, len)) if found == encoding => len,
            _ => 0,
        };
        map.at(0, bom);
        if encoding == UTF_8 {
            // Rust's lossy decoding replaces the same runs with U+FFFD as
            // the Encoding Standard's, and far faster than byte by byte.
            let (mut decoded, mut source) = (0, bom);
            for chunk in bytes[bom..].utf8_chunks() {
                decoded += chunk.valid().len();
                source += chunk.valid().len();
                if !chunk.invalid().is_empty() {
                    decoded += '\u{FFFD}'.len_utf8();
                    source += chunk.invalid().len();
                    map.at(decoded, source);
                }
            }
            return map;
        }
        if encoding.is_single_byte() {
            // One byte, one character, so one decode of the whole will do.
            let (text, _) = encoding.decode_without_bom_handling(bytes);
            let mut decoded = 0;
            for (i, c) in text.chars().enumerate() {
                map.at(decoded, i);
                decoded += c.len_utf8();
            }
            return map;
        }
        // Outside UTF-16, an ASCII character out is the one byte just read
        // in (escape sequences in ISO-2022-JP produce nothing).
        let narrow = encoding != UTF_16BE && encoding != UTF_16LE;
        let mut decoder = encoding.new_decoder_without_bom_handling();
        let mut out = [0u8; 64];
        let mut decoded = 0;
        // Where the bytes of the next character to come out began.
        let mut from = bom;
        for i in bom..=bytes.len() {
            let last = i == bytes.len();
            let mut input = if last { &[][..] } else { &bytes[i..i + 1] };
            let mut first = true;
            loop {
                let (result, read, written, _) = decoder.decode_to_utf8(input, &mut out, last);
                input = &input[read..];
                for (k, &b) in out[..written].iter().enumerate() {
                    // Every byte but a UTF-8 continuation byte starts one.
                    if (b as i8) >= -0x40 {
                        // More than one at once is a U+FFFD for bytes
                        // before this one, then what this one began.
                        let start = if first && !(narrow && b < 0x80) {
                            from
                        } else {
                            i
                        };
                        map.at(decoded + k, start);
                        first = false;
                    }
                }
                decoded += written;
                if result == CoderResult::InputEmpty {
                    break;
                }
            }
            if !first {
                from = i + 1;
            }
        }
        map.at(decoded, bytes.len());
        map
    }

    /// Notes that decoded offset `decoded` is source offset `source`,
    /// unless counting on from the last point says so already.
    fn at(&mut self, decoded: usize, source: usize) {
        let (d, s) = self.points.last().copied().unwrap_or((0, 0));
        if source as u64 != u64::from(s) + (decoded as u64 - u64::from(d)) {
            // A document holds at most 512 MiB, so both fit.
            self.points.push((decoded as u32, source as u32));
        }
    }

    /// Whether every offset is the same in both: ASCII, or UTF-8 with no
    /// byte order mark and nothing replaced.
    pub fn is_identity(&self) -> bool {
        self.points.is_empty()
    }

    /// The offset into the bytes of offset `decoded` into the decoded text.
    pub fn to_source(&self, decoded: usize) -> usize {
        let i = self.points.partition_point(|&(d, _)| d as usize <= decoded);
        match i.checked_sub(1).map(|i| self.points[i]) {
            Some((d, s)) => s as usize + (decoded - d as usize),
            None => decoded,
        }
    }

    /// The offset into the decoded text of offset `source` into the bytes.
    /// One inside a character's bytes gives one inside its decoding, never
    /// past it.
    pub fn to_decoded(&self, source: usize) -> usize {
        let i = self.points.partition_point(|&(_, s)| s as usize <= source);
        match i.checked_sub(1).map(|i| self.points[i]) {
            Some((d, s)) => {
                let at = d as usize + (source - s as usize);
                self.points
                    .get(i)
                    .map_or(at, |&(next, _)| at.min(next as usize))
            }
            None => source,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::*;

    /// Every ASCII character of the decoding maps to the same byte in the
    /// source, and back.
    fn check(bytes: &[u8], encoding: &'static Encoding) -> Offsets {
        let (text, _) = encoding.decode_with_bom_removal(bytes);
        let map = Offsets::new(bytes, encoding);
        for (d, c) in text.char_indices() {
            if c.is_ascii() && encoding != UTF_16BE {
                let s = map.to_source(d);
                assert_eq!(bytes[s], c as u8, "{} at {d}", encoding.name());
                assert_eq!(map.to_decoded(s), d);
            }
        }
        assert_eq!(map.to_source(text.len()), bytes.len());
        map
    }

    #[test]
    fn ascii_and_clean_utf8_are_the_identity() {
        assert!(check(b"<p>plain", UTF_8).is_identity());
        assert!(check("<p>caf\u{e9}".as_bytes(), UTF_8).is_identity());
        assert!(check(b"<p>plain", WINDOWS_1252).is_identity());
    }

    #[test]
    fn utf8_boms_and_bad_bytes() {
        let map = check(b"\xef\xbb\xbf<p>\xff\xfe</p><b>x", UTF_8);
        // A U+FFFD for each bad byte: 3 bytes decoded, 1 in the source.
        assert_eq!(map.to_source(3 + 6 + 2), 3 + 3 + 2 + 2);
    }

    #[test]
    fn legacy_multibyte() {
        // 日本 in each, with markup around.
        check(b"<p>\x93\xfa\x96\x7b</p><b>x</b>", SHIFT_JIS);
        check(b"<p>\xc6\xfc\xcb\xdc</p><b>x</b>", EUC_JP);
        check(b"<p>\x1b$BF|K\\\x1b(B</p><b>x</b>", ISO_2022_JP);
        check(b"<p>\xc8\xd5\xb1\xbe</p><b>x</b>", GBK);
        check(b"<p>\x81\x30\x81\x30\xc8\xd5</p><b>x</b>", GB18030);
        check(b"<p>\xa4\xe9\xa5\xbb</p><b>x</b>", BIG5);
        check(b"<p>\xc0\xcf\xba\xbb</p><b>x</b>", EUC_KR);
        check(b"<p>\xcf\xf0\xe8</p><b>x</b>", WINDOWS_1251);
    }

    #[test]
    fn a_bad_lead_byte_then_ascii() {
        // 0x93 wants a trail byte; `<` isn't one, so the decoder gives
        // U+FFFD for the 0x93 and then the `<` it reread.
        let map = check(b"<p>\x93<b>x</b>", SHIFT_JIS);
        assert_eq!(map.to_source(6), 4);
        // And one cut off at the end.
        check(b"<p>x\x93", SHIFT_JIS);
    }

    #[test]
    fn utf16() {
        let mut bytes = vec![0xff, 0xfe];
        for u in "<p>\u{65e5}\u{1F600}x".encode_utf16() {
            bytes.extend(u.to_le_bytes());
        }
        let map = check(&bytes, UTF_16LE);
        // `<` at 2 after the byte order mark, then 2 bytes a unit.
        assert_eq!(map.to_source(0), 2);
        assert_eq!(map.to_source(3), 8);
        // 日 is 3 bytes decoded, 😀 4 bytes in both.
        assert_eq!(map.to_source(6), 10);
        assert_eq!(map.to_source(10), 14);
        // Inside 日's two bytes: inside its decoding.
        assert_eq!(map.to_decoded(9), 4);
    }
}
