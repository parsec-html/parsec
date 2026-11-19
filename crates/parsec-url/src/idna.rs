// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! UTS #46 processing with the options the URL Standard picks for its
//! domain to ASCII and domain to Unicode: CheckHyphens, UseSTD3ASCIIRules
//! and VerifyDnsLength off, CheckBidi and CheckJoiners on, nontransitional.
//! The URL parser only comes here for a domain with a non-ASCII code point;
//! an ASCII one is lowercased and kept, since June 2026.

use crate::normalize::{ccc, in_ranges, is_nfc_quick, nfc, run_value};
use crate::punycode;
use crate::tables::{
    BIDI_STARTS, BIDI_VALUES, IDNA_MAPPED, IDNA_STARTS, IDNA_VALUES, JOINING_STARTS,
    JOINING_VALUES, MARKS,
};

/// Bidi_Class values, in the order the tables number them, with the
/// UCD's short names.
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum Bidi {
    L,
    R,
    AL,
    AN,
    EN,
    ES,
    CS,
    ET,
    ON,
    BN,
    NSM,
    Other,
}

fn bidi(c: char) -> Bidi {
    use Bidi::*;
    match run_value(BIDI_STARTS, BIDI_VALUES, c as u32) {
        0 => L,
        1 => R,
        2 => AL,
        3 => AN,
        4 => EN,
        5 => ES,
        6 => CS,
        7 => ET,
        8 => ON,
        9 => BN,
        10 => NSM,
        _ => Other,
    }
}

