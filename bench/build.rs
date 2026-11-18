// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Builds Lexbor from the submodule at bench/lexbor and Ada from the one at
//! bench/ada, each pinned to a release tag: Lexbor with its own default
//! optimisation, Ada at -O3 as its CMake release build does.

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
