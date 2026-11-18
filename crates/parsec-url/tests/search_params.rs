// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `URLSearchParams`, IDNA and Punycode. The parsing, sorting and
//! serializing cases are WPT's (`url/urlencoded-parser.any.js`,
//! `urlsearchparams-*.any.js`), which keep their data in JavaScript rather
//! than JSON, so they're copied here.

use parsec_url::{SearchParams, Url, domain_to_ascii, domain_to_unicode, punycode};

fn pairs(p: &SearchParams) -> Vec<(&str, &str)> {
    p.iter().collect()
}

#[test]
fn parses_as_urlencoded() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        ("test", &[("test", "")]),
        ("\u{FEFF}test=\u{FEFF}", &[("\u{FEFF}test", "\u{FEFF}")]),
        ("%EF%BB%BFtest=%EF%BB%BF", &[("\u{FEFF}test", "\u{FEFF}")]),
        ("%EF%BF%BF=%EF%BF%BF", &[("\u{FFFF}", "\u{FFFF}")]),
        ("%FE%FF", &[("\u{FFFD}\u{FFFD}", "")]),
        ("%FF%FE", &[("\u{FFFD}\u{FFFD}", "")]),
        ("†&†=x", &[("†", ""), ("†", "x")]),
        ("%C2", &[("\u{FFFD}", "")]),
        ("%C2x", &[("\u{FFFD}x", "")]),
        (
            "_charset_=windows-1252&test=%C2x",
            &[("_charset_", "windows-1252"), ("test", "\u{FFFD}x")],
        ),
        ("", &[]),
        ("a", &[("a", "")]),
        ("a=b", &[("a", "b")]),
        ("a=", &[("a", "")]),
        ("=b", &[("", "b")]),
        ("&", &[]),
        ("&a", &[("a", "")]),
        ("a&", &[("a", "")]),
        ("a&a", &[("a", ""), ("a", "")]),
        ("a&b&c", &[("a", ""), ("b", ""), ("c", "")]),
        ("a=b&c=d", &[("a", "b"), ("c", "d")]),
        ("a=b&c=d&", &[("a", "b"), ("c", "d")]),
        ("&&&a=b&&&&c=d&", &[("a", "b"), ("c", "d")]),
        ("a=a&a=b&a=c", &[("a", "a"), ("a", "b"), ("a", "c")]),
        ("a==a", &[("a", "=a")]),
        ("a=a+b+c+d", &[("a", "a b c d")]),
        ("%=a", &[("%", "a")]),
        ("%a=a", &[("%a", "a")]),
        ("%a_=a", &[("%a_", "a")]),
        ("%61=a", &[("a", "a")]),
        ("%61+%4d%4D=", &[("a MM", "")]),
        ("id=0&value=%", &[("id", "0"), ("value", "%")]),
        ("b=%2sf%2a", &[("b", "%2sf*")]),
        ("b=%2%2af%2a", &[("b", "%2*f*")]),
        ("b=%%2a", &[("b", "%*")]),
    ];
    for (input, want) in cases {
        assert_eq!(pairs(&SearchParams::parse(input)), *want, "{input:?}");
    }
    // The constructor drops one leading "?", not two.
    assert_eq!(pairs(&SearchParams::parse("??a=b")), [("?a", "b")]);
}

#[test]
fn sorts_by_utf16_code_units() {
    let cases: &[(&str, &[(&str, &str)])] = &[
        (
            "z=b&a=b&z=a&a=a",
            &[("a", "b"), ("a", "a"), ("z", "b"), ("z", "a")],
        ),
        (
            "\u{FFFD}=x&\u{FFFC}&\u{FFFD}=a",
            &[("\u{FFFC}", ""), ("\u{FFFD}", "x"), ("\u{FFFD}", "a")],
        ),
        ("ﬃ&🌈", &[("🌈", ""), ("ﬃ", "")]),
        (
            "é&e\u{FFFD}&e\u{0301}",
            &[("e\u{0301}", ""), ("e\u{FFFD}", ""), ("é", "")],
        ),
        (
            "bbb&bb&aaa&aa=x&aa=y",
            &[
                ("aa", "x"),
                ("aa", "y"),
                ("aaa", ""),
                ("bb", ""),
                ("bbb", ""),
            ],
        ),
        (
            "z=z&=f&=t&=x",
            &[("", "f"), ("", "t"), ("", "x"), ("z", "z")],
        ),
        ("a🌈&a💩", &[("a🌈", ""), ("a💩", "")]),
    ];
    for (input, want) in cases {
        let mut p = SearchParams::parse(input);
        p.sort();
        assert_eq!(pairs(&p), *want, "{input:?}");

        let mut url = Url::parse_with_base(
            &format!("?{input}"),
            &Url::parse("https://example/").unwrap(),
        )
        .unwrap();
        let mut p = url.search_params();
        p.sort();
        url.set_search_params(&p);
        assert_eq!(
            pairs(&SearchParams::parse(url.search())),
            *want,
            "{input:?}"
        );
    }

    // Sorting an empty query takes the "?" away.
    let mut url = Url::parse("http://example.com/?").unwrap();
    let mut p = url.search_params();
    p.sort();
    url.set_search_params(&p);
    assert_eq!(url.href(), "http://example.com/");
    assert_eq!(url.search(), "");
}

