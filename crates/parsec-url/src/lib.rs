// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The WHATWG URL Standard: URLs with every getter and setter the `URL`
//! interface has, `URLSearchParams`, and the host, IDNA and Punycode
//! algorithms under them.
//!
//! It's written from the standard's current text, IDNA (UTS #46) and NFC
//! included, with the Unicode 18.0 tables in `src/tables.rs`. The API has
//! the shape the standard's JavaScript one has, so `url.pathname()` here is
//! `url.pathname` there. `tests/wpt.rs` runs the web-platform-tests URL
//! suite against it, every case of which passes.
//!
//! ```
//! use parsec_url::Url;
//!
//! let base = Url::parse("https://example.com/a/b?x=1").unwrap();
//! let mut url = Url::parse_with_base("../c?q=two words#top", &base).unwrap();
//! assert_eq!(url.href(), "https://example.com/c?q=two%20words#top");
//!
//! let mut params = url.search_params();
//! params.append("page", "2");
//! url.set_search_params(&params);
//! assert_eq!(url.search(), "?q=two+words&page=2");
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

mod form;
mod host;
mod idna;
pub mod normalize;
mod parser;
mod percent;
pub mod punycode;
mod tables;

pub use host::Host;
use host::HostKind;
use parser::Scheme;

/// Why a URL didn't parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ParseError {
    EmptyHost,
    IdnaError,
    InvalidPort,
    InvalidIpv4Address,
    InvalidIpv6Address,
    InvalidDomainCharacter,
    RelativeUrlWithoutBase,
    RelativeUrlWithCannotBeABaseBase,
    /// The URL would be 4 GiB or more.
    Overflow,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ParseError::EmptyHost => "empty host",
            ParseError::IdnaError => "invalid international domain name",
            ParseError::InvalidPort => "invalid port number",
            ParseError::InvalidIpv4Address => "invalid IPv4 address",
            ParseError::InvalidIpv6Address => "invalid IPv6 address",
            ParseError::InvalidDomainCharacter => "invalid domain character",
            ParseError::RelativeUrlWithoutBase => "relative URL without a base",
            ParseError::RelativeUrlWithCannotBeABaseBase => {
                "relative URL with a cannot-be-a-base base"
            }
            ParseError::Overflow => "URLs of 4 GiB or more are not supported",
        })
    }
}

impl std::error::Error for ParseError {}

/// A parsed URL. Getters and setters are named after the `URL` interface's
/// attributes and behave as they do: a setter given a value the spec
/// rejects leaves the URL as it was.
///
/// It's the serialization, in one string, and where each component of it
/// starts. Two URLs are equal, and ordered, by their serializations.
#[derive(Clone, Debug)]
pub struct Url {
    s: String,
    /// The `:` after the scheme.
    scheme_end: u32,
    /// Where the username ends; with no host, all of these up to
    /// `host_end` are `scheme_end + 1`.
    username_end: u32,
    host_start: u32,
    host_end: u32,
    /// After the host and port; with no host, after "scheme:" and the
    /// "/." that can follow it.
    path_start: u32,
    /// The `?` and the `#`, if there are a query and a fragment.
    query_start: Option<u32>,
    fragment_start: Option<u32>,
    port: Option<u16>,
    host: HostKind,
    scheme: Scheme,
    opaque_path: bool,
}

impl Url {
    /// Parses an absolute URL, as `new URL(input)` does.
    pub fn parse(input: &str) -> Result<Url, ParseError> {
        parser::parse(input, None)
    }

    /// Parses `input` against `base`, as `new URL(input, base)` does.
    pub fn parse_with_base(input: &str, base: &Url) -> Result<Url, ParseError> {
        parser::parse(input, Some(base))
    }

    /// Whether `input` parses, against `base` if there is one: `URL.canParse`.
    pub fn can_parse(input: &str, base: Option<&Url>) -> bool {
        parser::parse(input, base).is_ok()
    }

    /// The whole serialized URL.
    pub fn href(&self) -> &str {
        &self.s
    }

    /// Replaces the whole URL; unlike the other setters, this one fails
    /// when the new value doesn't parse, as the `href` setter throws.
    pub fn set_href(&mut self, value: &str) -> Result<(), ParseError> {
        *self = Url::parse(value)?;
        Ok(())
    }

