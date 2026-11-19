// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! RFC 3492 Punycode, one label at a time and without the `xn--` prefix.
//!
//! ```
//! use parsec_url::punycode;
//!
//! assert_eq!(punycode::encode_str("bücher").unwrap(), "bcher-kva");
//! assert_eq!(punycode::decode_to_string("bcher-kva").unwrap(), "bücher");
//! ```

const BASE: u32 = 36;
const T_MIN: u32 = 1;
const T_MAX: u32 = 26;
const SKEW: u32 = 38;
const DAMP: u32 = 700;
const INITIAL_BIAS: u32 = 72;
const INITIAL_N: u32 = 128;

fn adapt(mut delta: u32, points: u32, first: bool) -> u32 {
    delta /= if first { DAMP } else { 2 };
    delta += delta / points;
    let mut k = 0;
    while delta > ((BASE - T_MIN) * T_MAX) / 2 {
        delta /= BASE - T_MIN;
        k += BASE;
    }
    k + (BASE - T_MIN + 1) * delta / (delta + SKEW)
}

fn threshold(k: u32, bias: u32) -> u32 {
    if k <= bias {
        T_MIN
    } else if k >= bias + T_MAX {
        T_MAX
    } else {
        k - bias
    }
}

fn digit_value(b: u8) -> Option<u32> {
    match b {
        b'a'..=b'z' => Some((b - b'a') as u32),
        b'A'..=b'Z' => Some((b - b'A') as u32),
        b'0'..=b'9' => Some((b - b'0') as u32 + 26),
        _ => None,
    }
}

/// Decodes a Punycode label into code points, or `None` if it isn't
/// valid Punycode or decodes to something that isn't a scalar value.
pub(crate) fn decode(input: &str) -> Option<Vec<char>> {
    let bytes = input.as_bytes();
    let (basic, extended) = match bytes.iter().rposition(|&b| b == b'-') {
        Some(at) => (&bytes[..at], &bytes[at + 1..]),
        None => (&bytes[..0], bytes),
    };
    if !basic.is_ascii() {
        return None;
    }
    let mut out: Vec<char> = basic.iter().map(|&b| b as char).collect();
    let (mut n, mut i, mut bias) = (INITIAL_N, 0u32, INITIAL_BIAS);
    let mut digits = extended.iter();
    while digits.len() > 0 {
        let old_i = i;
        let mut w = 1u32;
        let mut k = BASE;
        loop {
            let digit = digit_value(*digits.next()?)?;
            i = i.checked_add(digit.checked_mul(w)?)?;
            let t = threshold(k, bias);
            if digit < t {
                break;
            }
            w = w.checked_mul(BASE - t)?;
            k += BASE;
        }
        let len = out.len() as u32 + 1;
        bias = adapt(i - old_i, len, old_i == 0);
        n = n.checked_add(i / len)?;
        i %= len;
        out.insert(i as usize, char::from_u32(n)?);
        i += 1;
    }
    Some(out)
}

/// Encodes code points as Punycode, or `None` on overflow.
pub(crate) fn encode(input: &[char], out: &mut String) -> Option<()> {
    let start = out.len();
    out.extend(input.iter().filter(|c| c.is_ascii()));
    let basic = (out.len() - start) as u32;
    if basic > 0 {
        out.push('-');
    }
    let (mut n, mut delta, mut bias) = (INITIAL_N, 0u32, INITIAL_BIAS);
    let mut handled = basic;
    let total = input.len() as u32;
    while handled < total {
        let m = input.iter().map(|&c| c as u32).filter(|&c| c >= n).min()?;
        delta = delta.checked_add((m - n).checked_mul(handled + 1)?)?;
        n = m;
        for &c in input {
            let c = c as u32;
            if c < n {
                delta = delta.checked_add(1)?;
            }
            if c == n {
                let mut q = delta;
                let mut k = BASE;
                loop {
                    let t = threshold(k, bias);
                    if q < t {
                        break;
                    }
                    let d = t + (q - t) % (BASE - t);
                    out.push(digit(d));
                    q = (q - t) / (BASE - t);
                    k += BASE;
                }
                out.push(digit(q));
                bias = adapt(delta, handled + 1, handled == basic);
                delta = 0;
                handled += 1;
            }
        }
        delta = delta.checked_add(1)?;
        n = n.checked_add(1)?;
    }
    Some(())
}

fn digit(d: u32) -> char {
    (if d < 26 {
        b'a' + d as u8
    } else {
        b'0' + (d - 26) as u8
    }) as char
}

/// Encodes a label as Punycode: `"bücher"` becomes `"bcher-kva"`. `None`
/// means the arithmetic overflowed, which takes a very long label.
pub fn encode_str(input: &str) -> Option<String> {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    encode(&chars, &mut out)?;
    Some(out)
}

/// Decodes a Punycode label: `"bcher-kva"` becomes `"bücher"`.
pub fn decode_to_string(input: &str) -> Option<String> {
    decode(input).map(|chars| chars.into_iter().collect())
}
