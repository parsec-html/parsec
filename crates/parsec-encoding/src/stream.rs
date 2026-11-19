// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Bytes as they arrive: [`ByteParser`] sniffs, decodes and parses a page
//! a piece at a time into what [`parse_bytes`](crate::parse_bytes) makes
//! of the whole.

use std::time::{Duration, Instant};

use encoding_rs::{Decoder, Encoding, UTF_16BE, UTF_16LE};
use parsec::ParseError;

use crate::{Confidence, Options, Parsed, changed, declared, prescan};

/// Parses a page given as bytes a piece at a time, as a browser does while
/// it downloads one. You get what [`parse_bytes`](crate::parse_bytes)
/// gives for all the pieces joined: the same document, encoding and
/// confidence, however the bytes were cut.
///
/// ```
/// use parsec_encoding::{ByteParser, Confidence, Options};
///
/// let mut parser = ByteParser::new(Options::default());
/// // Shift_JIS for 日本, cut inside the first character.
/// for piece in [&b"<meta charset=sjis><p>\x93"[..], b"\xfa\x96\x7b"] {
///     parser.feed(piece);
/// }
/// let parsed = parser.finish().unwrap();
/// assert_eq!(parsed.encoding, encoding_rs::SHIFT_JIS);
/// assert_eq!(parsed.confidence, Confidence::Certain);
/// assert_eq!(parsed.document.text(parsed.document.root()), "日本");
/// ```
///
/// How it goes, in the HTML Standard's terms:
///
/// - Until it knows the encoding, it holds the bytes back. A byte order
///   mark decides at once, even cut over two pieces. Then the transport
///   layer's encoding, if you gave one. Otherwise the prescan does, which
///   the spec lets a browser wait up to 1,024 bytes for: this waits until
///   the prescan finds a whole `<meta>` naming an encoding, or there are
///   1,024 bytes, or the input ends.
/// - From then on each piece goes through encoding_rs's [`Decoder`],
///   which keeps a character cut between pieces for the next one, and on
///   into [`parsec::Parser`].
/// - When the encoding was only a guess, a `<meta>` the parser meets later
///   can name another. The spec has the browser start again in that one.
///   Here that's at [`ByteParser::finish`]: one more decode and parse,
///   of every byte, once at most, as `parse_bytes` does.
///
/// Memory: while the encoding is a guess (no byte order mark, no
/// transport encoding), every byte is kept for that second parse, so the
/// input sits in memory once as bytes beside the document, which holds
/// the decoded text anyway. Give the transport layer's encoding when you
/// have it and only the decoder's few bytes of state are kept. Before the
/// encoding is known, the pieces fed so far wait, which stops once there
/// are 1,024 bytes.
///
/// A time budget in [`Options::html`] counts the time spent in
/// [`ByteParser::feed`] and [`ByteParser::finish`], both parses together,
/// not the time waiting for input.
pub struct ByteParser {
    options: Options,
    /// The bytes held back while sniffing, then, while the encoding is a
    /// guess, every byte for a second parse.
    bytes: Vec<u8>,
    state: Option<Running>,
    /// Time spent in `feed` and `finish`, counted only with a budget, so
    /// that WebAssembly, which has no clock, still works.
    spent: Duration,
}

struct Running {
    encoding: &'static Encoding,
    confidence: Confidence,
    /// Whether a later `<meta>` could still change the encoding, and so
    /// whether every byte has to be kept.
    keep: bool,
    decoder: Decoder,
    parser: parsec::Parser,
    /// Where decoded text goes on its way into the parser, reused.
    text: String,
}

impl ByteParser {
    pub fn new(options: Options) -> ByteParser {
        ByteParser {
            options,
            bytes: Vec::new(),
            state: None,
            spent: Duration::ZERO,
        }
    }

