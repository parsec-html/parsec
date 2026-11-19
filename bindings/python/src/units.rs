// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Offsets in the caller's units. The core counts UTF-8 bytes into the
//! text it parsed; a caller slices what it passed in, which is code points
//! for a Python `str` and UTF-16 units for a JavaScript string. The same
//! `Counted` is in bindings/wasm/src/units.rs, counting UTF-16 instead.

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
    /// They're the same: an ASCII `str`, or `bytes` that were all UTF-8.
    Same,
    /// A `str` with more than ASCII in it: code point indexes.
    Chars(Counted),
    /// `bytes` with sequences that weren't UTF-8, each of which the parse
    /// read as a three-byte U+FFFD: after each, where the parsed text and
    /// the bytes had got to.
    Repaired(Vec<(usize, usize)>),
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

    /// For `bytes`, which the parse was given as their lossy decoding.
    pub fn of_bytes(bytes: &[u8]) -> Units {
        let mut fixes = Vec::new();
        let (mut parsed, mut given) = (0, 0);
        for chunk in bytes.utf8_chunks() {
            parsed += chunk.valid().len();
            given += chunk.valid().len();
            if !chunk.invalid().is_empty() {
                parsed += '\u{FFFD}'.len_utf8();
                given += chunk.invalid().len();
                fixes.push((parsed, given));
            }
        }
        if fixes.is_empty() {
            Units::Same
        } else {
            Units::Repaired(fixes)
        }
    }

    /// A core offset in the caller's units.
    pub fn out(&self, offset: usize) -> usize {
        match self {
            Units::Same => offset,
            Units::Chars(c) => c.to_units(offset),
            Units::Repaired(fixes) => {
                let i = fixes.partition_point(|&(p, _)| p <= offset);
                match i.checked_sub(1).map(|i| fixes[i]) {
                    Some((p, g)) => g + (offset - p),
                    None => offset,
                }
            }
        }
    }

    /// An offset in the caller's units as a core offset.
    pub fn back(&self, offset: usize) -> usize {
        match self {
            Units::Same => offset,
            Units::Chars(c) => c.to_bytes(offset),
            Units::Repaired(fixes) => {
                let i = fixes.partition_point(|&(_, g)| g <= offset);
                match i.checked_sub(1).map(|i| fixes[i]) {
                    Some((p, g)) => p + (offset - g),
                    None => offset,
                }
            }
        }
    }
}
