// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! JSON text to a [`Json`] value, as strictly as RFC 8259 and
//! `JSON.parse` read it.

use parsec_base::Span;

use super::{Json, Object, Refused, refuse};
use crate::DEFAULT_MAX_DEPTH;

/// Parses `text` as JSON, the way `JSON.parse` does without a reviver:
/// no comments, no trailing commas, no single quotes, no byte order mark.
/// A key given twice keeps its first place and its last value. Spans in
/// a refusal are byte offsets in `text`.
///
/// Use it for state a page keeps as JSON in a script element of its own,
/// like Next.js's `<script id="__NEXT_DATA__" type="application/json">`
/// or Nuxt 3's `__NUXT_DATA__`: pass the element's text. Nuxt 3's is
/// in devalue's format, a flat array whose numbers point at other
/// entries, which this reads as the array it is.
///
/// ```
/// use parsec_js::data::json;
///
/// let v = json(r#"{"props": {"pageProps": {"id": 7}}, "page": "/item"}"#).unwrap();
/// assert_eq!(v.to_json_string(), r#"{"props":{"pageProps":{"id":7}},"page":"/item"}"#);
/// assert_eq!(json("[1,]").unwrap_err().reason, "json-syntax");
/// ```
pub fn json(text: &str) -> Result<Json, Refused> {
    if text.len() > u32::MAX as usize {
        return refuse(Span::at(0), "too-large");
    }
    let mut p = Parser {
        b: text.as_bytes(),
        text,
        i: 0,
        depth: 0,
    };
    p.ws();
    let v = p.value()?;
    p.ws();
    if p.i < p.b.len() {
        return p.error();
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    text: &'a str,
    i: usize,
    depth: u32,
}

impl Parser<'_> {
    fn error<T>(&self) -> Result<T, Refused> {
        refuse(Span::at(self.i as u32), "json-syntax")
    }

    fn ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.b.get(self.i) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        let yes = self.b.get(self.i) == Some(&c);
        self.i += yes as usize;
        yes
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, Refused> {
        if self.b[self.i..].starts_with(w.as_bytes()) {
            self.i += w.len();
            Ok(v)
        } else {
            self.error()
        }
    }

    fn value(&mut self) -> Result<Json, Refused> {
        match self.b.get(self.i) {
            Some(b'{') => self.nested(Parser::object),
            Some(b'[') => self.nested(Parser::array),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => self.error(),
        }
    }

    fn nested(&mut self, f: fn(&mut Self) -> Result<Json, Refused>) -> Result<Json, Refused> {
        if self.depth >= DEFAULT_MAX_DEPTH {
            return refuse(Span::at(self.i as u32), "too-deep");
        }
        self.depth += 1;
        self.i += 1;
        let v = f(self);
        self.depth -= 1;
        v
    }

    fn array(&mut self) -> Result<Json, Refused> {
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value()?);
            self.ws();
            if self.eat(b']') {
                return Ok(Json::Array(items));
            }
            if !self.eat(b',') {
                return self.error();
            }
        }
    }

    fn object(&mut self) -> Result<Json, Refused> {
        let mut object = Object::default();
        self.ws();
        if self.eat(b'}') {
            return Ok(object.finish());
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return self.error();
            }
            let key = self.string()?;
            self.ws();
            if !self.eat(b':') {
                return self.error();
            }
            self.ws();
            let v = self.value()?;
            object.set(key, Some(v));
            self.ws();
            if self.eat(b'}') {
                return Ok(object.finish());
            }
            if !self.eat(b',') {
                return self.error();
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.b.get(self.i).is_some_and(u8::is_ascii_digit) {
            self.i += 1;
        }
        self.i - start
    }

    fn number(&mut self) -> Result<Json, Refused> {
        let start = self.i;
        self.eat(b'-');
        if self.eat(b'0') {
            // No leading zeros: `01` is `0` and then an error.
        } else if self.digits() == 0 {
            return self.error();
        }
        if self.eat(b'.') && self.digits() == 0 {
            return self.error();
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if self.digits() == 0 {
                return self.error();
            }
        }
        // The grammar above is a subset of what Rust's parser reads, and
        // it rounds the same way JavaScript does: to nearest, ties to even.
        let x: f64 = self.text[start..self.i].parse().unwrap_or(f64::NAN);
        Ok(Json::Number(x))
    }

    fn hex4(&mut self) -> Result<u16, Refused> {
        let hex = self.text.get(self.i..self.i + 4).unwrap_or("");
        if hex.len() != 4 || !hex.bytes().all(|c| c.is_ascii_hexdigit()) {
            return self.error();
        }
        self.i += 4;
        Ok(u16::from_str_radix(hex, 16).unwrap_or(0))
    }

    /// A string, from its opening quote.
    fn string(&mut self) -> Result<String, Refused> {
        self.i += 1;
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote, backslash or control
            // character in one go.
            let start = self.i;
            while let Some(&c) = self.b.get(self.i) {
                if c == b'"' || c == b'\\' || c < 0x20 {
                    break;
                }
                self.i += 1;
            }
            out.push_str(&self.text[start..self.i]);
            match self.b.get(self.i) {
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    let escape = self.i;
                    self.i += 1;
                    let c = match self.b.get(self.i) {
                        Some(b'"') => '"',
                        Some(b'\\') => '\\',
                        Some(b'/') => '/',
                        Some(b'b') => '\u{8}',
                        Some(b'f') => '\u{c}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b't') => '\t',
                        Some(b'u') => {
                            self.i += 1;
                            let lone = refuse(
                                Span::new(escape as u32, escape as u32 + 6),
                                "lone-surrogate",
                            );
                            let hi = self.hex4()?;
                            let cp = match hi {
                                0xD800..=0xDBFF => {
                                    if !self.b[self.i..].starts_with(b"\\u") {
                                        return lone;
                                    }
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&lo) {
                                        return lone;
                                    }
                                    0x10000 + ((hi as u32 - 0xD800) << 10) + (lo as u32 - 0xDC00)
                                }
                                0xDC00..=0xDFFF => return lone,
                                _ => hi as u32,
                            };
                            out.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                            continue;
                        }
                        _ => return self.error(),
                    };
                    self.i += 1;
                    out.push(c);
                }
                _ => return self.error(),
            }
        }
    }
}
