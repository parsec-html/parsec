// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Builds Lexbor from the submodule at bench/lexbor (with src/lexbor_css.c,
//! the CSS benchmark's calls into it), Ada from the one at bench/ada and
//! libxml2 from the one at bench/libxml2, each pinned to a release tag:
//! Lexbor with its own default optimisation, Ada at -O3 as its CMake
//! release build does, libxml2 as its CMake release build does.

use std::path::Path;

fn main() {
    let src = Path::new("lexbor");
    if !src.join("CMakeLists.txt").exists() {
        panic!("bench/lexbor is empty: run `git submodule update --init bench/lexbor`");
    }
    let out = cmake::Config::new(src)
        .define("LEXBOR_BUILD_SHARED", "OFF")
        .define("LEXBOR_BUILD_STATIC", "ON")
        .define("LEXBOR_BUILD_TESTS", "OFF")
        .define("LEXBOR_BUILD_EXAMPLES", "OFF")
        .define("LEXBOR_INSTALL_HEADERS", "OFF")
        .profile("Release")
        .build();
    // The CSS benchmark's calls into Lexbor, built against its headers.
    cc::Build::new()
        .include(src.join("source"))
        .define("LEXBOR_STATIC", None)
        .file("src/lexbor_css.c")
        .opt_level(2)
        .compile("lexbor_css");
    println!("cargo:rerun-if-changed=src/lexbor_css.c");
    println!("cargo:rustc-link-search=native={}/lib", out.display());
    println!("cargo:rustc-link-lib=static=lexbor_static");
    println!("cargo:rerun-if-changed=lexbor/version");

    if !Path::new("ada/src/ada.cpp").exists() {
        panic!("bench/ada is empty: run `git submodule update --init bench/ada`");
    }
    // ada.cpp includes the rest of Ada's sources. NDEBUG turns off its
    // development checks, as a release build does, and URL patterns (which
    // pull in a regex engine) aren't timed.
    cc::Build::new()
        .cpp(true)
        .std("c++20")
        .opt_level(3)
        .define("NDEBUG", None)
        .define("ADA_INCLUDE_URL_PATTERN", "0")
        .include("ada/include")
        .file("ada/src/ada.cpp")
        .file("src/ada.cpp")
        .warnings(false)
        .compile("ada");
    println!("cargo:rerun-if-changed=src/ada.cpp");
    println!("cargo:rerun-if-changed=ada/include/ada/ada_version.h");

    // libxml2, without what the benchmark doesn't use: no iconv, zlib,
    // ICU or Python, no programs or tests.
    let libxml2 = Path::new("libxml2");
    if !libxml2.join("CMakeLists.txt").exists() {
        panic!("bench/libxml2 is empty: run `git submodule update --init bench/libxml2`");
    }
    let out = cmake::Config::new(libxml2)
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("LIBXML2_WITH_ICONV", "OFF")
        .define("LIBXML2_WITH_ICU", "OFF")
        .define("LIBXML2_WITH_LZMA", "OFF")
        .define("LIBXML2_WITH_ZLIB", "OFF")
        .define("LIBXML2_WITH_PYTHON", "OFF")
        .define("LIBXML2_WITH_PROGRAMS", "OFF")
        .define("LIBXML2_WITH_TESTS", "OFF")
        .define("LIBXML2_WITH_MODULES", "OFF")
        .define("LIBXML2_WITH_DOCS", "OFF")
        .profile("Release")
        .build();
    println!("cargo:rustc-link-search=native={}/lib", out.display());
    if cfg!(windows) {
        println!("cargo:rustc-link-lib=static=libxml2s");
    } else {
        println!("cargo:rustc-link-lib=static=xml2");
        println!("cargo:rustc-link-lib=m");
    }
    println!("cargo:rerun-if-changed=libxml2/VERSION");
    let libxml2_version = std::fs::read_to_string(libxml2.join("VERSION")).unwrap_or_default();
    println!("cargo:rustc-env=LIBXML2_VERSION={}", libxml2_version.trim());

    // Versions for the report's first line.
    let version = std::fs::read_to_string(src.join("version")).unwrap_or_default();
    let lexbor = version.trim().trim_start_matches("LEXBOR_VERSION=");
    println!("cargo:rustc-env=LEXBOR_VERSION={lexbor}");
    let ada = std::fs::read_to_string("ada/include/ada/ada_version.h").unwrap_or_default();
    let ada = ada
        .lines()
        .find_map(|l| l.strip_prefix("#define ADA_VERSION \""))
        .map_or("?", |v| v.trim_end_matches('"'));
    println!("cargo:rustc-env=ADA_VERSION={ada}");
    let lock = std::fs::read_to_string("Cargo.lock").unwrap_or_default();
    let html5ever = lock
        .split("[[package]]")
        .find(|p| p.contains("name = \"html5ever\""))
        .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = ")))
        .map_or("?", |v| v.trim_matches('"'));
    println!("cargo:rustc-env=HTML5EVER_VERSION={html5ever}");
    for (name, var) in [
        ("roxmltree", "ROXMLTREE_VERSION"),
        ("quick-xml", "QUICK_XML_VERSION"),
        ("oxc_parser", "OXC_VERSION"),
        ("swc_ecma_parser", "SWC_VERSION"),
    ] {
        let version = lock
            .split("[[package]]")
            .find(|p| p.contains(&format!("name = \"{name}\"")))
            .and_then(|p| p.lines().find_map(|l| l.strip_prefix("version = ")))
            .map_or("?", |v| v.trim_matches('"'));
        println!("cargo:rustc-env={var}={version}");
    }
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "?".into());
    println!("cargo:rustc-env=PARSEC_COMMIT={commit}");
    println!("cargo:rerun-if-changed=Cargo.lock");
    println!("cargo:rerun-if-changed=../.git/HEAD");
    // A commit moves the branch, not HEAD, which only names the branch.
    if let Some(branch) = std::process::Command::new("git")
        .args(["symbolic-ref", "-q", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
    {
        let branch = String::from_utf8_lossy(&branch.stdout);
        println!("cargo:rerun-if-changed=../.git/{}", branch.trim());
    }
}
