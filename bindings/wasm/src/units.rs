// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Offsets in the caller's units. The core counts UTF-8 bytes into the
//! text it parsed; JavaScript slices a string in UTF-16 units. `Counted`
//! is bindings/python/src/units.rs's, which counts code points instead
//! and also maps back, for `line_col`; nothing here takes an offset in.

use std::sync::OnceLock;

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
}

/// UTF-16 units: one for each character, two for one outside the Basic
/// Multilingual Plane, which UTF-8 writes in four bytes.
pub fn utf16(b: u8) -> u32 {
    match b {
        0x80..=0xBF => 0,
        0xF0.. => 2,
        _ => 1,
    }
}
