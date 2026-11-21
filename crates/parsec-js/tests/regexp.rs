// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Regular expression patterns, rule by rule of ECMA-262 22.2.1 and its
//! early errors, in each mode: no flag (with Annex B's B.1.2), `u`, and
//! `v`.

use parsec_js::regexp::validate;

/// Each pattern is fine with `flags`.
fn ok(flags: &str, patterns: &[&str]) {
    for p in patterns {
        if let Err(d) = validate(p, flags) {
            panic!("/{p}/{flags} should be fine: {d}");
        }
    }
}

/// Each pattern fails with `flags`, with `code`.
fn bad(flags: &str, code: &str, patterns: &[&str]) {
    for p in patterns {
        match validate(p, flags) {
            Ok(()) => panic!("/{p}/{flags} should fail with {code}"),
            Err(d) => assert_eq!(d.code, code, "/{p}/{flags}: {d}"),
        }
    }
}

#[test]
fn flags() {
    ok("dgimsuy", &["a"]);
    ok("v", &["a"]);
    bad("", "regexp-flags", &[]);
    for flags in ["gg", "x", "uv", "G"] {
        assert_eq!(
            validate("a", flags).unwrap_err().code,
            "regexp-flags",
            "{flags}"
        );
    }
}

#[test]
fn atoms_and_alternatives() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "", "a", "abc", "a|b|", "|", ".", "^a$", "(a)", "(?:a)", "a(b(c))", "\\b\\B",
            ],
        );
    }
}

#[test]
fn quantifiers() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "a*",
                "a+",
                "a?",
                "a*?",
                "a{2}",
                "a{2,}",
                "a{2,3}",
                "a{2,3}?",
                "a{0010,11}",
                "(a)*",
            ],
        );
        bad(
            flags,
            "regexp-nothing-to-repeat",
            &[
                "*", "+a", "?", "a**", "a+*", "|*", "(*)", "^*", "$+", "\\b*", "a{1}{2}",
            ],
        );
        bad(
            flags,
            "regexp-quantifier-range",
            &[
                "a{2,1}",
                "a{10,9}",
                "a{100000000000000000001,100000000000000000000}",
            ],
        );
    }
    // B.1.2: a lookahead may be repeated without u, a lookbehind never.
    ok("", &["(?=a)*", "(?!a){2}"]);
    bad("u", "regexp-nothing-to-repeat", &["(?=a)*", "(?!a)+"]);
    bad("", "regexp-nothing-to-repeat", &["(?<=a)*", "(?<!a)?"]);
    // B.1.2: without u, a brace that isn't a quantifier is itself; one
    // that is, with nothing before it, is an error.
    ok("", &["{", "a{", "a{1", "a{,1}", "}", "]", "a{1,2", "x{a}"]);
    bad("", "regexp-nothing-to-repeat", &["{1}", "{1,}", "a|{2,3}"]);
}

#[test]
fn brackets_need_escaping_in_unicode_mode() {
    for flags in ["u", "v"] {
        bad(
            flags,
            "regexp-lone-bracket",
            &["{", "a{", "a{1", "}", "]", "a{,1}"],
        );
        ok(flags, &["\\{", "\\}", "\\]"]);
    }
    for flags in ["", "u", "v"] {
        bad(flags, "regexp-lone-bracket", &[")", "a)", "(a))"]);
    }
}

#[test]
fn groups() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "(?=a)",
                "(?!a)",
                "(?<=a)",
                "(?<!a)",
                "(?<name>a)",
                "(?<$_\\u0041b>a)",
            ],
        );
        bad(
            flags,
            "regexp-unterminated",
            &["(", "(a", "(?:a", "(?=a", "((a)"],
        );
        bad(
            flags,
            "regexp-group",
            &["(?", "(?a)", "(?=)(?x)", "(?#a)", "(?x:a)"],
        );
    }
}

#[test]
fn modifiers() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &["(?i:a)", "(?ims:a)", "(?-i:a)", "(?i-ms:a)", "(?m-:a)"],
        );
        bad(
            flags,
            "regexp-modifiers",
            &[
                "(?ii:a)",
                "(?i-i:a)",
                "(?-:a)",
                "(?i-x:a)",
                "(?i)",
                "(?-)",
                "(?i-m-s:a)",
            ],
        );
    }
}

#[test]
fn group_names() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "(?<a>.)\\k<a>",
                "\\k<a>(?<a>.)",
                "(?<\\u{1d4d1}>.)",
                "(?<\\ud835\\udcd1>.)",
                "(?<π>.)",
                "(?<a1>.)",
            ],
        );
        bad(
            flags,
            "regexp-group-name",
            &[
                "(?<1>.)",
                "(?<a-b>.)",
                "(?<a",
                "(?<\\x41>.)",
                "(?<>.)",
                "(?<)",
                "(?<a\\>.)",
            ],
        );
        bad(flags, "regexp-unknown-group-name", &["(?<a>.)\\k<b>"]);
    }
    bad("u", "regexp-unknown-group-name", &["\\k<a>"]);
    bad("v", "regexp-unknown-group-name", &["\\k<a>"]);
    // A literal astral name, which without u is a surrogate pair.
    ok("", &["(?<𝓑>.)"]);
    // ES2025's duplicate names, in different alternatives only.
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "(?<a>x)|(?<a>y)",
                "((?<a>x)|(?<a>y))",
                "(?:(?<a>x)|(?<a>y))\\k<a>",
                "(?<a>x)|(?:z|(?<a>y))",
            ],
        );
        bad(
            flags,
            "regexp-duplicate-group-name",
            &[
                "(?<a>x)(?<a>y)",
                "(?<a>x)|(?<b>y)(?<b>z)",
                "((?<a>x)|y)(?<a>z)",
                "(?<a>(?<a>x))",
            ],
        );
    }
}