enum Status {
    Valid,
    Ignored,
    Disallowed,
    Mapped(&'static str),
    MappedTo(char),
}

fn status(c: char) -> Status {
    let cp = c as u32;
    let v = run_value(IDNA_STARTS, IDNA_VALUES, cp);
    match v >> 29 {
        0 => Status::Valid,
        1 => Status::Ignored,
        2 => Status::Disallowed,
        3 => {
            let at = (v >> 8 & 0x1F_FFFF) as usize;
            Status::Mapped(&IDNA_MAPPED[at..at + (v & 0xFF) as usize])
        }
        4 => match char::from_u32(cp.wrapping_add(v & 0x1FFF_FFFF).wrapping_sub(1 << 28)) {
            Some(to) => Status::MappedTo(to),
            None => Status::Disallowed,
        },
        _ if cp.is_multiple_of(2) => {
            char::from_u32(cp + 1).map_or(Status::Disallowed, Status::MappedTo)
        }
        _ => Status::Valid,
    }
}

/// The joining types ContextJ looks at.
const JOIN_L: u8 = 1;
const JOIN_D: u8 = 2;
const JOIN_R: u8 = 3;
const JOIN_T: u8 = 4;

fn joining(c: char) -> u8 {
    run_value(JOINING_STARTS, JOINING_VALUES, c as u32)
}

const VIRAMA: u8 = 9;

/// RFC 5892's ContextJ rules for U+200C and U+200D at `i`.
fn joiner_ok(label: &[char], i: usize) -> bool {
    if i > 0 && ccc(label[i - 1]) == VIRAMA {
        return true;
    }
    if label[i] == '\u{200D}' {
        return false;
    }
    // (L|D) T* ZWNJ T* (R|D)
    let before = label[..i].iter().rev().find(|&&c| joining(c) != JOIN_T);
    let after = label[i + 1..].iter().find(|&&c| joining(c) != JOIN_T);
    before.is_some_and(|&c| matches!(joining(c), JOIN_L | JOIN_D))
        && after.is_some_and(|&c| matches!(joining(c), JOIN_R | JOIN_D))
}

/// RFC 5893's six Bidi conditions, for one label of a Bidi domain name.
fn bidi_ok(label: &[char]) -> bool {
    use Bidi::*;
    let Some(&first) = label.first() else {
        return true;
    };
    let classes: Vec<Bidi> = label.iter().map(|&c| bidi(c)).collect();
    let last = classes.iter().rev().find(|&&b| b != NSM).copied();
    match bidi(first) {
        R | AL => {
            let allowed = classes
                .iter()
                .all(|b| matches!(b, R | AL | AN | EN | ES | CS | ET | ON | BN | NSM));
            let ends = matches!(last, Some(R | AL | EN | AN));
            let numbers = !(classes.contains(&EN) && classes.contains(&AN));
            allowed && ends && numbers
        }
        L => {
            let allowed = classes
                .iter()
                .all(|b| matches!(b, L | EN | ES | CS | ET | ON | BN | NSM));
            allowed && matches!(last, Some(L | EN))
        }
        _ => false,
    }
}

/// UTS #46's validity criteria, less the Bidi rule, which needs the
/// whole domain. `decoded` is for a label that came from Punycode, which
/// must also be NFC and have only valid code points.
fn label_ok(label: &[char], decoded: bool) -> bool {
    if label.starts_with(&['x', 'n', '-', '-']) || label.contains(&'.') {
        return false;
    }
    if label.first().is_some_and(|&c| in_ranges(MARKS, c as u32)) {
        return false;
    }
    if decoded {
        let s: String = label.iter().collect();
        if !is_nfc_quick(&s) && nfc(&s) != s {
            return false;
        }
        if !label.iter().all(|&c| matches!(status(c), Status::Valid)) {
            return false;
        }
    }
    label
        .iter()
        .enumerate()
        .all(|(i, &c)| !matches!(c, '\u{200C}' | '\u{200D}') || joiner_ok(label, i))
}

/// UTS #46 Processing: the domain's labels as code points, and whether
/// any error was recorded.
fn process(domain: &str) -> (Vec<Vec<char>>, bool) {
    let mut error = false;
    let mut mapped = String::with_capacity(domain.len());
    for c in domain.chars() {
        if c.is_ascii() {
            mapped.push(c.to_ascii_lowercase());
            continue;
        }
        match status(c) {
            Status::Valid => mapped.push(c),
            Status::Ignored => {}
            Status::Disallowed => {
                error = true;
                mapped.push(c);
            }
            Status::Mapped(s) => mapped.push_str(s),
            Status::MappedTo(to) => mapped.push(to),
        }
    }
    let normalized = nfc(&mapped);
    let mut labels = Vec::new();
    for label in normalized.split('.') {
        if let Some(rest) = label.strip_prefix("xn--") {
            if !label.is_ascii() {
                error = true;
                labels.push(label.chars().collect());
                continue;
            }
            match punycode::decode(rest) {
                Some(chars) => {
                    if chars.is_empty()
                        || chars.iter().all(char::is_ascii)
                        || !label_ok(&chars, true)
                    {
                        error = true;
                    }
                    labels.push(chars);
                }
                None => {
                    error = true;
                    labels.push(label.chars().collect());
                }
            }
        } else {
            let chars: Vec<char> = label.chars().collect();
            if !label_ok(&chars, false) {
                error = true;
            }
            labels.push(chars);
        }
    }
    // A Bidi domain name has a right-to-left character in some label, and
    // then every label has to pass.
    let is_bidi = labels
        .iter()
        .flatten()
        .any(|&c| !c.is_ascii() && matches!(bidi(c), Bidi::R | Bidi::AL | Bidi::AN));
    if is_bidi && !labels.iter().all(|l| bidi_ok(l)) {
        error = true;
    }
    (labels, error)
}

/// UTS #46 ToASCII, or `None` if an error was recorded.
pub(crate) fn to_ascii(domain: &str) -> Option<String> {
    let (labels, error) = process(domain);
    if error {
        return None;
    }
    let mut out = String::with_capacity(domain.len() + 8);
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        if label.iter().all(char::is_ascii) {
            out.extend(label);
        } else {
            out.push_str("xn--");
            punycode::encode(label, &mut out)?;
        }
    }
    Some(out)
}

/// UTS #46 ToUnicode, and whether an error was recorded.
pub(crate) fn to_unicode(domain: &str) -> (String, bool) {
    let (labels, error) = process(domain);
    let mut out = String::with_capacity(domain.len());
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            out.push('.');
        }
        out.extend(label);
    }
    (out, error)
}
