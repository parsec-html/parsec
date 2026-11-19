// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Every property in the CSS specs, from w3c/webref's data: its grammar,
//! initial value, whether it inherits, and its longhands if it's a
//! shorthand. `generated.rs` holds the table; this is the way in.

use std::collections::HashSet;
use std::sync::OnceLock;

use super::generated::{FUNCTIONS, PROPERTIES, TYPES};
use super::grammar::Grammar;

/// One property, as its spec defines it.
#[derive(Debug, PartialEq, Eq)]
pub struct PropertyData {
    pub name: &'static str,
    /// The value grammar, in the value definition syntax; `None` for the
    /// few legacy aliases defined only by what they alias.
    pub syntax: Option<&'static str>,
    /// The initial value as the spec writes it, which for a shorthand is
    /// prose ("see individual properties").
    pub initial: Option<&'static str>,
    /// `None` where the spec says something other than yes or no, as a
    /// shorthand's "see individual properties".
    pub inherited: Option<bool>,
    /// For a shorthand, the longhands it sets; empty for a longhand.
    pub longhands: &'static [&'static str],
    /// For a legacy alias (`-webkit-box-flex` and the like), the property
    /// it's another name for.
    pub alias_of: Option<&'static str>,
    /// Where the spec defines it.
    pub href: &'static str,
}

/// A property: [`PropertyData`] with its grammar parsed.
pub type Property = PropertyData;

impl PropertyData {
    pub fn is_shorthand(&self) -> bool {
        !self.longhands.is_empty()
    }

    /// Where this is in the table, from its address, so no lookup by
    /// name; a `PropertyData` made elsewhere is looked up.
    pub(crate) fn position(&self) -> Option<usize> {
        let at = (self as *const PropertyData as usize).wrapping_sub(PROPERTIES.as_ptr() as usize)
            / std::mem::size_of::<PropertyData>();
        match PROPERTIES.get(at) {
            Some(p) if std::ptr::eq(p, self) => Some(at),
            _ => index(self.name),
        }
    }

    /// Every keyword its grammar has outside functions, through the types
    /// and properties it refers to: what a `<custom-ident>` in its value
    /// can't be, by Values 4 §4.2's rule that a keyword some other part of
    /// the grammar can take isn't an identifier of the author's.
    pub(crate) fn keywords(&self) -> &'static Keywords {
        static ALL: OnceLock<Vec<OnceLock<Keywords>>> = OnceLock::new();
        static NONE: OnceLock<Keywords> = OnceLock::new();
        let all = ALL.get_or_init(|| PROPERTIES.iter().map(|_| OnceLock::new()).collect());
        let (Some(i), Some(g)) = (self.position(), self.grammar()) else {
            return NONE.get_or_init(Keywords::default);
        };
        all[i].get_or_init(|| keywords_of(g))
    }

    /// Whether a lone identifier that isn't one of its keywords can be a
    /// value of it: a name, a color's, or anything for a grammar that takes
    /// tokens as they are.
    pub(crate) fn takes_names(&self) -> bool {
        static ALL: OnceLock<Vec<OnceLock<bool>>> = OnceLock::new();
        let all = ALL.get_or_init(|| PROPERTIES.iter().map(|_| OnceLock::new()).collect());
        let (Some(i), Some(g)) = (self.position(), self.grammar()) else {
            return true;
        };
        *all[i].get_or_init(|| names_in(g))
    }

    /// The grammar of its value, parsed: the alias target's for an alias.
    pub fn grammar(&self) -> Option<&'static Grammar> {
        let grammars = grammars();
        let i = self.position()?;
        if let Some(g) = &grammars.properties[i] {
            return Some(g);
        }
        let target = property(self.alias_of?)?;
        grammars.properties[index(target.name)?].as_ref()
    }
}

/// Looks a property up by name, ignoring ASCII case.
///
/// ```
/// let p = parsec_css::values::property("Margin-Top").unwrap();
/// assert_eq!((p.name, p.initial, p.inherited), ("margin-top", Some("0"), Some(false)));
/// assert!(parsec_css::values::property("margin").unwrap().is_shorthand());
/// ```
pub fn property(name: &str) -> Option<&'static Property> {
    index(name).map(|i| &PROPERTIES[i])
}

/// Every property in the table, sorted by name.
pub fn properties() -> &'static [Property] {
    PROPERTIES
}

fn index(name: &str) -> Option<usize> {
    // Open addressing on FNV-1a of the lowercased name, worked out as it's
    // read, so no copy: this is looked up once a declaration.
    let table = by_name();
    let mask = table.len() - 1;
    let mut slot = fnv_lower(name.as_bytes()) as usize & mask;
    loop {
        match table[slot] {
            0 => return None,
            i => {
                let i = i as usize - 1;
                let found = PROPERTIES[i].name;
                if found.len() == name.len() && (found == name || found.eq_ignore_ascii_case(name))
                {
                    return Some(i);
                }
            }
        }
        slot = (slot + 1) & mask;
    }
}

/// A hash of a name that ignores ASCII case: its first and last eight
/// bytes and its length, so a few instructions rather than a loop. Bit
/// 0x20 set lowercases a letter and leaves `-` and digits as they are;
/// the comparison after it settles everything else.
fn fnv_lower(bytes: &[u8]) -> u32 {
    let word = |b: &[u8]| {
        let mut w = [0u8; 8];
        w[..b.len()].copy_from_slice(b);
        u64::from_le_bytes(w) | 0x2020_2020_2020_2020
    };
    let n = bytes.len();
    let (head, tail) = if n >= 8 {
        (word(&bytes[..8]), word(&bytes[n - 8..]))
    } else {
        (word(bytes), 0)
    };
    let h = (head ^ tail.rotate_left(29) ^ n as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    (h >> 32) as u32
}

/// Each property's place in the table plus one, by its name's hash; 0
/// is an empty slot.
fn by_name() -> &'static [u16] {
    static TABLE: OnceLock<Vec<u16>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let size = (PROPERTIES.len() * 2).next_power_of_two();
        let mut t = vec![0u16; size];
        for (i, p) in PROPERTIES.iter().enumerate() {
            let mut slot = fnv_lower(p.name.as_bytes()) as usize & (size - 1);
            while t[slot] != 0 {
                slot = (slot + 1) & (size - 1);
            }
            t[slot] = i as u16 + 1;
        }
        t
    })
}

