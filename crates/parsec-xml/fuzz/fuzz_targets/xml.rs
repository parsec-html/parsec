// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Any bytes parse without panicking, with namespaces on and off, and
//! within limits small enough that no input runs long. A document that
//! parses has spans inside its text, on character boundaries, and writes
//! out to XML that parses back to the same XML.

#![no_main]

use libfuzzer_sys::fuzz_target;
use parsec_xml::{Limits, Options, parse_bytes, parse_with};

fuzz_target!(|data: &[u8]| {
    for namespaces in [true, false] {
        let options = Options {
            limits: Limits {
                max_depth: Some(256),
                max_steps: Some(100_000),
                ..Limits::default()
            },
            namespaces,
            entity_allowance: 1 << 16,
            entity_ratio: 4,
            ..Options::default()
        };
        let Ok(parsed) = parse_bytes(data, &options) else {
            continue;
        };
        let (doc, text) = (&parsed.document, &*parsed.text);
        let inside = |s: parsec_xml::Span| {
            s.start <= s.end
                && text.is_char_boundary(s.start as usize)
                && text.is_char_boundary(s.end as usize)
        };
        for n in doc.descendants(doc.root()) {
            assert!(inside(doc.span(n)), "{:?}", doc.span(n));
            for a in doc.attributes(n) {
                assert!(inside(a.span), "{:?}", a.span);
            }
        }
        for w in doc.warnings() {
            assert!(inside(w.span) && !w.is_error());
        }
        let xml = doc.to_xml();
        let again = parse_with(&xml, &options)
            .unwrap_or_else(|e| panic!("written out, it doesn't parse: {e}\n{xml}"));
        assert_eq!(again.to_xml(), xml);
    }
});
