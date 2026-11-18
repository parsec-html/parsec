// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Decoding throughput, bytes in and UTF-8 text out: Parsec (through
//! encoding_rs) against Lexbor's encoding module. Name `encoding` to run
//! only this table.
//!
//! The inputs are real pages transcoded from UTF-8 with encoding_rs (a
//! character an encoding lacks becomes a numeric reference, as a browser's
//! form submission does), so they're mostly ASCII markup, plus a generated
//! page of dense text in each encoding's own script, which is the hard
//! case. Lexbor decodes to code points, so it's timed twice: to code
//! points only, and then to UTF-8 with its own encoder, which is what a
//! caller handing text to a parser needs. Both outputs are checked against
//! encoding_rs's before anything is timed.

use std::hint::black_box;
use std::time::Instant;

use parsec_encoding::Encoding;
use parsec_encoding::encoding_rs::{GBK, SHIFT_JIS, UTF_8, WINDOWS_1251};

use crate::pages::Page;

/// `lxb_encoding_t` values from lexbor/encoding/const.h.
const LXB_GBK: u32 = 0x06;
const LXB_SHIFT_JIS: u32 = 0x18;
const LXB_UTF_8: u32 = 0x1b;
const LXB_WINDOWS_1251: u32 = 0x20;
/// `LXB_STATUS_CONTINUE`: the input ended inside a sequence, which
/// `lxb_encoding_decode_finish_noi` then replaces.
const CONTINUE: u32 = 14;
/// `LXB_STATUS_SMALL_BUFFER`: the output buffer is full; empty it and call
/// again.
const SMALL_BUFFER: u32 = 15;

/// Room for `lxb_encoding_decode_t` or `lxb_encoding_encode_t`, which are
/// under 128 bytes; Lexbor's `_init_noi` functions fill them in.
#[repr(C, align(16))]
struct Ctx([u8; 256]);

#[repr(C)]
struct LxbEncodingData {
    _private: [u8; 0],
}

type DecodeFn = unsafe extern "C" fn(*mut Ctx, *mut *const u8, *const u8) -> u32;

unsafe extern "C" {
    fn lxb_encoding_data(encoding: u32) -> *const LxbEncodingData;
    fn lxb_encoding_decode_function(encoding: u32) -> DecodeFn;
    fn lxb_encoding_decode_init_noi(
        ctx: *mut Ctx,
        data: *const LxbEncodingData,
        out: *mut u32,
        len: usize,
    ) -> u32;
    fn lxb_encoding_decode_replace_set_noi(ctx: *mut Ctx, replace: *const u32, len: usize) -> u32;
    fn lxb_encoding_decode_buf_used_noi(ctx: *mut Ctx) -> usize;
    fn lxb_encoding_decode_buf_used_set_noi(ctx: *mut Ctx, used: usize);
    fn lxb_encoding_decode_finish_noi(ctx: *mut Ctx) -> u32;
    fn lxb_encoding_encode_init_noi(
        ctx: *mut Ctx,
        data: *const LxbEncodingData,
        out: *mut u8,
        len: usize,
    ) -> u32;
    fn lxb_encoding_encode_buf_used_noi(ctx: *mut Ctx) -> usize;
    fn lxb_encoding_encode_buf_used_set_noi(ctx: *mut Ctx, used: usize);
    fn lxb_encoding_encode_utf_8(ctx: *mut Ctx, cps: *mut *const u32, end: *const u32) -> u32;
}

const REPLACEMENT: [u32; 1] = [0xFFFD];

/// Lexbor's decoder over `bytes`, into one buffer with room for all of it.
/// Lexbor's own examples decode in small batches, but in 3.0 a
/// single-byte decoder drops the byte that meets a full buffer (fixed after
/// 3.0.1, lexbor#425), and one batch is also its fastest case.
fn lexbor_decode(encoding: u32, bytes: &[u8], mut sink: impl FnMut(&[u32])) {
    let mut cps = vec![0u32; bytes.len() + 4];
    let mut ctx = Ctx([0; 256]);
    // SAFETY: `ctx` is big enough and aligned for Lexbor's decode context,
    // `cps` outlives it, and `data` walks only `bytes`.
    unsafe {
        let data = lxb_encoding_data(encoding);
        let decode = lxb_encoding_decode_function(encoding);
        assert_eq!(
            lxb_encoding_decode_init_noi(&mut ctx, data, cps.as_mut_ptr(), cps.len()),
            0
        );
        assert_eq!(
            lxb_encoding_decode_replace_set_noi(&mut ctx, REPLACEMENT.as_ptr(), 1),
            0
        );
        let mut at = bytes.as_ptr();
        let end = at.add(bytes.len());
        loop {
            let status = decode(&mut ctx, &mut at, end);
            sink(&cps[..lxb_encoding_decode_buf_used_noi(&mut ctx)]);
            lxb_encoding_decode_buf_used_set_noi(&mut ctx, 0);
            if status != SMALL_BUFFER {
                assert!(status == 0 || status == CONTINUE, "Lexbor's decoder failed");
                break;
            }
        }
        lxb_encoding_decode_finish_noi(&mut ctx);
        sink(&cps[..lxb_encoding_decode_buf_used_noi(&mut ctx)]);
    }
}

