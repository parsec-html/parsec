// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The lexer, production by production of ECMA-262 clause 12 and Annex
//! B.1.

mod driver;

use parsec_js::{Flags, Goal, Keyword, Kind, Lexer, Limits, Punct, Severity, SourceType, Span};

/// Each token's kind and text, trivia included.
fn lex(src: &str) -> Vec<(Kind, &str)> {
    lex_as(src, SourceType::Script)
}

fn lex_as(src: &str, source: SourceType) -> Vec<(Kind, &str)> {
    let (tokens, _) = driver::tokenize(src, source);
    tokens.iter().map(|t| (t.kind, t.text(src))).collect()
}

/// The diagnostics' codes.
fn codes(src: &str) -> Vec<&'static str> {
    codes_as(src, SourceType::Script)
}

fn codes_as(src: &str, source: SourceType) -> Vec<&'static str> {
    driver::tokenize(src, source)
        .1
        .iter()
        .map(|d| d.code)
        .collect()
}

/// The one token `src` is, with no diagnostics.
fn one(src: &str) -> Kind {
    let tokens = lex(src);
    assert_eq!(tokens.len(), 1, "{src:?} gave {tokens:?}");
    assert_eq!(tokens[0].1, src);
    assert_eq!(codes(src), Vec::<&str>::new(), "{src:?}");
    tokens[0].0
}

/// The flags on the first token of `src`.
fn flags(src: &str) -> Flags {
    driver::tokenize(src, SourceType::Script).0[0].flags
}

use Kind::Punct as P;

// 12.2 WhiteSpace and 12.3 LineTerminators.

#[test]
fn whitespace_is_skipped() {
    let ws = "\t\u{B}\u{C} \u{A0}\u{FEFF}\u{1680}\u{2000}\u{200A}\u{202F}\u{205F}\u{3000}";
    let src = format!("{ws}a{ws}b{ws}");
    assert_eq!(lex(&src), [(Kind::Ident, "a"), (Kind::Ident, "b")]);
    assert!(codes(&src).is_empty());
    // U+180E was whitespace once; it isn't now.
    assert_eq!(codes("a\u{180E}b"), ["unexpected-character"]);
}

#[test]
fn line_terminators_set_newline_before() {
    for lt in ["\n", "\r", "\r\n", "\u{2028}", "\u{2029}"] {
        let src = format!("a{lt}b");
        let (tokens, d) = driver::tokenize(&src, SourceType::Script);
        assert!(d.is_empty());
        assert!(!tokens[0].newline_before());
        assert!(tokens[1].newline_before(), "{lt:?}");
    }
    let (tokens, _) = driver::tokenize("a b", SourceType::Script);
    assert!(!tokens[1].newline_before());
}

// 12.4 Comments.

#[test]
fn comments() {
    assert_eq!(
        lex("a// x\nb"),
        [
            (Kind::Ident, "a"),
            (Kind::LineComment, "// x"),
            (Kind::Ident, "b")
        ]
    );
    assert_eq!(lex("// x\u{2028}b")[0], (Kind::LineComment, "// x"));
    assert_eq!(lex("a/* x */b")[1], (Kind::BlockComment, "/* x */"));
    assert_eq!(lex("/**/")[0], (Kind::BlockComment, "/**/"));
    assert_eq!(lex("/***/")[0], (Kind::BlockComment, "/***/"));
    assert_eq!(lex("// at the end"), [(Kind::LineComment, "// at the end")]);
    assert_eq!(codes("/* open"), ["unterminated-comment"]);
    assert_eq!(lex("/* open"), [(Kind::BlockComment, "/* open")]);
}

#[test]
fn a_comment_with_a_line_terminator_is_one() {
    // A MultiLineComment with a LineTerminator in it counts as one for
    // automatic semicolon insertion; one without doesn't.
    let (t, _) = driver::tokenize("a/*\n*/b", SourceType::Script);
    assert!(t[2].newline_before());
    let (t, _) = driver::tokenize("a/**/b", SourceType::Script);
    assert!(!t[2].newline_before());
    let (t, _) = driver::tokenize("a/*\u{2029}*/b", SourceType::Script);
    assert!(t[2].newline_before());
    // A line comment's terminator does too, and the comment itself carries
    // the flag for one before it.
    let (t, _) = driver::tokenize("a\n// c\nb", SourceType::Script);
    assert!(t[1].newline_before() && t[2].newline_before());
}

