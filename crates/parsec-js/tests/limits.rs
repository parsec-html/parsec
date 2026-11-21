// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Sources built to be deep or to blow up: each parses, or fails with
//! an error, on a small stack and in time that grows with the input, not
//! faster.

use std::time::{Duration, Instant};

use parsec_js::{DEFAULT_MAX_DEPTH, Limits, Options, SourceType, parse, parse_script};

/// The stack each case runs on: a debug build's parser needs about 700 KB
/// at the default depth, a release build's about 250 KB.
const STACK: usize = 1 << 20;

/// Runs `f` on a thread with `stack` bytes of stack, failing if it takes
/// longer than a quadratic path would let it finish in.
fn bounded(name: &str, stack: usize, f: impl FnOnce() + Send + 'static) {
    let start = Instant::now();
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap_or_else(|_| panic!("{name} panicked"));
    let took = start.elapsed();
    assert!(took < Duration::from_secs(20), "{name} took {took:?}");
}

fn time(src: &str) -> Duration {
    let start = Instant::now();
    if let Ok(ast) = std::hint::black_box(parse_script(src)) {
        std::hint::black_box(ast.to_estree(src));
    }
    start.elapsed()
}

/// Fails if four times the input takes more than eight times as long:
/// linear work takes four, quadratic sixteen. The two sizes are timed
/// back to back, five times, and one round inside the limit passes, so a
/// slow spell on a loaded machine can't fail it and quadratic work fails
/// every round.
fn grows_linearly(name: &str, make: impl Fn(usize) -> String) {
    let (small, large) = (make(5_000), make(20_000));
    let rounds: Vec<_> = (0..5).map(|_| (time(&small), time(&large))).collect();
    assert!(
        rounds
            .iter()
            .any(|&(a, b)| b < a * 8 + Duration::from_millis(20)),
        "{name}: {} bytes, then {} bytes, took {rounds:?}",
        small.len(),
        large.len()
    );
}

/// Sources that nest `n` deep, one way each.
fn deep(n: usize) -> Vec<(&'static str, String)> {
    vec![
        ("parentheses", "(".repeat(n) + "a" + &")".repeat(n)),
        ("arrays", "[".repeat(n) + &"]".repeat(n)),
        (
            "objects",
            "x = ".to_string() + &"{a:".repeat(n) + "1" + &"}".repeat(n),
        ),
        ("blocks", "{".repeat(n) + &"}".repeat(n)),
        ("ifs", "if (a) ".repeat(n) + ";"),
        ("loops", "while (a) ".repeat(n) + ";"),
        (
            "labels",
            (0..n).map(|i| format!("l{i}: ")).collect::<String>() + ";",
        ),
        ("functions", "function f() {".repeat(n) + &"}".repeat(n)),
        ("classes", "class A { m() {".repeat(n) + &"} }".repeat(n)),
        ("arrows", "a => ".repeat(n) + "a"),
        ("conditionals", "a ? b : ".repeat(n) + "c"),
        ("assignments", "a = ".repeat(n) + "b"),
        ("prefix operators", "!".repeat(n) + "a"),
        ("new", "new ".repeat(n) + "A"),
        ("exponents", "a ** ".repeat(n) + "a"),
        ("calls", "f(".repeat(n) + &")".repeat(n)),
        ("templates", "`${".repeat(n) + "a" + &"}`".repeat(n)),
        ("computed keys", "({[".repeat(n) + "a" + &"]: 1})".repeat(n)),
        (
            "binding patterns",
            "let ".to_string() + &"[".repeat(n) + &"]".repeat(n) + " = a",
        ),
        (
            "assignment patterns",
            "[".repeat(n) + &"]".repeat(n) + " = a",
        ),
        (
            "arrow parameters",
            "(".to_string() + &"[".repeat(n) + &"]".repeat(n) + ") => 1",
        ),
    ]
}

#[test]
fn a_million_deep_parentheses_are_an_error_not_a_stack_overflow() {
    let src = "(".repeat(1_000_000);
    bounded("1,000,000 (", STACK, move || {
        let err = parse_script(&src).unwrap_err();
        assert_eq!(err.code, "too-deep");
        // It stops where the limit is, not at the end of the input.
        assert!(err.span.start < 1_000, "{err}");
    });
    // The time doesn't grow with the input past the limit.
    grows_linearly("( past the limit", |n| "(".repeat(n * 50));
}

#[test]
fn deep_nesting_of_every_kind_is_an_error_not_a_stack_overflow() {
    for (name, src) in deep(100_000) {
        bounded(name, STACK, move || {
            let err = parse_script(&src).unwrap_err();
            assert_eq!(err.code, "too-deep", "{name}: {err}");
        });
    }
}

