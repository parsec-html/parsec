// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The host parser and serializer: domains, IPv4, IPv6 and opaque hosts.
//! A host is written straight into the URL's serialization; only the kind
//! is kept beside it.

use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::ParseError;
use crate::idna;
use crate::percent::{self, C0};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HostKind {
    None,
    Domain,
    Ipv4,
    Ipv6,
    Opaque,
    Empty,
}

/// Forbidden host code points, and the forbidden domain code points that
/// add the C0 controls, `%` and DEL to them.
const FORBIDDEN_HOST: u8 = 1;
const FORBIDDEN_DOMAIN: u8 = 2;

static FORBIDDEN: [u8; 128] = {
    let mut t = [0; 128];
    let mut b = 0;
    while b < 128 {
        let host = matches!(
            b as u8,
            0 | b'\t'
                | b'\n'
                | b'\r'
                | b' '
                | b'#'
                | b'/'
                | b':'
                | b'<'
                | b'>'
                | b'?'
                | b'@'
                | b'['
                | b'\\'
                | b']'
                | b'^'
                | b'|'
        );
        let domain = host || b < 0x20 || b == b'%' as usize || b == 0x7F;
        t[b] = ((host as u8) * FORBIDDEN_HOST) | ((domain as u8) * FORBIDDEN_DOMAIN);
        b += 1;
    }
    t
};

/// What a byte of a domain needs: lowercasing, rejecting, or the slow
/// path for percent-decoding and IDNA.
const UPPER: u8 = 1;
const FORBIDDEN_BYTE: u8 = 2;
const SLOW: u8 = 4;

static DOMAIN_BYTE: [u8; 256] = {
    let mut t = [0; 256];
    let mut b = 0;
    while b < 256 {
        t[b] = if b >= 0x80 || b == b'%' as usize {
            SLOW
        } else if FORBIDDEN[b] & FORBIDDEN_DOMAIN != 0 {
            FORBIDDEN_BYTE
        } else if (b as u8).is_ascii_uppercase() {
            UPPER
        } else {
            0
        };
        b += 1;
    }
    t
};

pub(crate) fn forbidden_domain(b: u8) -> bool {
    b < 128 && FORBIDDEN[b as usize] & FORBIDDEN_DOMAIN != 0
}

/// Parses `input`, which isn't empty unless `special` is false, and
/// appends the serialized host to `out`.
pub(crate) fn parse_into(
    out: &mut String,
    input: &str,
    special: bool,
) -> Result<HostKind, ParseError> {
    let bytes = input.as_bytes();
    if bytes.first() == Some(&b'[') {
        if bytes.last() != Some(&b']') || bytes.len() < 2 {
            return Err(ParseError::InvalidIpv6Address);
        }
        let address =
            parse_ipv6(&bytes[1..bytes.len() - 1]).ok_or(ParseError::InvalidIpv6Address)?;
        out.push('[');
        write_ipv6(out, address);
        out.push(']');
        return Ok(HostKind::Ipv6);
    }
    if !special {
        return parse_opaque(out, input);
    }
    let start = out.len();
    // The common case: ASCII and no percent sign, lowercased as it's
    // copied, with no IDNA to do. One pass gathers what the host has.
    let seen = bytes
        .iter()
        .fold(0, |seen, &b| seen | DOMAIN_BYTE[b as usize]);
    if seen & SLOW == 0 {
        if seen & FORBIDDEN_BYTE != 0 {
            return Err(ParseError::InvalidDomainCharacter);
        }
        if seen & UPPER != 0 {
            out.extend(bytes.iter().map(|b| b.to_ascii_lowercase() as char));
        } else {
            out.push_str(input);
        }
    } else {
        let decoded = percent::decode(bytes);
        let domain = std::str::from_utf8(&decoded).map_err(|_| ParseError::IdnaError)?;
        if domain.is_ascii() {
            out.extend(domain.bytes().map(|b| b.to_ascii_lowercase() as char));
        } else {
            out.push_str(&idna::to_ascii(domain).ok_or(ParseError::IdnaError)?);
        }
        let ascii = &out.as_bytes()[start..];
        if ascii.is_empty() {
            return Err(ParseError::IdnaError);
        }
        if ascii.iter().any(|&b| forbidden_domain(b)) {
            return Err(ParseError::InvalidDomainCharacter);
        }
    }
    if ends_in_number(&out.as_bytes()[start..]) {
        let address = parse_ipv4(&out.as_bytes()[start..]).ok_or(ParseError::InvalidIpv4Address)?;
        out.truncate(start);
        write_ipv4(out, address);
        return Ok(HostKind::Ipv4);
    }
    Ok(HostKind::Domain)
}

