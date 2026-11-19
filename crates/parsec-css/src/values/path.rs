// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! SVG path data, the string in `path()`: parsed by SVG 2's grammar
//! (§9.3.9) and written as browsers write it back, each command with its
//! letter, numbers in their shortest form, one space apart, and `z` as
//! `Z`. `path()` with data that doesn't parse, or with none, is invalid.

use super::units::write_number;

/// How many numbers each command takes.
fn arity(command: u8) -> Option<usize> {
    Some(match command.to_ascii_lowercase() {
        b'm' | b'l' | b't' => 2,
        b'h' | b'v' => 1,
        b'c' => 6,
        b's' | b'q' => 4,
        b'a' => 7,
        b'z' => 0,
        _ => return None,
    })
}

struct Reader<'s> {
    b: &'s [u8],
    i: usize,
}

impl Reader<'_> {
    fn space(&mut self) {
        while self
            .b
            .get(self.i)
            .is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c'))
        {
            self.i += 1;
        }
    }

    /// Whitespace with at most one comma in it.
    fn separator(&mut self) {
        self.space();
        if self.b.get(self.i) == Some(&b',') {
            self.i += 1;
            self.space();
        }
    }

    fn number(&mut self) -> Option<f64> {
        let start = self.i;
        let b = self.b;
        if matches!(b.get(self.i), Some(b'+' | b'-')) {
            self.i += 1;
        }
        let digits = |r: &mut Self| {
            let s = r.i;
            while b.get(r.i).is_some_and(u8::is_ascii_digit) {
                r.i += 1;
            }
            r.i > s
        };
        let whole = digits(self);
        let mut fraction = false;
        if b.get(self.i) == Some(&b'.') {
            self.i += 1;
            fraction = digits(self);
        }
        if !whole && !fraction {
            self.i = start;
            return None;
        }
        if matches!(b.get(self.i), Some(b'e' | b'E')) {
            let mark = self.i;
            self.i += 1;
            if matches!(b.get(self.i), Some(b'+' | b'-')) {
                self.i += 1;
            }
            if !digits(self) {
                self.i = mark;
            }
        }
        std::str::from_utf8(&b[start..self.i]).ok()?.parse().ok()
    }

    /// An arc's flag: `0` or `1`, which needs nothing after it.
    fn flag(&mut self) -> Option<f64> {
        match self.b.get(self.i) {
            Some(b'0') => {
                self.i += 1;
                Some(0.0)
            }
            Some(b'1') => {
                self.i += 1;
                Some(1.0)
            }
            _ => None,
        }
    }
}

/// The path data in `d` as it's written back, or `None` if it doesn't
/// parse or is empty.
pub(crate) fn normalize(d: &str) -> Option<String> {
    let mut r = Reader {
        b: d.as_bytes(),
        i: 0,
    };
    let mut out = String::new();
    r.space();
    let mut command: Option<u8> = None;
    while r.i < r.b.len() {
        let c = r.b[r.i];
        let letter = if arity(c).is_some() && c.is_ascii_alphabetic() {
            r.i += 1;
            r.space();
            c
        } else {
            // Numbers after a command's own repeat it; after a moveto, they
            // are linetos.
            match command? {
                b'M' => b'L',
                b'm' => b'l',
                b'z' | b'Z' => return None,
                other => other,
            }
        };
        if command.is_none() && !matches!(letter, b'M' | b'm') {
            return None;
        }
        let n = arity(letter)?;
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(if letter == b'z' { 'Z' } else { letter as char });
        for k in 0..n {
            if k > 0 {
                r.separator();
            }
            let arc_flag = letter.eq_ignore_ascii_case(&b'a') && matches!(k, 3 | 4);
            let v = if arc_flag { r.flag()? } else { r.number()? };
            out.push(' ');
            write_number(&mut out, v).ok()?;
        }
        command = Some(letter);
        if n > 0 {
            r.separator();
        } else {
            r.space();
        }
    }
    (!out.is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn path_data() {
        let cases = [
            ("M10,10h80v80h-80z", Some("M 10 10 h 80 v 80 h -80 Z")),
            ("m 1 2   v 3.00 z", Some("m 1 2 v 3 Z")),
            ("M0,0A25,25,0,0,1,50,0", Some("M 0 0 A 25 25 0 0 1 50 0")),
            ("M 0 0 10 10", Some("M 0 0 L 10 10")),
            ("M0 0a1 1 0 1150 0", Some("M 0 0 a 1 1 0 1 1 50 0")),
            ("", None),
            (" ", None),
            ("not-a-path", None),
            ("M 20 30 A 60 70 80", None),
            ("L 1 2", None),
        ];
        for (d, want) in cases {
            assert_eq!(normalize(d).as_deref(), want, "{d}");
        }
    }
}
