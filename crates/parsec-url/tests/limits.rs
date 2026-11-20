// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Punycode takes time that grows with the square of a label, so labels
//! over punycode::MAX_LABEL fail rather than take seconds.

use std::time::{Duration, Instant};

use parsec_url::{Url, domain_to_ascii, domain_to_unicode, punycode};

fn cjk(n: usize) -> String {
    (0..n as u32)
        .map(|i| char::from_u32(0x4E00 + i % 20_000).unwrap())
        .collect()
}

#[test]
fn long_labels_fail_fast() {
    let start = Instant::now();
    let host = cjk(67_759);
    assert!(Url::parse(&format!("http://{host}/")).is_err());
    assert!(domain_to_ascii(&host).is_err());
    assert_eq!(punycode::encode_str(&host), None);
    let label = format!("xn--{}", "a".repeat(1 << 20));
    let (unicode, took) = (domain_to_unicode(&label), start.elapsed());
    assert_eq!(unicode, label, "an undecodable label is kept as it is");
    assert_eq!(punycode::decode_to_string(&label[4..]), None);
    assert!(took < Duration::from_secs(2), "took {took:?}");
}

#[test]
fn labels_up_to_the_limit_still_work() {
    let label = cjk(punycode::MAX_LABEL);
    let encoded = punycode::encode_str(&label).unwrap();
    assert_eq!(
        punycode::decode_to_string(&encoded).as_deref(),
        Some(&*label)
    );
    assert_eq!(punycode::encode_str(&cjk(punycode::MAX_LABEL + 1)), None);
    assert_eq!(
        Url::parse("http://bücher.example/").unwrap().host(),
        "xn--bcher-kva.example"
    );
}