#[test]
fn backreferences() {
    ok("u", &["(a)\\1", "\\1(a)", "(a)(b)\\2"]);
    bad("u", "regexp-backreference", &["\\1", "(a)\\2", "\\10(a)"]);
    // B.1.2: without u, `\n` past the groups is an octal escape, or the
    // digit itself.
    ok(
        "",
        &["\\1", "(a)\\2", "\\8", "\\9", "\\10", "\\377", "\\400"],
    );
    // Without u, `\k` is `k`, until a named group makes it a reference.
    ok("", &["\\k", "\\k<a"]);
    bad("", "regexp-escape", &["\\k(?<a>.)", "(?<a>.)\\k"]);
    bad("u", "regexp-escape", &["\\k", "\\ka"]);
    bad("u", "regexp-group-name", &["\\k<a", "\\k<>"]);
}

#[test]
fn character_escapes() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "\\f\\n\\r\\t\\v",
                "\\cA\\cz",
                "\\0",
                "\\x41",
                "\\u0041",
                "\\d\\D\\s\\S\\w\\W",
                "\\^\\$\\\\\\.\\*\\+\\?\\(\\)\\[\\]\\{\\}\\|\\/",
            ],
        );
    }
    for flags in ["u", "v"] {
        ok(flags, &["\\u{41}", "\\u{10FFFF}", "\\ud83d\\ude00"]);
        bad(
            flags,
            "regexp-escape",
            &[
                "\\a",
                "\\-",
                "\\c",
                "\\c1",
                "\\x4",
                "\\u004",
                "\\u{110000}",
                "\\u{}",
                "\\00",
                "\\01",
                "\\e",
                "\\_",
                "\\",
            ],
        );
    }
    // B.1.2: without u, an escape that isn't one is the character itself.
    ok(
        "",
        &[
            "\\a", "\\-", "\\e", "\\x4", "\\u004", "\\u{41}", "\\00", "\\01", "\\c", "\\c1",
            "\\p{L}", "\\_",
        ],
    );
    bad("", "regexp-escape", &["\\"]);
}

#[test]
fn classes() {
    for flags in ["", "u", "v"] {
        ok(
            flags,
            &[
                "[]",
                "[^]",
                "[a]",
                "[abc]",
                "[a-z]",
                "[^a-z0-9]",
                "[\\d]",
                "[\\b]",
                "[\\]]",
                "[a-a]",
                "[\\u0041-\\u005a]",
            ],
        );
        bad(
            flags,
            "regexp-range-order",
            &["[z-a]", "[\\u0042-\\u0041]", "[b-a]"],
        );
        bad(flags, "regexp-unterminated", &["[", "[a", "[^"]);
    }
    for flags in ["", "u"] {
        ok(flags, &["[a-]", "[-a]", "[a-b-c]", "[(){}|/]", "[\\-]"]);
    }
    // B.1.2: a range with a class at an end is a union without u.
    ok("", &["[\\d-a]", "[a-\\w]", "[\\s-\\d]"]);
    bad(
        "u",
        "regexp-range-class",
        &["[\\d-a]", "[a-\\w]", "[\\p{L}-z]"],
    );
    // In a class, `\c` takes a digit or `_` too, and is itself otherwise,
    // without u.
    ok("", &["[\\c1]", "[\\c_]", "[\\c]", "[\\1]", "[\\8]"]);
    bad("u", "regexp-escape", &["[\\c1]", "[\\c]", "[\\1]", "[\\8]"]);
    // Without u, an astral character is two code units, so a range
    // between two of them runs backwards.
    bad("", "regexp-range-order", &["[💩-💫]"]);
    ok("u", &["[💩-💫]"]);
}

