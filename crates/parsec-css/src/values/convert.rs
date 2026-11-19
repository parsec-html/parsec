// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Colours worked out: conversion between every colour space of CSS
//! Color 4 (its sample code's matrices and transfer functions, §18), and
//! CSS Color 5's relative colours and `color-mix()`, as a computed value
//! has them.

// The matrices are CSS Color 4's sample code's, digit for digit.
#![allow(clippy::excessive_precision)]

use super::color::{Channel, Channels, Color, Model, Predefined, Relative, RelativeChannel};
use super::math::Leaf;
use super::units::write_number;
use crate::parser::ComponentValue;
use crate::tokenizer::Token;

/// A colour space a colour can be worked out in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Space {
    Srgb,
    SrgbLinear,
    DisplayP3,
    DisplayP3Linear,
    A98Rgb,
    ProphotoRgb,
    Rec2020,
    XyzD50,
    XyzD65,
    Lab,
    Lch,
    Oklab,
    Oklch,
    Hsl,
    Hwb,
}

impl Space {
    fn parse(name: &str) -> Option<Space> {
        Some(match name.to_ascii_lowercase().as_str() {
            "srgb" => Space::Srgb,
            "srgb-linear" => Space::SrgbLinear,
            "display-p3" => Space::DisplayP3,
            "display-p3-linear" => Space::DisplayP3Linear,
            "a98-rgb" => Space::A98Rgb,
            "prophoto-rgb" => Space::ProphotoRgb,
            "rec2020" => Space::Rec2020,
            "xyz-d50" => Space::XyzD50,
            "xyz" | "xyz-d65" => Space::XyzD65,
            "lab" => Space::Lab,
            "lch" => Space::Lch,
            "oklab" => Space::Oklab,
            "oklch" => Space::Oklch,
            "hsl" => Space::Hsl,
            "hwb" => Space::Hwb,
            _ => return None,
        })
    }

    fn from_predefined(p: Predefined) -> Space {
        match p {
            Predefined::Srgb => Space::Srgb,
            Predefined::SrgbLinear => Space::SrgbLinear,
            Predefined::DisplayP3 => Space::DisplayP3,
            Predefined::DisplayP3Linear => Space::DisplayP3Linear,
            Predefined::A98Rgb => Space::A98Rgb,
            Predefined::ProphotoRgb => Space::ProphotoRgb,
            Predefined::Rec2020 => Space::Rec2020,
            Predefined::XyzD50 => Space::XyzD50,
            Predefined::XyzD65 => Space::XyzD65,
        }
    }

