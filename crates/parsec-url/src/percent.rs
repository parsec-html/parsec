// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The URL Standard's percent-encode sets, and percent-encoding and
//! decoding with them.
//!
//! Every set holds every byte from 0x80 up, so encoding a UTF-8 string byte
//! by byte is the standard's UTF-8 percent-encode, and what's left unencoded
//! is always ASCII. That's why runs of plain bytes can be copied as `&str`
//! slices: they start and end on ASCII bytes, which are char boundaries.

/// The sets, as bits of one table entry per byte.
pub(crate) const C0: u8 = 1;
pub(crate) const FRAGMENT: u8 = 1 << 1;
pub(crate) const QUERY: u8 = 1 << 2;
pub(crate) const SPECIAL_QUERY: u8 = 1 << 3;
pub(crate) const PATH: u8 = 1 << 4;
pub(crate) const USERINFO: u8 = 1 << 5;
/// The application/x-www-form-urlencoded set. The component set it
/// builds on has no use here of its own.
pub(crate) const FORM: u8 = 1 << 6;

const fn sets(b: u8) -> u8 {
    let c0 = b < 0x20 || b > 0x7E;
    let fragment = c0 || matches!(b, b' ' | b'"' | b'<' | b'>' | b'`');
    let query = c0 || matches!(b, b' ' | b'"' | b'#' | b'<' | b'>');
    let special_query = query || b == b'\'';
    let path = query || matches!(b, b'?' | b'^' | b'`' | b'{' | b'}');
    let userinfo = path || matches!(b, b'/' | b':' | b';' | b'=' | b'@' | b'['..=b']' | b'|');
    let component = userinfo || matches!(b, b'$'..=b'&' | b'+' | b',');
    let form = component || matches!(b, b'!' | b'\''..=b')' | b'~');
    (c0 as u8)
        | (fragment as u8) << 1
        | (query as u8) << 2
        | (special_query as u8) << 3
        | (path as u8) << 4
        | (userinfo as u8) << 5
        | (form as u8) << 6
}

pub(crate) static SETS: [u8; 256] = {
    let mut t = [0; 256];
    let mut b = 0;
    while b < 256 {
        t[b] = sets(b as u8);
        b += 1;
    }
    t
};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

pub(crate) fn push_encoded_byte(out: &mut String, b: u8) {
    out.push('%');
    out.push(HEX[(b >> 4) as usize] as char);
    out.push(HEX[(b & 15) as usize] as char);
}

/// Appends `input` UTF-8 percent-encoded with `set`.
pub(crate) fn encode_into(out: &mut String, input: &str, set: u8) {
    let bytes = input.as_bytes();
    let mut plain = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if SETS[b as usize] & set != 0 {
            if plain < i {
                out.push_str(&input[plain..i]);
            }
            push_encoded_byte(out, b);
            plain = i + 1;
        }
    }
    out.push_str(&input[plain..]);
}

pub(crate) fn encode(input: &str, set: u8) -> String {
    let mut out = String::with_capacity(input.len());
    encode_into(&mut out, input, set);
    out
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decodes `input`; a `%` not followed by two hex digits stays.
pub(crate) fn decode(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b == b'%'
            && let (Some(hi), Some(lo)) = (
                input.get(i + 1).and_then(|&h| hex_value(h)),
                input.get(i + 2).and_then(|&l| hex_value(l)),
            )
        {
            out.push(hi << 4 | lo);
            i += 3;
            continue;
        }
        out.push(b);
        i += 1;
    }
    out
}