#[test]
fn property_escapes() {
    for flags in ["u", "v"] {
        ok(
            flags,
            &[
                "\\p{L}",
                "\\p{Letter}",
                "\\P{Lu}",
                "\\p{General_Category=Decimal_Number}",
                "\\p{gc=Nd}",
                "\\p{Script=Greek}",
                "\\p{sc=Grek}",
                "\\p{Script_Extensions=Latin}",
                "\\p{scx=Latn}",
                "\\p{ASCII}",
                "\\p{Any}",
                "\\p{Alpha}",
                "\\P{White_Space}",
                "[\\p{L}\\P{N}]",
            ],
        );
        bad(
            flags,
            "regexp-property",
            &[
                "\\p{letter}",
                "\\p{L",
                "\\p",
                "\\p{}",
                "\\p{Script}",
                "\\p{gc=Greek}",
                "\\p{sc=L}",
                "\\p{Block=Basic_Latin}",
                "\\p{ascii}",
                "\\p{ Lu}",
                "\\p{Lu }",
                "\\p{InGreek}",
                "\\p{RGI_Emoji=Yes}",
            ],
        );
    }
    // Properties of strings are the v flag's.
    ok(
        "v",
        &[
            "\\p{RGI_Emoji}",
            "\\p{Basic_Emoji}",
            "[\\p{Emoji_Keycap_Sequence}a]",
        ],
    );
    bad(
        "u",
        "regexp-property",
        &["\\p{RGI_Emoji}", "\\p{Basic_Emoji}"],
    );
    // A subtraction may hold strings if its first operand may; an
    // intersection, if every operand may.
    bad(
        "v",
        "regexp-negated-strings",
        &[
            "\\P{RGI_Emoji}",
            "[^\\p{RGI_Emoji}]",
            "[^\\q{ab}]",
            "[^[\\p{RGI_Emoji}]]",
            "[^\\q{}]",
            "[^\\p{RGI_Emoji}--\\q{x}]",
            "[^\\p{RGI_Emoji}&&\\q{ab}]",
        ],
    );
}

#[test]
fn v_flag_classes() {
    ok(
        "v",
        &[
            "[\\p{L}--\\p{Lu}]",
            "[\\p{L}&&\\p{Lu}]",
            "[[a-z]--[aeiou]]",
            "[a&&a]",
            "[a--b--c]",
            "[[a]&&[b]&&[c]]",
            "[\\q{abc|d}]",
            "[\\q{}]",
            "[^\\q{a|b}]",
            "[^[a-z]&&\\p{L}]",
            "[\\&\\-\\!\\#\\%\\,\\:\\;\\<\\=\\>\\@\\`\\~]",
            "[\\(\\)\\[\\]\\{\\}\\/\\|]",
            "[&!#%,:;<=>@`~]",
            "[\\p{RGI_Emoji}&&a]",
            "[^[\\p{RGI_Emoji}]&&a]",
        ],
    );
    bad(
        "v",
        "regexp-class-syntax",
        &[
            "[a-z&&b]",
            "[a&&b--c]",
            "[a&&&b]",
            "[ab&&c]",
            "[a--bc]",
            "[(]",
            "[)]",
            "[{]",
            "[}]",
            "[/]",
            "[|]",
            "[-]",
            "[a-]",
            "[&&a]",
            "[a!!b]",
            "[a##]",
            "[$$]",
            "[%%]",
            "[**]",
            "[++]",
            "[,,]",
            "[..]",
            "[::]",
            "[;;]",
            "[<<]",
            "[==]",
            "[>>]",
            "[??]",
            "[@@]",
            "[a^^]",
            "[``]",
            "[~~]",
        ],
    );
    bad(
        "v",
        "regexp-escape",
        &["[\\q{\\d}]", "[\\1]", "[\\c]", "\\q{a}"],
    );
    bad("v", "regexp-range-class", &["[a-\\d]", "[a-\\q{b}]"]);
    bad("v", "regexp-class-syntax", &["[\\q{a}-z]", "[\\d-z]"]);
    bad("v", "regexp-unterminated", &["[[a]", "[a&&[b]", "[\\q{a"]);
    // The `u` flag reads the same text as a class without operators.
    ok("u", &["[a-z&&b]", "[(]", "[--a]", "[a&&&b]"]);
}

#[test]
fn spans_point_at_the_trouble() {
    let d = validate("ab[z-a]", "").unwrap_err();
    assert_eq!((d.span.start, d.span.end), (3, 6));
    let d = validate("é(?<a>x)(?<a>y)", "").unwrap_err();
    assert_eq!(d.span.start, 12);
    let d = validate("💩\\q", "u").unwrap_err();
    assert_eq!((d.span.start, d.span.end), (4, 6));
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let deep = format!("{}a{}", "(".repeat(100_000), ")".repeat(100_000));
    let class = format!("{}a{}", "[".repeat(100_000), "]".repeat(100_000));
    std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || {
            assert_eq!(validate(&deep, "").unwrap_err().code, "too-deep");
            assert_eq!(validate(&class, "v").unwrap_err().code, "too-deep");
            let fine = format!("{}a{}", "(".repeat(200), ")".repeat(200));
            assert!(validate(&fine, "u").is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn wide_patterns_take_linear_time() {
    let time = |n: usize| {
        let pattern = "(?<a>x)|".repeat(n) + &"[a-z\\d]".repeat(n) + &"\\k<a>".repeat(n);
        let t = std::time::Instant::now();
        assert!(validate(&pattern, "u").is_ok());
        t.elapsed()
    };
    time(1_000);
    let small = time(10_000);
    let large = time(100_000);
    assert!(large < small * 40, "{small:?} then {large:?}");
}
