// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The two microsyntaxes CSS Syntax defines over tokens: An+B, for
//! `:nth-child()`, and `<urange>`, for `@font-face`'s `unicode-range`.

use crate::parser::{ComponentValue, parse_component_values, parse_unicode_range_value};
use crate::tokenizer::{Number, Token};

/// Reads An+B from a list of component values: `(A, B)`, or `None` if
/// they aren't An+B. Values beyond `i32` saturate.
///
/// ```
/// assert_eq!(parsec_css::parse_anb("2n+1"), Some((2, 1)));
/// assert_eq!(parsec_css::parse_anb(" -n- 3 "), Some((-1, -3)));
/// assert_eq!(parsec_css::parse_anb("+ 2n"), None);
/// ```
pub fn anb(values: &[ComponentValue<'_>]) -> Option<(i32, i32)> {
    let mut rest = values.iter();
    let skip_ws = |it: &mut std::slice::Iter<'_, ComponentValue<'_>>| {
        while it
            .as_slice()
            .first()
            .is_some_and(ComponentValue::is_whitespace)
        {
            it.next();
        }
    };
    skip_ws(&mut rest);
    let first = rest.next()?.token()?;
    let result = match first {
        Token::Number(n) if n.integer => (0, n.int()),
        Token::Ident(v) => match v.to_ascii_lowercase().as_str() {
            "odd" => (2, 1),
            "even" => (2, 0),
            s => match s.strip_prefix('-') {
                Some(s) => after_n(-1, s, &mut rest)?,
                None => after_n(1, s, &mut rest)?,
            },
        },
        Token::Dimension { number, unit } if number.integer => {
            after_n(number.int(), &unit.to_ascii_lowercase(), &mut rest)?
        }
        // `+n…`, with nothing between the `+` and the `n`.
        Token::Delim('+') => match rest.next()?.token()? {
            Token::Ident(v) if !v.starts_with('-') => {
                after_n(1, &v.to_ascii_lowercase(), &mut rest)?
            }
            _ => return None,
        },
        _ => return None,
    };
    skip_ws(&mut rest);
    rest.next().is_none().then_some(result)
}

/// The rest of An+B once A is known; `n` is what followed A, lowercased:
/// `n`, `n-` or `n-` and digits.
fn after_n(
    a: i32,
    n: &str,
    rest: &mut std::slice::Iter<'_, ComponentValue<'_>>,
) -> Option<(i32, i32)> {
    let signless = |t: Option<&Token<'_>>| match t {
        Some(Token::Number(n)) if n.integer && n.sign().is_none() => Some(n.int()),
        _ => None,
    };
    match n {
        "n" => {
            let mut look = rest.clone();
            match next_solid(&mut look) {
                None => Some((a, 0)),
                Some(Token::Number(n @ Number { integer: true, .. })) if n.sign().is_some() => {
                    *rest = look;
                    Some((a, n.int()))
                }
                Some(&Token::Delim(sign @ ('+' | '-'))) => {
                    let b = signless(next_solid(&mut look))?;
                    *rest = look;
                    Some((a, if sign == '-' { b.saturating_neg() } else { b }))
                }
                _ => None,
            }
        }
        "n-" => {
            let b = signless(next_solid(rest))?;
            Some((a, b.saturating_neg()))
        }
        _ => {
            let digits = n.strip_prefix("n-")?;
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let b: f64 = digits.parse().ok()?;
            Some((a, -(b as i32)))
        }
    }
}

fn next_solid<'t, 'a>(it: &mut std::slice::Iter<'t, ComponentValue<'a>>) -> Option<&'t Token<'a>> {
    it.find(|v| !v.is_whitespace())
        .and_then(ComponentValue::token)
}

/// [`anb`] over CSS text.
pub fn parse_anb(css: &str) -> Option<(i32, i32)> {
    anb(&parse_component_values(css))
}

/// Writes An+B the way CSS Syntax serializes it: `2n+1`, `-n`, `5`.
pub fn serialize_anb(a: i32, b: i32) -> String {
    if a == 0 {
        return b.to_string();
    }
    let mut out = match a {
        1 => "n".to_string(),
        -1 => "-n".to_string(),
        _ => format!("{a}n"),
    };
    if b > 0 {
        out.push('+');
    }
    if b != 0 {
        out.push_str(&b.to_string());
    }
    out
}

/// A `<urange>` from CSS Fonts: the code points `start..=end`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct UnicodeRange {
    pub start: u32,
    pub end: u32,
}

/// Reads a `unicode-range` descriptor's comma-separated `<urange>`s. Each
/// is `None` if it isn't one range, or its range is backwards or past
/// U+10FFFF.
///
/// ```
/// use parsec_css::{UnicodeRange, parse_unicode_ranges};
///
/// let ranges = parse_unicode_ranges("U+0-7F, u+4??, U+5-1");
/// assert_eq!(ranges[0], Some(UnicodeRange { start: 0, end: 0x7f }));
/// assert_eq!(ranges[1], Some(UnicodeRange { start: 0x400, end: 0x4ff }));
/// assert_eq!(ranges[2], None);
/// ```
pub fn parse_unicode_ranges(css: &str) -> Vec<Option<UnicodeRange>> {
    let values = parse_unicode_range_value(css);
    values
        .split(|v| v.token() == Some(&Token::Comma))
        .map(|group| {
            let mut solid = group.iter().filter(|v| !v.is_whitespace());
            match (solid.next().and_then(ComponentValue::token), solid.next()) {
                (Some(&Token::UnicodeRange { start, end }), None)
                    if start <= end && end <= 0x10ffff =>
                {
                    Some(UnicodeRange { start, end })
                }
                _ => None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_anb() {
        for (a, b, s) in [(0, 5, "5"), (1, 0, "n"), (-1, 3, "-n+3"), (2, -1, "2n-1")] {
            assert_eq!(serialize_anb(a, b), s);
            assert_eq!(parse_anb(s), Some((a, b)));
        }
    }
}