#[test]
fn nesting_inside_the_limit_parses_and_writes_out() {
    for (name, src) in deep(DEFAULT_MAX_DEPTH as usize / 8) {
        bounded(name, STACK, move || {
            let ast = parse_script(&src).unwrap_or_else(|d| panic!("{name}: {d}"));
            let json = ast.to_estree(&src);
            assert!(json.ends_with(r#""sourceType":"script"}"#), "{name}");
        });
    }
}

#[test]
fn the_limits_are_the_ones_asked_for() {
    let with = |limits: Limits| Options {
        limits,
        ..Options::default()
    };
    let depth = |n| {
        with(Limits {
            max_depth: Some(n),
            ..Limits::default()
        })
    };
    // An expression statement is two levels, and each parenthesis one more.
    let parens = |n| "(".repeat(n) + "a" + &")".repeat(n);
    assert!(parse(&parens(8), &depth(10)).is_ok());
    assert_eq!(parse(&parens(9), &depth(10)).unwrap_err().code, "too-deep");
    // A higher limit is for a thread with the stack it needs, about 3 KB
    // a level in a debug build.
    let deep = parens(2_000);
    bounded("a raised limit", 64 << 20, move || {
        let options = Options {
            limits: Limits {
                max_depth: Some(2_100),
                ..Limits::default()
            },
            ..Options::default()
        };
        assert!(parse(&deep, &options).is_ok());
    });

    // The step budget counts tokens.
    let src = "a;".repeat(1_000);
    let steps = |n| {
        with(Limits {
            max_steps: Some(n),
            ..Limits::default()
        })
    };
    assert!(parse(&src, &steps(2_000)).is_ok());
    let err = parse(&src, &steps(1_999)).unwrap_err();
    assert_eq!(err.code, "step-budget");

    let input = with(Limits {
        max_input: Some(1_000),
        ..Limits::default()
    });
    assert!(parse(&src[..1_000], &input).is_ok());
    assert_eq!(parse(&src, &input).unwrap_err().code, "input-too-large");
}

/// Sources `n` wide, each at a place where a parser might look back over
/// what it already read.
fn wide(n: usize) -> Vec<(&'static str, String)> {
    vec![
        ("binary operators", "a".to_string() + &" + a".repeat(n)),
        ("logical operators", "a".to_string() + &" || a".repeat(n)),
        ("member accesses", "a".to_string() + &".b".repeat(n)),
        ("calls", "a".to_string() + &"()".repeat(n)),
        ("optional chains", "a".to_string() + &"?.b".repeat(n)),
        ("sequences", "a".to_string() + &", a".repeat(n)),
        ("statements", "a;".repeat(n)),
        (
            "declarations",
            "let a0".to_string() + &(1..n).map(|i| format!(", a{i}")).collect::<String>(),
        ),
        (
            "parameters",
            "function f(".to_string()
                + &(0..n).map(|i| format!("a{i},")).collect::<String>()
                + ") { 'use strict' }",
        ),
        (
            "arrow parameters",
            "(".to_string() + &(0..n).map(|i| format!("a{i},")).collect::<String>() + ") => 1",
        ),
        ("array elements", "[".to_string() + &"a,".repeat(n) + "]"),
        ("holes", "[".to_string() + &",".repeat(n) + "]"),
        (
            "object properties",
            "({".to_string() + &(0..n).map(|i| format!("a{i}: 1,")).collect::<String>() + "})",
        ),
        (
            "shorthand defaults",
            "({".to_string() + &(0..n).map(|i| format!("a{i} = 1,")).collect::<String>() + "} = b)",
        ),
        (
            "objects with defaults",
            "[".to_string() + &"{a = 1},".repeat(n) + "] = b",
        ),
        (
            "private names",
            "class A {".to_string()
                + &(0..n)
                    .map(|i| format!("#a{i}; m{i}() {{ this.#a{i} }}"))
                    .collect::<String>()
                + "}",
        ),
        (
            "labels",
            (0..n).map(|i| format!("l{i}: ;")).collect::<String>(),
        ),
        ("else if", "if (a) b; else ".repeat(n) + "c"),
        (
            "cases",
            "switch (a) {".to_string() + &"case 1: b;".repeat(n) + "}",
        ),
        (
            "template substitutions",
            "`".to_string() + &"${a}".repeat(n) + "`",
        ),
        (
            "string escapes",
            "'".to_string() + &"\\u0041".repeat(n) + "'",
        ),
        (
            "comments before lookahead",
            "let /* x */ a = 1;".repeat(n / 2) + &"async /* x */ function f() {}".repeat(n / 2),
        ),
        (
            "nested patterns",
            "[".repeat(50) + &"a,".repeat(n) + &"] = 1".repeat(50),
        ),
    ]
}

#[test]
fn wide_inputs_parse_in_linear_time() {
    for (name, _) in wide(1) {
        let make = |n: usize| wide(n).into_iter().find(|(k, _)| *k == name).unwrap().1;
        assert!(
            parse_script(&make(100)).is_ok(),
            "{name}: {:?}",
            parse_script(&make(100)).err()
        );
        grows_linearly(name, make);
    }
}

#[test]
fn deep_trees_write_out_on_a_small_stack() {
    // A member chain or a run of `+` is a tree a million deep, built in a
    // loop; writing it out mustn't recurse either.
    for (name, src) in [
        ("member chain", "a".to_string() + &".b".repeat(1_000_000)),
        ("binary chain", "a".to_string() + &"+a".repeat(1_000_000)),
    ] {
        bounded(name, 256 << 10, move || {
            let ast = parse_script(&src).unwrap();
            let json = ast.to_estree(&src);
            assert!(json.len() > src.len());
        });
    }
}

#[test]
fn modules_have_the_same_limits() {
    let options = Options {
        source_type: SourceType::Module,
        ..Options::default()
    };
    let src = "export default ".to_string() + &"[".repeat(1_000_000);
    bounded("module", STACK, move || {
        assert_eq!(parse(&src, &options).unwrap_err().code, "too-deep");
    });
}
