// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! What a token's text means: an identifier's name with its escapes
//! decoded, a string's or template's value in UTF-16 code units, a
//! number's value, and those values written out as JSON.

use std::borrow::Cow;
use std::fmt::Write;

/// An IdentifierName's StringValue: `a` and `\u{61}` decoded.
pub fn ident_name(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }
    let mut out = String::with_capacity(raw.len());
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && b.get(i + 1) == Some(&b'u') {
            let (value, len) = unicode_escape(&raw[i + 2..]);
            if let Some(c) = value.and_then(char::from_u32) {
                out.push(c);
            }
            i += 2 + len;
        } else {
            let c = raw[i..].chars().next().unwrap_or('\0');
            out.push(c);
            i += c.len_utf8().max(1);
        }
    }
    Cow::Owned(out)
}

/// The value of the hex digits after `\u`, four or in braces, and how
/// many bytes they took.
fn unicode_escape(s: &str) -> (Option<u32>, usize) {
    let b = s.as_bytes();
    if b.first() == Some(&b'{') {
        let end = b.iter().position(|&c| c == b'}').unwrap_or(b.len());
        let value = u32::from_str_radix(&s[1..end], 16).ok();
        (value, (end + 1).min(b.len()))
    } else {
        // Up to four hex digits, which a tagged template's invalid escape
        // (`\u/0é`) may stop short of.
        let end = b
            .iter()
            .take(4)
            .take_while(|c| c.is_ascii_hexdigit())
            .count();
        let value = u32::from_str_radix(&s[..end], 16).ok().filter(|_| end == 4);
        (value, end)
    }
}

fn push_char(out: &mut Vec<u16>, c: char) {
    let mut buf = [0; 2];
    out.extend_from_slice(c.encode_utf16(&mut buf));
}

fn push_code_point(out: &mut Vec<u16>, cp: u32) {
    match char::from_u32(cp) {
        Some(c) => push_char(out, c),
        // A lone surrogate, which a JavaScript string can hold.
        None => out.push(cp as u16),
    }
}

fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// A string literal's value, from its text without the quotes, as
/// UTF-16 code units: it can hold a lone surrogate (`"\uD800"`), which no
/// Rust string can.
pub fn string_value(body: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(body.len());
    let mut chars = body.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c != '\\' {
            push_char(&mut out, c);
            continue;
        }
        let Some((_, e)) = chars.next() else { break };
        match e {
            'b' => out.push(8),
            'f' => out.push(12),
            'n' => out.push(10),
            'r' => out.push(13),
            't' => out.push(9),
            'v' => out.push(11),
            '\r' => {
                chars.next_if(|&(_, c)| c == '\n');
            }
            '\n' | '\u{2028}' | '\u{2029}' => {}
            'x' => {
                let hex = body.get(i + 2..i + 4).unwrap_or("");
                out.push(u16::from_str_radix(hex, 16).unwrap_or(0));
                chars.next();
                chars.next();
            }
            'u' => {
                let (value, len) = unicode_escape(&body[i + 2..]);
                push_code_point(&mut out, value.unwrap_or(0));
                for _ in 0..len {
                    chars.next();
                }
            }
            '0'..='7' => {
                // LegacyOctalEscapeSequence: up to three digits from 0-3,
                // two from 4-7, and a lone \0 is NUL.
                let max = if e <= '3' { 3 } else { 2 };
                let mut value = e as u32 - '0' as u32;
                let mut n = 1;
                while n < max {
                    match chars.peek() {
                        Some(&(_, d @ '0'..='7')) => {
                            value = value * 8 + (d as u32 - '0' as u32);
                            chars.next();
                            n += 1;
                        }
                        _ => break,
                    }
                }
                out.push(value as u16);
            }
            other => push_char(&mut out, other),
        }
    }
    out
}

