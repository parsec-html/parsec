// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! `parsec_js::data`: literals to JSON, the strict JSON parser, and the
//! page-state shapes, each on a real page saved in `tests/pages`.

use std::time::{Duration, Instant};

use parsec_js::data::{Found, Json, Refused, json, literal, next_flight, page_state};
use parsec_js::{Node, parse_script};

/// `expr`'s value as JSON text, or the refusal's reason and the text
/// at its span.
fn lit(expr: &str) -> Result<String, (&'static str, String)> {
    let src = format!("({expr})");
    let ast = parse_script(&src).unwrap_or_else(|e| panic!("{expr}: {e}"));
    let Node::Program { body, .. } = *ast.node(ast.root()) else {
        unreachable!()
    };
    let Node::Expression { expression, .. } = *ast.node(ast.get(body)[0]) else {
        panic!("{expr}: not an expression")
    };
    literal(&ast, &src, ast.unparenthesized(expression))
        .map(|v| v.to_json_string())
        .map_err(|r| (r.reason, r.span.text(&src).to_string()))
}

fn refused(expr: &str, reason: &str, at: &str) {
    match lit(expr) {
        Err((r, text)) => assert_eq!((r, text.as_str()), (reason, at), "{expr}"),
        Ok(v) => panic!("{expr} gave {v}, should refuse with {reason}"),
    }
}

