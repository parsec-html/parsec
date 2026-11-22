// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Bytes in: the encoding from the byte order mark, else the first bytes'
//! pattern and the XML declaration (XML 1.0, appendix F), decoded with
//! encoding_rs, strictly, since bytes that aren't the encoding are a
//! fatal error in XML.

use std::borrow::Cow;

use encoding_rs::{DecoderResult, Encoding, UTF_8, UTF_16BE, UTF_16LE};
use parsec_base::{Diagnostic, LineCol, Span};

use crate::{Document, Error, Options, parse_with};

/// A document parsed from bytes, and the text it was decoded to.
#[derive(Debug)]
pub struct Parsed<'b> {
    pub document: Document,
    /// The encoding the bytes were decoded from.
    pub encoding: &'static Encoding,
    /// The decoded text, without a byte order mark: the document's spans
    /// are offsets into it. Borrowed from the bytes when they're UTF-8.
    pub text: Cow<'b, str>,
}

fn fail(code: &'static str, message: String) -> Error {
    Error {
        diagnostic: Diagnostic::error(code, Span::at(0), message),
        position: LineCol { line: 1, column: 1 },
    }
}

/// The encoding an XML declaration in ASCII names, if there is one.
fn declared(bytes: &[u8]) -> Option<&[u8]> {
    let end = bytes.iter().take(1024).position(|&b| b == b'>')?;
    let decl = bytes[..end].strip_prefix(b"<?xml")?;
    let at = decl.windows(8).position(|w| w == b"encoding")?;
    let rest = &decl[at + 8..];
    let rest = rest
        .trim_ascii_start()
        .strip_prefix(b"=")?
        .trim_ascii_start();
    let (&q, rest) = rest.split_first()?;
    if q != b'"' && q != b'\'' {
        return None;
    }
    Some(&rest[..rest.iter().position(|&b| b == q)?])
}

/// The encoding of `bytes` and the length of its byte order mark.
fn sniff(bytes: &[u8]) -> Result<(&'static Encoding, usize), Error> {
    if let Some(found) = Encoding::for_bom(bytes) {
        return Ok(found);
    }
    match bytes.get(..4) {
        Some([0x3C, 0, 0x3F, 0]) => return Ok((UTF_16LE, 0)),
        Some([0, 0x3C, 0, 0x3F]) => return Ok((UTF_16BE, 0)),
        Some([0, 0, 0, 0x3C] | [0x3C, 0, 0, 0] | [0, 0, 0x3C, 0] | [0, 0x3C, 0, 0])
        | Some([0, 0, 0xFE, 0xFF] | [0xFF, 0xFE, 0, 0]) => {
            return Err(fail(
                "unsupported-encoding",
                "the document is in UCS-4, which Parsec doesn't decode".into(),
            ));
        }
        Some([0x4C, 0x6F, 0xA7, 0x94]) => {
            return Err(fail(
                "unsupported-encoding",
                "the document is in EBCDIC, which Parsec doesn't decode".into(),
            ));
        }
        _ => {}
    }
    let Some(label) = declared(bytes) else {
        return Ok((UTF_8, 0));
    };
    let name = String::from_utf8_lossy(label);
    match Encoding::for_label_no_replacement(label) {
        // The bytes so far were ASCII, so a document in UTF-16 would have
        // started with a byte order mark.
        Some(e) if e == UTF_16LE || e == UTF_16BE => Err(fail(
            "encoding-mismatch",
            format!("the document says it's in {name}, but it has no byte order mark"),
        )),
        Some(e) => Ok((e, 0)),
        None => Err(fail(
            "unsupported-encoding",
            format!("`{name}` isn't an encoding Parsec knows"),
        )),
    }
}

/// Parses a document given as bytes.
///
/// ```
/// let latin1 = b"<?xml version='1.0' encoding='ISO-8859-1'?><p>caf\xe9</p>";
/// let parsed = parsec_xml::parse_bytes(latin1, &Default::default()).unwrap();
/// assert_eq!(parsed.encoding.name(), "windows-1252");
/// assert_eq!(parsed.document.text(parsed.document.root()), "café");
/// ```
pub fn parse_bytes<'b>(bytes: &'b [u8], options: &Options) -> Result<Parsed<'b>, Error> {
    options.limits.check_input(bytes.len()).map_err(|d| Error {
        diagnostic: d,
        position: LineCol { line: 1, column: 1 },
    })?;
    let (encoding, bom) = sniff(bytes)?;
    let body = &bytes[bom..];
    let text: Cow<'b, str> = if encoding == UTF_8 {
        match std::str::from_utf8(body) {
            Ok(s) => Cow::Borrowed(s),
            Err(e) => {
                let good = std::str::from_utf8(&body[..e.valid_up_to()]).unwrap_or("");
                return Err(malformed(good, encoding));
            }
        }
    } else {
        let mut decoder = encoding.new_decoder_without_bom_handling();
        let mut out = String::with_capacity(
            decoder
                .max_utf8_buffer_length_without_replacement(body.len())
                .unwrap_or(body.len()),
        );
        let (result, _) = decoder.decode_to_string_without_replacement(body, &mut out, true);
        match result {
            DecoderResult::InputEmpty => Cow::Owned(out),
            DecoderResult::Malformed(..) | DecoderResult::OutputFull => {
                return Err(malformed(&out, encoding));
            }
        }
    };
    let document = parse_with(&text, options)?;
    // A declaration can't contradict the byte order mark or the bytes'
    // pattern that the encoding was found from.
    if let Some(decl) = document.xml_decl()
        && let Some(label) = &decl.encoding
        && (bom > 0 || encoding == UTF_16LE || encoding == UTF_16BE)
    {
        let utf16 = |e: &'static Encoding| e == UTF_16LE || e == UTF_16BE;
        let agrees = Encoding::for_label(label.as_bytes())
            .is_some_and(|d| d == encoding || utf16(d) && utf16(encoding));
        if !agrees {
            return Err(Error::new(
                Diagnostic::error(
                    "encoding-mismatch",
                    decl.span,
                    format!(
                        "the document says it's in {label}, but its first bytes say {}",
                        encoding.name()
                    ),
                ),
                &text,
            ));
        }
    }
    Ok(Parsed {
        document,
        encoding,
        text,
    })
}

/// An error for bytes that aren't `encoding`, after the text `good`.
fn malformed(good: &str, encoding: &'static Encoding) -> Error {
    let at = good.len() as u32;
    Error::new(
        Diagnostic::error(
            "invalid-byte-sequence",
            Span::at(at),
            format!("the bytes here aren't {}", encoding.name()),
        ),
        good,
    )
}
