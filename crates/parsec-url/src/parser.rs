// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The basic URL parser. It writes the serialization as it goes, into one
//! string, and records where each component starts, so a parsed URL is
//! one allocation and the getters are slices of it. The setters take the
//! URL apart into those slices, change one, and put it back together.
//!
//! The state machine is the standard's, walked over runs of bytes rather
//! than one code point at a time: a state that only collects, like the
//! scheme or a path segment, is a scan to the byte that ends it.

use std::borrow::Cow;

use crate::host::{self, HostKind};
use crate::percent::{C0, FRAGMENT, PATH, QUERY, SPECIAL_QUERY, USERINFO, encode, encode_into};
use crate::{ParseError, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scheme {
    Http,
    Https,
    Ws,
    Wss,
    Ftp,
    File,
    Other,
}

impl Scheme {
    pub(crate) fn of(scheme: &str) -> Scheme {
        match scheme {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            "ws" => Scheme::Ws,
            "wss" => Scheme::Wss,
            "ftp" => Scheme::Ftp,
            "file" => Scheme::File,
            _ => Scheme::Other,
        }
    }

    pub(crate) fn is_special(self) -> bool {
        self != Scheme::Other
    }

    pub(crate) fn default_port(self) -> Option<u16> {
        match self {
            Scheme::Http | Scheme::Ws => Some(80),
            Scheme::Https | Scheme::Wss => Some(443),
            Scheme::Ftp => Some(21),
            Scheme::File | Scheme::Other => None,
        }
    }
}

fn is_scheme_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.')
}

/// The length of the scheme at the start of `bytes`, if a `:` ends it.
fn scheme_len(bytes: &[u8]) -> Option<usize> {
    if !bytes.first()?.is_ascii_alphabetic() {
        return None;
    }
    let len = bytes.iter().position(|&b| !is_scheme_byte(b))?;
    (bytes[len] == b':').then_some(len)
}

fn is_slash(b: u8, special: bool) -> bool {
    b == b'/' || (special && b == b'\\')
}

fn skip_slashes(bytes: &[u8], mut p: usize) -> usize {
    while p < bytes.len() && matches!(bytes[p], b'/' | b'\\') {
        p += 1;
    }
    p
}

fn is_single_dot(seg: &[u8]) -> bool {
    seg == b"." || seg.eq_ignore_ascii_case(b"%2e")
}

fn is_double_dot(seg: &[u8]) -> bool {
    match seg.len() {
        2 => seg == b"..",
        4 => seg.eq_ignore_ascii_case(b".%2e") || seg.eq_ignore_ascii_case(b"%2e."),
        6 => seg.eq_ignore_ascii_case(b"%2e%2e"),
        _ => false,
    }
}

/// A Windows drive letter: an ASCII letter, then `:` or `|`.
fn is_drive_letter(seg: &[u8]) -> bool {
    seg.len() == 2 && seg[0].is_ascii_alphabetic() && matches!(seg[1], b':' | b'|')
}

fn starts_with_drive_letter(bytes: &[u8]) -> bool {
    bytes.len() >= 2
        && is_drive_letter(&bytes[..2])
        && (bytes.len() == 2 || matches!(bytes[2], b'/' | b'\\' | b'?' | b'#'))
}

/// Removes the path's last segment, unless it's a file URL's lone drive
/// letter.
fn shorten(out: &mut String, path_start: usize, scheme: Scheme) {
    let path = &out.as_bytes()[path_start..];
    if path.is_empty()
        || (scheme == Scheme::File
            && path.len() == 3
            && path[1].is_ascii_alphabetic()
            && path[2] == b':')
    {
        return;
    }
    let at = path.iter().rposition(|&b| b == b'/').unwrap_or(0);
    out.truncate(path_start + at);
}

/// Bytes that end a path segment or need encoding in one.
static PATH_STOP: [bool; 256] = {
    let mut t = [false; 256];
    let mut b = 0;
    while b < 256 {
        t[b] = crate::percent::SETS[b] & PATH != 0 || b == b'/' as usize || b == b'\\' as usize;
        b += 1;
    }
    t
};

