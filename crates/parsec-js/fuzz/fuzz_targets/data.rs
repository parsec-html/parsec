// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any input read as page state, and as JSON, gives values or refusals
//! without panicking: every span is inside the input, and every value
//! writes out as JSON that reads back as the same text.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_js::data::{Json, json, next_flight, page_state};

fn round_trips(v: &Json) {
    let text = v.to_json_string();
    let again = json(&text).unwrap_or_else(|r| panic!("{text}: {r}"));
    assert_eq!(again.to_json_string(), text);
}

fuzz_target!(|data: &[u8]| {
    let src = String::from_utf8_lossy(data);
    if let Ok(found) = page_state(&src) {
        for f in &found {
            assert!(f.span.end as usize <= src.len(), "{f:?}");
            match &f.value {
                Ok(v) => round_trips(v),
                Err(r) => assert!(
                    r.span.start <= r.span.end && r.span.end as usize <= src.len(),
                    "{r}"
                ),
            }
        }
        let _ = next_flight(&found);
    }
    match json(&src) {
        Ok(v) => round_trips(&v),
        Err(r) => assert!(r.span.end as usize <= src.len(), "{r}"),
    }
});
