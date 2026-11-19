// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! A stylesheet's bytes into text, CSS Syntax §3.2: a byte order mark,
//! then the charset HTTP gave, then an `@charset "…";` at the very start,
//! then the encoding of the document that linked it, then UTF-8.

use std::borrow::Cow;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE};

/// The fallback encoding: what the bytes are decoded with unless they
/// start with a byte order mark. `protocol` is a charset label from HTTP
/// or the like; `environment` is the linking document's encoding.
pub fn fallback_encoding(
    css: &[u8],
    protocol: Option<&str>,
    environment: Option<&'static Encoding>,
) -> &'static Encoding {
    if let Some(e) = protocol.and_then(|label| Encoding::for_label(label.as_bytes())) {
        return e;
    }
    // Exactly `@charset "label";`, in the first 1,024 bytes.
    let head = &css[..css.len().min(1024)];
    if let Some(rest) = head.strip_prefix(b"@charset \"") {
        let len = rest
            .iter()
            .take_while(|&&b| matches!(b, 0x00..=0x21 | 0x23..=0x7f))
            .count();
        if rest[len..].starts_with(b"\";") {
            match Encoding::for_label(&rest[..len]) {
                Some(e) if e == UTF_16BE || e == UTF_16LE => return UTF_8,
                Some(e) => return e,
                None => {}
            }
        }
    }
    environment.unwrap_or(UTF_8)
}

/// Decodes a stylesheet's bytes, returning the text and the encoding it
/// was in. Malformed bytes become U+FFFD.
///
/// ```
/// let (text, encoding) = parsec_css::decode_stylesheet(b"@charset \"iso-8859-5\"; @\xe9", None, None);
/// assert_eq!((&*text, encoding.name()), ("@charset \"iso-8859-5\"; @щ", "ISO-8859-5"));
/// ```
pub fn decode_stylesheet<'b>(
    css: &'b [u8],
    protocol: Option<&str>,
    environment: Option<&'static Encoding>,
) -> (Cow<'b, str>, &'static Encoding) {
    let (text, encoding, _) = fallback_encoding(css, protocol, environment).decode(css);
    (text, encoding)
}
