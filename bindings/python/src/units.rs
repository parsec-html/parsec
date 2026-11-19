// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Offsets in the caller's units. The core counts UTF-8 bytes into the
//! text it parsed; a caller slices what it passed in, which is code points
//! for a Python `str`, bytes in their own encoding for `bytes`, and UTF-16
//! units for a JavaScript string. The same `Counted` is in
//! bindings/wasm/src/units.rs, counting UTF-16 instead.

use std::sync::OnceLock;

use parsec_encoding::{Encoding, Offsets};

const BLOCK: usize = 64;

/// The parsed text, kept so a byte offset can be counted out in other
/// units: a table of the units before each 64-byte block, built on the
/// first lookup, and a count within the block.
pub struct Counted {
    src: Box<[u8]>,
    /// How many units the character a byte starts takes; 0 for a UTF-8
    /// continuation byte.
    width: fn(u8) -> u32,
    blocks: OnceLock<Vec<u32>>,
}

impl Counted {
    pub fn new(src: &str, width: fn(u8) -> u32) -> Counted {
        Counted {
            src: src.as_bytes().into(),
            width,
            blocks: OnceLock::new(),
        }
    }

    fn blocks(&self) -> &[u32] {
        self.blocks.get_or_init(|| {
            let mut out = Vec::with_capacity(self.src.len() / BLOCK + 1);
            let mut units = 0;
            for block in self.src.chunks(BLOCK) {
                out.push(units);
                units += block.iter().map(|&b| (self.width)(b)).sum::<u32>();
            }
            out.push(units);
            out
        })
    }

    /// The units before byte `offset`.
    pub fn to_units(&self, offset: usize) -> usize {
        let at = offset.min(self.src.len());
        let block = at / BLOCK;
        let within: u32 = self.src[block * BLOCK..at]
            .iter()
            .map(|&b| (self.width)(b))
            .sum();
        (self.blocks()[block] + within) as usize
    }

    /// The byte offset of the first character at or after `units` units.
    pub fn to_bytes(&self, units: usize) -> usize {
        let blocks = self.blocks();
        // The last block that starts at or before `units`.
        let block = blocks.partition_point(|&u| u as usize <= units).max(1) - 1;
        let mut at = (block * BLOCK).min(self.src.len());
        let mut count = blocks[block] as usize;
        while at < self.src.len() {
            let w = (self.width)(self.src[at]) as usize;
            if w > 0 && count >= units {
                break;
            }
            count += w;
            at += 1;
        }
        at
    }
}

/// Code points: every byte but a continuation byte starts one.
pub fn code_point(b: u8) -> u32 {
    u32::from((b as i8) >= -0x40)
}

/// How the core's offsets become the caller's.
pub enum Units {
    /// They're the same: an ASCII `str`, or `bytes` that decoded to the
    /// same bytes (ASCII, or UTF-8 with nothing replaced).
    Same,
    /// A `str` with more than ASCII in it: code point indexes.
    Chars(Counted),
    /// Any other `bytes`: byte offsets into them, in the encoding they
    /// were decoded from.
    Bytes(Offsets),
}

impl Units {
    /// For a `str` the parse was given as `text`.
    pub fn of_str(text: &str) -> Units {
        if text.is_ascii() {
            Units::Same
        } else {
            Units::Chars(Counted::new(text, code_point))
        }
    }

    /// For `bytes`, which the parse was given decoded from `encoding`.
    pub fn of_bytes(bytes: &[u8], encoding: &'static Encoding) -> Units {
        let offsets = Offsets::new(bytes, encoding);
        if offsets.is_identity() {
            Units::Same
        } else {
            Units::Bytes(offsets)
        }
    }

    /// A core offset in the caller's units.
    pub fn out(&self, offset: usize) -> usize {
        match self {
            Units::Same => offset,
            Units::Chars(c) => c.to_units(offset),
            Units::Bytes(o) => o.to_source(offset),
        }
    }

    /// An offset in the caller's units as a core offset.
    pub fn back(&self, offset: usize) -> usize {
        match self {
            Units::Same => offset,
            Units::Chars(c) => c.to_bytes(offset),
            Units::Bytes(o) => o.to_decoded(offset),
        }
    }
}