    /// The next piece of the input.
    pub fn feed(&mut self, chunk: &[u8]) {
        let start = self.options.html.budget.map(|_| Instant::now());
        match &mut self.state {
            Some(running) => {
                if running.keep {
                    self.bytes.extend_from_slice(chunk);
                }
                running.decode(chunk, false);
            }
            None => {
                self.bytes.extend_from_slice(chunk);
                if let Some((encoding, confidence)) = self.decided() {
                    self.run(encoding, confidence);
                }
            }
        }
        self.spent += start.map_or(Duration::ZERO, |t| t.elapsed());
    }

    /// The end of the input: the document and its encoding, or why there's
    /// none, as [`parse_bytes`](crate::parse_bytes) gives them.
    pub fn finish(mut self) -> Result<Parsed, ParseError> {
        let start = self.options.html.budget.map(|_| Instant::now());
        if self.state.is_none() {
            let (encoding, confidence) =
                crate::sniff(&self.bytes, self.options.transport, self.options.fallback);
            self.run(encoding, confidence);
        }
        let mut running = self.state.take().expect("set just above");
        running.decode(&[], true);
        let mut document = running.parser.finish()?;
        let (mut encoding, mut confidence) = (running.encoding, running.confidence);
        if confidence == Confidence::Tentative
            && let Some(declared) = declared(&document)
        {
            confidence = Confidence::Certain;
            if let Some(new) = changed(encoding, declared) {
                encoding = new;
                let mut html = self.options.html;
                let spent = self.spent + start.map_or(Duration::ZERO, |t| t.elapsed());
                html.budget = html.budget.map(|b| b.saturating_sub(spent));
                document = crate::decode_and_parse(&self.bytes, encoding, html)?;
            }
        }
        Ok(Parsed {
            document,
            encoding,
            confidence,
        })
    }

    /// The encoding sniffing's answer, once the bytes so far are enough to
    /// give the one it would give for the whole input.
    fn decided(&self) -> Option<(&'static Encoding, Confidence)> {
        let b = &self.bytes;
        if let Some((bom, _)) = Encoding::for_bom(b) {
            return Some((bom, Confidence::Certain));
        }
        // `EF BB` or `FE` alone could still be the start of one.
        let boms: [&[u8]; 3] = [b"\xEF\xBB\xBF", b"\xFE\xFF", b"\xFF\xFE"];
        if boms.iter().any(|bom| bom.starts_with(b)) {
            return None;
        }
        if let Some(given) = self.options.transport {
            return Some((given, Confidence::Certain));
        }
        if b.len() >= 1024 {
            return Some(crate::sniff(b, None, self.options.fallback));
        }
        // A `<meta>` the scan read to its `>` says the same whatever comes
        // after it, so there's no need to wait for the rest.
        prescan::scan(b).map(|found| (found, Confidence::Tentative))
    }

    /// Starts decoding and parsing in `encoding`, with the bytes held back.
    fn run(&mut self, encoding: &'static Encoding, confidence: Confidence) {
        // A guess of UTF-16 (from the prescan's `<\0?\0x\0`) is never
        // changed, so the bytes aren't needed again.
        let keep =
            confidence == Confidence::Tentative && encoding != UTF_16BE && encoding != UTF_16LE;
        let mut running = Running {
            encoding,
            confidence,
            keep,
            // As `parse_bytes` decodes: only this encoding's own byte order
            // mark is removed.
            decoder: encoding.new_decoder_with_bom_removal(),
            parser: parsec::Parser::new(self.options.html),
            text: String::new(),
        };
        running.decode(&self.bytes, false);
        if !keep {
            self.bytes = Vec::new();
        }
        self.state = Some(running);
    }
}

