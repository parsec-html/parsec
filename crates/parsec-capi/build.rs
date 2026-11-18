// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

fn main() {
    // Without this a dylib records the absolute path it was built at, and
    // a program linked against a prebuilt copy looks for it there.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rustc-cdylib-link-arg=-Wl,-install_name,@rpath/libparsec.dylib");
    }
}
