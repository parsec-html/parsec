// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Compiles tests/capi.c against include/parsec.h and the shared library,
//! and runs it. On Windows, where there's no `cc` to call, CI builds and
//! runs the same file through CMake instead (the capi job in ci.yml).

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn the_c_test_program_passes() {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib = shared_library();
    let lib_dir = lib.parent().unwrap();
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("capi");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let built = Command::new(&cc)
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror", "-pedantic"])
        .arg("-I")
        .arg(here.join("include"))
        .arg(format!(
            "-DHTML5LIB_ENCODING=\"{}\"",
            here.join("../parsec/tests/html5lib-tests/encoding")
                .display()
        ))
        .arg(here.join("tests/capi.c"))
        .arg("-L")
        .arg(lib_dir)
        .arg("-lparsec")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()))
        .arg("-o")
        .arg(&out)
        .status()
        .unwrap_or_else(|e| panic!("couldn't run {cc}: {e}; set CC to a C compiler"));
    assert!(built.success(), "{cc} couldn't build tests/capi.c");
    let run = Command::new(&out).output().unwrap();
    print!("{}", String::from_utf8_lossy(&run.stdout));
    assert!(
        run.status.success(),
        "tests/capi.c failed:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
}

/// Where cargo puts libparsec.so or .dylib. `cargo test` builds the library
/// but only `cargo build` copies it out of its build directory, so this
/// asks cargo for it (a no-op build when it's there already).
fn shared_library() -> PathBuf {
    let name = format!("libparsec{}", std::env::consts::DLL_SUFFIX);
    let out = Command::new(env!("CARGO"))
        .args([
            "build",
            "-p",
            "parsec-capi",
            "--lib",
            "--message-format=json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .split('"')
        .find(|s| s.ends_with(&name))
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("cargo built no {name}"))
}
