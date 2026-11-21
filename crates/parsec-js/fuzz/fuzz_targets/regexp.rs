// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any pattern validates or fails without panicking, with each set of
//! flags that changes the grammar, and every error's span is inside the
//! pattern and on character boundaries.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_js::regexp::validate;

fuzz_target!(|data: &[u8]| {
    let pattern = String::from_utf8_lossy(data);
    for flags in ["", "u", "v", "gimsyd"] {
        if let Err(d) = validate(&pattern, flags) {
            let (start, end) = (d.span.start as usize, d.span.end as usize);
            assert!(
                start <= end
                    && end <= pattern.len()
                    && pattern.is_char_boundary(start)
                    && pattern.is_char_boundary(end),
                "{d}"
            );
        }
    }
});