    /// The serialized origin, `"null"` for an opaque one. A `blob:` URL's
    /// is its inner URL's, when that's http, https or file.
    pub fn origin(&self) -> String {
        match self.scheme {
            Scheme::Http | Scheme::Https | Scheme::Ws | Scheme::Wss | Scheme::Ftp => {
                format!("{}//{}", self.protocol(), self.host())
            }
            Scheme::Other if self.scheme_str() == "blob" => match Url::parse(self.pathname()) {
                Ok(inner) if matches!(inner.scheme, Scheme::Http | Scheme::Https) => inner.origin(),
                _ => "null".into(),
            },
            _ => "null".into(),
        }
    }

    /// The scheme with its colon, like `"https:"`.
    pub fn protocol(&self) -> &str {
        &self.s[..=self.scheme_end as usize]
    }

    pub fn set_protocol(&mut self, value: &str) {
        self.set_scheme(value);
    }

    pub fn username(&self) -> &str {
        if self.host == HostKind::None {
            return "";
        }
        &self.s[self.scheme_end as usize + 3..self.username_end as usize]
    }

    pub fn set_username(&mut self, value: &str) {
        self.set_userinfo(value, |p| &mut p.username);
    }

    pub fn password(&self) -> &str {
        let (end, start) = (self.username_end as usize, self.host_start as usize);
        if end < start && self.s.as_bytes()[end] == b':' {
            &self.s[end + 1..start - 1]
        } else {
            ""
        }
    }

    pub fn set_password(&mut self, value: &str) {
        self.set_userinfo(value, |p| &mut p.password);
    }

    /// The host and, if there is one, the port.
    pub fn host(&self) -> &str {
        if self.host == HostKind::None {
            return "";
        }
        &self.s[self.host_start as usize..self.path_start as usize]
    }

    pub fn set_host(&mut self, value: &str) {
        self.set_host_with(value, false);
    }

    /// The host without the port.
    pub fn hostname(&self) -> &str {
        &self.s[self.host_start as usize..self.host_end as usize]
    }

    pub fn set_hostname(&mut self, value: &str) {
        self.set_host_with(value, true);
    }

    /// The port, empty when it's absent or the scheme's default.
    pub fn port(&self) -> &str {
        match self.port {
            Some(_) => &self.s[self.host_end as usize + 1..self.path_start as usize],
            None => "",
        }
    }

    pub fn set_port(&mut self, value: &str) {
        self.set_port_value(value);
    }

    pub fn pathname(&self) -> &str {
        let end = self
            .query_start
            .or(self.fragment_start)
            .map_or(self.s.len(), |e| e as usize);
        &self.s[self.path_start as usize..end]
    }

    pub fn set_pathname(&mut self, value: &str) {
        self.set_path(value);
    }

    /// The query with its `?`, or empty when it's absent or empty.
    pub fn search(&self) -> &str {
        match self.query() {
            Some(q) if !q.is_empty() => {
                let start = self.query_start.unwrap_or(0) as usize;
                &self.s[start..start + 1 + q.len()]
            }
            _ => "",
        }
    }

    pub fn set_search(&mut self, value: &str) {
        self.set_query_value(value);
    }

    /// The fragment with its `#`, or empty when it's absent or empty.
    pub fn hash(&self) -> &str {
        match self.fragment_start {
            Some(f) if (f as usize) + 1 < self.s.len() => &self.s[f as usize..],
            _ => "",
        }
    }

    pub fn set_hash(&mut self, value: &str) {
        self.set_fragment_value(value);
    }

    /// The query parsed as `application/x-www-form-urlencoded`, the list
    /// `url.searchParams` starts from.
    pub fn search_params(&self) -> SearchParams {
        // Not `SearchParams::parse`, which drops a leading `?`: the query of
        // `??a` is `?a`, and its first name is `?a`.
        SearchParams {
            list: form::parse(self.query().unwrap_or("")),
        }
    }

    /// Writes `params` back as the query, as changing `url.searchParams`
    /// does: an empty list removes the query, `?` and all.
    pub fn set_search_params(&mut self, params: &SearchParams) {
        if params.is_empty() {
            self.set_raw_query(None);
        } else {
            self.set_raw_query(Some(&params.to_string()));
        }
    }
}

impl PartialEq for Url {
    fn eq(&self, other: &Url) -> bool {
        self.s == other.s
    }
}

impl Eq for Url {}

