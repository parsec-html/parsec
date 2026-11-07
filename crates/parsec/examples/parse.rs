// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Times a whole parse over a file: `cargo run --release --example parse FILE`.

use std::time::Instant;

fn main() {
    let html = std::fs::read_to_string(std::env::args().nth(1).expect("a file")).unwrap();
    let mut best = f64::MAX;
    let mut nodes = 0;
    for _ in 0..300 {
        let t = Instant::now();
        let doc = parsec::parse(&html, false);
        best = best.min(t.elapsed().as_secs_f64());
        nodes = doc.len();
    }
    println!("{:.3} ms ({nodes} nodes)", best * 1000.0);
}