/// The path state, from `p` until `?`, `#` or the end (or only the end,
/// for the pathname setter), appending segments to `out`, whose path
/// begins at `path_start`. Returns where it stopped.
pub(crate) fn parse_path(
    out: &mut String,
    path_start: usize,
    input: &str,
    mut p: usize,
    scheme: Scheme,
    until_query: bool,
) -> usize {
    let bytes = input.as_bytes();
    let special = scheme.is_special();
    loop {
        // The segment's end, and whether anything in it needs encoding.
        let mut q = p;
        let mut plain = true;
        while q < bytes.len() {
            let b = bytes[q];
            if PATH_STOP[b as usize] {
                if is_slash(b, special) || (until_query && (b == b'?' || b == b'#')) {
                    break;
                }
                plain &= b == b'\\';
            }
            q += 1;
        }
        let seg = &bytes[p..q];
        let slash = q < bytes.len() && is_slash(bytes[q], special);
        if seg.first().is_some_and(|&b| b == b'.' || b == b'%') && is_double_dot(seg) {
            shorten(out, path_start, scheme);
            if !slash {
                out.push('/');
            }
        } else if seg.first().is_some_and(|&b| b == b'.' || b == b'%') && is_single_dot(seg) {
            if !slash {
                out.push('/');
            }
        } else {
            out.push('/');
            if scheme == Scheme::File && out.len() == path_start + 1 && is_drive_letter(seg) {
                out.push(seg[0] as char);
                out.push(':');
            } else if plain {
                out.push_str(&input[p..q]);
            } else {
                encode_into(out, &input[p..q], PATH);
            }
        }
        if !slash {
            return q;
        }
        p = q + 1;
    }
}

fn push_port(out: &mut String, port: u16) {
    let mut digits = [0u8; 5];
    let mut n = port;
    let mut i = 5;
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    for &d in &digits[i..] {
        out.push(d as char);
    }
}

/// Parses a port's digits; `Ok(None)` for none, or for the default.
fn parse_port(digits: &[u8], scheme: Scheme) -> Result<Option<u16>, ParseError> {
    if digits.is_empty() {
        return Ok(None);
    }
    let mut value: u32 = 0;
    for &b in digits {
        if !b.is_ascii_digit() {
            return Err(ParseError::InvalidPort);
        }
        value = value * 10 + (b - b'0') as u32;
        if value > 65535 {
            return Err(ParseError::InvalidPort);
        }
    }
    let port = value as u16;
    Ok((scheme.default_port() != Some(port)).then_some(port))
}

/// A URL being written, with offsets as `usize` until it's done.
struct Builder {
    s: String,
    scheme_end: usize,
    username_end: usize,
    host_start: usize,
    host_end: usize,
    path_start: usize,
    query_start: Option<usize>,
    fragment_start: Option<usize>,
    port: Option<u16>,
    host: HostKind,
    scheme: Scheme,
    opaque_path: bool,
}

impl Builder {
    fn finish(self) -> Result<Url, ParseError> {
        let len = |n: usize| u32::try_from(n).map_err(|_| ParseError::Overflow);
        len(self.s.len())?;
        Ok(Url {
            scheme_end: self.scheme_end as u32,
            username_end: self.username_end as u32,
            host_start: self.host_start as u32,
            host_end: self.host_end as u32,
            path_start: self.path_start as u32,
            query_start: self.query_start.map(|q| q as u32),
            fragment_start: self.fragment_start.map(|f| f as u32),
            port: self.port,
            host: self.host,
            scheme: self.scheme,
            opaque_path: self.opaque_path,
            s: self.s,
        })
    }

    fn special(&self) -> bool {
        self.scheme.is_special()
    }

    fn no_host(&mut self) {
        self.username_end = self.s.len();
        self.host_start = self.s.len();
        self.host_end = self.s.len();
        self.path_start = self.s.len();
        self.host = HostKind::None;
    }