fn parse_opaque(out: &mut String, input: &str) -> Result<HostKind, ParseError> {
    if input
        .bytes()
        .any(|b| b < 128 && FORBIDDEN[b as usize] & FORBIDDEN_HOST != 0)
    {
        return Err(ParseError::InvalidDomainCharacter);
    }
    if input.is_empty() {
        return Ok(HostKind::Empty);
    }
    percent::encode_into(out, input, C0);
    Ok(HostKind::Opaque)
}

/// Whether the last label, a trailing empty one aside, is a number.
fn ends_in_number(domain: &[u8]) -> bool {
    let domain = domain.strip_suffix(b".").unwrap_or(domain);
    let last = match domain.iter().rposition(|&b| b == b'.') {
        Some(at) => &domain[at + 1..],
        None => domain,
    };
    if !last.is_empty() && last.iter().all(u8::is_ascii_digit) {
        return true;
    }
    matches!(last, [b'0', b'x' | b'X', rest @ ..] if rest.iter().all(u8::is_ascii_hexdigit))
}

/// An IPv4 number, `None` if it isn't one; values past `u32` come back as
/// `u64::MAX`, which every caller rejects.
fn parse_ipv4_number(part: &[u8]) -> Option<u64> {
    if part.is_empty() {
        return None;
    }
    let (digits, radix) = match part {
        [b'0', b'x' | b'X', rest @ ..] => (rest, 16),
        [b'0', rest @ ..] if !rest.is_empty() => (rest, 8),
        _ => (part, 10),
    };
    let mut value: u64 = 0;
    for &b in digits {
        let d = (b as char).to_digit(radix)? as u64;
        value = value.saturating_mul(radix as u64).saturating_add(d);
    }
    Some(value)
}

fn parse_ipv4(input: &[u8]) -> Option<u32> {
    let input = input.strip_suffix(b".").unwrap_or(input);
    let mut numbers = [0u64; 4];
    let mut count = 0;
    for part in input.split(|&b| b == b'.') {
        if count == 4 {
            return None;
        }
        numbers[count] = parse_ipv4_number(part)?;
        count += 1;
    }
    let (last, rest) = numbers[..count].split_last()?;
    if rest.iter().any(|&n| n > 255) || *last >= 256u64.pow(5 - count as u32) {
        return None;
    }
    let mut ipv4 = *last;
    for (i, &n) in rest.iter().enumerate() {
        ipv4 += n << (8 * (3 - i));
    }
    Some(ipv4 as u32)
}

fn write_ipv4(out: &mut String, address: u32) {
    use std::fmt::Write;
    let _ = write!(out, "{}", Ipv4Addr::from(address));
}

fn parse_ipv6(input: &[u8]) -> Option<[u16; 8]> {
    let mut address = [0u16; 8];
    let mut piece = 0;
    let mut compress = None;
    let mut p = 0;
    let at = |p: usize| input.get(p).copied();
    if at(0) == Some(b':') {
        if at(1) != Some(b':') {
            return None;
        }
        p = 2;
        piece = 1;
        compress = Some(1);
    }
    while let Some(c) = at(p) {
        if piece == 8 {
            return None;
        }
        if c == b':' {
            if compress.is_some() {
                return None;
            }
            p += 1;
            piece += 1;
            compress = Some(piece);
            continue;
        }
        let (mut value, mut length) = (0u16, 0);
        while length < 4
            && let Some(d) = at(p).and_then(|c| (c as char).to_digit(16))
        {
            value = value * 0x10 + d as u16;
            p += 1;
            length += 1;
        }
        match at(p) {
            Some(b'.') => {
                if length == 0 {
                    return None;
                }
                p -= length;
                if piece > 6 {
                    return None;
                }
                let mut seen = 0;
                while let Some(c) = at(p) {
                    if seen > 0 {
                        if c == b'.' && seen < 4 {
                            p += 1;
                        } else {
                            return None;
                        }
                    }
                    if !at(p).is_some_and(|c| c.is_ascii_digit()) {
                        return None;
                    }
                    let mut part: Option<u16> = None;
                    while let Some(c) = at(p).filter(u8::is_ascii_digit) {
                        let n = (c - b'0') as u16;
                        part = match part {
                            None => Some(n),
                            Some(0) => return None,
                            Some(v) => Some(v * 10 + n),
                        };
                        if part > Some(255) {
                            return None;
                        }
                        p += 1;
                    }
                    address[piece] = address[piece] * 0x100 + part?;
                    seen += 1;
                    if seen == 2 || seen == 4 {
                        piece += 1;
                    }
                }
                if seen != 4 {
                    return None;
                }
                break;
            }
            Some(b':') => {
                p += 1;
                at(p)?;
            }
            Some(_) => return None,
            None => {}
        }
        address[piece] = value;
        piece += 1;
    }
    if let Some(compress) = compress {
        let mut swaps = piece - compress;
        piece = 7;
        while piece != 0 && swaps > 0 {
            address.swap(piece, compress + swaps - 1);
            piece -= 1;
            swaps -= 1;
        }
    } else if piece != 8 {
        return None;
    }
    Some(address)
}

