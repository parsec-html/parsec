// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Builds Lexbor from the submodule at bench/lexbor, pinned to a release
//! tag, with its own default optimisation.

fn main() {
    let src = std::path::Path::new("lexbor");
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

    // Versions for the report's first line.
    let version = std::fs::read_to_string(src.join("version")).unwrap_or_default();
    let lexbor = version.trim().trim_start_matches("LEXBOR_VERSION=");
    println!("cargo:rustc-env=LEXBOR_VERSION={lexbor}");
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
}