#[test]
fn serializes_as_urlencoded() {
    let cases: &[(&[(&str, &str)], &str)] = &[
        (&[("a", "b c")], "a=b+c"),
        (&[("a b", "c")], "a+b=c"),
        (&[("a", ""), ("a", ""), ("", "b"), ("", "")], "a=&a=&=b&="),
        (&[("a", "b+c")], "a=b%2Bc"),
        (&[("=", "a"), ("b", "=")], "%3D=a&b=%3D"),
        (&[("&", "a"), ("b", "&")], "%26=a&b=%26"),
        (&[("a", "*-._"), ("*-._", "c")], "a=*-._&*-._=c"),
        (&[("a", "b%c"), ("a%b", "c")], "a=b%25c&a%25b=c"),
        (&[("a", "b\0c")], "a=b%00c"),
        (&[("a", "b\u{1F4A9}c")], "a=b%F0%9F%92%A9c"),
    ];
    for (list, want) in cases {
        let p: SearchParams = list.iter().copied().collect();
        assert_eq!(p.to_string(), *want);
    }
    let roundtrips = [
        ("a=b&c=d&&e&&", "a=b&c=d&e="),
        ("a = b &a=b&c=d%20", "a+=+b+&a=b&c=d+"),
        ("a=&a=b", "a=&a=b"),
        ("b=%2sf%2a", "b=%252sf*"),
        ("id=0&value=%", "id=0&value=%25"),
    ];
    for (input, want) in roundtrips {
        assert_eq!(SearchParams::parse(input).to_string(), want);
    }
}

#[test]
fn get_set_delete_has() {
    let mut p = SearchParams::parse("a=1&b=2&a=3&c=4");
    assert_eq!(p.len(), 4);
    assert_eq!(p.get("a"), Some("1"));
    assert_eq!(p.get("z"), None);
    assert_eq!(p.get_all("a"), ["1", "3"]);
    assert!(p.has("a", None));
    assert!(p.has("a", Some("3")));
    assert!(!p.has("a", Some("2")));

    p.set("a", "x");
    assert_eq!(p.to_string(), "a=x&b=2&c=4");
    p.set("d", "5");
    assert_eq!(p.to_string(), "a=x&b=2&c=4&d=5");

    p.append("b", "6");
    p.delete("b", Some("2"));
    assert_eq!(p.to_string(), "a=x&c=4&d=5&b=6");
    p.delete("b", None);
    assert_eq!(p.to_string(), "a=x&c=4&d=5");
    assert!(!p.is_empty());
}

#[test]
fn url_getters_and_setters() {
    let mut url = Url::parse("https://user:pw@example.com:8080/a/b?x=1#frag").unwrap();
    assert_eq!(url.origin(), "https://example.com:8080");
    assert_eq!(url.protocol(), "https:");
    assert_eq!(url.username(), "user");
    assert_eq!(url.password(), "pw");
    assert_eq!(url.host(), "example.com:8080");
    assert_eq!(url.hostname(), "example.com");
    assert_eq!(url.port(), "8080");
    assert_eq!(url.pathname(), "/a/b");
    assert_eq!(url.search(), "?x=1");
    assert_eq!(url.hash(), "#frag");

    url.set_port("443");
    assert_eq!(url.port(), "");
    url.set_hostname("Bücher.example");
    assert_eq!(url.hostname(), "xn--bcher-kva.example");
    // A value the spec rejects leaves the URL alone.
    url.set_port("nope");
    url.set_protocol("1http");
    assert_eq!(
        url.href(),
        "https://user:pw@xn--bcher-kva.example/a/b?x=1#frag"
    );
    assert!(url.set_href("not a url").is_err());

    assert!(Url::can_parse("/x", Some(&url)));
    assert!(!Url::can_parse("/x", None));
    assert_eq!("http://a/".parse::<Url>().unwrap().to_string(), "http://a/");
}

#[test]
fn hosts_idna_and_punycode() {
    assert_eq!(
        domain_to_ascii("Bücher.example").unwrap(),
        "xn--bcher-kva.example"
    );
    assert_eq!(domain_to_ascii("EXAMPLE.com").unwrap(), "example.com");
    assert!(domain_to_ascii("a b.example").is_err());
    assert_eq!(domain_to_unicode("xn--bcher-kva.example"), "bücher.example");
    assert_eq!(punycode::encode_str("bücher").unwrap(), "bcher-kva");
    assert_eq!(punycode::decode_to_string("bcher-kva").unwrap(), "bücher");
    assert_eq!(
        parsec_url::Host::parse("[::1]").unwrap().to_string(),
        "[::1]"
    );
}