    fn predefined_name(self) -> Option<&'static str> {
        Some(match self {
            Space::Srgb => "srgb",
            Space::SrgbLinear => "srgb-linear",
            Space::DisplayP3 => "display-p3",
            Space::DisplayP3Linear => "display-p3-linear",
            Space::A98Rgb => "a98-rgb",
            Space::ProphotoRgb => "prophoto-rgb",
            Space::Rec2020 => "rec2020",
            Space::XyzD50 => "xyz-d50",
            Space::XyzD65 => "xyz-d65",
            _ => return None,
        })
    }

    /// Which channel is the hue, for the polar spaces.
    fn hue(self) -> Option<usize> {
        match self {
            Space::Hsl | Space::Hwb => Some(0),
            Space::Lch | Space::Oklch => Some(2),
            _ => None,
        }
    }

    /// What kind of thing each channel is, for carrying a missing one
    /// across a conversion (CSS Color 4 §12.2).
    fn kinds(self) -> [Category; 3] {
        use Category::*;
        match self {
            Space::Lab | Space::Oklab => [Lightness, Other, Other],
            Space::Lch | Space::Oklch => [Lightness, Colorfulness, Hue],
            Space::Hsl => [Hue, Colorfulness, Lightness],
            Space::Hwb => [Hue, Other, Other],
            Space::XyzD50 | Space::XyzD65 => [Red, Green, Blue],
            _ => [Red, Green, Blue],
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Category {
    Red,
    Green,
    Blue,
    Lightness,
    Colorfulness,
    Hue,
    Other,
}

/// A colour with numbers for channels, `None` where one is missing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Absolute {
    pub space: Space,
    pub channels: [Option<f64>; 3],
    pub alpha: Option<f64>,
}

type M = [[f64; 3]; 3];

fn mul(m: &M, v: [f64; 3]) -> [f64; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

const SRGB_TO_XYZ: M = [
    [0.41239079926595934, 0.357584339383878, 0.1804807884018343],
    [0.21263900587151027, 0.715168678767756, 0.07219231536073371],
    [0.01933081871559182, 0.11919477979462598, 0.9505321522496607],
];
const XYZ_TO_SRGB: M = [
    [3.2409699419045226, -1.537383177570094, -0.4986107602930034],
    [-0.9692436362808796, 1.8759675015077202, 0.04155505740717559],
    [
        0.05563007969699366,
        -0.20397695888897652,
        1.0569715142428786,
    ],
];
const P3_TO_XYZ: M = [
    [0.4865709486482162, 0.26566769316909306, 0.1982172852343625],
    [0.2289745640697488, 0.6917385218365064, 0.079286914093745],
    [0.0, 0.04511338185890264, 1.043944368900976],
];
const XYZ_TO_P3: M = [
    [2.493496911941425, -0.9313836179191239, -0.40271078445071684],
    [
        -0.8294889695615747,
        1.7626640603183463,
        0.023624685841943577,
    ],
    [
        0.03584583024378447,
        -0.07617238926804182,
        0.9568845240076872,
    ],
];
const A98_TO_XYZ: M = [
    [0.5766690429101305, 0.1855582379065463, 0.1882286462349947],
    [0.29734497525053605, 0.6273635662554661, 0.07529145849399788],
    [0.02703136138641234, 0.07068885253582723, 0.9913375368376388],
];
const XYZ_TO_A98: M = [
    [
        2.0415879038107465,
        -0.5650069742788596,
        -0.34473135077832956,
    ],
    [-0.9692436362808795, 1.8759675015077202, 0.04155505740717557],
    [
        0.013444280632031142,
        -0.11836239223101838,
        1.0151749943912054,
    ],
];
const PROPHOTO_TO_XYZ50: M = [
    [0.7977666449006423, 0.13518129740053308, 0.0313477341283922],
    [0.2880748288194013, 0.711835234241873, 0.00008993693872564],
    [0.0, 0.0, 0.8251046025104602],
];
const XYZ50_TO_PROPHOTO: M = [
    [
        1.3457868816471583,
        -0.25557208737979464,
        -0.05110186497554526,
    ],
    [-0.5446307051249019, 1.5082477428451468, 0.02052744743642139],
    [0.0, 0.0, 1.2119675456389452],
];
const REC2020_TO_XYZ: M = [
    [0.6369580483012914, 0.14461690358620832, 0.1688809751641721],
    [0.2627002120112671, 0.6779980715188708, 0.05930171646986196],
    [0.0, 0.028072693049087428, 1.060985057710791],
];
const XYZ_TO_REC2020: M = [
    [1.716651187971268, -0.355670783776392, -0.25336628137366],
    [-0.666684351832489, 1.616481236634939, 0.0157685458139111],
    [0.017639857445311, -0.042770613257809, 0.942103121235474],
];
const D65_TO_D50: M = [
    [
        1.0479298208405488,
        0.022946793341019088,
        -0.05019222954313557,
    ],
    [
        0.029627815688159344,
        0.990434484573249,
        -0.01707382502938514,
    ],
    [
        -0.009243058152591178,
        0.015055144896577895,
        0.7518742899580008,
    ],
];
const D50_TO_D65: M = [
    [
        0.9554734527042182,
        -0.023098536874261423,
        0.0632593086610217,
    ],
    [
        -0.028369706963208136,
        1.0099954580058226,
        0.021041398966943008,
    ],
    [
        0.012314001688319899,
        -0.020507696433477912,
        1.3303659366080753,
    ],
];
const XYZ_TO_LMS: M = [
    [0.8190224379967030, 0.3619062600528904, -0.1288737815209879],
    [0.0329836539323885, 0.9292868615863434, 0.0361446663506424],
    [0.0481771893596242, 0.2642395317527308, 0.6335478284694309],
];
const LMS_TO_OKLAB: M = [
    [0.2104542683093140, 0.7936177747023054, -0.0040720430116193],
    [1.9779985324311684, -2.4285922420485799, 0.4505937096174110],
    [0.0259040424655478, 0.7827717124575296, -0.8086757548750674],
];
const OKLAB_TO_LMS: M = [
    [1.0, 0.3963377773761749, 0.2158037573099136],
    [1.0, -0.1055613458156586, -0.0638541728258133],
    [1.0, -0.0894841775298119, -1.2914855480194092],
];
const LMS_TO_XYZ: M = [
    [1.2268798758459243, -0.5578149944602171, 0.2813910456659647],
    [-0.0405757452148008, 1.1122868032803170, -0.0717110580655164],
    [-0.0763729366746601, -0.4214933324022432, 1.5869240198367816],
];
const D50_WHITE: [f64; 3] = [0.3457 / 0.3585, 1.0, (1.0 - 0.3457 - 0.3585) / 0.3585];

fn srgb_to_linear(c: f64) -> f64 {
    let a = c.abs();
    if a <= 0.04045 {
        c / 12.92
    } else {
        c.signum() * ((a + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f64) -> f64 {
    let a = c.abs();
    if a > 0.0031308 {
        c.signum() * (1.055 * a.powf(1.0 / 2.4) - 0.055)
    } else {
        12.92 * c
    }
}

fn rec2020_to_linear(c: f64) -> f64 {
    const A: f64 = 1.09929682680944;
    const B: f64 = 0.018053968510807;
    let a = c.abs();
    if a < B * 4.5 {
        c / 4.5
    } else {
        c.signum() * ((a + A - 1.0) / A).powf(1.0 / 0.45)
    }
}

fn linear_to_rec2020(c: f64) -> f64 {
    const A: f64 = 1.09929682680944;
    const B: f64 = 0.018053968510807;
    let a = c.abs();
    if a > B {
        c.signum() * (A * a.powf(0.45) - (A - 1.0))
    } else {
        4.5 * c
    }
}

fn hsl_to_srgb(h: f64, s: f64, l: f64) -> [f64; 3] {
    let (s, l) = (s / 100.0, l / 100.0);
    let f = |n: f64| {
        let k = (n + h / 30.0).rem_euclid(12.0);
        let a = s * l.min(1.0 - l);
        l - a * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)
    };
    [f(0.0), f(8.0), f(4.0)]
}

fn srgb_to_hsl(rgb: [f64; 3]) -> [Option<f64>; 3] {
    let [r, g, b] = rgb;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (min + max) / 2.0;
    let d = max - min;
    let mut h = f64::NAN;
    let mut s = 0.0;
    // Near-equal channels, out of gamut by rounding: achromatic.
    if d.abs() > 1e-6 {
        s = if l == 0.0 || l == 1.0 {
            0.0
        } else {
            (max - l) / l.min(1.0 - l)
        };
        h = if max == r {
            (g - b) / d + if g < b { 6.0 } else { 0.0 }
        } else if max == g {
            (b - r) / d + 2.0
        } else {
            (r - g) / d + 4.0
        } * 60.0;
    }
    if s < 0.0 {
        h += 180.0;
        s = s.abs();
    }
    if h >= 360.0 {
        h -= 360.0;
    }
    // An achromatic colour's hue is powerless: missing.
    let hue = if s.abs() < 1e-9 || h.is_nan() {
        None
    } else {
        Some(h)
    };
    [hue, Some(s * 100.0), Some(l * 100.0)]
}

fn hwb_to_srgb(h: f64, w: f64, b: f64) -> [f64; 3] {
    let (w, b) = (w / 100.0, b / 100.0);
    if w + b >= 1.0 {
        let gray = w / (w + b);
        return [gray; 3];
    }
    hsl_to_srgb(h, 100.0, 50.0).map(|c| c * (1.0 - w - b) + w)
}

fn srgb_to_hwb(rgb: [f64; 3]) -> [Option<f64>; 3] {
    let hsl = srgb_to_hsl(rgb);
    let w = rgb[0].min(rgb[1]).min(rgb[2]);
    let b = 1.0 - rgb[0].max(rgb[1]).max(rgb[2]);
    let hue = if w + b >= 1.0 - 1e-4 { None } else { hsl[0] };
    [hue, Some(w * 100.0), Some(b * 100.0)]
}

fn xyz50_to_lab(v: [f64; 3]) -> [f64; 3] {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    let f = |x: f64| {
        if x > E {
            x.cbrt()
        } else {
            (K * x + 16.0) / 116.0
        }
    };
    let [x, y, z] = [
        v[0] / D50_WHITE[0],
        v[1] / D50_WHITE[1],
        v[2] / D50_WHITE[2],
    ]
    .map(f);
    [116.0 * y - 16.0, 500.0 * (x - y), 200.0 * (y - z)]
}

fn lab_to_xyz50(lab: [f64; 3]) -> [f64; 3] {
    const E: f64 = 216.0 / 24389.0;
    const K: f64 = 24389.0 / 27.0;
    let f1 = (lab[0] + 16.0) / 116.0;
    let f0 = lab[1] / 500.0 + f1;
    let f2 = f1 - lab[2] / 200.0;
    let x = if f0.powi(3) > E {
        f0.powi(3)
    } else {
        (116.0 * f0 - 16.0) / K
    };
    let y = if lab[0] > K * E {
        f1.powi(3)
    } else {
        lab[0] / K
    };
    let z = if f2.powi(3) > E {
        f2.powi(3)
    } else {
        (116.0 * f2 - 16.0) / K
    };
    [x * D50_WHITE[0], y * D50_WHITE[1], z * D50_WHITE[2]]
}

fn xyz_to_oklab(v: [f64; 3]) -> [f64; 3] {
    let lms = mul(&XYZ_TO_LMS, v).map(f64::cbrt);
    mul(&LMS_TO_OKLAB, lms)
}

fn oklab_to_xyz(v: [f64; 3]) -> [f64; 3] {
    let lms = mul(&OKLAB_TO_LMS, v).map(|x| x * x * x);
    mul(&LMS_TO_XYZ, lms)
}

fn to_polar(lab: [f64; 3], powerless: f64) -> [Option<f64>; 3] {
    let c = (lab[1] * lab[1] + lab[2] * lab[2]).sqrt();
    let h = lab[2].atan2(lab[1]).to_degrees().rem_euclid(360.0);
    [
        Some(lab[0]),
        Some(c),
        if c <= powerless { None } else { Some(h) },
    ]
}

fn from_polar(lch: [f64; 3]) -> [f64; 3] {
    let h = lch[2].to_radians();
    [lch[0], lch[1] * h.cos(), lch[1] * h.sin()]
}

/// A colour's channels in XYZ D65, missing ones as 0.
fn to_xyz(c: &Absolute) -> [f64; 3] {
    let v = c.channels.map(|x| x.unwrap_or(0.0));
    match c.space {
        Space::Srgb => mul(&SRGB_TO_XYZ, v.map(srgb_to_linear)),
        Space::SrgbLinear => mul(&SRGB_TO_XYZ, v),
        Space::DisplayP3 => mul(&P3_TO_XYZ, v.map(srgb_to_linear)),
        Space::DisplayP3Linear => mul(&P3_TO_XYZ, v),
        Space::A98Rgb => mul(
            &A98_TO_XYZ,
            v.map(|x| x.signum() * x.abs().powf(563.0 / 256.0)),
        ),
        Space::ProphotoRgb => {
            let lin = v.map(|x| {
                if x.abs() <= 16.0 / 512.0 {
                    x / 16.0
                } else {
                    x.signum() * x.abs().powf(1.8)
                }
            });
            mul(&D50_TO_D65, mul(&PROPHOTO_TO_XYZ50, lin))
        }
        Space::Rec2020 => mul(&REC2020_TO_XYZ, v.map(rec2020_to_linear)),
        Space::XyzD50 => mul(&D50_TO_D65, v),
        Space::XyzD65 => v,
        Space::Lab => mul(&D50_TO_D65, lab_to_xyz50(v)),
        Space::Lch => mul(&D50_TO_D65, lab_to_xyz50(from_polar(v))),
        Space::Oklab => oklab_to_xyz(v),
        Space::Oklch => oklab_to_xyz(from_polar(v)),
        Space::Hsl => mul(
            &SRGB_TO_XYZ,
            hsl_to_srgb(v[0], v[1], v[2]).map(srgb_to_linear),
        ),
        Space::Hwb => mul(
            &SRGB_TO_XYZ,
            hwb_to_srgb(v[0], v[1], v[2]).map(srgb_to_linear),
        ),
    }
}

fn from_xyz(v: [f64; 3], space: Space) -> [Option<f64>; 3] {
    let some = |a: [f64; 3]| a.map(Some);
    let srgb = || mul(&XYZ_TO_SRGB, v).map(linear_to_srgb);
    match space {
        Space::Srgb => some(srgb()),
        Space::SrgbLinear => some(mul(&XYZ_TO_SRGB, v)),
        Space::DisplayP3 => some(mul(&XYZ_TO_P3, v).map(linear_to_srgb)),
        Space::DisplayP3Linear => some(mul(&XYZ_TO_P3, v)),
        Space::A98Rgb => {
            some(mul(&XYZ_TO_A98, v).map(|x| x.signum() * x.abs().powf(256.0 / 563.0)))
        }
        Space::ProphotoRgb => some(mul(&XYZ50_TO_PROPHOTO, mul(&D65_TO_D50, v)).map(|x| {
            if x.abs() >= 1.0 / 512.0 {
                x.signum() * x.abs().powf(1.0 / 1.8)
            } else {
                16.0 * x
            }
        })),
        Space::Rec2020 => some(mul(&XYZ_TO_REC2020, v).map(linear_to_rec2020)),
        Space::XyzD50 => some(mul(&D65_TO_D50, v)),
        Space::XyzD65 => some(v),
        Space::Lab => some(xyz50_to_lab(mul(&D65_TO_D50, v))),
        Space::Lch => to_polar(xyz50_to_lab(mul(&D65_TO_D50, v)), 0.0015),
        Space::Oklab => some(xyz_to_oklab(v)),
        Space::Oklch => to_polar(xyz_to_oklab(v), 0.000004),
        Space::Hsl => srgb_to_hsl(srgb()),
        Space::Hwb => srgb_to_hwb(srgb()),
    }
}

impl Absolute {
    /// The colour in `space`, missing channels carried over where the two
    /// spaces have a channel of the same kind.
    pub fn convert(&self, space: Space) -> Absolute {
        if space == self.space {
            return *self;
        }
        // Nothing known converts to nothing known.
        if self.channels.iter().all(Option::is_none) {
            return Absolute {
                space,
                channels: [None; 3],
                alpha: self.alpha,
            };
        }
        let mut channels = from_xyz(to_xyz(self), space);
        let (from, to) = (self.space.kinds(), space.kinds());
        // Lab-like to LCh-like with both opponents missing: chroma and
        // hue are missing too.
        if matches!(self.space, Space::Lab | Space::Oklab)
            && matches!(space, Space::Lch | Space::Oklch)
            && self.channels[1].is_none()
            && self.channels[2].is_none()
        {
            channels[1] = None;
            channels[2] = None;
        }
        for (i, k) in to.iter().enumerate() {
            if *k == Category::Other {
                continue;
            }
            if let Some(j) = from.iter().position(|f| f == k)
                && self.channels[j].is_none()
            {
                channels[i] = None;
            }
        }
        Absolute {
            space,
            channels,
            alpha: self.alpha,
        }
    }

    /// Written as a computed colour: in its own function, the sRGB-based
    /// ones as `color(srgb …)` unless a channel is missing.
    pub fn write(&self) -> String {
        let mut s = String::new();
        let n = |s: &mut String, v: Option<f64>, pct: bool| match v {
            None => s.push_str("none"),
            Some(v) => {
                let v = if v.abs() < 5e-7 || v.is_nan() { 0.0 } else { v };
                write_number(s, v).unwrap();
                if pct {
                    s.push('%');
                }
            }
        };
        let (name, pcts, chans) = match self.space {
            Space::Hsl | Space::Hwb if self.channels.iter().any(Option::is_none) => (
                if self.space == Space::Hsl {
                    "hsl("
                } else {
                    "hwb("
                },
                [false, true, true],
                self.channels,
            ),
            Space::Hsl | Space::Hwb => (
                "color(srgb ",
                [false; 3],
                self.convert(Space::Srgb).channels,
            ),
            Space::Lab => ("lab(", [false; 3], self.channels),
            Space::Lch => ("lch(", [false; 3], self.channels),
            Space::Oklab => ("oklab(", [false; 3], self.channels),
            Space::Oklch => ("oklch(", [false; 3], self.channels),
            sp => {
                s.push_str("color(");
                s.push_str(sp.predefined_name().unwrap());
                s.push(' ');
                ("", [false; 3], self.channels)
            }
        };
        s.push_str(name);
        for (i, (c, p)) in chans.iter().zip(pcts).enumerate() {
            if i > 0 {
                s.push(' ');
            }
            n(&mut s, *c, p);
        }
        match self.alpha {
            Some(a) if (a - 1.0).abs() < 1e-9 => {}
            a => {
                s.push_str(" / ");
                n(&mut s, a.map(|a| a.clamp(0.0, 1.0)), false);
            }
        }
        s.push(')');
        s
    }
}

/// The channels a colour has, in its space; `None` for one that isn't
/// known without an element (`currentcolor`) or unresolved math.
pub fn absolute(c: &Color) -> Option<Absolute> {
    match c {
        Color::Named(_) => absolute(&c.computed()),
        Color::Channels(ch) => from_channels(ch),
        Color::Relative(r) => relative(r),
        Color::Text(t) => absolute(&Color::parse(crate::parse_component_values(t).first()?)?),
        Color::Mix(m) => {
            let (space, hue, items) = m.parts.as_ref()?;
            let items: Vec<(Absolute, Option<f64>)> = items
                .iter()
                .map(|(c, p)| Some((absolute(c)?, *p)))
                .collect::<Option<_>>()?;
            mix(*space, hue, &items)
        }
        _ => None,
    }
}

/// A channel's number; math giving a percentage is `percent` a percent.
fn channel(c: &Channel, percent: f64) -> Option<Option<f64>> {
    match c {
        Channel::None => Some(None),
        Channel::Number(v) => Some(Some(*v)),
        Channel::Math(m) => match m.leaf()? {
            Leaf::Number(v) => Some(Some(v)),
            Leaf::Percentage(p) => Some(Some(p * percent)),
            Leaf::Dimension(v, u) => Some(Some(v * u.to_canonical()?)),
        },
    }
}

fn from_channels(ch: &Channels) -> Option<Absolute> {
    let percent: [f64; 3] = match ch.model {
        Model::Rgb => [2.55; 3],
        Model::Hsl | Model::Hwb => [1.0; 3],
        Model::Lab => [1.0, 1.25, 1.25],
        Model::Lch => [1.0, 1.5, 1.0],
        Model::Oklab => [0.01, 0.004, 0.004],
        Model::Oklch => [0.01, 0.004, 1.0],
        _ => [0.01; 3],
    };
    let v: Vec<Option<f64>> = ch
        .channels
        .iter()
        .zip(percent)
        .map(|(c, p)| channel(c, p))
        .collect::<Option<_>>()?;
    let alpha = channel(&ch.alpha, 0.01)?;
    let space = match ch.model {
        Model::Rgb => {
            return Some(Absolute {
                space: Space::Srgb,
                channels: [
                    v[0].map(|x| x / 255.0),
                    v[1].map(|x| x / 255.0),
                    v[2].map(|x| x / 255.0),
                ],
                alpha,
            });
        }
        Model::Hsl => Space::Hsl,
        Model::Hwb => Space::Hwb,
        Model::Lab => Space::Lab,
        Model::Lch => Space::Lch,
        Model::Oklab => Space::Oklab,
        Model::Oklch => Space::Oklch,
        Model::Color(p) => Space::from_predefined(p),
        Model::DeviceCmyk => return None,
    };
    // Computed values clamp what parsing couldn't, in math: lightness,
    // chroma, alpha; and hues go round.
    let mut channels = [v[0], v[1], v[2]];
    let clamp = |c: &mut Option<f64>, lo: f64, hi: f64| {
        if let Some(x) = c {
            *x = if x.is_nan() { 0.0 } else { x.clamp(lo, hi) };
        }
    };
    match space {
        Space::Lab => clamp(&mut channels[0], 0.0, 100.0),
        Space::Lch => {
            clamp(&mut channels[0], 0.0, 100.0);
            clamp(&mut channels[1], 0.0, f64::INFINITY);
        }
        Space::Oklab => clamp(&mut channels[0], 0.0, 1.0),
        Space::Oklch => {
            clamp(&mut channels[0], 0.0, 1.0);
            clamp(&mut channels[1], 0.0, f64::INFINITY);
        }
        _ => {}
    }
    if let Some(h) = space.hue()
        && let Some(x) = &mut channels[h]
    {
        *x = if x.is_finite() {
            x.rem_euclid(360.0)
        } else {
            0.0
        };
    }
    for c in &mut channels {
        if let Some(x) = c
            && x.is_nan()
        {
            *x = 0.0;
        }
    }
    let mut alpha = alpha;
    clamp(&mut alpha, 0.0, 1.0);
    Some(Absolute {
        space,
        channels,
        alpha,
    })
}

/// What 100% of each channel is in a relative colour's function, and
/// the scale its channel names have there.
fn scales(function: &str, space: Space) -> [f64; 3] {
    match (function, space) {
        ("rgb", _) => [255.0; 3],
        ("hsl" | "hwb", _) => [1.0, 100.0, 100.0],
        (_, Space::Lab) => [100.0, 125.0, 125.0],
        (_, Space::Lch) => [100.0, 150.0, 1.0],
        (_, Space::Oklab) => [1.0, 0.4, 0.4],
        (_, Space::Oklch) => [1.0, 0.4, 1.0],
        _ => [1.0; 3],
    }
}

fn relative(r: &Relative) -> Option<Absolute> {
    let space = match (r.function, r.space) {
        ("rgb", _) => Space::Srgb,
        ("color", Some(p)) => Space::from_predefined(p),
        ("alpha", _) => absolute(&r.origin)?.space,
        (f, _) => Space::parse(f)?,
    };
    let origin = absolute(&r.origin)?.convert(space);
    let scale = scales(r.function, space);
    // The channel names, in the units the function writes its channels in.
    let names: &[&str] = match r.function {
        "rgb" => &["r", "g", "b"],
        "hsl" => &["h", "s", "l"],
        "hwb" => &["h", "w", "b"],
        "lab" | "oklab" => &["l", "a", "b"],
        "lch" | "oklch" => &["l", "c", "h"],
        "alpha" => &[],
        _ if matches!(space, Space::XyzD50 | Space::XyzD65) => &["x", "y", "z"],
        _ => &["r", "g", "b"],
    };
    let rgb = r.function == "rgb";
    // A name for a missing channel is missing too; in math it's 0.
    let named_missing = |k: &str| -> Option<Option<f64>> {
        if k == "alpha" {
            return Some(origin.alpha);
        }
        let i = names.iter().position(|n| *n == k)?;
        Some(origin.channels[i].map(|v| if rgb { v * 255.0 } else { v }))
    };
    let named = |k: &str| named_missing(k).map(|v| v.unwrap_or(0.0));
    let value = |c: &RelativeChannel, i: Option<usize>| -> Option<Option<f64>> {
        let pct = i.map_or(1.0, |i| scale[i]);
        Some(match c {
            RelativeChannel::None => None,
            RelativeChannel::Number(v) => Some(*v),
            RelativeChannel::Percentage(p) => Some(p / 100.0 * pct),
            RelativeChannel::Angle(v, u) => Some(v * u.to_canonical()?),
            RelativeChannel::Keyword(k) => named_missing(k)?,
            RelativeChannel::Math(m) => {
                let m = m.substitute_keywords(&named);
                Some(match m.leaf()? {
                    Leaf::Number(v) => v,
                    Leaf::Percentage(p) => p / 100.0 * pct,
                    Leaf::Dimension(v, u) => v * u.to_canonical()?,
                })
            }
        })
    };
    let mut channels = origin.channels;
    for (i, c) in r.channels.iter().enumerate() {
        let mut v = value(c, Some(i))?;
        if rgb {
            v = v.map(|x| x / 255.0);
        }
        channels[i] = v;
    }
    let alpha = match &r.alpha {
        Some(a) => value(a, None)?.map(|a| a.clamp(0.0, 1.0)),
        None => origin.alpha,
    };
    Some(Absolute {
        space,
        channels,
        alpha,
    })
}

/// What `color-mix()` mixes, and how: its space, its hue method, and
/// each colour with its percentage.
pub(crate) fn mix_parts(f: &crate::parser::Function<'_>) -> Option<super::color::MixParts> {
    if !f.name.eq_ignore_ascii_case("color-mix") {
        return None;
    }
    let parts: Vec<Vec<&ComponentValue<'_>>> = f
        .arguments
        .split(|v| matches!(v, ComponentValue::Token(Token::Comma, _)))
        .map(|p| p.iter().filter(|v| !v.is_whitespace()).collect())
        .collect();
    let (mut space, mut hue, mut start) = (Space::Oklab, "shorter", 0);
    if let Some(ComponentValue::Token(Token::Ident(w), _)) = parts[0].first().copied()
        && w.eq_ignore_ascii_case("in")
    {
        let ComponentValue::Token(Token::Ident(s), _) = parts[0].get(1)? else {
            return None;
        };
        space = Space::parse(s)?;
        if let Some(ComponentValue::Token(Token::Ident(h), _)) = parts[0].get(2) {
            hue = match h.to_ascii_lowercase().as_str() {
                "shorter" => "shorter",
                "longer" => "longer",
                "increasing" => "increasing",
                "decreasing" => "decreasing",
                _ => return None,
            };
        }
        start = 1;
    }
    let mut items = Vec::new();
    for p in &parts[start..] {
        let (color, pct) = match p[..] {
            [c] => (c, None),
            [c, ComponentValue::Token(Token::Percentage(n), _)]
            | [ComponentValue::Token(Token::Percentage(n), _), c] => (c, Some(n.value)),
            [c, ComponentValue::Function(m)] | [ComponentValue::Function(m), c]
                if super::math::Math::is_math_function(&m.name) =>
            {
                match super::math::Math::parse(m)?.leaf()? {
                    Leaf::Percentage(p) => (c, Some(p)),
                    _ => return None,
                }
            }
            _ => return None,
        };
        items.push((Color::parse(color)?, pct));
    }
    Some((space, hue, items))
}

/// CSS Color 5 §3: mix colours in `space`, premultiplied, the hue taken
/// the way `hue` says. Percentages left out share what the others leave
/// of 100%; more than two colours mix in turn, each by its share of the
/// weight so far.
fn mix(space: Space, hue: &str, items: &[(Absolute, Option<f64>)]) -> Option<Absolute> {
    if items.is_empty() {
        return None;
    }
    if items.len() == 1 {
        return Some(items[0].0.convert(space));
    }
    let given: f64 = items.iter().filter_map(|(_, p)| *p).sum();
    let missing = items.iter().filter(|(_, p)| p.is_none()).count();
    let share = if missing > 0 {
        ((100.0 - given) / missing as f64).max(0.0)
    } else {
        0.0
    };
    let weights: Vec<f64> = items.iter().map(|(_, p)| p.unwrap_or(share)).collect();
    let sum: f64 = weights.iter().sum();
    // Nothing of any: an even mix, transparent.
    let (multiplier, weights) = if sum == 0.0 {
        (0.0, vec![1.0; items.len()])
    } else {
        (if sum < 100.0 { sum / 100.0 } else { 1.0 }, weights)
    };
    let mut acc = items[0].0.convert(space);
    let mut acc_w = weights[0];
    for ((c, _), w) in items[1..].iter().zip(&weights[1..]) {
        let t = if acc_w + w == 0.0 {
            0.5
        } else {
            w / (acc_w + w)
        };
        acc = mix2(space, hue, &acc, c, t);
        acc_w += w;
    }
    acc.alpha = acc.alpha.map(|a| a * multiplier);
    Some(acc)
}

fn mix2(space: Space, hue: &str, a: &Absolute, b: &Absolute, t: f64) -> Absolute {
    let (a, b) = (a.convert(space), b.convert(space));
    let hue_at = space.hue();
    let (aa, ba) = (a.alpha, b.alpha);
    let alpha = match (aa, ba) {
        (None, None) => None,
        (x, y) => {
            let (x, y) = (x.or(y).unwrap(), y.or(x).unwrap());
            Some(x + (y - x) * t)
        }
    };
    let (wa, wb) = (aa.or(ba).unwrap_or(1.0), ba.or(aa).unwrap_or(1.0));
    let mut out = [None; 3];
    for i in 0..3 {
        let (x, y) = match (a.channels[i], b.channels[i]) {
            (None, None) => continue,
            (Some(x), None) => (x, x),
            (None, Some(y)) => (y, y),
            (Some(x), Some(y)) => (x, y),
        };
        out[i] = Some(if Some(i) == hue_at {
            let (mut x, mut y) = (x.rem_euclid(360.0), y.rem_euclid(360.0));
            let d = y - x;
            match hue {
                "shorter" if d > 180.0 => x += 360.0,
                "shorter" if d < -180.0 => y += 360.0,
                "longer" if d > 0.0 && d < 180.0 => x += 360.0,
                "longer" if d > -180.0 && d <= 0.0 => y += 360.0,
                "increasing" if y < x => y += 360.0,
                "decreasing" if x < y => x += 360.0,
                _ => {}
            }
            (x + (y - x) * t).rem_euclid(360.0)
        } else {
            // Premultiplied by alpha.
            let v = x * wa + (y * wb - x * wa) * t;
            match alpha {
                Some(al) if al != 0.0 => v / al,
                _ => v,
            }
        });
    }
    Absolute {
        space,
        channels: out,
        alpha,
    }
}
