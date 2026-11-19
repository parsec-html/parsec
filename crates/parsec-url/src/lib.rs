// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The WHATWG URL Standard: URLs with every getter and setter the `URL`
//! interface has, `URLSearchParams`, and the host, IDNA and Punycode
//! algorithms under them.
//!
//! The parsing is Servo's `url` and `idna` crates, which follow the spec;
//! this crate gives them the shape the standard's JavaScript API has, so
//! `url.pathname()` here is `url.pathname` there. `tests/wpt.rs` runs the
//! web-platform-tests URL suite against it.
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

use std::fmt;
use std::str::FromStr;

pub use url::{Host, ParseError};

/// A parsed URL. Getters and setters are named after the `URL` interface's
/// attributes and behave as they do: a setter given a value the spec
/// rejects leaves the URL as it was.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Url(url::Url);

impl Url {
    /// Parses an absolute URL, as `new URL(input)` does.
    pub fn parse(input: &str) -> Result<Url, ParseError> {
        url::Url::parse(input).map(Url)
    }

    /// Parses `input` against `base`, as `new URL(input, base)` does.
    pub fn parse_with_base(input: &str, base: &Url) -> Result<Url, ParseError> {
        url::Url::options()
            .base_url(Some(&base.0))
            .parse(input)
            .map(Url)
    }

    /// Whether `input` parses, against `base` if there is one: `URL.canParse`.
    pub fn can_parse(input: &str, base: Option<&Url>) -> bool {
        url::Url::options()
            .base_url(base.map(|b| &b.0))
            .parse(input)
            .is_ok()
    }

    /// The whole serialized URL.
    pub fn href(&self) -> &str {
        self.0.as_str()
    }

    /// Replaces the whole URL; unlike the other setters, this one fails
    /// when the new value doesn't parse, as the `href` setter throws.
    pub fn set_href(&mut self, value: &str) -> Result<(), ParseError> {
        url::quirks::set_href(&mut self.0, value)
    }

    /// The serialized origin, `"null"` for an opaque one.
    pub fn origin(&self) -> String {
        url::quirks::origin(&self.0)
    }

    /// The scheme with its colon, like `"https:"`.
    pub fn protocol(&self) -> &str {
        url::quirks::protocol(&self.0)
    }

    pub fn set_protocol(&mut self, value: &str) {
        let _ = url::quirks::set_protocol(&mut self.0, value);
    }

    pub fn username(&self) -> &str {
        url::quirks::username(&self.0)
    }

    pub fn set_username(&mut self, value: &str) {
        let _ = url::quirks::set_username(&mut self.0, value);
    }

    pub fn password(&self) -> &str {
        url::quirks::password(&self.0)
    }

    pub fn set_password(&mut self, value: &str) {
        let _ = url::quirks::set_password(&mut self.0, value);
    }

    /// The host and, if there is one, the port.
    pub fn host(&self) -> &str {
        url::quirks::host(&self.0)
    }

    pub fn set_host(&mut self, value: &str) {
        let _ = url::quirks::set_host(&mut self.0, value);
    }

    /// The host without the port.
    pub fn hostname(&self) -> &str {
        url::quirks::hostname(&self.0)
    }

    pub fn set_hostname(&mut self, value: &str) {
        let _ = url::quirks::set_hostname(&mut self.0, value);
    }

    /// The port, empty when it's absent or the scheme's default.
    pub fn port(&self) -> &str {
        url::quirks::port(&self.0)
    }

    pub fn set_port(&mut self, value: &str) {
        // The spec removes the port only for "" itself. The url crate
        // strips tabs and newlines first, so "\n\t" would remove it too;
        // per the spec that parses to no port digits and changes nothing.
        if !value.is_empty() && value.bytes().all(|b| matches!(b, b'\t' | b'\n' | b'\r')) {
            return;
        }
        let _ = url::quirks::set_port(&mut self.0, value);
    }

    pub fn pathname(&self) -> &str {
        url::quirks::pathname(&self.0)
    }

    pub fn set_pathname(&mut self, value: &str) {
        url::quirks::set_pathname(&mut self.0, value)
    }

    /// The query with its `?`, or empty when it's absent or empty.
    pub fn search(&self) -> &str {
        url::quirks::search(&self.0)
    }

    pub fn set_search(&mut self, value: &str) {
        url::quirks::set_search(&mut self.0, value)
    }

    /// The fragment with its `#`, or empty when it's absent or empty.
    pub fn hash(&self) -> &str {
        url::quirks::hash(&self.0)
    }

    pub fn set_hash(&mut self, value: &str) {
        url::quirks::set_hash(&mut self.0, value)
    }

    /// The query parsed as `application/x-www-form-urlencoded`, the list
    /// `url.searchParams` starts from.
    pub fn search_params(&self) -> SearchParams {
        // Not `SearchParams::parse`, which drops a leading `?`: the query of
        // `??a` is `?a`, and its first name is `?a`.
        let query = self.0.query().unwrap_or("");
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect()
    }

    /// Writes `params` back as the query, as changing `url.searchParams`
    /// does: an empty list removes the query, `?` and all.
    pub fn set_search_params(&mut self, params: &SearchParams) {
        if params.is_empty() {
            self.0.set_query(None);
        } else {
            self.0.set_query(Some(&params.to_string()));
        }
    }

    /// The `url` crate's URL, for its wider Rust API.
    pub fn as_url(&self) -> &url::Url {
        &self.0
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

impl From<url::Url> for Url {
    fn from(url: url::Url) -> Url {
        Url(url)
    }
}

impl From<Url> for url::Url {
    fn from(url: Url) -> url::Url {
        url.0
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
        let init = init.strip_prefix('?').unwrap_or(init);
        url::form_urlencoded::parse(init.as_bytes())
            .into_owned()
            .collect()
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
        f.write_str(
            &url::form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&self.list)
                .finish(),
        )
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

/// The URL Standard's [domain to ASCII], with the forbidden domain code
/// point check the host parser adds: `"Bücher.example"` becomes
/// `"xn--bcher-kva.example"`.
///
/// [domain to ASCII]: https://url.spec.whatwg.org/#concept-domain-to-ascii
pub fn domain_to_ascii(domain: &str) -> Result<String, InvalidDomain> {
    idna::domain_to_ascii_cow(domain.as_bytes(), idna::AsciiDenyList::URL)
        .map(|d| d.into_owned())
        .map_err(|_| InvalidDomain)
}

/// The URL Standard's [domain to Unicode], for showing a host to a person.
/// It never fails; a label with an error comes back marked with U+FFFD,
/// so it shouldn't go back into a request.
///
/// [domain to Unicode]: https://url.spec.whatwg.org/#concept-domain-to-unicode
pub fn domain_to_unicode(domain: &str) -> String {
    idna::domain_to_unicode(domain).0
}

/// RFC 3492 Punycode, one label at a time and without the `xn--` prefix.
pub mod punycode {
    pub use idna::punycode::{decode_to_string, encode_str};
}