/// FNV-1a, for the keyword sets: keywords are short.
#[derive(Default)]
pub(crate) struct Fnv(u64);

impl std::hash::Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut h = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3);
        }
        self.0 = h;
    }
}

pub(crate) type FnvBuild = std::hash::BuildHasherDefault<Fnv>;

/// A property's keywords, looked up by FNV: they're short.
pub(crate) type Keywords = HashSet<&'static str, FnvBuild>;

fn keywords_of(g: &'static Grammar) -> Keywords {
    let mut out = Keywords::default();
    let mut seen: HashSet<*const Grammar> = HashSet::new();
    let mut stack = vec![g];
    while let Some(g) = stack.pop() {
        if !seen.insert(g) {
            continue;
        }
        match g {
            Grammar::Keyword(k) => {
                out.insert(&**k);
            }
            Grammar::Type(t) => {
                let found = match t.name.strip_suffix("()") {
                    Some(_) => None,
                    None => type_grammar(&t.name),
                };
                stack.extend(found);
            }
            Grammar::Property(r) => stack.extend(property(&r.name).and_then(|p| p.grammar())),
            Grammar::Sequence(v) | Grammar::All(v) | Grammar::Any(v) | Grammar::One(v) => {
                stack.extend(v.iter())
            }
            Grammar::Repeat { item, .. } | Grammar::NonEmpty(item) => stack.push(item),
            // A function's arguments are a context of their own.
            _ => {}
        }
    }
    out
}

fn names_in(g: &'static Grammar) -> bool {
    let mut seen: HashSet<*const Grammar> = HashSet::new();
    let mut stack = vec![g];
    while let Some(g) = stack.pop() {
        if !seen.insert(g) {
            continue;
        }
        match g {
            Grammar::Type(t) => {
                if t.name.ends_with("()") {
                    continue;
                }
                match &*t.name {
                    "ident"
                    | "custom-ident"
                    | "dashed-ident"
                    | "custom-property-name"
                    | "color"
                    | "declaration-value"
                    | "any-value" => return true,
                    // A type that's prose only matches nothing.
                    name => stack.extend(type_grammar(name)),
                }
            }
            Grammar::Property(r) => stack.extend(property(&r.name).and_then(|p| p.grammar())),
            Grammar::Sequence(v) | Grammar::All(v) | Grammar::Any(v) | Grammar::One(v) => {
                stack.extend(v.iter())
            }
            Grammar::Repeat { item, .. } | Grammar::NonEmpty(item) => stack.push(item),
            _ => {}
        }
    }
    false
}

/// Every grammar in the table, parsed once, on first use.
pub(crate) struct Grammars {
    pub properties: Vec<Option<Grammar>>,
    pub types: Vec<Option<Grammar>>,
    pub functions: Vec<Option<Grammar>>,
}

pub(crate) fn grammars() -> &'static Grammars {
    static G: OnceLock<Grammars> = OnceLock::new();
    G.get_or_init(|| Grammars {
        properties: PROPERTIES
            .iter()
            .map(|p| p.syntax.and_then(Grammar::parse))
            .collect(),
        types: TYPES.iter().map(|(_, s)| Grammar::parse(s)).collect(),
        functions: FUNCTIONS.iter().map(|(_, s)| Grammar::parse(s)).collect(),
    })
}

/// A named type's grammar, `<line-style>` say, when the specs give one.
pub(crate) fn type_grammar(name: &str) -> Option<&'static Grammar> {
    let i = TYPES.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
    grammars().types[i].as_ref()
}

/// A function's grammar, `<rect()>` say, by its name without the `()`.
pub(crate) fn function_grammar(name: &str) -> Option<&'static Grammar> {
    let i = FUNCTIONS.binary_search_by(|(n, _)| (*n).cmp(name)).ok()?;
    grammars().functions[i].as_ref()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table() {
        let longhands: Vec<_> = PROPERTIES
            .iter()
            .filter(|p| !p.is_shorthand() && p.alias_of.is_none())
            .collect();
        assert_eq!(PROPERTIES.len(), 822);
        assert_eq!(longhands.len(), 597);
        // Every grammar parses, aliases through their target.
        assert!(
            PROPERTIES
                .iter()
                .all(|p| p.syntax.is_none() || p.grammar().is_some())
        );
        // And every longhand's initial value, where the spec gives one as a
        // value rather than prose, is a value of its own grammar.
        let mut prose = Vec::new();
        for p in &longhands {
            let Some(initial) = p.initial else { continue };
            let values = crate::parse_component_values(initial);
            if super::super::parse_value(p.name, &values).is_err() {
                prose.push(format!("{}: {initial}", p.name));
            }
        }
        // Prose, or (`fill`, `text-spacing-trim`) a grammar whose types the
        // data doesn't define.
        assert_eq!(
            prose,
            [
                "all: see individual properties",
                "background-tbd: see individual properties",
                "fill: black",
                "glyph-orientation-vertical: n/a",
                "text-spacing-trim: normal",
                "voice-family: implementation-dependent",
            ]
        );
    }
}
