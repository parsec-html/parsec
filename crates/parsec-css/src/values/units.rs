// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! The dimensions of CSS Values and Units Level 4: every unit, what it
//! measures, and how the absolute ones convert to their canonical unit.

use std::fmt;

/// What a dimension measures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Length,
    Angle,
    Time,
    Frequency,
    Resolution,
    Flex,
}

macro_rules! units {
    ($($unit:ident $text:literal $kind:ident $factor:expr,)*) => {
        /// A unit, from the dimension tokens CSS knows.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum Unit {
            $($unit,)*
        }

        impl Unit {

            /// The unit as CSS writes it, in lowercase.
            pub fn as_str(self) -> &'static str {
                match self {
                    $(Unit::$unit => $text,)*
                }
            }

            fn from_lowercase(unit: &str) -> Option<Unit> {
                match unit {
                    $($text => Some(Unit::$unit),)*
                    _ => None,
                }
            }

            pub fn kind(self) -> Kind {
                match self {
                    $(Unit::$unit => Kind::$kind,)*
                }
            }

            /// How many of the canonical unit (`px`, `deg`, `s`, `hz`,
            /// `dppx`) one of this is, for the units with a fixed ratio.
            pub fn to_canonical(self) -> Option<f64> {
                match self {
                    $(Unit::$unit => $factor,)*
                }
            }
        }
    };
}

const FONT: Option<f64> = None;

units! {
    // Font-relative lengths.
    Em "em" Length FONT,
    Rem "rem" Length FONT,
    Ex "ex" Length FONT,
    Rex "rex" Length FONT,
    Cap "cap" Length FONT,
    Rcap "rcap" Length FONT,
    Ch "ch" Length FONT,
    Rch "rch" Length FONT,
    Ic "ic" Length FONT,
    Ric "ric" Length FONT,
    Lh "lh" Length FONT,
    Rlh "rlh" Length FONT,
    // Viewport lengths: default, small, large and dynamic.
    Vw "vw" Length FONT,
    Vh "vh" Length FONT,
    Vi "vi" Length FONT,
    Vb "vb" Length FONT,
    Vmin "vmin" Length FONT,
    Vmax "vmax" Length FONT,
    Svw "svw" Length FONT,
    Svh "svh" Length FONT,
    Svi "svi" Length FONT,
    Svb "svb" Length FONT,
    Svmin "svmin" Length FONT,
    Svmax "svmax" Length FONT,
    Lvw "lvw" Length FONT,
    Lvh "lvh" Length FONT,
    Lvi "lvi" Length FONT,
    Lvb "lvb" Length FONT,
    Lvmin "lvmin" Length FONT,
    Lvmax "lvmax" Length FONT,
    Dvw "dvw" Length FONT,
    Dvh "dvh" Length FONT,
    Dvi "dvi" Length FONT,
    Dvb "dvb" Length FONT,
    Dvmin "dvmin" Length FONT,
    Dvmax "dvmax" Length FONT,
    // Container query lengths.
    Cqw "cqw" Length FONT,
    Cqh "cqh" Length FONT,
    Cqi "cqi" Length FONT,
    Cqb "cqb" Length FONT,
    Cqmin "cqmin" Length FONT,
    Cqmax "cqmax" Length FONT,
    // Absolute lengths.
    Px "px" Length Some(1.0),
    Cm "cm" Length Some(96.0 / 2.54),
    Mm "mm" Length Some(96.0 / 25.4),
    Q "q" Length Some(96.0 / 101.6),
    In "in" Length Some(96.0),
    Pt "pt" Length Some(96.0 / 72.0),
    Pc "pc" Length Some(16.0),
    Deg "deg" Angle Some(1.0),
    Grad "grad" Angle Some(0.9),
    Rad "rad" Angle Some(180.0 / std::f64::consts::PI),
    Turn "turn" Angle Some(360.0),
    S "s" Time Some(1.0),
    Ms "ms" Time Some(0.001),
    Hz "hz" Frequency Some(1.0),
    Khz "khz" Frequency Some(1000.0),
    Dppx "dppx" Resolution Some(1.0),
    X "x" Resolution Some(1.0),
    Dpi "dpi" Resolution Some(1.0 / 96.0),
    Dpcm "dpcm" Resolution Some(2.54 / 96.0),
    Fr "fr" Flex FONT,
}

impl Unit {
    /// The unit a dimension token's unit names, ignoring ASCII case.
    pub fn parse(unit: &str) -> Option<Unit> {
        let mut buf = [0; 32];
        let lower = lowercase(unit, &mut buf)?;
        Unit::from_lowercase(lower)
    }

    /// The canonical unit of its kind, if it has one.
    pub fn canonical(self) -> Option<Unit> {
        self.to_canonical()?;
        Some(match self.kind() {
            Kind::Length => Unit::Px,
            Kind::Angle => Unit::Deg,
            Kind::Time => Unit::S,
            Kind::Frequency => Unit::Hz,
            Kind::Resolution => Unit::Dppx,
            Kind::Flex => return None,
        })
    }
}

impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `s` in ASCII lowercase, in `buf`, without allocating; `None` if it's
/// longer than any keyword this is used to look up.
pub(crate) fn lowercase<'b>(s: &str, buf: &'b mut [u8; 32]) -> Option<&'b str> {
    let out = buf.get_mut(..s.len())?;
    out.copy_from_slice(s.as_bytes());
    out.make_ascii_lowercase();
    // Only ASCII bytes changed, so it's still UTF-8.
    std::str::from_utf8(out).ok()
}

/// A number, a value with a unit, written as CSSOM serializes one: to six
/// significant figures, without an exponent, `-0` as `0`.
pub fn write_number(out: &mut impl fmt::Write, v: f64) -> fmt::Result {
    if v.is_nan() {
        return out.write_str("NaN");
    }
    if v.is_infinite() {
        return out.write_str(if v > 0.0 { "infinity" } else { "-infinity" });
    }
    if v == v.trunc() && v.abs() < 1e15 {
        return write!(out, "{}", v as i64);
    }
    let magnitude = v.abs().log10().floor() as i32;
    let decimals = (5 - magnitude).clamp(0, 20) as usize;
    let s = format!("{v:.decimals$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    out.write_str(if s == "-0" { "0" } else { s })
}

/// [`write_number`] into a new string.
pub fn number_to_string(v: f64) -> String {
    let mut s = String::new();
    write_number(&mut s, v).unwrap();
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_to_six_figures() {
        let cases = [
            (1.0, "1"),
            (0.5, "0.5"),
            (1.0 / 3.0, "0.333333"),
            (-0.0, "0"),
            (-0.0000001, "-0.0000001"),
            (123.456789, "123.457"),
            (1e6 + 0.25, "1000000"),
            (0.000012345678, "0.0000123457"),
            (f64::INFINITY, "infinity"),
        ];
        for (v, s) in cases {
            assert_eq!(number_to_string(v), s, "{v}");
        }
    }

    #[test]
    fn units_parse_in_any_case() {
        assert_eq!(Unit::parse("PX"), Some(Unit::Px));
        assert_eq!(Unit::parse("kHz"), Some(Unit::Khz));
        assert_eq!(Unit::parse("pxx"), None);
        assert_eq!(Unit::In.to_canonical(), Some(96.0));
        assert_eq!(Unit::Em.canonical(), None);
    }
}