// 12.5 Hashbang comments.

#[test]
fn hashbang() {
    assert_eq!(
        lex("#!/usr/bin/env node\nx"),
        [(Kind::Hashbang, "#!/usr/bin/env node"), (Kind::Ident, "x")]
    );
    assert_eq!(lex_as("#!x", SourceType::Module), [(Kind::Hashbang, "#!x")]);
    // Only at the very start: not after whitespace or a line.
    assert_eq!(codes(" #!x"), ["unexpected-character"]);
    assert_eq!(codes("\n#!x"), ["unexpected-character"]);
}

// 12.6 Tokens, 12.7 Names and Keywords.

#[test]
fn identifiers() {
    for src in [
        "a",
        "$",
        "_",
        "abc123",
        "$_$",
        "café",
        "ಠ_ಠ",
        "𐊧",
        "x\u{200C}y",
        "x\u{200D}",
        "a\u{300}",
    ] {
        assert_eq!(one(src), Kind::Ident, "{src:?}");
    }
    // Not ID_Start: a combining mark or a digit can't begin one.
    assert_eq!(codes("\u{300}"), ["unexpected-character"]);
    assert_eq!(lex("1a")[0], (Kind::Number, "1a"));
    // ZWJ and ZWNJ continue, but don't start.
    assert_eq!(codes("\u{200C}x"), ["unexpected-character"]);
}