fn write_ipv6(out: &mut String, address: [u16; 8]) {
    use std::fmt::Write;
    // The first longest run of two or more zero pieces becomes "::".
    let (mut best, mut best_len, mut run, mut run_len) = (None, 1, 0, 0);
    for (i, &piece) in address.iter().enumerate() {
        if piece == 0 {
            if run_len == 0 {
                run = i;
            }
            run_len += 1;
            if run_len > best_len {
                best = Some(run);
                best_len = run_len;
            }
        } else {
            run_len = 0;
        }
    }
    let mut i = 0;
    while i < 8 {
        if best == Some(i) {
            out.push_str(if i == 0 { "::" } else { ":" });
            i += best_len;
            continue;
        }
        let _ = write!(out, "{:x}", address[i]);
        if i != 7 {
            out.push(':');
        }
        i += 1;
    }
}

/// A host as `Host::parse` gives it: what a special URL's host can be.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Host {
    Domain(String),
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),
}

impl Host {
    /// Runs the host parser as a special URL's host would, so a domain
    /// goes through IDNA and a number-ending one must be IPv4.
    pub fn parse(input: &str) -> Result<Host, ParseError> {
        if input.is_empty() {
            return Err(ParseError::EmptyHost);
        }
        let mut s = String::new();
        Ok(match parse_into(&mut s, input, true)? {
            // The serializations are canonical, so std reads them back.
            HostKind::Ipv4 => Host::Ipv4(s.parse().map_err(|_| ParseError::InvalidIpv4Address)?),
            HostKind::Ipv6 => Host::Ipv6(
                s[1..s.len() - 1]
                    .parse()
                    .map_err(|_| ParseError::InvalidIpv6Address)?,
            ),
            _ => Host::Domain(s),
        })
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Host::Domain(d) => f.write_str(d),
            Host::Ipv4(a) => write!(f, "{a}"),
            Host::Ipv6(a) => {
                let mut s = String::from("[");
                write_ipv6(&mut s, a.segments());
                s.push(']');
                f.write_str(&s)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(input: &str) -> Option<String> {
        let mut s = String::new();
        parse_into(&mut s, input, true).ok().map(|_| s)
    }

    #[test]
    fn ipv4_forms() {
        assert_eq!(host("0x7f.1").as_deref(), Some("127.0.0.1"));
        assert_eq!(host("4294967295").as_deref(), Some("255.255.255.255"));
        assert_eq!(host("4294967296"), None);
        assert_eq!(host("1.2.3.4.5"), None);
        assert_eq!(host("09"), None);
        assert_eq!(host("a.0x"), None);
        assert_eq!(host("foo.09"), None);
        assert_eq!(host("example.com").as_deref(), Some("example.com"));
    }

    #[test]
    fn ipv6_forms() {
        assert_eq!(host("[::1]").as_deref(), Some("[::1]"));
        assert_eq!(host("[0:0:1:0:0:0:0:0]").as_deref(), Some("[0:0:1::]"));
        assert_eq!(host("[::127.0.0.1]").as_deref(), Some("[::7f00:1]"));
        assert_eq!(host("[1:2:3:4:5:6:7:8:9]"), None);
        assert_eq!(host("[::1"), None);
    }
}