/// Lexbor decoding to code points, which it then drops: the count.
fn lexbor_code_points(encoding: u32, bytes: &[u8]) -> usize {
    let mut n = 0;
    lexbor_decode(encoding, bytes, |cps| n += black_box(cps).len());
    n
}

/// Lexbor decoding and then encoding the code points as UTF-8.
fn lexbor_utf8(encoding: u32, bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len() * 3 / 2);
    let mut buf = vec![0u8; 16384];
    let mut ctx = Ctx([0; 256]);
    // SAFETY: as in `lexbor_decode`, for the encode context and `buf`.
    unsafe {
        let utf8 = lxb_encoding_data(LXB_UTF_8);
        assert_eq!(
            lxb_encoding_encode_init_noi(&mut ctx, utf8, buf.as_mut_ptr(), buf.len()),
            0
        );
    }
    lexbor_decode(encoding, bytes, |cps| {
        let mut at = cps.as_ptr();
        // SAFETY: `at` walks only `cps`; `buf` outlives the context.
        unsafe {
            let end = at.add(cps.len());
            loop {
                let status = lxb_encoding_encode_utf_8(&mut ctx, &mut at, end);
                out.extend_from_slice(&buf[..lxb_encoding_encode_buf_used_noi(&mut ctx)]);
                lxb_encoding_encode_buf_used_set_noi(&mut ctx, 0);
                if status != SMALL_BUFFER {
                    assert_eq!(status, 0, "Lexbor's encoder failed");
                    break;
                }
            }
        }
    });
    out
}

fn parsec(encoding: &'static Encoding, bytes: &[u8]) -> usize {
    encoding.decode_without_bom_handling(bytes).0.len()
}

/// Median seconds of `f` over enough runs to fill about half a second.
fn median(mut f: impl FnMut() -> usize) -> f64 {
    for _ in 0..3 {
        black_box(f());
    }
    let t = Instant::now();
    black_box(f());
    let once = t.elapsed().as_secs_f64().max(1e-6);
    let runs = (0.5 / once).clamp(30.0, 2000.0) as usize;
    let mut times: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            black_box(f());
            t.elapsed().as_secs_f64()
        })
        .collect();
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

/// A page of paragraphs of every character from `from` up that `encoding`
/// can write, about 1 MB once encoded.
fn dense(encoding: &'static Encoding, from: u32) -> Vec<u8> {
    let chars: Vec<char> = (from..0xFFFF)
        .filter_map(char::from_u32)
        .filter(|c| !encoding.encode(&c.to_string()).2)
        .take(3000)
        .collect();
    let mut text = String::from("<!doctype html><title>dense</title>");
    while encoding.encode(&text).0.len() < 1_000_000 {
        for line in chars.chunks(80) {
            text.push_str("<p>");
            text.extend(line);
            text.push_str("</p>\n");
        }
    }
    encoding.encode(&text).0.into_owned()
}