#[test]
fn identifier_escapes() {
    for src in [
        r"\u0061",
        r"\u{61}",
        r"a\u0062",
        r"\u{1d4d0}",
        r"\u{0000000061}",
        r"\u{24}",
        r"\u005F",
    ] {
        assert_eq!(one(src), Kind::Ident, "{src:?}");
        assert!(flags(src).contains(Flags::ESCAPED));
    }
    // A keyword spelled with an escape is an identifier the parser must
    // reject where the keyword is reserved.
    assert_eq!(one(r"\u0076ar"), Kind::Ident);
    // Escapes for characters that can't be there, and malformed ones.
    assert_eq!(codes(r"\u0031"), ["invalid-identifier-escape"]);
    assert_eq!(codes(r"a\u002E"), ["invalid-identifier-escape"]);
    assert_eq!(codes(r"\u{110000}"), ["invalid-identifier-escape"]);
    assert_eq!(codes(r"\u{}"), ["invalid-identifier-escape"]);
    assert_eq!(codes(r"\u00"), ["invalid-identifier-escape"]);
    assert_eq!(codes(r"a\x41"), ["invalid-identifier-escape"]);
    assert_eq!(lex(r"\ a")[0], (Kind::Invalid, r"\"));
    assert_eq!(codes(r"\ a"), ["invalid-escape"]);
}

#[test]
fn keywords() {
    assert_eq!(one("var"), Kind::Keyword(Keyword::Var));
    assert_eq!(one("yield"), Kind::Keyword(Keyword::Yield));
    assert_eq!(one("of"), Kind::Keyword(Keyword::Of));
    assert_eq!(one("variable"), Kind::Ident);
    assert_eq!(one("Var"), Kind::Ident);
    for k in ["await", "break", "enum", "with", "yield"] {
        assert!(Keyword::from_str(k).unwrap().is_reserved(), "{k}");
    }
    for k in ["let", "static", "implements", "public"] {
        let k = Keyword::from_str(k).unwrap();
        assert!(!k.is_reserved() && k.is_strict_reserved(), "{k:?}");
    }
    for k in ["async", "of", "get", "target"] {
        let k = Keyword::from_str(k).unwrap();
        assert!(!k.is_reserved() && !k.is_strict_reserved(), "{k:?}");
    }
    assert_eq!(Keyword::Instanceof.as_str(), "instanceof");
}

#[test]
fn private_names() {
    assert_eq!(one("#x"), Kind::PrivateName);
    assert_eq!(one("#$y1"), Kind::PrivateName);
    assert_eq!(one(r"#\u0061"), Kind::PrivateName);
    assert_eq!(one("#é"), Kind::PrivateName);
    assert_eq!(
        lex("this.#a"),
        [
            (Kind::Keyword(Keyword::This), "this"),
            (P(Punct::Dot), "."),
            (Kind::PrivateName, "#a")
        ]
    );
    assert_eq!(codes("# x"), ["unexpected-character"]);
    assert_eq!(lex("#1"), [(Kind::Invalid, "#"), (Kind::Number, "1")]);
}

// 12.8 Punctuators.

#[test]
fn every_punctuator() {
    let all = "{ } ( ) [ ] . ... ; , < > <= >= == != === !== + - * % ** ++ -- << >> >>> & | ^ ! ~ && || ?? ? ?. : = += -= *= %= **= <<= >>= >>>= &= |= ^= &&= ||= ??= =>";
    for p in all.split(' ') {
        assert_eq!(one(p), P(Punct::from_str(p).unwrap()), "{p}");
        assert_eq!(Punct::from_str(p).unwrap().as_str(), p);
    }
    // Longest match.
    assert_eq!(lex("a>>>=b")[1], (P(Punct::UShrEq), ">>>="));
    assert_eq!(lex("a....b")[1], (P(Punct::Ellipsis), "..."));
    assert_eq!(lex("a=>b")[1], (P(Punct::Arrow), "=>"));
}

#[test]
fn optional_chaining_isnt_before_a_digit() {
    assert_eq!(lex("a?.b")[1], (P(Punct::QuestionDot), "?."));
    assert_eq!(
        lex("a?.5:b"),
        [
            (Kind::Ident, "a"),
            (P(Punct::Question), "?"),
            (Kind::Number, ".5"),
            (P(Punct::Colon), ":"),
            (Kind::Ident, "b")
        ]
    );
}

#[test]
fn characters_that_start_nothing() {
    for src in ["@", "€", "\u{1}", "😀"] {
        assert_eq!(lex(src), [(Kind::Invalid, src)], "{src:?}");
        assert_eq!(codes(src), ["unexpected-character"]);
    }
}

// The goals: DivPunctuator, RightBracePunctuator, and what the caller
// chooses.

#[test]
fn the_goal_decides_slash_and_brace() {
    let src = "/a/g";
    let mut l = Lexer::new(src, SourceType::Script);
    assert_eq!(l.next_token(Goal::Div).kind, P(Punct::Slash));
    let mut l = Lexer::new(src, SourceType::Script);
    assert_eq!(l.next_token(Goal::RegExp).kind, Kind::RegExp);
    let mut l = Lexer::new("/=a/", SourceType::Script);
    assert_eq!(l.next_token(Goal::Div).kind, P(Punct::SlashEq));
    let mut l = Lexer::new("/=a/", SourceType::Script);
    assert_eq!(l.next_token(Goal::RegExpOrTemplateTail).kind, Kind::RegExp);

    let mut l = Lexer::new("} x`", SourceType::Script);
    assert_eq!(l.next_token(Goal::Div).kind, P(Punct::RBrace));
    let mut l = Lexer::new("} x`", SourceType::Script);
    let t = l.next_token(Goal::TemplateTail);
    assert_eq!((t.kind, t.span), (Kind::TemplateTail, Span::new(0, 4)));
    // Comments aren't affected by the goal.
    let mut l = Lexer::new("// a", SourceType::Script);
    assert_eq!(l.next_token(Goal::RegExp).kind, Kind::LineComment);
}

#[test]
fn division_and_regexps_from_the_driver() {
    assert_eq!(
        lex("a / b / c")
            .iter()
            .filter(|t| t.0 == P(Punct::Slash))
            .count(),
        2
    );
    assert_eq!(lex("x = /=/g")[2], (Kind::RegExp, "/=/g"));
    assert_eq!(lex("(1) / 2")[3], (P(Punct::Slash), "/"));
    assert_eq!(lex("return /x/")[1], (Kind::RegExp, "/x/"));
    assert_eq!(lex("this / 2")[1], (P(Punct::Slash), "/"));
    assert_eq!(lex("a++ / 2")[2], (P(Punct::Slash), "/"));
    assert_eq!(lex("if (a) /b/.test(c)")[4], (Kind::RegExp, "/b/"));
    assert_eq!(lex("f(a) / 2")[4], (P(Punct::Slash), "/"));
    // `of` is an identifier more often than not.
    assert_eq!(lex("of / 2")[1], (P(Punct::Slash), "/"));
}

// 12.9.3 Numeric literals.

#[test]
fn decimal_literals() {
    for src in [
        "0", "7", "42", "3.14", "3.", ".5", "1e3", "1E+3", "1e-3", "2.5e10", ".5e1", "1.e2", "0.0",
        "0e0",
    ] {
        assert_eq!(one(src), Kind::Number, "{src}");
    }
    assert_eq!(lex("1..toString()")[0], (Kind::Number, "1."));
    assert_eq!(
        lex("1.5.x")[..2],
        [(Kind::Number, "1.5"), (P(Punct::Dot), ".")]
    );
    assert_eq!(codes("1e"), ["missing-exponent"]);
    assert_eq!(codes("1e+"), ["missing-exponent"]);
}

#[test]
fn non_decimal_literals() {
    for src in ["0x0", "0xFF", "0XaBc", "0o17", "0O7", "0b1010", "0B1"] {
        assert_eq!(one(src), Kind::Number, "{src}");
    }
    assert_eq!(codes("0x"), ["missing-digits"]);
    assert_eq!(codes("0b"), ["missing-digits"]);
    assert_eq!(codes("0b12"), ["identifier-after-number"]);
    assert_eq!(codes("0o8"), ["missing-digits", "identifier-after-number"]);
    assert_eq!(codes("0xG"), ["missing-digits", "identifier-after-number"]);
    // No fraction after a prefix.
    assert_eq!(
        lex("0x1.5")[..2],
        [(Kind::Number, "0x1"), (Kind::Number, ".5")]
    );
}

#[test]
fn bigints() {
    for src in ["0n", "1n", "123n", "0x1Fn", "0o7n", "0b1n", "1_000n"] {
        assert_eq!(one(src), Kind::BigInt, "{src}");
    }
    // Not with a fraction or exponent, or a leading zero.
    for src in ["1.5n", "1e3n", ".5n", "01n", "08n"] {
        assert!(codes(src).contains(&"identifier-after-number"), "{src}");
        assert_eq!(lex(src)[0].0, Kind::Number);
    }
}

#[test]
fn numeric_separators() {
    for src in [
        "1_000", "1_0.0_1", "1e1_0", "0xF_F", "0b1_0", "0o7_7", ".0_1",
    ] {
        assert_eq!(one(src), Kind::Number, "{src}");
    }
    // Only between two digits.
    for src in [
        "1__0", "1_", "1_.5", "1._5", "1e_1", "1_e1", "0x_1", "0b1_", ".5_",
    ] {
        assert!(
            codes(src).contains(&"invalid-numeric-separator"),
            "{src}: {:?}",
            codes(src)
        );
    }
    // Not after a leading zero, nor in legacy forms: there `_` starts an
    // identifier.
    assert_eq!(codes("0_1"), ["identifier-after-number"]);
    assert_eq!(
        codes_as("07_1", SourceType::Module),
        ["legacy-octal-literal", "identifier-after-number"]
    );
}

#[test]
fn legacy_octal_and_leading_zero_literals() {
    // B.1.1: legal in sloppy scripts, so a warning there and a flag for
    // the parser; always an error in a module, which is strict.
    for (src, code) in [
        ("017", "legacy-octal-literal"),
        ("00", "legacy-octal-literal"),
        ("08", "leading-zero-literal"),
        ("09.5", "leading-zero-literal"),
        ("019e1", "leading-zero-literal"),
    ] {
        let (t, d) = driver::tokenize(src, SourceType::Script);
        assert_eq!(
            (t.len(), t[0].kind, t[0].text(src)),
            (1, Kind::Number, src),
            "{src}"
        );
        assert!(t[0].flags.contains(Flags::LEGACY_OCTAL));
        assert_eq!(
            (d.len(), d[0].code, d[0].severity),
            (1, code, Severity::Warning),
            "{src}"
        );
        let (_, d) = driver::tokenize(src, SourceType::Module);
        assert_eq!((d[0].code, d[0].severity), (code, Severity::Error));
    }
    // A legacy octal literal has no fraction: `07.5` is `07` then `.5`.
    assert_eq!(
        lex("07.5")[..2],
        [(Kind::Number, "07"), (Kind::Number, ".5")]
    );
    assert!(!flags("0.5").contains(Flags::LEGACY_OCTAL));
    assert!(!flags("0").contains(Flags::LEGACY_OCTAL));
}

#[test]
fn nothing_straight_after_a_number() {
    assert_eq!(lex("3in x"), [(Kind::Number, "3in"), (Kind::Ident, "x")]);
    assert_eq!(codes("3in"), ["identifier-after-number"]);
    assert_eq!(codes("3é"), ["identifier-after-number"]);
    assert_eq!(codes(r"3\u0061"), ["identifier-after-number"]);
    assert!(codes("3 in").is_empty());
    assert!(codes("3+x").is_empty());
}

// 12.9.4 String literals.

#[test]
fn strings() {
    for src in [
        "''",
        "\"\"",
        "'a'",
        "\"it's\"",
        "'say \"hi\"'",
        "'é😀'",
        "'\u{2028}\u{2029}'",
    ] {
        assert_eq!(one(src), Kind::String, "{src}");
        assert!(!flags(src).contains(Flags::ESCAPED));
    }
    assert_eq!(codes("'abc"), ["unterminated-string"]);
    assert_eq!(lex("'ab\ncd'")[0], (Kind::String, "'ab"));
    assert_eq!(codes("'a\rb'").first(), Some(&"unterminated-string"));
}

#[test]
fn string_escapes() {
    for src in [
        r"'\n\t\r\b\f\v\0'",
        r"'\''",
        r#""\"""#,
        r"'\\'",
        r"'\x41'",
        r"'\u0041'",
        r"'\u{1F600}'",
        r"'\u{10FFFF}'",
        r"'\a\q\é'",
        "'a\\\nb'",
        "'a\\\r\nb'",
        "'a\\\rb'",
        "'a\\\u{2028}b'",
    ] {
        assert_eq!(one(src), Kind::String, "{src:?}");
        assert!(flags(src).contains(Flags::ESCAPED), "{src:?}");
        assert!(!flags(src).contains(Flags::LEGACY_OCTAL), "{src:?}");
    }
    for src in [
        r"'\x4'",
        r"'\xg0'",
        r"'\u004'",
        r"'\u{}'",
        r"'\u{110000}'",
        r"'\u{12'",
    ] {
        assert_eq!(codes(src), ["invalid-escape"], "{src}");
        // A bad escape never takes the closing quote with it.
        assert_eq!(lex(src).len(), 1, "{src}");
    }
    assert_eq!(codes(r"'\"), ["unterminated-string"]);
}

#[test]
fn legacy_octal_escapes() {
    for src in [
        r"'\1'", r"'\01'", r"'\00'", r"'\08'", r"'\377'", r"'\47'", r"'\8'", r"'\9'",
    ] {
        let (t, d) = driver::tokenize(src, SourceType::Script);
        assert_eq!((t.len(), t[0].kind), (1, Kind::String), "{src}");
        assert!(t[0].flags.contains(Flags::LEGACY_OCTAL), "{src}");
        assert_eq!(
            (d[0].code, d[0].severity),
            ("legacy-octal-escape", Severity::Warning)
        );
        assert_eq!(codes_as(src, SourceType::Module), ["legacy-octal-escape"]);
    }
    // \0 before anything but a digit is the null character, not octal.
    assert!(!flags(r"'\0a'").contains(Flags::LEGACY_OCTAL));
    // An octal escape takes at most three digits, starting 0-3, or two.
    let (_, d) = driver::tokenize(r"'\3777'", SourceType::Script);
    assert_eq!(d[0].span, Span::new(1, 5));
    let (_, d) = driver::tokenize(r"'\477'", SourceType::Script);
    assert_eq!(d[0].span, Span::new(1, 4));
}

// 12.9.5 Regular expression literals.

#[test]
fn regexps() {
    for src in [
        "/a/",
        "/a/g",
        "/[/]/",
        "/\\//",
        "/a\\/b/i",
        "/[\\]/]/",
        "/é😀/u",
        "/(?:)/dgimsy",
        "/a/v",
        "/[a-z]+/gv",
    ] {
        assert_eq!(one(src), Kind::RegExp, "{src}");
    }
    assert_eq!(lex("x = /a/.source")[2], (Kind::RegExp, "/a/"));
    assert_eq!(codes("/a"), ["unterminated-regexp"]);
    assert_eq!(codes("/a\n/"), ["unterminated-regexp"]);
    assert_eq!(codes("/a\u{2028}/"), ["unterminated-regexp"]);
    assert_eq!(codes("/a\\\n/"), ["unterminated-regexp"]);
    assert_eq!(codes("/[/"), ["unterminated-regexp"]);
}

#[test]
fn regexp_flags() {
    for src in ["/a/gg", "/a/x", "/a/uv", "/a/gimsuyx", "/a/é"] {
        assert_eq!(codes(src), ["invalid-regexp-flags"], "{src}");
        assert_eq!(lex(src).len(), 1, "{src}");
    }
    assert_eq!(codes(r"/a/\u0067"), ["invalid-regexp-flags"]);
    assert_eq!(lex(r"/a/\u0067")[0].1, r"/a/\u0067");
}

// 12.9.6 Template literal lexical components.

#[test]
fn templates() {
    assert_eq!(one("``"), Kind::Template);
    assert_eq!(one("`a\nb`"), Kind::Template);
    assert_eq!(one("`$`"), Kind::Template);
    assert_eq!(one("`{}$ {`"), Kind::Template);
    assert_eq!(
        lex("`a${b}c${d}e`"),
        [
            (Kind::TemplateHead, "`a${"),
            (Kind::Ident, "b"),
            (Kind::TemplateMiddle, "}c${"),
            (Kind::Ident, "d"),
            (Kind::TemplateTail, "}e`"),
        ]
    );
    // Braces inside a substitution are the code's own.
    assert_eq!(
        lex("`${ {a: 1} }`").iter().map(|t| t.0).collect::<Vec<_>>(),
        [
            Kind::TemplateHead,
            P(Punct::LBrace),
            Kind::Ident,
            P(Punct::Colon),
            Kind::Number,
            P(Punct::RBrace),
            Kind::TemplateTail
        ]
    );
    // And templates nest.
    assert_eq!(lex("`${`${a}`}`").len(), 5);
    assert_eq!(codes("`abc"), ["unterminated-template"]);
    assert_eq!(codes("`${a}b"), ["unterminated-template"]);
}

#[test]
fn template_escapes() {
    for src in [
        r"`\n`",
        r"`\x41`",
        r"`\u0041`",
        r"`\u{41}`",
        r"`\0`",
        r"`\``",
        r"`\${`",
        "`\\\r\n`",
    ] {
        assert_eq!(one(src), Kind::Template, "{src:?}");
        assert!(flags(src).contains(Flags::ESCAPED));
        assert!(!flags(src).contains(Flags::INVALID_ESCAPE), "{src:?}");
    }
    // NotEscapeSequences: fine when tagged, so a flag and no diagnostic,
    // and the template still ends where it should.
    for src in [
        r"`\01`",
        r"`\1`",
        r"`\8`",
        r"`\x`",
        r"`\xg`",
        r"`\u`",
        r"`\u{`",
        r"`\u{110000}`",
        r"`\u00g`",
    ] {
        assert_eq!(one(src), Kind::Template, "{src:?}");
        assert!(flags(src).contains(Flags::INVALID_ESCAPE), "{src:?}");
    }
    assert_eq!(lex(r"`\u{${a}`")[0], (Kind::TemplateHead, r"`\u{${"));
}

// B.1.1 HTML-like comments.

#[test]
fn html_open_comments() {
    assert_eq!(
        lex("a <!-- x\nb"),
        [
            (Kind::Ident, "a"),
            (Kind::HtmlOpenComment, "<!-- x"),
            (Kind::Ident, "b")
        ]
    );
    assert_eq!(lex("<!--")[0], (Kind::HtmlOpenComment, "<!--"));
    // In a module, `<!--` is `<`, `!`, `--`.
    assert_eq!(
        lex_as("a<!--b", SourceType::Module)
            .iter()
            .map(|t| t.0)
            .collect::<Vec<_>>(),
        [
            Kind::Ident,
            P(Punct::Lt),
            P(Punct::Bang),
            P(Punct::MinusMinus),
            Kind::Ident
        ]
    );
}

#[test]
fn html_close_comments() {
    assert_eq!(
        lex("a\n--> x\nb"),
        [
            (Kind::Ident, "a"),
            (Kind::HtmlCloseComment, "--> x"),
            (Kind::Ident, "b")
        ]
    );
    assert_eq!(
        lex("a\n  /* c */ /**/ -->")[3],
        (Kind::HtmlCloseComment, "-->")
    );
    assert_eq!(lex("a/*\n*/-->")[2], (Kind::HtmlCloseComment, "-->"));
    // Not after something else on the line, and not in a module.
    assert_eq!(lex("a-->b")[1], (P(Punct::MinusMinus), "--"));
    assert_eq!(lex("a\nb-->c")[2], (P(Punct::MinusMinus), "--"));
    assert_eq!(lex("a\n/**/b-->")[3], (P(Punct::MinusMinus), "--"));
    assert_eq!(
        lex_as("a\n-->b", SourceType::Module)[1],
        (P(Punct::MinusMinus), "--")
    );
    assert_eq!(lex("x = a\n// c\n-->")[4], (Kind::HtmlCloseComment, "-->"));
    // The first line is a line start too.
    assert_eq!(lex("--> x\ny")[0], (Kind::HtmlCloseComment, "--> x"));
    assert_eq!(lex(" /* a */ --> x")[1], (Kind::HtmlCloseComment, "--> x"));
}

// Tokens and spans as a whole.

#[test]
fn spans_tile_the_source() {
    let src = "#!x\nlet a = `t${ b /* c */ }u` + /re/g.flags; // end\n  <!-- old\n--> older\n'\\\u{2028}' é\u{2029}";
    let (tokens, d) = driver::tokenize(src, SourceType::Script);
    assert!(d.is_empty(), "{d:?}");
    let mut end = 0;
    for t in &tokens {
        assert!(t.span.start >= end && t.span.end > t.span.start, "{t:?}");
        // Only whitespace and line terminators between tokens.
        assert!(
            src[end as usize..t.span.start as usize]
                .chars()
                .all(char::is_whitespace),
            "{t:?}"
        );
        end = t.span.end;
    }
    assert!(src[end as usize..].chars().all(char::is_whitespace));
    assert_eq!(std::mem::size_of::<parsec_js::Token>(), 12);
}

#[test]
fn eof_again_and_again() {
    let mut l = Lexer::new("a", SourceType::Script);
    assert_eq!(l.next_token(Goal::Div).kind, Kind::Ident);
    for _ in 0..3 {
        let t = l.next_token(Goal::RegExp);
        assert_eq!((t.kind, t.span), (Kind::Eof, Span::at(1)));
    }
    assert_eq!(
        Lexer::new("", SourceType::Module)
            .next_token(Goal::Div)
            .kind,
        Kind::Eof
    );
}

#[test]
fn input_over_the_limit() {
    let limits = Limits {
        max_input: Some(3),
        ..Limits::default()
    };
    let mut l = Lexer::with_limits("abcd", SourceType::Script, &limits);
    assert_eq!(l.next_token(Goal::Div).kind, Kind::Eof);
    assert_eq!(l.diagnostics()[0].code, "input-too-large");
    let mut l = Lexer::with_limits("abc", SourceType::Script, &limits);
    assert_eq!(l.next_token(Goal::Div).kind, Kind::Ident);
}

#[test]
fn positions_through_line_index() {
    let src = "a\u{2028}é + 'x";
    let (_, d) = driver::tokenize(src, SourceType::Script);
    let lines = parsec_js::LineIndex::ecmascript(src);
    assert_eq!(lines.line_col(d[0].span.start), (2, 5).into());
}

/// Hostile input stays linear: each of these, a megabyte long, lexes in
/// one pass, where a lexer that rescanned would take minutes.
#[test]
fn hostile_inputs_are_linear() {
    let n = 1 << 20;
    let inputs = [
        "/*".to_string() + &"*".repeat(n),
        "'".to_string() + &"\\".repeat(n),
        "`".to_string() + &"${".repeat(n / 2),
        "/".to_string() + &"[".repeat(n),
        "1".repeat(n) + "_",
        "a".repeat(n),
        "\\u{".repeat(n / 3),
        "<!--".repeat(n / 4),
        "\n-->".repeat(n / 4),
        "0".repeat(n),
        "#".repeat(n),
        "é".repeat(n / 2),
    ];
    let start = std::time::Instant::now();
    for src in &inputs {
        let (tokens, _) = driver::tokenize(src, SourceType::Script);
        assert!(!tokens.is_empty());
    }
    // Generous for a debug build on a slow runner; quadratic would be hours.
    assert!(start.elapsed().as_secs() < 60, "{:?}", start.elapsed());
}