#[test]
fn json_values_round_trip_through_literals() {
    for (text, out) in [
        ("null", "null"),
        ("true", "true"),
        ("false", "false"),
        ("0", "0"),
        ("-0", "0"),
        ("1.5e3", "1500"),
        ("-2.5E-7", "-2.5e-7"),
        ("1e400", "null"),
        ("123456789012345678901234", "1.2345678901234569e+23"),
        (r#""""#, r#""""#),
        (r#""a\"b\\c\/d\b\f\n\r\t""#, r#""a\"b\\c/d\b\f\n\r\t""#),
        (r#""\u00e9\ud83d\ude00\u0001""#, r#""é😀\u0001""#),
        ("[]", "[]"),
        ("{}", "{}"),
        (
            r#"{"a":[1,{"b":null}],"c":"d","":true}"#,
            r#"{"a":[1,{"b":null}],"c":"d","":true}"#,
        ),
        (" [ 1 , 2 ]\n", "[1,2]"),
        (r#"{"a":1,"b":2,"a":3}"#, r#"{"a":3,"b":2}"#),
    ] {
        assert_eq!(json(text).unwrap().to_json_string(), out, "json {text}");
        assert_eq!(lit(text).unwrap(), out, "literal {text}");
        // What it writes reads back the same.
        assert_eq!(json(out).unwrap().to_json_string(), out, "again {text}");
    }
}

/// serde_json's reading of `v`, as a [`Json`].
fn from_serde(v: serde_json::Value) -> Json {
    match v {
        serde_json::Value::Null => Json::Null,
        serde_json::Value::Bool(b) => Json::Bool(b),
        serde_json::Value::Number(n) => Json::Number(n.as_f64().unwrap()),
        serde_json::Value::String(s) => Json::String(s),
        serde_json::Value::Array(a) => Json::Array(a.into_iter().map(from_serde).collect()),
        serde_json::Value::Object(o) => {
            Json::Object(o.into_iter().map(|(k, v)| (k, from_serde(v))).collect())
        }
    }
}

#[test]
fn json_agrees_with_serde_json() {
    for text in [
        r#"{"a":[1,2.5,-3e2,true,null,"x\u0041\n"],"b":{"c":{}}}"#,
        "[0.1,1e-7,123e45,-0.0,5e-324,1.7976931348623157e308,9007199254740993]",
    ] {
        let theirs = from_serde(serde_json::from_str(text).unwrap());
        assert_eq!(json(text).unwrap(), theirs, "{text}");
    }
}

#[test]
fn json_refuses_what_json_parse_would() {
    for (text, reason, at) in [
        ("", "json-syntax", 0),
        ("[1,]", "json-syntax", 3),
        ("{a:1}", "json-syntax", 1),
        ("'a'", "json-syntax", 0),
        ("01", "json-syntax", 1),
        ("1.", "json-syntax", 2),
        (".5", "json-syntax", 0),
        ("+1", "json-syntax", 0),
        ("NaN", "json-syntax", 0),
        ("[1] 2", "json-syntax", 4),
        ("\"a\nb\"", "json-syntax", 2),
        (r#""\x41""#, "json-syntax", 2),
        (r#""\ud800""#, "lone-surrogate", 1),
        (r#""\ude00x""#, "lone-surrogate", 1),
        ("\u{feff}1", "json-syntax", 0),
        ("{\"a\":1 \"b\":2}", "json-syntax", 7),
        ("[\"a", "json-syntax", 3),
    ] {
        let r = json(text).unwrap_err();
        assert_eq!((r.reason, r.span.start), (reason, at), "{text:?}");
    }
}

#[test]
fn literal_accepts_what_minified_javascript_writes() {
    for (expr, out) in [
        ("'single'", r#""single""#),
        (
            r#""\x41\u{42}\103\
D""#,
            r#""ABCD""#,
        ),
        ("`template`", r#""template""#),
        ("`a\\nb`", r#""a\nb""#),
        (
            "{a: 1, 'b': 2, \"c\": 3, 4: 4, 0x10: 5, 1.50: 6}",
            r#"{"a":1,"b":2,"c":3,"4":4,"16":5,"1.5":6}"#,
        ),
        ("[1, , 3]", "[1,null,3]"),
        ("[1, 2, ]", "[1,2]"),
        ("{a: 1,}", r#"{"a":1}"#),
        ("-1", "-1"),
        ("+1", "1"),
        ("- -1", "1"),
        ("-(1)", "-1"),
        ("0x1F", "31"),
        ("1_000", "1000"),
        (".5", "0.5"),
        ("!0", "true"),
        ("!1", "false"),
        ("!!0", "false"),
        ("{a: void 0, b: 1}", r#"{"b":1}"#),
        ("[void 0, undefined]", "[null,null]"),
        ("{a: undefined}", "{}"),
        ("{a: 1, a: void 0}", "{}"),
        ("{a: 1, b: 2, a: 3}", r#"{"a":3,"b":2}"#),
        ("((([1])))", "[1]"),
        ("{if: 1, class: 2}", r#"{"if":1,"class":2}"#),
        ("{\\u0061: 1}", r#"{"a":1}"#),
        ("{__proto__x: 1}", r#"{"__proto__x":1}"#),
        ("1e999", "null"),
    ] {
        assert_eq!(lit(expr).as_deref(), Ok(out), "{expr}");
    }
}

#[test]
fn literal_refuses_everything_else_where_it_stops() {
    for (expr, reason, at) in [
        ("a", "not-a-literal", "a"),
        ("{a: b}", "not-a-literal", "b"),
        ("{a}", "not-a-literal", "a"),
        ("[1, f()]", "not-a-literal", "f()"),
        ("a.b", "not-a-literal", "a.b"),
        ("1 + 2", "not-a-literal", "1 + 2"),
        ("typeof 1", "not-a-literal", "typeof 1"),
        ("~1", "not-a-literal", "~1"),
        ("new A", "not-a-literal", "new A"),
        ("function () {}", "not-a-literal", "function () {}"),
        ("Infinity", "not-a-literal", "Infinity"),
        ("NaN", "not-a-literal", "NaN"),
        ("this", "not-a-literal", "this"),
        ("tag`x`", "not-a-literal", "tag`x`"),
        ("`a${b}c`", "substitution", "b"),
        ("1n", "bigint", "1n"),
        ("{1n: 1}", "bigint", "1n"),
        ("/re/", "regexp", "/re/"),
        ("{[k]: 1}", "computed-key", "k"),
        ("{['k']: 1}", "computed-key", "'k'"),
        ("{m() {}}", "method", "m() {}"),
        ("{get a() { return 1 }}", "method", "get a() { return 1 }"),
        ("{...o}", "spread", "...o"),
        ("[...a]", "spread", "...a"),
        ("{__proto__: null}", "proto", "__proto__"),
        ("{'__proto__': {}}", "proto", "'__proto__'"),
        ("-'1'", "not-a-number", "'1'"),
        ("-true", "not-a-number", "true"),
        ("!'a'", "not-a-number", "'a'"),
        ("void f()", "not-a-literal", "f()"),
        ("void 0", "undefined", "void 0"),
        ("undefined", "undefined", "undefined"),
        ("'\\uD800'", "lone-surrogate", "'\\uD800'"),
        ("{'\\uDC00': 1}", "lone-surrogate", "'\\uDC00'"),
    ] {
        refused(expr, reason, at);
    }
}

#[test]
fn refusals_print_their_reason_and_where() {
    let r = Refused {
        span: parsec_js::Span::new(3, 5),
        reason: "bigint",
    };
    assert_eq!(r.to_string(), "bigint at 3..5");
}

fn found(src: &str) -> Vec<(String, Result<String, &'static str>)> {
    page_state(src)
        .unwrap()
        .into_iter()
        .map(|f| {
            (
                f.name,
                f.value.map(|v| v.to_json_string()).map_err(|r| r.reason),
            )
        })
        .collect()
}

fn one(name: &str, value: &str) -> Vec<(String, Result<String, &'static str>)> {
    vec![(name.to_string(), Ok(value.to_string()))]
}

#[test]
fn page_state_reads_assignments_to_globals() {
    assert_eq!(found("window.X = {a: 1};"), one("window.X", r#"{"a":1}"#));
    assert_eq!(found("self.X = [1]"), one("self.X", "[1]"));
    assert_eq!(found("globalThis.X = 'x'"), one("globalThis.X", r#""x""#));
    assert_eq!(found("window['X'] = 1"), one("window.X", "1"));
    assert_eq!(found("window.a.b = 1"), one("window.a.b", "1"));
    assert_eq!(found("window['a-b'] = 1"), one(r#"window["a-b"]"#, "1"));
    assert_eq!(found("(window.X = 1)"), one("window.X", "1"));
    assert_eq!(
        found("window.a = 1, window.b = !0"),
        vec![
            ("window.a".into(), Ok("1".into())),
            ("window.b".into(), Ok("true".into()))
        ]
    );
    // Not page state: other objects, other operators, nested code.
    for src in [
        "document.X = 1",
        "window = 1",
        "window.X += 1",
        "window[k] = 1",
        "window.X == 1",
        "if (a) window.X = 1",
        "function f() { window.X = 1 }",
        "a.window.X = 1",
    ] {
        assert_eq!(found(src), vec![], "{src}");
    }
    // A recognized shape whose value isn't a literal says why.
    assert_eq!(
        found("window.X = f()"),
        vec![("window.X".into(), Err("not-a-literal"))]
    );
}

#[test]
fn page_state_reads_top_level_declarations() {
    assert_eq!(found("var X = {a: 1};"), one("X", r#"{"a":1}"#));
    assert_eq!(found("let X = [1]"), one("X", "[1]"));
    assert_eq!(
        found("let a = 1, b = '2', c;"),
        vec![
            ("a".into(), Ok("1".into())),
            ("b".into(), Ok(r#""2""#.into()))
        ]
    );
    assert_eq!(found("var {a} = {a: 1}"), vec![]);
    assert_eq!(found("{ var X = 1 }"), vec![]);
}

#[test]
fn page_state_parses_json_parse_strings() {
    assert_eq!(
        found(r#"window.X = JSON.parse("{\"a\":[1,true]}")"#),
        one("window.X", r#"{"a":[1,true]}"#)
    );
    assert_eq!(found("var X = JSON.parse('[1]')"), one("X", "[1]"));
    assert_eq!(found("var X = JSON.parse(`null`)"), one("X", "null"));
    assert_eq!(
        found(r#"var X = JSON.parse('\u007B\u0022a\u0022\u003A1\u007D')"#),
        one("X", r#"{"a":1}"#)
    );
    let src = r#"window.X = JSON.parse("[1,]")"#;
    let f = &page_state(src).unwrap()[0];
    let r = f.value.as_ref().unwrap_err();
    assert_eq!((r.reason, r.span.text(src)), ("json-syntax", r#""[1,]""#));
    assert_eq!(found("var X = JSON.parse(s)")[0].1, Err("not-a-literal"));
    assert_eq!(
        found("var X = JSON.parse('1', r)")[0].1,
        Err("not-a-literal")
    );
    assert_eq!(
        found("var X = JSON['parse']('1')")[0].1,
        Err("not-a-literal")
    );
}

#[test]
fn page_state_substitutes_nuxt_arguments() {
    assert_eq!(
        found(r#"window.__NUXT__=(function(a,b,c){return {x:a,y:[b,c],z:{a:a}}}(1,"x",null))"#),
        one("window.__NUXT__", r#"{"x":1,"y":["x",null],"z":{"a":1}}"#)
    );
    // Called after the parentheses, an arrow, a missing argument, void 0.
    assert_eq!(
        found("window.__NUXT__=(function(a,b){return {a:a,b:b}})(void 0)"),
        one("window.__NUXT__", "{}")
    );
    assert_eq!(
        found("var X = ((a) => ({a}))([1])"),
        one("X", r#"{"a":[1]}"#)
    );
    assert_eq!(found("var X = ((a) => { return [a] })(2)"), one("X", "[2]"));
    // A parameter shadows `undefined`; the last of two names wins.
    assert_eq!(
        found("var X = (function(undefined){return [undefined]}(1))"),
        one("X", "[1]")
    );
    assert_eq!(
        found("var X = (function(a,a){return a}(1,2))"),
        one("X", "2")
    );
    for (src, reason) in [
        ("var X = (function(a){a.b=1;return a}({}))", "function-body"),
        ("var X = (function(a){}(1))", "function-body"),
        ("var X = (function(a){return}(1))", "function-body"),
        ("var X = (function(a=1){return a}())", "parameters"),
        ("var X = (function(...a){return a}())", "parameters"),
        ("var X = (function({a}){return a}({}))", "parameters"),
        ("var X = (function*(a){return a}(1))", "not-a-literal"),
        ("var X = (async function(a){return a}(1))", "not-a-literal"),
        ("var X = (function(a){return a}(f()))", "not-a-literal"),
        ("var X = (function(a){return a}(...b))", "spread"),
        ("var X = (function(a){return b}(1))", "not-a-literal"),
        ("var X = (function(a){return a}())", "undefined"),
    ] {
        assert_eq!(found(src)[0].1, Err(reason), "{src}");
    }
}

#[test]
fn page_state_collects_next_flight_pushes() {
    let src = r#"(self.__next_f=self.__next_f||[]).push([0]);
self.__next_f.push([1,"a:1\nb:"]);
self.__next_f.push([2,null]);
self.__next_f.push([1,"[2]\n"]);
window.__next_f.push([1,"c"]);
self.__next_f.push(x);"#;
    let all = page_state(src).unwrap();
    let names: Vec<&str> = all.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["self.__next_f"; 4]);
    assert!(all[3].value.is_err());
    assert_eq!(next_flight(&all), "a:1\nb:[2]\n");
}

#[test]
fn page_state_reports_a_syntax_error() {
    assert_eq!(
        page_state("window.X = {").unwrap_err().code,
        "unexpected-end"
    );
}

fn page(name: &str) -> String {
    let path = format!("{}/tests/pages/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn get<'a>(v: &'a Json, path: &[&str]) -> &'a Json {
    path.iter().fold(v, |v, key| match v {
        Json::Object(entries) => {
            &entries
                .iter()
                .find(|(k, _)| k == key)
                .unwrap_or_else(|| panic!("no {key}"))
                .1
        }
        Json::Array(items) => &items[key.parse::<usize>().unwrap()],
        _ => panic!("{key} in a {v:?}"),
    })
}

fn only(found: &[Found], name: &str) -> Json {
    let f = found
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no {name}"));
    f.value.clone().unwrap_or_else(|r| panic!("{name}: {r}"))
}

#[test]
fn real_pages() {
    // Next.js, App Router.
    let f = page_state(&page("nextjs-app-router.js")).unwrap();
    assert_eq!(f.len(), 2);
    let flight = next_flight(&f);
    assert!(flight.starts_with("1:\"$Sreact.fragment\"\n"), "{flight}");
    assert!(
        flight.contains("\n3:\"static/immutable/chunks/1mdrv-5-e1p7n.js\"\n"),
        "{flight}"
    );

    // Next.js, Pages Router: __NEXT_DATA__ is JSON.
    let v = json(&page("next-data-hulu.json")).unwrap();
    assert_eq!(
        get(&v, &["props", "pageProps", "pageType"]),
        &Json::String("LandingPage".into())
    );
    assert_eq!(get(&v, &["isFallback"]), &Json::Bool(false));

    // Nuxt 2: the function form.
    let f = page_state(&page("nuxt2-backmarket.js")).unwrap();
    let v = only(&f, "window.__NUXT__");
    assert_eq!(get(&v, &["layout"]), &Json::String("default".into()));
    let first = get(&v, &["data", "0", "apiHome", "top_categories", "0"]);
    assert_eq!(get(first, &["title"]), &Json::String("iPhone".into()));
    assert_eq!(get(first, &["slug_title"]), get(first, &["title"]));
    assert_eq!(get(&v, &["error"]), &Json::Null);
    assert_eq!(get(&v, &["serverRendered"]), &Json::Bool(true));
    assert_eq!(
        get(&v, &["state", "rates", "backmarket", "rate"]),
        &Json::Object(vec![])
    );
    assert_eq!(
        get(&v, &["state", "countryConfig", "sales", "0", "indice"]),
        &Json::Number(-10.0)
    );
    assert_eq!(get(&v, &["config", "DD_RUM_SAMPLE"]), &Json::Number(15.0));

    // Nuxt 3: the plain object form, and its config.
    let f = page_state(&page("nuxt3-gitlab.js")).unwrap();
    assert_eq!(only(&f, "window.__NUXT__"), Json::Object(vec![]));
    let config = only(&f, "window.__NUXT__.config");
    assert_eq!(get(&config, &["app", "baseURL"]), &Json::String("/".into()));
    assert_eq!(
        get(&config, &["public", "mdc", "headings", "anchorLinks", "h1"]),
        &Json::Bool(false)
    );

    // A Redux store, assigned with a computed name.
    let f = page_state(&page("redux-lowes.js")).unwrap();
    let v = only(&f, "window.__PRELOADED_STATE__");
    assert_eq!(get(&v, &["channel"]), &Json::String("desktop".into()));

    // JSON.parse of a string literal.
    let f = page_state(&page("json-parse-target.js")).unwrap();
    let v = only(&f, "window.__FLAGS__");
    assert_eq!(
        get(&v, &["GLOBAL_ADOBE_TAG_MANAGER_ENABLED"]),
        &Json::Bool(true)
    );

    // A top-level var.
    let f = page_state(&page("var-youtube.js")).unwrap();
    let v = only(&f, "ytInitialData");
    assert_eq!(
        get(
            &v,
            &["responseContext", "serviceTrackingParams", "0", "service"]
        ),
        &Json::String("GFEEDBACK".into())
    );
}

#[test]
fn deep_nesting_is_an_error_not_a_crash() {
    let n = 100_000;
    let deep = "[".repeat(n) + &"]".repeat(n);
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            assert_eq!(json(&deep).unwrap_err().reason, "too-deep");
            // The parser's own limit stops it first.
            assert_eq!(
                page_state(&format!("var X = {deep}")).unwrap_err().code,
                "too-deep"
            );
            let ok = "[".repeat(200) + &"]".repeat(200);
            assert!(json(&ok).is_ok());
            assert!(
                page_state(&format!("var X = {ok}")).unwrap()[0]
                    .value
                    .is_ok()
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

fn time(f: impl Fn()) -> Duration {
    let start = Instant::now();
    f();
    start.elapsed()
}

/// Four times the input takes less than eight times as long, in one of
/// five rounds: linear work passes every round, quadratic none.
fn grows_linearly(name: &str, make: impl Fn(usize) -> String) {
    let (small, large) = (make(20_000), make(80_000));
    let run = |src: &str| {
        let f = page_state(src).unwrap();
        assert!(f[0].value.is_ok(), "{name}: {:?}", f[0].value);
    };
    let rounds: Vec<_> = (0..5)
        .map(|_| (time(|| run(&small)), time(|| run(&large))))
        .collect();
    assert!(
        rounds
            .iter()
            .any(|&(a, b)| b < a * 8 + Duration::from_millis(20)),
        "{name}: {rounds:?}"
    );
}

#[test]
fn wide_input_takes_linear_time() {
    grows_linearly("array", |n| format!("var X = [{}0]", "0,".repeat(n)));
    grows_linearly("object", |n| {
        format!(
            "var X = {{{}}}",
            (0..n).map(|i| format!("k{i}:1,")).collect::<String>()
        )
    });
    grows_linearly("duplicate keys", |n| {
        format!("var X = {{{}}}", "a:1,".repeat(n))
    });
    grows_linearly("json", |n| {
        format!("var X = JSON.parse('[{}0]')", "0,".repeat(n))
    });
    grows_linearly("nuxt", |n| {
        format!("var X = (function(a){{return [{}a]}}(1))", "a,".repeat(n))
    });
}

#[test]
fn substitution_cannot_blow_up() {
    // A 100 KB argument named 10,000 times would copy a gigabyte.
    let src = format!(
        "var X = (function(a){{return [{}a]}}('{}'))",
        "a,".repeat(10_000),
        "x".repeat(100_000)
    );
    let start = Instant::now();
    assert_eq!(found(&src)[0].1, Err("too-large"));
    assert!(start.elapsed() < Duration::from_secs(5));
}