/// A template part's cooked value, from its text between delimiters, or
/// `None` if it has an escape that isn't one. A CR or CRLF in the text is
/// an LF in the value.
pub fn template_cooked(body: &str) -> Option<Vec<u16>> {
    let mut out = Vec::with_capacity(body.len());
    let mut chars = body.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '\r' {
            chars.next_if(|&(_, c)| c == '\n');
            out.push(10);
            continue;
        }
        if c != '\\' {
            push_char(&mut out, c);
            continue;
        }
        let Some((_, e)) = chars.next() else { break };
        match e {
            'b' => out.push(8),
            'f' => out.push(12),
            'n' => out.push(10),
            'r' => out.push(13),
            't' => out.push(9),
            'v' => out.push(11),
            '\r' => {
                chars.next_if(|&(_, c)| c == '\n');
            }
            c if is_line_terminator(c) => {}
            '0' if !chars.peek().is_some_and(|&(_, d)| d.is_ascii_digit()) => out.push(0),
            '0'..='9' => return None,
            'x' => {
                let hex = body.get(i + 2..i + 4)?;
                out.push(u16::from_str_radix(hex, 16).ok()?);
                chars.next();
                chars.next();
            }
            'u' => {
                let (value, len) = unicode_escape(&body[i + 2..]);
                let value = value.filter(|&v| v <= 0x10_FFFF)?;
                if !body[i + 2..].starts_with('{') && len < 4 {
                    return None;
                }
                push_code_point(&mut out, value);
                for _ in 0..len {
                    chars.next();
                }
            }
            other => push_char(&mut out, other),
        }
    }
    Some(out)
}

/// A template part's raw value: its text, with each CR or CRLF an LF.
pub fn template_raw(body: &str) -> Cow<'_, str> {
    if !body.contains('\r') {
        return Cow::Borrowed(body);
    }
    Cow::Owned(body.replace("\r\n", "\n").replace('\r', "\n"))
}

/// Writes UTF-16 `units` as a JSON string, the way `JSON.stringify`
/// does: a lone surrogate as `\udxxx`, a control character as an escape,
/// everything else as itself.
pub fn write_json_units(out: &mut String, units: &[u16]) {
    out.push('"');
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(c) => push_json_char(out, c),
            Err(e) => {
                let _ = write!(out, "\\u{:04x}", e.unpaired_surrogate());
            }
        }
    }
    out.push('"');
}

/// Writes `s` as a JSON string.
pub fn write_json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        push_json_char(out, c);
    }
    out.push('"');
}

fn push_json_char(out: &mut String, c: char) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c if (c as u32) < 0x20 => {
            let _ = write!(out, "\\u{:04x}", c as u32);
        }
        c => out.push(c),
    }
}

/// The digits of a numeric literal without its separators and prefix,
/// and its radix.
fn digits(raw: &str) -> (String, u32) {
    let lower = raw.as_bytes().get(1).map(u8::to_ascii_lowercase);
    let (radix, body) = match (raw.as_bytes().first(), lower) {
        (Some(b'0'), Some(b'x')) => (16, &raw[2..]),
        (Some(b'0'), Some(b'o')) => (8, &raw[2..]),
        (Some(b'0'), Some(b'b')) => (2, &raw[2..]),
        // A legacy octal literal: all its digits are octal ones.
        (Some(b'0'), Some(b'0'..=b'7')) if raw.bytes().all(|c| (b'0'..=b'7').contains(&c)) => {
            (8, &raw[1..])
        }
        _ => (10, raw),
    };
    (body.chars().filter(|&c| c != '_').collect(), radix)
}

/// A NumericLiteral's value (not a BigInt's), rounded as the spec says.
pub fn number_value(raw: &str) -> f64 {
    let (digits, radix) = digits(raw);
    if radix == 10 {
        return digits.parse().unwrap_or(f64::NAN);
    }
    // Exact up to 128 bits, and rounded once from there; past that, a
    // value this long is far outside anything a test or a bundle holds.
    let mut value: u128 = 0;
    let mut overflow = 1.0;
    for c in digits.chars() {
        let d = c.to_digit(radix).unwrap_or(0) as u128;
        match value
            .checked_mul(radix as u128)
            .and_then(|v| v.checked_add(d))
        {
            Some(v) if overflow == 1.0 => value = v,
            _ => overflow *= radix as f64,
        }
    }
    value as f64 * overflow
}

/// A BigInt literal's value in decimal, as acorn's `bigint` field has it.
pub fn bigint_decimal(raw: &str) -> String {
    let (digits, radix) = digits(raw.strip_suffix('n').unwrap_or(raw));
    if radix == 10 {
        return digits;
    }
    // Base 10^9 limbs, least significant first.
    let mut limbs: Vec<u32> = vec![0];
    for c in digits.chars() {
        let mut carry = c.to_digit(radix).unwrap_or(0) as u64;
        for limb in &mut limbs {
            let v = *limb as u64 * radix as u64 + carry;
            *limb = (v % 1_000_000_000) as u32;
            carry = v / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry as u32);
        }
    }
    let mut out = String::new();
    let mut iter = limbs.iter().rev();
    let _ = write!(out, "{}", iter.next().unwrap_or(&0));
    for limb in iter {
        let _ = write!(out, "{limb:09}");
    }
    out
}

