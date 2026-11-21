// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Prints each file's syntax tree as ESTree JSON, one line per file, or
//! its syntax error.
//!
//! ```text
//! cargo run --example estree -- app.js
//! cargo run --example estree -- --module --loc app.mjs
//! ```

use parsec_js::{EstreeOptions, Options, SourceType, parse};

fn main() {
    let mut options = Options::default();
    let mut estree = EstreeOptions::default();
    let mut status = 0;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--module" => options.source_type = SourceType::Module,
            "--loc" => estree.loc = true,
            path => {
                let src = match std::fs::read_to_string(path) {
                    Ok(src) => src,
                    Err(e) => {
                        eprintln!("{path}: {e}");
                        status = 1;
                        continue;
                    }
                };
                match parse(&src, &options) {
                    Ok(ast) => println!("{}", ast.to_estree_with(&src, estree)),
                    Err(d) => {
                        println!("{{\"error\":\"{}\"}}", d.code);
                        eprintln!("{path}: {d}");
                        status = 1;
                    }
                }
            }
        }
    }
    std::process::exit(status);
}