impl PartialOrd for Url {
    fn partial_cmp(&self, other: &Url) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Url {
    fn cmp(&self, other: &Url) -> Ordering {
        self.s.cmp(&other.s)
    }
}

impl Hash for Url {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.s.hash(state);
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.href())
    }
}

impl FromStr for Url {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Url, ParseError> {
        Url::parse(s)
    }
}

impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
        self.href()
    }
}

/// `URLSearchParams`: an ordered list of name-value pairs, read from and
/// written as `application/x-www-form-urlencoded`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchParams {
    list: Vec<(String, String)>,
}

impl SearchParams {
    pub fn new() -> SearchParams {
        SearchParams::default()
    }

    /// Parses a query string, `new URLSearchParams(init)`; one leading `?`
    /// is dropped. Bytes that aren't UTF-8 once percent-decoded become
    /// U+FFFD, as the spec says.
    pub fn parse(init: &str) -> SearchParams {
        SearchParams {
            list: form::parse(init.strip_prefix('?').unwrap_or(init)),
        }
    }

    /// How many pairs there are: `size`.
    pub fn len(&self) -> usize {
        self.list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn append(&mut self, name: &str, value: &str) {
        self.list.push((name.to_owned(), value.to_owned()));
    }

    /// Removes every pair named `name`, or only those whose value is also
    /// `value` when one is given.
    pub fn delete(&mut self, name: &str, value: Option<&str>) {
        self.list
            .retain(|(n, v)| n != name || value.is_some_and(|value| v != value));
    }

    /// The first value named `name`.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.list
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Every value named `name`, in order.
    pub fn get_all(&self, name: &str) -> Vec<&str> {
        self.list
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// Whether a pair is named `name`, and has `value` if one is given.
    pub fn has(&self, name: &str, value: Option<&str>) -> bool {
        self.list
            .iter()
            .any(|(n, v)| n == name && value.is_none_or(|value| v == value))
    }

    /// Gives the first pair named `name` this value and removes the rest,
    /// or appends the pair if there's none.
    pub fn set(&mut self, name: &str, value: &str) {
        let mut found = false;
        self.list.retain_mut(|(n, v)| {
            if n != name {
                return true;
            }
            if found {
                return false;
            }
            found = true;
            value.clone_into(v);
            true
        });
        if !found {
            self.append(name, value);
        }
    }

    /// Sorts by name, keeping pairs with the same name in their order. The
    /// spec compares UTF-16 code units, which puts U+1F308 before U+FB03.
    pub fn sort(&mut self) {
        self.list
            .sort_by(|(a, _), (b, _)| a.encode_utf16().cmp(b.encode_utf16()));
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.list.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }
}

/// The `application/x-www-form-urlencoded` serialization: `toString()`.
impl fmt::Display for SearchParams {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&form::serialize(self.iter()))
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for SearchParams {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(pairs: I) -> SearchParams {
        SearchParams {
            list: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }
}

/// A domain the URL Standard's domain to ASCII rejects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidDomain;

impl fmt::Display for InvalidDomain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid domain")
    }
}

impl std::error::Error for InvalidDomain {}

/// The URL Standard's [domain parser], which the host parser runs:
/// `"Bücher.example"` becomes `"xn--bcher-kva.example"`. An ASCII domain
/// is only lowercased; any other goes through UTS #46 ToASCII. Either way
/// the result can't be empty or hold a forbidden domain code point.
///
/// [domain parser]: https://url.spec.whatwg.org/#concept-domain-to-ascii
pub fn domain_to_ascii(domain: &str) -> Result<String, InvalidDomain> {
    let ascii = if domain.is_ascii() {
        domain.to_ascii_lowercase()
    } else {
        idna::to_ascii(domain).ok_or(InvalidDomain)?
    };
    if ascii.is_empty() || ascii.bytes().any(host::forbidden_domain) {
        return Err(InvalidDomain);
    }
    Ok(ascii)
}

/// The URL Standard's [domain to Unicode], for showing a host to a person:
/// `"xn--bcher-kva.example"` becomes `"bücher.example"`. A domain that
/// UTS #46 ToUnicode records an error for comes back as it was.
///
/// [domain to Unicode]: https://url.spec.whatwg.org/#concept-domain-to-unicode
pub fn domain_to_unicode(domain: &str) -> String {
    match idna::to_unicode(domain) {
        (unicode, false) => unicode,
        (_, true) => domain.to_owned(),
    }
}