/// `x` as JavaScript's Number::toString writes it, which is what
/// `JSON.stringify` writes for a finite number.
pub fn write_js_number(out: &mut String, x: f64) {
    if !x.is_finite() {
        out.push_str("null");
        return;
    }
    if x == 0.0 {
        out.push('0');
        return;
    }
    if x < 0.0 {
        out.push('-');
    }
    // Rust's `{:e}` gives the shortest digits that read back as `x`.
    let e = format!("{:e}", x.abs());
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let digits: String = mantissa.chars().filter(|&c| c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap_or(0) + 1;
    if k <= n && n <= 21 {
        out.push_str(&digits);
        out.extend(std::iter::repeat_n('0', (n - k) as usize));
    } else if 0 < n && n <= 21 {
        out.push_str(&digits[..n as usize]);
        out.push('.');
        out.push_str(&digits[n as usize..]);
    } else if -6 < n && n <= 0 {
        out.push_str("0.");
        out.extend(std::iter::repeat_n('0', (-n) as usize));
        out.push_str(&digits);
    } else {
        out.push_str(&digits[..1]);
        if k > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        let _ = write!(out, "e{}{}", if n > 0 { '+' } else { '-' }, (n - 1).abs());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn number(x: f64) -> String {
        let mut s = String::new();
        write_js_number(&mut s, x);
        s
    }

    #[test]
    fn numbers_print_as_javascript_prints_them() {
        for (x, s) in [
            (1.0, "1"),
            (0.1, "0.1"),
            (123.456, "123.456"),
            (1e21, "1e+21"),
            (1e20, "100000000000000000000"),
            (5e-7, "5e-7"),
            (0.000001, "0.000001"),
            (1.5e-10, "1.5e-10"),
            (-2.5, "-2.5"),
            (f64::INFINITY, "null"),
            (2f64.powi(53), "9007199254740992"),
        ] {
            assert_eq!(number(x), s);
        }
    }

    #[test]
    fn number_values() {
        assert_eq!(number_value("0x1_0"), 16.0);
        assert_eq!(number_value("017"), 15.0);
        assert_eq!(number_value("019"), 19.0);
        assert_eq!(number_value("0b101"), 5.0);
        assert_eq!(number_value(".5e1"), 5.0);
        assert_eq!(number_value("1_000.5"), 1000.5);
        assert_eq!(
            number_value("0xFFFFFFFFFFFFFFFFFF"),
            4722366482869645213696.0
        );
        assert_eq!(bigint_decimal("0x1_0n"), "16");
        assert_eq!(
            bigint_decimal("0xFFFFFFFFFFFFFFFFFFFFn"),
            "1208925819614629174706175"
        );
        assert_eq!(bigint_decimal("12n"), "12");
    }

    #[test]
    fn string_values() {
        let s = |body: &str| String::from_utf16(&string_value(body)).unwrap();
        assert_eq!(s(r"a\nb"), "a\nb");
        assert_eq!(s(r"\x41B\u{43}"), "ABC");
        assert_eq!(s("a\\\r\nb"), "ab");
        assert_eq!(s(r"\101\0\8"), "A\u{0}8");
        assert_eq!(s(r"\q"), "q");
        assert_eq!(string_value(r"\uD800"), [0xD800]);
        let mut out = String::new();
        write_json_units(&mut out, &string_value(r"\uD800\u0001\x22é"));
        assert_eq!(out, r#""\ud800\u0001\"é""#);
    }

    #[test]
    fn template_values() {
        let c = |body: &str| template_cooked(body).map(|u| String::from_utf16(&u).unwrap());
        assert_eq!(c("a\r\nb\rc").as_deref(), Some("a\nb\nc"));
        assert_eq!(c(r"\01"), None);
        assert_eq!(c(r"\xg"), None);
        assert_eq!(c(r"\u{110000}"), None);
        assert_eq!(c(r"\0"), Some("\0".into()));
        // Found by the parse fuzz target: an escape cut short before a
        // character of more than one byte.
        assert_eq!(c("\\u/00\u{e1}g"), None);
        assert_eq!(c("\\u00\u{e1}"), None);
        assert_eq!(template_raw("a\r\nb\r"), "a\nb\n");
        assert_eq!(ident_name(r"ab\u{63}"), "abc");
    }
}
