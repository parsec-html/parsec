// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `application/x-www-form-urlencoded`, which `URLSearchParams` reads and
//! writes.

use crate::percent::{self, FORM, SETS};

fn decode(bytes: &[u8]) -> String {
    if !bytes.iter().any(|&b| b == b'%' || b == b'+') {
        // Came from a `&str` split on ASCII bytes, so it's UTF-8 already.
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let plus: Vec<u8> = bytes
        .iter()
        .map(|&b| if b == b'+' { b' ' } else { b })
        .collect();
    String::from_utf8_lossy(&percent::decode(&plus)).into_owned()
}

/// The parser: name-value pairs split on `&` and `=`, with `+` as a
/// space and bytes that aren't UTF-8 once decoded as U+FFFD.
pub(crate) fn parse(input: &str) -> Vec<(String, String)> {
    input
        .as_bytes()
        .split(|&b| b == b'&')
        .filter(|bytes| !bytes.is_empty())
        .map(|bytes| match bytes.iter().position(|&b| b == b'=') {
            Some(at) => (decode(&bytes[..at]), decode(&bytes[at + 1..])),
            None => (decode(bytes), String::new()),
        })
        .collect()
}

fn encode_into(out: &mut String, s: &str) {
    let mut plain = 0;
    for (i, b) in s.bytes().enumerate() {
        if SETS[b as usize] & FORM != 0 {
            if plain < i {
                out.push_str(&s[plain..i]);
            }
            if b == b' ' {
                out.push('+');
            } else {
                percent::push_encoded_byte(out, b);
            }
            plain = i + 1;
        }
    }
    out.push_str(&s[plain..]);
}

/// The serializer.
pub(crate) fn serialize<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    let mut out = String::new();
    for (name, value) in pairs {
        if !out.is_empty() {
            out.push('&');
        }
        encode_into(&mut out, name);
        out.push('=');
        encode_into(&mut out, value);
    }
    out
}