impl Running {
    fn decode(&mut self, mut bytes: &[u8], last: bool) {
        loop {
            self.text.clear();
            let room = self
                .decoder
                .max_utf8_buffer_length(bytes.len())
                .unwrap_or(usize::MAX);
            // A cap on the piece of text made at once, for huge chunks.
            self.text.reserve(room.min(1 << 20));
            let (result, read, _) = self.decoder.decode_to_string(bytes, &mut self.text, last);
            self.parser.feed(&self.text);
            bytes = &bytes[read..];
            if result == encoding_rs::CoderResult::InputEmpty {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{ISO_8859_15, SHIFT_JIS, UTF_8, UTF_16LE, WINDOWS_1252};

    fn decided(p: &ByteParser) -> Option<(&'static Encoding, Confidence, bool)> {
        p.state.as_ref().map(|r| (r.encoding, r.confidence, r.keep))
    }

    #[test]
    fn a_whole_meta_decides_before_1024_bytes() {
        let mut p = ByteParser::new(Options::default());
        // `iso-8859-1` is windows-1252's label: the scan mustn't stop on it.
        p.feed(b"<meta charset=iso-8859-1");
        assert_eq!(decided(&p), None);
        p.feed(b"5><p>\xa4");
        assert_eq!(
            decided(&p),
            Some((ISO_8859_15, Confidence::Tentative, true))
        );
        let parsed = p.finish().unwrap();
        assert_eq!(parsed.confidence, Confidence::Certain);
        assert_eq!(parsed.document.text(parsed.document.root()), "€");
    }

    #[test]
    fn no_meta_waits_for_1024_bytes() {
        let mut p = ByteParser::new(Options::default());
        p.feed(&[b'x'; 1023]);
        assert_eq!(decided(&p), None);
        p.feed(b"x");
        assert_eq!(
            decided(&p),
            Some((WINDOWS_1252, Confidence::Tentative, true))
        );
    }

    #[test]
    fn a_bom_cut_in_three_decides_on_its_last_byte() {
        let mut p = ByteParser::new(Options {
            transport: Some(SHIFT_JIS),
            ..Options::default()
        });
        p.feed(b"\xef");
        p.feed(b"\xbb");
        assert_eq!(decided(&p), None);
        p.feed(b"\xbf\xc3");
        assert_eq!(decided(&p), Some((UTF_8, Confidence::Certain, false)));
        assert!(p.bytes.is_empty());
        p.feed(b"\xa9");
        assert_eq!(p.finish().unwrap().document.text(0), "é");
    }

    #[test]
    fn transport_decides_once_no_bom_can_come() {
        let options = Options {
            transport: Some(SHIFT_JIS),
            ..Options::default()
        };
        let mut p = ByteParser::new(options);
        p.feed(b"\xfe");
        assert_eq!(decided(&p), None);
        p.feed(b"<");
        assert_eq!(decided(&p), Some((SHIFT_JIS, Confidence::Certain, false)));
        let mut p = ByteParser::new(options);
        p.feed(b"\xff");
        p.feed(b"\xfe");
        assert_eq!(decided(&p), Some((UTF_16LE, Confidence::Certain, false)));
    }

    #[test]
    fn a_late_meta_reparses_within_the_budget() {
        let mut p = ByteParser::new(Options {
            html: parsec::Options {
                budget: Some(Duration::from_secs(60)),
                ..parsec::Options::default()
            },
            ..Options::default()
        });
        p.feed(b"<title>");
        for _ in 0..20 {
            p.feed(&[b'x'; 100]);
        }
        p.feed(b"</title><meta charset=shift_jis><p>\x93\xfa");
        p.feed(b"\x96\x7b");
        let parsed = p.finish().unwrap();
        assert_eq!(
            (parsed.encoding, parsed.confidence),
            (SHIFT_JIS, Confidence::Certain)
        );
        assert!(parsed.document.text(0).ends_with("日本"));
    }

    #[test]
    fn nothing_at_all() {
        let p = ByteParser::new(Options::default());
        let parsed = p.finish().unwrap();
        assert_eq!(
            (parsed.encoding, parsed.confidence),
            (WINDOWS_1252, Confidence::Tentative)
        );
    }
}