    /// From `p`, after the `//`: credentials, host and port, then the path.
    fn authority(&mut self, input: &str, start: usize) -> Result<(), ParseError> {
        let bytes = input.as_bytes();
        let special = self.special();
        let end = bytes[start..]
            .iter()
            .position(|&b| matches!(b, b'/' | b'?' | b'#') || (special && b == b'\\'))
            .map_or(bytes.len(), |n| start + n);
        let authority = &input[start..end];
        self.s.push_str("//");
        let hostport = match authority.rfind('@') {
            Some(at) => {
                let hostport = &authority[at + 1..];
                if hostport.is_empty() {
                    return Err(ParseError::EmptyHost);
                }
                let userinfo = &authority[..at];
                let (username, password) = match userinfo.find(':') {
                    Some(c) => (&userinfo[..c], &userinfo[c + 1..]),
                    None => (userinfo, ""),
                };
                encode_into(&mut self.s, username, USERINFO);
                self.username_end = self.s.len();
                if !password.is_empty() {
                    self.s.push(':');
                    encode_into(&mut self.s, password, USERINFO);
                }
                if self.s.len() > self.scheme_end + 3 {
                    self.s.push('@');
                }
                hostport
            }
            None => {
                self.username_end = self.s.len();
                authority
            }
        };
        self.host_start = self.s.len();
        // The host state: a `:` outside brackets starts the port.
        let hb = hostport.as_bytes();
        let mut inside = false;
        let mut colon = None;
        for (i, &b) in hb.iter().enumerate() {
            match b {
                b'[' => inside = true,
                b']' => inside = false,
                b':' if !inside => {
                    colon = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let host = &hostport[..colon.unwrap_or(hb.len())];
        if host.is_empty() && (special || colon.is_some()) {
            return Err(ParseError::EmptyHost);
        }
        self.host = host::parse_into(&mut self.s, host, special)?;
        self.host_end = self.s.len();
        if let Some(c) = colon {
            self.port = parse_port(&hb[c + 1..], self.scheme)?;
            if let Some(port) = self.port {
                self.s.push(':');
                push_port(&mut self.s, port);
            }
        }
        self.path_start = self.s.len();
        self.path_start_state(input, end)
    }

    /// The path start state, after an authority.
    fn path_start_state(&mut self, input: &str, p: usize) -> Result<(), ParseError> {
        let bytes = input.as_bytes();
        self.path_start = self.s.len();
        let next = bytes.get(p).copied();
        if self.special() {
            let p = if matches!(next, Some(b'/' | b'\\')) {
                p + 1
            } else {
                p
            };
            return self.path_then_rest(input, p);
        }
        match next {
            None => Ok(()),
            Some(b'?' | b'#') => self.rest(input, p),
            Some(b'/') => self.path_then_rest(input, p + 1),
            Some(_) => self.path_then_rest(input, p),
        }
    }

    fn path_then_rest(&mut self, input: &str, p: usize) -> Result<(), ParseError> {
        let q = parse_path(&mut self.s, self.path_start, input, p, self.scheme, true);
        self.rest(input, q)
    }

    /// A URL without a host whose path starts with an empty segment gets
    /// "/." before the path, so it doesn't read back as a host. Called as
    /// the path ends, before a query is written.
    fn end_path(&mut self) {
        if self.host != HostKind::None || self.opaque_path {
            return;
        }
        let have = self.path_start == self.scheme_end + 3;
        if !have && self.s.as_bytes()[self.path_start..].starts_with(b"//") {
            self.s.insert_str(self.path_start, "/.");
            self.path_start += 2;
        }
    }

    /// The query and fragment, from `p`, which is at `?`, `#` or the end.
    fn rest(&mut self, input: &str, mut p: usize) -> Result<(), ParseError> {
        self.end_path();
        let bytes = input.as_bytes();
        if bytes.get(p) == Some(&b'?') {
            let end = bytes[p..]
                .iter()
                .position(|&b| b == b'#')
                .map_or(bytes.len(), |n| p + n);
            self.query_start = Some(self.s.len());
            self.s.push('?');
            let set = if self.special() { SPECIAL_QUERY } else { QUERY };
            encode_into(&mut self.s, &input[p + 1..end], set);
            p = end;
        }
        if bytes.get(p) == Some(&b'#') {
            self.fragment_start = Some(self.s.len());
            self.s.push('#');
            encode_into(&mut self.s, &input[p + 1..], FRAGMENT);
        }
        Ok(())
    }

    fn opaque_path(&mut self, input: &str, p: usize) -> Result<(), ParseError> {
        self.no_host();
        self.opaque_path = true;
        let bytes = input.as_bytes();
        let end = bytes[p..]
            .iter()
            .position(|&b| b == b'?' || b == b'#')
            .map_or(bytes.len(), |n| p + n);
        encode_into(&mut self.s, &input[p..end], C0);
        // A space just before the query or fragment is "%20", so it can't
        // be lost off the end if they're removed.
        if end < bytes.len() && self.s.len() > self.path_start && self.s.ends_with(' ') {
            self.s.pop();
            self.s.push_str("%20");
        }
        self.rest(input, end)
    }

    /// Copies the base's credentials, host and port.
    fn copy_authority(&mut self, base: &Url) {
        if base.host == HostKind::None {
            self.no_host();
            return;
        }
        let from = base.scheme_end as usize + 1;
        let to = base.path_start as usize;
        self.s.push_str(&base.s[from..to]);
        self.username_end = base.username_end as usize;
        self.host_start = base.host_start as usize;
        self.host_end = base.host_end as usize;
        self.port = base.port;
        self.host = base.host;
        self.path_start = self.s.len();
    }

    fn copy_path(&mut self, base: &Url) {
        self.s.push_str(base.pathname());
    }

    fn copy_query(&mut self, base: &Url) {
        if let Some(q) = base.query_start {
            self.query_start = Some(self.s.len());
            let end = base.fragment_start.map_or(base.s.len(), |f| f as usize);
            self.s.push_str(&base.s[q as usize..end]);
        }
    }

    /// The relative state, at `p`, with the scheme already written.
    fn relative(&mut self, input: &str, p: usize, base: &Url) -> Result<(), ParseError> {
        let bytes = input.as_bytes();
        let special = self.special();
        match bytes.get(p).copied() {
            Some(c) if is_slash(c, special) => {
                // The relative slash state.
                match bytes.get(p + 1).copied() {
                    Some(c) if special && is_slash(c, true) => {
                        self.authority(input, skip_slashes(bytes, p + 2))
                    }
                    Some(b'/') => self.authority(input, p + 2),
                    _ => {
                        self.copy_authority(base);
                        self.path_then_rest(input, p + 1)
                    }
                }
            }
            next => {
                self.copy_authority(base);
                self.copy_path(base);
                match next {
                    Some(b'?') => self.rest(input, p),
                    Some(b'#') => {
                        self.end_path();
                        self.copy_query(base);
                        self.rest(input, p)
                    }
                    None => {
                        self.end_path();
                        self.copy_query(base);
                        Ok(())
                    }
                    Some(_) => {
                        shorten(&mut self.s, self.path_start, self.scheme);
                        self.path_then_rest(input, p)
                    }
                }
            }
        }
    }

    /// The file state, at `p`, with "file:" written. `base` is only given
    /// when it's a file URL too.
    fn file(&mut self, input: &str, p: usize, base: Option<&Url>) -> Result<(), ParseError> {
        let bytes = input.as_bytes();
        self.s.push_str("//");
        self.username_end = self.s.len();
        self.host_start = self.s.len();
        self.host_end = self.s.len();
        self.host = HostKind::Empty;
        let copy_host = |b: &mut Builder, base: &Url| {
            b.s.push_str(base.hostname());
            b.host_end = b.s.len();
            b.host = base.host;
        };
        match bytes.get(p).copied() {
            Some(b'/' | b'\\') => {
                if matches!(bytes.get(p + 1), Some(b'/' | b'\\')) {
                    return self.file_host(input, p + 2);
                }
                self.path_start = self.s.len();
                if let Some(base) = base {
                    copy_host(self, base);
                    self.path_start = self.s.len();
                    let first = base.pathname().get(1..3).filter(|d| {
                        let d = d.as_bytes();
                        d[0].is_ascii_alphabetic()
                            && d[1] == b':'
                            && matches!(base.pathname().as_bytes().get(3), None | Some(b'/'))
                    });
                    if !starts_with_drive_letter(&bytes[p + 1..])
                        && let Some(drive) = first
                    {
                        self.s.push('/');
                        self.s.push_str(drive);
                    }
                }
                self.path_then_rest(input, p + 1)
            }
            next => {
                let Some(base) = base else {
                    self.path_start = self.s.len();
                    return self.path_then_rest(input, p);
                };
                copy_host(self, base);
                self.path_start = self.s.len();
                match next {
                    Some(b'?') => {
                        self.copy_path(base);
                        self.rest(input, p)
                    }
                    Some(b'#') => {
                        self.copy_path(base);
                        self.copy_query(base);
                        self.rest(input, p)
                    }
                    None => {
                        self.copy_path(base);
                        self.copy_query(base);
                        Ok(())
                    }
                    Some(_) => {
                        if !starts_with_drive_letter(&bytes[p..]) {
                            self.copy_path(base);
                            shorten(&mut self.s, self.path_start, self.scheme);
                        }
                        self.path_then_rest(input, p)
                    }
                }
            }
        }
    }

    /// The file host state, at `p`, after "file://".
    fn file_host(&mut self, input: &str, p: usize) -> Result<(), ParseError> {
        let bytes = input.as_bytes();
        let end = bytes[p..]
            .iter()
            .position(|&b| matches!(b, b'/' | b'\\' | b'?' | b'#'))
            .map_or(bytes.len(), |n| p + n);
        let buffer = &input[p..end];
        if is_drive_letter(buffer.as_bytes()) {
            // The drive letter starts the path, and the host stays empty.
            self.path_start = self.s.len();
            return self.path_then_rest(input, p);
        }
        if !buffer.is_empty() {
            self.host = host::parse_into(&mut self.s, buffer, true)?;
            if &self.s[self.host_start..] == "localhost" {
                self.s.truncate(self.host_start);
                self.host = HostKind::Empty;
            }
        }
        self.host_end = self.s.len();
        self.path_start_state(input, end)
    }
}

/// Removes ASCII tabs and newlines, which the parser ignores anywhere.
pub(crate) fn strip_tabs(input: &str) -> Cow<'_, str> {
    if input.bytes().any(|b| matches!(b, b'\t' | b'\n' | b'\r')) {
        Cow::Owned(input.replace(['\t', '\n', '\r'], ""))
    } else {
        Cow::Borrowed(input)
    }
}

/// The basic URL parser, without a URL or state override.
pub(crate) fn parse(input: &str, base: Option<&Url>) -> Result<Url, ParseError> {
    let input = strip_tabs(input.trim_matches(|c: char| c <= ' '));
    let input: &str = &input;
    let bytes = input.as_bytes();
    let mut b = Builder {
        s: String::with_capacity(input.len() + base.map_or(0, |b| b.s.len()) + 1),
        scheme_end: 0,
        username_end: 0,
        host_start: 0,
        host_end: 0,
        path_start: 0,
        query_start: None,
        fragment_start: None,
        port: None,
        host: HostKind::None,
        scheme: Scheme::Other,
        opaque_path: false,
    };
    match scheme_len(bytes) {
        Some(len) => {
            let scheme = &input[..len];
            if scheme.bytes().any(|c| c.is_ascii_uppercase()) {
                b.s.extend(scheme.bytes().map(|c| c.to_ascii_lowercase() as char));
            } else {
                b.s.push_str(scheme);
            }
            b.scheme = Scheme::of(&b.s);
            b.scheme_end = len;
            b.s.push(':');
            let p = len + 1;
            if b.scheme == Scheme::File {
                b.file(input, p, base.filter(|u| u.scheme == Scheme::File))?;
            } else if b.special() {
                match base.filter(|u| u.scheme == b.scheme) {
                    Some(base) if !bytes[p..].starts_with(b"//") => b.relative(input, p, base)?,
                    _ => b.authority(input, skip_slashes(bytes, p))?,
                }
            } else if bytes.get(p) == Some(&b'/') {
                if bytes.get(p + 1) == Some(&b'/') {
                    b.authority(input, p + 2)?;
                } else {
                    b.no_host();
                    b.path_then_rest(input, p + 1)?;
                }
            } else {
                b.opaque_path(input, p)?;
            }
        }
        None => {
            let base = base.ok_or(ParseError::RelativeUrlWithoutBase)?;
            if base.opaque_path {
                if bytes.first() != Some(&b'#') {
                    return Err(ParseError::RelativeUrlWithCannotBeABaseBase);
                }
                let end = base.fragment_start.map_or(base.s.len(), |f| f as usize);
                b.s.push_str(&base.s[..end]);
                b.scheme_end = base.scheme_end as usize;
                b.scheme = base.scheme;
                b.username_end = base.username_end as usize;
                b.host_start = base.host_start as usize;
                b.host_end = base.host_end as usize;
                b.path_start = base.path_start as usize;
                b.query_start = base.query_start.map(|q| q as usize);
                b.opaque_path = true;
                b.rest(input, 0)?;
            } else {
                b.s.push_str(base.scheme_str());
                b.s.push(':');
                b.scheme_end = base.scheme_end as usize;
                b.scheme = base.scheme;
                if base.scheme == Scheme::File {
                    b.file(input, 0, Some(base))?;
                } else {
                    b.relative(input, 0, base)?;
                }
            }
        }
    }
    b.finish()
}

/// A URL taken apart, for the setters.
pub(crate) struct Parts<'a> {
    pub(crate) scheme: Cow<'a, str>,
    pub(crate) username: Cow<'a, str>,
    pub(crate) password: Cow<'a, str>,
    pub(crate) host: Option<(Cow<'a, str>, HostKind)>,
    pub(crate) port: Option<u16>,
    pub(crate) path: Cow<'a, str>,
    pub(crate) opaque_path: bool,
    pub(crate) query: Option<Cow<'a, str>>,
    pub(crate) fragment: Option<Cow<'a, str>>,
}

impl Url {
    pub(crate) fn scheme_str(&self) -> &str {
        &self.s[..self.scheme_end as usize]
    }

    pub(crate) fn query(&self) -> Option<&str> {
        let q = self.query_start? as usize;
        let end = self.fragment_start.map_or(self.s.len(), |f| f as usize);
        Some(&self.s[q + 1..end])
    }

    pub(crate) fn fragment(&self) -> Option<&str> {
        Some(&self.s[self.fragment_start? as usize + 1..])
    }

    pub(crate) fn parts(&self) -> Parts<'_> {
        Parts {
            scheme: Cow::Borrowed(self.scheme_str()),
            username: Cow::Borrowed(self.username()),
            password: Cow::Borrowed(self.password()),
            host: (self.host != HostKind::None)
                .then(|| (Cow::Borrowed(self.hostname()), self.host)),
            port: self.port,
            path: Cow::Borrowed(self.pathname()),
            opaque_path: self.opaque_path,
            query: self.query().map(Cow::Borrowed),
            fragment: self.fragment().map(Cow::Borrowed),
        }
    }

    /// Serializes `parts`, which the setters keep valid, as a URL. The
    /// result is no longer than a parsed URL can be, unless a setter made
    /// it past 4 GiB, which leaves the URL as it was.
    pub(crate) fn assemble(parts: &Parts<'_>) -> Option<Url> {
        let mut s = String::with_capacity(
            parts.scheme.len()
                + parts.username.len()
                + parts.password.len()
                + parts.host.as_ref().map_or(0, |h| h.0.len())
                + parts.path.len()
                + parts.query.as_ref().map_or(0, |q| q.len())
                + parts.fragment.as_ref().map_or(0, |f| f.len())
                + 16,
        );
        s.push_str(&parts.scheme);
        let scheme_end = s.len();
        s.push(':');
        let (username_end, host_start, host_end, kind);
        if let Some((host, k)) = &parts.host {
            s.push_str("//");
            s.push_str(&parts.username);
            username_end = s.len();
            if !parts.password.is_empty() {
                s.push(':');
                s.push_str(&parts.password);
            }
            if s.len() > scheme_end + 3 {
                s.push('@');
            }
            host_start = s.len();
            s.push_str(host);
            host_end = s.len();
            if let Some(port) = parts.port {
                s.push(':');
                push_port(&mut s, port);
            }
            kind = *k;
        } else {
            username_end = s.len();
            host_start = s.len();
            host_end = s.len();
            kind = HostKind::None;
            if !parts.opaque_path && parts.path.starts_with("//") {
                s.push_str("/.");
            }
        }
        let path_start = s.len();
        s.push_str(&parts.path);
        let query_start = parts.query.as_ref().map(|q| {
            let at = s.len();
            s.push('?');
            s.push_str(q);
            at
        });
        let fragment_start = parts.fragment.as_ref().map(|f| {
            let at = s.len();
            s.push('#');
            s.push_str(f);
            at
        });
        Builder {
            scheme: Scheme::of(&s[..scheme_end]),
            s,
            scheme_end,
            username_end,
            host_start,
            host_end,
            path_start,
            query_start,
            fragment_start,
            port: if kind == HostKind::None {
                None
            } else {
                parts.port
            },
            host: kind,
            opaque_path: parts.opaque_path,
        }
        .finish()
        .ok()
    }

    /// Whether the URL cannot have a username, password or port.
    pub(crate) fn cannot_have_credentials(&self) -> bool {
        matches!(self.host, HostKind::None | HostKind::Empty) || self.scheme == Scheme::File
    }

    pub(crate) fn includes_credentials(&self) -> bool {
        !self.username().is_empty() || !self.password().is_empty()
    }

    /// The protocol setter: the scheme state with a state override.
    pub(crate) fn set_scheme(&mut self, value: &str) {
        let value = strip_tabs(value);
        let bytes = value.as_bytes();
        if !bytes.first().is_some_and(u8::is_ascii_alphabetic) {
            return;
        }
        let len = bytes
            .iter()
            .position(|&b| !is_scheme_byte(b))
            .unwrap_or(bytes.len());
        // The setter appends a ":", so the scheme may also end the value.
        if len < bytes.len() && bytes[len] != b':' {
            return;
        }
        let scheme = value[..len].to_ascii_lowercase();
        let new = Scheme::of(&scheme);
        if self.scheme.is_special() != new.is_special()
            || (new == Scheme::File && (self.includes_credentials() || self.port.is_some()))
            || (self.scheme == Scheme::File && self.host == HostKind::Empty)
        {
            return;
        }
        let mut parts = self.parts();
        parts.scheme = Cow::Owned(scheme);
        if parts.port.is_some() && parts.port == new.default_port() {
            parts.port = None;
        }
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    /// The username and password setters; `field` picks which part the
    /// encoded value goes to.
    pub(crate) fn set_userinfo(
        &mut self,
        value: &str,
        field: for<'a, 'b> fn(&'a mut Parts<'b>) -> &'a mut Cow<'b, str>,
    ) {
        if self.cannot_have_credentials() {
            return;
        }
        let mut parts = self.parts();
        *field(&mut parts) = Cow::Owned(encode(value, USERINFO));
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    /// The host and hostname setters: the host state with a state override.
    pub(crate) fn set_host_with(&mut self, value: &str, hostname_only: bool) {
        if self.opaque_path {
            return;
        }
        let value = strip_tabs(value);
        let bytes = value.as_bytes();
        let special = self.scheme.is_special();
        if self.scheme == Scheme::File {
            let end = bytes
                .iter()
                .position(|&b| matches!(b, b'/' | b'\\' | b'?' | b'#'))
                .unwrap_or(bytes.len());
            let mut host = String::new();
            let mut kind = HostKind::Empty;
            if end > 0 {
                match host::parse_into(&mut host, &value[..end], true) {
                    Ok(k) => kind = k,
                    Err(_) => return,
                }
                if host == "localhost" {
                    host.clear();
                    kind = HostKind::Empty;
                }
            }
            let mut parts = self.parts();
            parts.host = Some((Cow::Owned(host), kind));
            if let Some(url) = Url::assemble(&parts) {
                *self = url;
            }
            return;
        }
        let mut inside = false;
        let mut end = bytes.len();
        let mut colon = false;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'[' => inside = true,
                b']' => inside = false,
                b':' if !inside => {
                    end = i;
                    colon = true;
                    break;
                }
                b'/' | b'?' | b'#' => {
                    end = i;
                    break;
                }
                b'\\' if special => {
                    end = i;
                    break;
                }
                _ => {}
            }
        }
        let buffer = &value[..end];
        if colon && (buffer.is_empty() || hostname_only) {
            return;
        }
        if !colon
            && buffer.is_empty()
            && (special || self.includes_credentials() || self.port.is_some())
        {
            return;
        }
        let mut host = String::new();
        let Ok(kind) = host::parse_into(&mut host, buffer, special) else {
            return;
        };
        let mut parts = self.parts();
        parts.host = Some((Cow::Owned(host), kind));
        if colon {
            // The port state with an override: digits up to the first
            // other byte. If there are none, or too many, the host stays
            // set and the port as it was.
            let digits = &bytes[end + 1..];
            let n = digits
                .iter()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(digits.len());
            if n > 0
                && let Ok(port) = parse_port(&digits[..n], self.scheme)
            {
                parts.port = port;
            }
        }
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    pub(crate) fn set_port_value(&mut self, value: &str) {
        if self.cannot_have_credentials() {
            return;
        }
        let mut parts = self.parts();
        if value.is_empty() {
            parts.port = None;
        } else {
            let value = strip_tabs(value);
            let bytes = value.as_bytes();
            let n = bytes
                .iter()
                .position(|b| !b.is_ascii_digit())
                .unwrap_or(bytes.len());
            if n == 0 {
                return;
            }
            match parse_port(&bytes[..n], self.scheme) {
                Ok(port) => parts.port = port,
                Err(_) => return,
            }
        }
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    pub(crate) fn set_path(&mut self, value: &str) {
        if self.opaque_path {
            return;
        }
        let value = strip_tabs(value);
        let bytes = value.as_bytes();
        let mut path = String::with_capacity(value.len() + 1);
        // The path start state with an override.
        if self.scheme.is_special() {
            let p = usize::from(matches!(bytes.first(), Some(b'/' | b'\\')));
            parse_path(&mut path, 0, &value, p, self.scheme, false);
        } else if !bytes.is_empty() {
            let p = usize::from(bytes[0] == b'/');
            parse_path(&mut path, 0, &value, p, self.scheme, false);
        } else if self.host == HostKind::None {
            path.push('/');
        }
        let mut parts = self.parts();
        parts.path = Cow::Owned(path);
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    pub(crate) fn set_query_value(&mut self, value: &str) {
        let mut parts = self.parts();
        if value.is_empty() {
            parts.query = None;
        } else {
            let value = value.strip_prefix('?').unwrap_or(value);
            let set = if self.scheme.is_special() {
                SPECIAL_QUERY
            } else {
                QUERY
            };
            parts.query = Some(Cow::Owned(encode(&strip_tabs(value), set)));
        }
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    pub(crate) fn set_fragment_value(&mut self, value: &str) {
        let mut parts = self.parts();
        if value.is_empty() {
            parts.fragment = None;
        } else {
            let value = value.strip_prefix('#').unwrap_or(value);
            parts.fragment = Some(Cow::Owned(encode(&strip_tabs(value), FRAGMENT)));
        }
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }

    /// Replaces the query with an already-serialized one, as
    /// `URLSearchParams` does.
    pub(crate) fn set_raw_query(&mut self, query: Option<&str>) {
        let mut parts = self.parts();
        parts.query = query.map(Cow::Borrowed);
        if let Some(url) = Url::assemble(&parts) {
            *self = url;
        }
    }
}
