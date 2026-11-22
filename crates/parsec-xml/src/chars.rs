// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! XML 1.0 Fifth Edition's character classes: `Char`, `S`, `NameStartChar`,
//! `NameChar` and `PubidChar`.

/// What a byte is, for the ASCII fast paths.
const NAME_START: u8 = 1;
const NAME: u8 = 2;
const PUBID: u8 = 4;

static ASCII: [u8; 128] = {
    let mut t = [0u8; 128];
    let mut c = 0;
    while c < 128 {
        let b = c as u8;
        let letter = b.is_ascii_alphabetic();
        if letter || b == b':' || b == b'_' {
            t[c] |= NAME_START | NAME;
        }
        if b.is_ascii_digit() || b == b'-' || b == b'.' {
            t[c] |= NAME;
        }
        if letter || b.is_ascii_digit() || matches!(b, b' ' | b'\r' | b'\n') {
            t[c] |= PUBID;
        }
        if matches!(
            b,
            b'-' | b'\''
                | b'('
                | b')'
                | b'+'
                | b','
                | b'.'
                | b'/'
                | b':'
                | b'='
                | b'?'
                | b';'
                | b'!'
                | b'*'
                | b'#'
                | b'@'
                | b'$'
                | b'_'
                | b'%'
        ) {
            t[c] |= PUBID;
        }
        c += 1;
    }
    t
};

/// `Char`: any Unicode character but the C0 controls other than tab, LF
/// and CR, and U+FFFE and U+FFFF. (A Rust `char` is never a surrogate.)
pub(crate) fn is_char(c: char) -> bool {
    match c as u32 {
        0x20..=0xFFFD => true,
        0x9 | 0xA | 0xD => true,
        n => n >= 0x10000,
    }
}

/// `S`.
pub(crate) fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

pub(crate) fn is_name_start(c: char) -> bool {
    if c.is_ascii() {
        return ASCII[c as usize] & NAME_START != 0;
    }
    matches!(c as u32,
        0xC0..=0xD6
        | 0xD8..=0xF6
        | 0xF8..=0x2FF
        | 0x370..=0x37D
        | 0x37F..=0x1FFF
        | 0x200C..=0x200D
        | 0x2070..=0x218F
        | 0x2C00..=0x2FEF
        | 0x3001..=0xD7FF
        | 0xF900..=0xFDCF
        | 0xFDF0..=0xFFFD
        | 0x10000..=0xEFFFF)
}

pub(crate) fn is_name_char(c: char) -> bool {
    if c.is_ascii() {
        return ASCII[c as usize] & NAME != 0;
    }
    is_name_start(c) || matches!(c as u32, 0xB7 | 0x300..=0x36F | 0x203F..=0x2040)
}

/// Whether an ASCII byte is a `NameChar`; the fast path for names.
#[inline]
pub(crate) fn is_ascii_name_byte(b: u8) -> bool {
    b < 128 && ASCII[b as usize] & NAME != 0
}

pub(crate) fn is_pubid(c: char) -> bool {
    c.is_ascii() && ASCII[c as usize] & PUBID != 0
}

/// The length of the `Name` at the start of `s`, 0 if there's none.
pub(crate) fn name_len(s: &str) -> usize {
    let b = s.as_bytes();
    let mut i = match b.first() {
        Some(&f) if f < 128 => {
            if ASCII[f as usize] & NAME_START == 0 {
                return 0;
            }
            1
        }
        Some(_) => match s.chars().next() {
            Some(c) if is_name_start(c) => c.len_utf8(),
            _ => return 0,
        },
        None => return 0,
    };
    loop {
        // Most names are ASCII to the end.
        while i < b.len() && is_ascii_name_byte(b[i]) {
            i += 1;
        }
        if i == b.len() || b[i] < 128 {
            return i;
        }
        match s[i..].chars().next() {
            Some(c) if is_name_char(c) => i += c.len_utf8(),
            _ => return i,
        }
    }
}

/// The length of the `Nmtoken` at the start of `s`.
pub(crate) fn nmtoken_len(s: &str) -> usize {
    s.char_indices()
        .find(|&(_, c)| !is_name_char(c))
        .map_or(s.len(), |(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert!(is_char('\t') && is_char('\u{10FFFF}') && is_char('\u{85}'));
        assert!(!is_char('\u{0}') && !is_char('\u{1F}') && !is_char('\u{FFFE}'));
        assert!(!is_char('\u{FFFF}'));
        assert!(is_name_start(':') && is_name_start('é') && !is_name_start('-'));
        assert!(!is_name_start('\u{B7}') && is_name_char('\u{B7}'));
        assert!(!is_name_start('\u{F0000}') && is_name_start('\u{EFFFF}'));
        assert!(is_pubid('\'') && is_pubid('%') && !is_pubid('"') && !is_pubid('\t'));
    }

    #[test]
    fn names() {
        assert_eq!(name_len("abc def"), 3);
        assert_eq!(name_len("-abc"), 0);
        assert_eq!(name_len("é·x>"), "é·x".len());
        assert_eq!(name_len("aé"), 3);
        assert_eq!(name_len("é"), 2);
        assert_eq!(name_len("a\u{2000}"), 1);
        assert_eq!(name_len(""), 0);
        assert_eq!(nmtoken_len("-1.a b"), 4);
    }
}
