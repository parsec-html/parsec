// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Unicode Normalization Forms C and D (UAX #15), from the Unicode 18.0
//! character data: canonical decomposition, canonical ordering, and
//! canonical composition with the composition exclusions, with Hangul
//! syllables done by arithmetic. IDNA needs NFC; `tests/normalization.rs`
//! runs all of Unicode's NormalizationTest.txt against both forms.
//!
//! ```
//! use parsec_url::normalize::{nfc, nfd};
//!
//! assert_eq!(nfc("e\u{301}"), "\u{e9}");
//! assert_eq!(nfd("\u{e9}"), "e\u{301}");
//! ```

use crate::tables::{
    CCC_STARTS, CCC_VALUES, COMPOSE_KEYS, COMPOSE_VALUES, DECOMP_INFO, DECOMP_KEYS, DECOMP_TEXT,
    NFC_QC_NO_OR_MAYBE,
};

const S_BASE: u32 = 0xAC00;
const L_BASE: u32 = 0x1100;
const V_BASE: u32 = 0x1161;
const T_BASE: u32 = 0x11A7;
const L_COUNT: u32 = 19;
const V_COUNT: u32 = 21;
const T_COUNT: u32 = 28;
const N_COUNT: u32 = V_COUNT * T_COUNT;
const S_COUNT: u32 = L_COUNT * N_COUNT;

/// The value of `cp`'s run in a run table.
pub(crate) fn run_value<T: Copy>(starts: &[u32], values: &[T], cp: u32) -> T {
    values[starts.partition_point(|&s| s <= cp) - 1]
}

/// Whether `cp` is in a flat lo, hi range table.
pub(crate) fn in_ranges(ranges: &[u32], cp: u32) -> bool {
    let pairs = ranges.len() / 2;
    // The first range whose hi is at least cp.
    let (mut lo, mut hi) = (0, pairs);
    while lo < hi {
        let mid = (lo + hi) / 2;
        if ranges[mid * 2 + 1] < cp {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    lo < pairs && ranges[lo * 2] <= cp
}

pub(crate) fn ccc(c: char) -> u8 {
    if (c as u32) < 0x300 {
        0
    } else {
        run_value(CCC_STARTS, CCC_VALUES, c as u32)
    }
}

fn decompose_into(c: char, out: &mut Vec<char>) {
    let cp = c as u32;
    if cp < 0xC0 {
        out.push(c);
    } else if (S_BASE..S_BASE + S_COUNT).contains(&cp) {
        let s = cp - S_BASE;
        let t = s % T_COUNT;
        // Each of these is a valid char by construction.
        out.extend(char::from_u32(L_BASE + s / N_COUNT));
        out.extend(char::from_u32(V_BASE + (s % N_COUNT) / T_COUNT));
        if t != 0 {
            out.extend(char::from_u32(T_BASE + t));
        }
    } else if let Ok(i) = DECOMP_KEYS.binary_search(&cp) {
        let info = DECOMP_INFO[i] as usize;
        out.extend(DECOMP_TEXT[info >> 8..(info >> 8) + (info & 0xFF)].chars());
    } else {
        out.push(c);
    }
}

/// Sorts each run of non-starters by combining class, keeping equal
/// classes in order.
fn reorder(chars: &mut [char]) {
    let mut i = 0;
    while i < chars.len() {
        if ccc(chars[i]) == 0 {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && ccc(chars[i]) != 0 {
            i += 1;
        }
        chars[start..i].sort_by_key(|&c| ccc(c));
    }
}

fn decompose(s: &str) -> Vec<char> {
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        decompose_into(c, &mut out);
    }
    reorder(&mut out);
    out
}

fn compose_pair(a: char, b: char) -> Option<char> {
    let (a, b) = (a as u32, b as u32);
    if (L_BASE..L_BASE + L_COUNT).contains(&a) && (V_BASE..V_BASE + V_COUNT).contains(&b) {
        return char::from_u32(S_BASE + ((a - L_BASE) * V_COUNT + (b - V_BASE)) * T_COUNT);
    }
    if (S_BASE..S_BASE + S_COUNT).contains(&a)
        && (a - S_BASE).is_multiple_of(T_COUNT)
        && (T_BASE + 1..T_BASE + T_COUNT).contains(&b)
    {
        return char::from_u32(a + (b - T_BASE));
    }
    let key = (a as u64) << 21 | b as u64;
    COMPOSE_KEYS
        .binary_search(&key)
        .ok()
        .and_then(|i| char::from_u32(COMPOSE_VALUES[i]))
}

/// Whether `s` is NFC by the quick check: no code point that can change
/// under NFC, and every run of marks already in canonical order.
pub(crate) fn is_nfc_quick(s: &str) -> bool {
    let mut last = 0;
    for c in s.chars() {
        if (c as u32) < 0x300 {
            last = 0;
            continue;
        }
        let class = ccc(c);
        if (class != 0 && last > class) || in_ranges(NFC_QC_NO_OR_MAYBE, c as u32) {
            return false;
        }
        last = class;
    }
    true
}

/// Canonical decomposition, Normalization Form D.
pub fn nfd(s: &str) -> String {
    if s.is_ascii() {
        return s.to_owned();
    }
    decompose(s).into_iter().collect()
}

/// Canonical decomposition then canonical composition, Normalization
/// Form C.
pub fn nfc(s: &str) -> String {
    if is_nfc_quick(s) {
        return s.to_owned();
    }
    let chars = decompose(s);
    let mut out: Vec<char> = Vec::with_capacity(chars.len());
    // Where the last starter is in `out`, and the class of the last char
    // pushed: a mark composes with the starter unless something between
    // them has a class at least as high, which would block it.
    let mut starter: Option<usize> = None;
    let mut last_class = 0;
    for c in chars {
        let class = ccc(c);
        if let Some(at) = starter {
            let adjacent = at + 1 == out.len();
            if (adjacent || (last_class != 0 && last_class < class))
                && let Some(composed) = compose_pair(out[at], c)
            {
                out[at] = composed;
                continue;
            }
        }
        if class == 0 {
            starter = Some(out.len());
        }
        last_class = class;
        out.push(c);
    }
    out.into_iter().collect()
}