pub fn run(pages: &[Page]) {
    println!(
        "\nDecoding to UTF-8, MB/s of input\n{:<34} {:>8}  {:>8}  {:>14}  {:>12}  {:>9}",
        "", "size", "Parsec", "Lexbor (UTF-8)", "Lexbor (cps)", "vs Lexbor"
    );
    let encodings: [(&'static Encoding, u32, u32); 4] = [
        (UTF_8, LXB_UTF_8, 0x4E00),
        (SHIFT_JIS, LXB_SHIFT_JIS, 0x4E00),
        (WINDOWS_1251, LXB_WINDOWS_1251, 0x0400),
        (GBK, LXB_GBK, 0x4E00),
    ];
    let real = ["wikipedia-population", "nytimes"];
    for (encoding, lxb, from) in encodings {
        println!("{}", encoding.name());
        let mut inputs: Vec<(String, Vec<u8>)> = pages
            .iter()
            .filter(|p| real.contains(&p.name.as_str()))
            .map(|p| (p.name.clone(), encoding.encode(&p.html).0.into_owned()))
            .collect();
        inputs.push(("dense text, generated".into(), dense(encoding, from)));
        for (name, bytes) in inputs {
            let want = encoding.decode_without_bom_handling(&bytes).0;
            let lexbor = lexbor_utf8(lxb, &bytes);
            let same = lexbor == want.as_bytes();
            if let Some(at) = (!same)
                .then(|| lexbor.iter().zip(want.as_bytes()).position(|(a, b)| a != b))
                .flatten()
            {
                let near = |b: &[u8]| {
                    String::from_utf8_lossy(&b[at.saturating_sub(8)..(at + 8).min(b.len())])
                        .into_owned()
                };
                eprintln!(
                    "{} {name}: first difference at byte {at}, Lexbor {:?}, spec {:?}",
                    encoding.name(),
                    near(&lexbor),
                    near(want.as_bytes())
                );
            }
            let mb = bytes.len() as f64 / 1e6;
            let p = median(|| parsec(encoding, &bytes));
            let l = median(|| lexbor_utf8(lxb, &bytes).len());
            let c = median(|| lexbor_code_points(lxb, &bytes));
            println!(
                "  {:<32} {:>5.2} MB  {:>8.0}  {:>14.0}  {:>12.0}  {:>8.1}x{}",
                name,
                mb,
                mb / p,
                mb / l,
                mb / c,
                l / p,
                if same { "" } else { "  Lexbor's text differs" }
            );
        }
    }
    wpt_pages();
}

/// Lexbor's decoders over each character of the web-platform-tests
/// `*_chars.html` pages that crates/parsec-encoding's tests run, against
/// encoding_rs, which passes all of them.
fn wpt_pages() {
    const LXB_BIG5: u32 = 0x03;
    const LXB_EUC_JP: u32 = 0x04;
    const LXB_EUC_KR: u32 = 0x05;
    const LXB_ISO_2022_JP: u32 = 0x08;
    use parsec_encoding::encoding_rs::{BIG5, EUC_JP, EUC_KR, ISO_2022_JP};
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/parsec-encoding/tests/wpt/"
    );
    let files: [(&str, &'static Encoding, u32); 6] = [
        (
            "legacy-mb-japanese/shift_jis/sjis_chars.html",
            SHIFT_JIS,
            LXB_SHIFT_JIS,
        ),
        (
            "legacy-mb-japanese/euc-jp/eucjp_chars.html",
            EUC_JP,
            LXB_EUC_JP,
        ),
        (
            "legacy-mb-japanese/iso-2022-jp/iso2022jp_chars.html",
            ISO_2022_JP,
            LXB_ISO_2022_JP,
        ),
        (
            "legacy-mb-korean/euc-kr/euckr_chars.html",
            EUC_KR,
            LXB_EUC_KR,
        ),
        ("legacy-mb-tchinese/big5/big5_chars.html", BIG5, LXB_BIG5),
        (
            "legacy-mb-tchinese/big5/big5_chars_extra.html",
            BIG5,
            LXB_BIG5,
        ),
    ];
    println!("\nweb-platform-tests characters Lexbor decodes as the spec does (Parsec: all)");
    for (file, encoding, lxb) in files {
        let page = std::fs::read(format!("{dir}{file}")).expect("the WPT pages");
        let page = String::from_utf8_lossy(&page);
        let (mut total, mut lexbor_ok) = (0, 0);
        for part in page.split("data-bytes=\"").skip(1) {
            let hex = &part[..part.find('"').unwrap()];
            let bytes: Vec<u8> = hex
                .split(' ')
                .map(|b| u8::from_str_radix(b, 16).unwrap())
                .collect();
            let want = encoding.decode_without_bom_handling(&bytes).0;
            let mut cps = Vec::new();
            lexbor_decode(lxb, &bytes, |c| cps.extend_from_slice(c));
            let lexbor: String = cps.iter().filter_map(|&c| char::from_u32(c)).collect();
            total += 1;
            lexbor_ok += usize::from(lexbor == want);
        }
        println!(
            "  {:<28} Lexbor {lexbor_ok:>6} of {total:>6}",
            file.rsplit('/').next().unwrap()
        );
    }
}
