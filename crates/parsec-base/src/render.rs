// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! [`Diagnostic::render`]: a diagnostic as a code frame, the lines it
//! points at with a gutter of line numbers and carets under the place.

use std::fmt::Write;

use crate::{Diagnostic, LineIndex};

/// How [`Diagnostic::render`] lays a code frame out.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderOptions<'a> {
    /// A name for the source, like a file's path, shown before the line
    /// and column.
    pub path: Option<&'a str>,
    /// A tab moves to the next multiple of this many columns.
    pub tab_width: u32,
    /// The most columns of a line shown. A longer line, like a minified
    /// script's, is cut to a window around what's marked on it, with `…`
    /// where it's cut.
    pub max_width: u32,
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        RenderOptions {
            path: None,
            tab_width: 4,
            max_width: 100,
        }
    }
}

/// A place to underline on one line: bytes `start..end` of the source.
struct Mark<'d> {
    line: u32,
    start: usize,
    end: usize,
    primary: bool,
    message: Option<&'d str>,
}

/// A line laid out in columns: each character's byte offset in the line,
/// its column and what's shown for it.
struct Laid {
    cells: Vec<(usize, u32, u32)>,
    shown: Vec<String>,
    width: u32,
}

impl Laid {
    fn new(text: &str, tab_width: u32) -> Laid {
        let tab_width = tab_width.max(1);
        let (mut cells, mut shown) = (Vec::new(), Vec::new());
        let mut col = 0;
        for (i, c) in text.char_indices() {
            let (w, s) = if c == '\t' {
                let w = tab_width - col % tab_width;
                (w, " ".repeat(w as usize))
            } else if is_hidden(c) {
                // Control characters and bidirectional overrides would act
                // on the terminal, or reorder the line, rather than show.
                (1, "\u{FFFD}".into())
            } else {
                (char_width(c), c.to_string())
            };
            cells.push((i, col, w));
            shown.push(s);
            col += w;
        }
        Laid {
            cells,
            shown,
            width: col,
        }
    }

    /// The column byte `offset` of the line starts at; inside a character
    /// counts as after it.
    fn col(&self, offset: usize) -> u32 {
        let i = self.cells.partition_point(|&(b, ..)| b < offset);
        self.cells.get(i).map_or(self.width, |&(_, c, _)| c)
    }
}

/// Characters a code frame shouldn't print as they are.
fn is_hidden(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Columns a character takes in a terminal: East Asian wide and
/// fullwidth characters and most emoji take two, combining marks and
/// joiners none, the rest one.
///
/// shortcut: a few ranges, not Unicode's EastAsianWidth table, so some
/// rare characters are measured wrong; use the table if one matters.
fn char_width(c: char) -> u32 {
    match c as u32 {
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x1DC0..=0x1DFF
        | 0x200B..=0x200F
        | 0x2060..=0x2064
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F
        | 0xFEFF
        | 0xE0100..=0xE01EF => 0,
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F680..=0x1F6FF
        | 0x1F900..=0x1F9FF
        | 0x1FA70..=0x1FAFF
        | 0x20000..=0x2FFFD
        | 0x30000..=0x3FFFD => 2,
        _ => 1,
    }
}

impl Diagnostic {
    /// The diagnostic as a code frame: a header with its severity, code
    /// and message, where it is, and each line it or a label points at,
    /// carets under its own span and dashes under each label's.
    ///
    /// `lines` is an index of `src`. Columns are counted as a terminal
    /// shows them: tabs expanded, and East Asian wide characters two
    /// columns.
    ///
    /// ```
    /// use parsec_base::{Diagnostic, LineIndex, RenderOptions, Span};
    ///
    /// let src = "if (ready {\n  go();\n}\n";
    /// let d = Diagnostic::error("unexpected-token", Span::new(10, 11), "`{` can't go here")
    ///     .with_label(Span::new(3, 4), "this `(` is still open");
    /// let frame = d.render(src, &LineIndex::new(src), &RenderOptions::default());
    /// assert_eq!(frame, "\
    /// error[unexpected-token]: `{` can't go here
    ///  --> 1:11
    ///   |
    /// 1 | if (ready {
    ///   |    -      ^
    ///   |    |
    ///   |    this `(` is still open
    /// ");
    /// ```
    pub fn render(&self, src: &str, lines: &LineIndex<'_>, options: &RenderOptions<'_>) -> String {
        let mut marks = Vec::new();
        push_marks(&mut marks, src, lines, self.span, true, None);
        for l in &self.labels {
            push_marks(&mut marks, src, lines, l.span, false, Some(&l.message));
        }
        let mut shown: Vec<u32> = marks.iter().map(|m| m.line).collect();
        shown.sort_unstable();
        shown.dedup();
        // A single line between two shown ones is shown too; it's no
        // longer than the `...` that would stand for it.
        let mut all = Vec::with_capacity(shown.len());
        for (i, &l) in shown.iter().enumerate() {
            if i > 0 && l == shown[i - 1] + 2 {
                all.push(l - 1);
            }
            all.push(l);
        }
        let gutter = all.last().map_or(1, |l| l.to_string().len());
        let pad = " ".repeat(gutter);

        let mut out = String::new();
        let _ = writeln!(out, "{}[{}]: {}", self.severity, self.code, self.message);
        let at = lines.line_col(self.span.start.min(src.len() as u32));
        match options.path {
            Some(p) => {
                let _ = writeln!(out, "{pad}--> {p}:{at}");
            }
            None => {
                let _ = writeln!(out, "{pad}--> {at}");
            }
        }
        let _ = writeln!(out, "{pad} |");
        for (i, &line) in all.iter().enumerate() {
            if i > 0 && line > all[i - 1] + 1 {
                out.push_str("...\n");
            }
            let (start, text) = line_text(src, lines, line);
            let laid = Laid::new(text, options.tab_width);
            let mut on: Vec<&Mark> = marks.iter().filter(|m| m.line == line).collect();
            on.sort_by_key(|m| (m.start, !m.primary));
            // Each mark's columns, start and end, on the line.
            let cols: Vec<(u32, u32)> = on
                .iter()
                .map(|m| {
                    let a = laid.col(m.start - start);
                    (a, laid.col(m.end - start).max(a + 1))
                })
                .collect();
            let (body, shift, lo, hi) = window(&laid, &cols, options.max_width);
            let _ = writeln!(out, "{line:>gutter$} | {body}");
            if on.is_empty() {
                continue;
            }
            // Columns in the shown line, clipped to the window.
            let place = |c: u32| c.clamp(lo, hi) - lo + shift;
            let spans: Vec<(u32, u32)> = cols
                .iter()
                .map(|&(a, b)| (place(a), place(b).max(place(a) + 1)))
                .collect();
            let mut row: Vec<char> = Vec::new();
            for primary in [false, true] {
                for (m, &(a, b)) in on.iter().zip(&spans) {
                    if m.primary != primary {
                        continue;
                    }
                    let fill = if primary { '^' } else { '-' };
                    if row.len() < b as usize {
                        row.resize(b as usize, ' ');
                    }
                    for c in &mut row[a as usize..b as usize] {
                        *c = fill;
                    }
                }
            }
            let mut first: String = row.into_iter().collect();
            // The rightmost mark's message goes on the underline; the
            // others hang below, rightmost first.
            let mut hanging: Vec<usize> =
                (0..on.len()).filter(|&k| on[k].message.is_some()).collect();
            let last = (0..on.len()).max_by_key(|&k| (spans[k].0, k));
            if let Some(k) = last.filter(|&k| hanging.last() == Some(&k)) {
                let _ = write!(first, " {}", on[k].message.unwrap_or(""));
                hanging.pop();
            }
            let _ = writeln!(out, "{pad} | {}", first.trim_end());
            while let Some(k) = hanging.pop() {
                let left: Vec<u32> = hanging.iter().map(|&j| spans[j].0).collect();
                let mut both = left.clone();
                both.push(spans[k].0);
                let _ = writeln!(out, "{pad} | {}", pipes(&both, None));
                let _ = writeln!(
                    out,
                    "{pad} | {}",
                    pipes(&left, Some((spans[k].0, on[k].message.unwrap_or(""))))
                );
            }
        }
        out
    }
}

/// A `|` at each of `cols`, and a message starting at a column.
fn pipes(cols: &[u32], message: Option<(u32, &str)>) -> String {
    let mut r: Vec<char> = Vec::new();
    for &c in cols {
        if r.len() <= c as usize {
            r.resize(c as usize + 1, ' ');
        }
        r[c as usize] = '|';
    }
    let mut s: String = r.into_iter().collect();
    if let Some((c, m)) = message {
        let w = s.chars().count();
        s.push_str(&" ".repeat((c as usize).saturating_sub(w)));
        s.push_str(m);
    }
    s
}

/// The marks for `span`: one on its line, or for a span over several
/// lines, one at its start and one at its end, which has the message.
fn push_marks<'d>(
    marks: &mut Vec<Mark<'d>>,
    src: &str,
    lines: &LineIndex<'_>,
    span: crate::Span,
    primary: bool,
    message: Option<&'d str>,
) {
    let start = (span.start as usize).min(src.len());
    let end = (span.end as usize).clamp(start, src.len());
    let first = lines.line(start as u32);
    // A span that ends with a line terminator ends on that line.
    let last = lines.line(end.saturating_sub(1).max(start) as u32);
    if first == last {
        marks.push(Mark {
            line: first,
            start,
            end,
            primary,
            message,
        });
        return;
    }
    let (s, text) = line_text(src, lines, first);
    marks.push(Mark {
        line: first,
        start,
        end: s + text.len(),
        primary,
        message: None,
    });
    let (s, _) = line_text(src, lines, last);
    marks.push(Mark {
        line: last,
        start: s,
        end,
        primary,
        message,
    });
}

/// Where line `line` starts, and its text without its terminator.
fn line_text<'s>(src: &'s str, lines: &LineIndex<'_>, line: u32) -> (usize, &'s str) {
    let start = lines.line_start(line).map_or(src.len(), |s| s as usize);
    let end = lines.line_start(line + 1).map_or(src.len(), |s| s as usize);
    let text = src.get(start..end).unwrap_or("");
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix(['\n', '\r', '\u{2028}', '\u{2029}']))
        .unwrap_or(text);
    (start, text)
}

/// The line as shown, cut to `max` columns around the marks if it's
/// longer: the text, the columns a leading `…` adds, and the window of
/// the line's columns shown.
fn window(laid: &Laid, cols: &[(u32, u32)], max: u32) -> (String, u32, u32, u32) {
    let all = |laid: &Laid| laid.shown.concat();
    if laid.width <= max || max < 3 {
        return (all(laid), 0, 0, laid.width.max(1) + 1);
    }
    let inner = max - 2;
    let lo_m = cols.iter().map(|c| c.0).min().unwrap_or(0);
    let hi_m = cols.iter().map(|c| c.1).max().unwrap_or(0);
    // Every mark if they fit, or else the first.
    let (a, b) = if hi_m - lo_m <= inner {
        (lo_m, hi_m)
    } else {
        cols.first().map_or((0, 1), |&(a, _)| (a, a + 1))
    };
    let centre = (a + b) / 2;
    let lo = centre.saturating_sub(inner / 2).min(laid.width - inner);
    let hi = lo + inner;
    let mut s = String::new();
    if lo > 0 {
        s.push('…');
    }
    for (k, &(_, c, w)) in laid.cells.iter().enumerate() {
        if c >= lo && c + w <= hi {
            s.push_str(&laid.shown[k]);
        }
    }
    if hi < laid.width {
        s.push('…');
    }
    (s, u32::from(lo > 0), lo, hi)
}

#[cfg(test)]
mod tests {
    use crate::{Diagnostic, LineIndex, RenderOptions, Span};

    fn render(src: &str, d: &Diagnostic) -> String {
        d.render(src, &LineIndex::new(src), &RenderOptions::default())
    }

    #[test]
    fn caret_under_the_span() {
        let src = "let x = ;\n";
        let d = Diagnostic::error("unexpected-token", Span::new(8, 9), "`;` can't go here");
        assert_eq!(
            render(src, &d),
            "error[unexpected-token]: `;` can't go here\n --> 1:9\n  |\n1 | let x = ;\n  |         ^\n"
        );
    }

    #[test]
    fn path_and_warning() {
        let src = "a";
        let d = Diagnostic::warning("odd", Span::new(0, 1), "odd");
        let o = RenderOptions {
            path: Some("src/app.js"),
            ..RenderOptions::default()
        };
        assert_eq!(
            d.render(src, &LineIndex::new(src), &o),
            "warning[odd]: odd\n --> src/app.js:1:1\n  |\n1 | a\n  | ^\n"
        );
    }

    #[test]
    fn empty_span_and_end_of_input() {
        // An empty span, at the end, gets one caret after the last
        // character.
        let src = "f(a, b";
        let d = Diagnostic::error("unexpected-end", Span::at(6), "the input ends")
            .with_label(Span::new(1, 2), "this `(` is still open");
        assert_eq!(
            render(src, &d),
            "error[unexpected-end]: the input ends\n --> 1:7\n  |\n1 | f(a, b\n  |  -    ^\n  |  |\n  |  this `(` is still open\n"
        );
    }

    #[test]
    fn labels_on_other_lines() {
        let src = "let a = 1;\nlet b;\nlet c;\nlet d;\nlet a = 2;\n";
        let d = Diagnostic::error(
            "duplicate-declaration",
            Span::new(36, 37),
            "`a` is declared twice",
        )
        .with_label(Span::new(4, 5), "declared here first");
        assert_eq!(
            render(src, &d),
            "error[duplicate-declaration]: `a` is declared twice\n --> 5:5\n  |\n1 | let a = 1;\n  |     - declared here first\n...\n5 | let a = 2;\n  |     ^\n"
        );
        // One line between is shown, not elided.
        let src = "let a;\nx;\nlet a;\n";
        let d = Diagnostic::error("dup", Span::new(14, 15), "twice")
            .with_label(Span::new(4, 5), "first");
        assert_eq!(
            render(src, &d),
            "error[dup]: twice\n --> 3:5\n  |\n1 | let a;\n  |     - first\n2 | x;\n3 | let a;\n  |     ^\n"
        );
    }

    #[test]
    fn rightmost_message_inline_others_hang() {
        let src = "f(a, [b, c";
        let d = Diagnostic::error("e", Span::at(10), "ends")
            .with_label(Span::new(1, 2), "this `(` is still open")
            .with_label(Span::new(5, 6), "this `[` is still open");
        assert_eq!(
            render(src, &d),
            "error[e]: ends\n --> 1:11\n  |\n1 | f(a, [b, c\n  |  -   -    ^\n  |  |   |\n  |  |   this `[` is still open\n  |  |\n  |  this `(` is still open\n"
        );
        // A label right of the primary has its message on the underline.
        let d =
            Diagnostic::error("e", Span::new(0, 1), "here").with_label(Span::new(5, 6), "there");
        assert!(render(src, &d).contains("  | ^    - there\n"));
    }

    #[test]
    fn tabs_expand() {
        let src = "\tx\t= ;";
        let d = Diagnostic::error("e", Span::new(5, 6), "m");
        // The tab to column 4, `x`, the tab to 8, `=` and a space.
        assert!(render(src, &d).ends_with("1 |     x   = ;\n  |           ^\n"));
        let o = RenderOptions {
            tab_width: 2,
            ..RenderOptions::default()
        };
        let out = d.render(src, &LineIndex::new(src), &o);
        assert!(out.ends_with("1 |   x = ;\n  |       ^\n"), "{out}");
    }

    #[test]
    fn wide_and_combining_characters() {
        // 名前 is two columns a character; é as e and a combining accent
        // is one.
        let src = "let 名前 = e\u{301} +;";
        let at = src.find(';').unwrap() as u32;
        let d = Diagnostic::error("e", Span::new(at, at + 1), "m");
        let out = render(src, &d);
        assert!(out.contains(&format!("  | {}^\n", " ".repeat(14))), "{out}");
        // A span over a wide character is two carets.
        let d = Diagnostic::error("e", Span::new(4, 7), "m");
        assert!(render(src, &d).contains("  |     ^^\n"));
        // The column in the header is in characters.
        assert!(out.contains(" --> 1:14\n"), "{out}");
    }

    #[test]
    fn control_and_bidi_characters_are_replaced() {
        let src = "a\u{1b}[2J\u{202E}b";
        let d = Diagnostic::error("e", Span::new(0, 1), "m");
        let out = render(src, &d);
        assert!(!out.contains('\u{1b}') && !out.contains('\u{202E}'));
        assert!(out.contains("1 | a\u{FFFD}[2J\u{FFFD}b\n"));
    }

    #[test]
    fn long_lines_are_windowed() {
        let src = format!("{}let x = ;{}", "a".repeat(500), "b".repeat(500));
        let d = Diagnostic::error("e", Span::new(508, 509), "m");
        let out = render(&src, &d);
        let lines: Vec<&str> = out.lines().collect();
        let body = lines[3].strip_prefix("1 | ").unwrap();
        assert!(body.starts_with('…') && body.ends_with('…'), "{body}");
        assert_eq!(body.chars().count(), 100);
        assert!(body.contains("let x = ;"));
        // The caret is still under the `;`.
        let caret = lines[4].strip_prefix("  | ").unwrap();
        let col = caret.find('^').unwrap();
        assert_eq!(body.chars().nth(col), Some(';'));
        // Near the start, nothing is cut there.
        let d = Diagnostic::error("e", Span::new(2, 3), "m");
        let out = render(&src, &d);
        let body = out.lines().nth(3).unwrap().strip_prefix("1 | ").unwrap();
        assert!(body.starts_with("aaa") && body.ends_with('…'));
        assert!(out.lines().nth(4).unwrap().ends_with("  ^"));
        // Marks too far apart to share a window: it's around the first.
        let d = Diagnostic::error("e", Span::new(990, 991), "m").with_label(Span::new(3, 4), "far");
        let out = render(&src, &d);
        assert!(out.contains("far"));
    }

    #[test]
    fn spans_over_several_lines() {
        let src = "f(function () {\n  a;\n  b;\n});\n";
        let d = Diagnostic::error("e", Span::new(2, 27), "m");
        let out = render(src, &d);
        assert_eq!(
            out,
            "error[e]: m\n --> 1:3\n  |\n1 | f(function () {\n  |   ^^^^^^^^^^^^^\n...\n4 | });\n  | ^\n"
        );
    }

    #[test]
    fn line_terminators_and_gutter_width() {
        let src = "a\r\nb\rc\n".repeat(4) + "bad";
        let at = src.len() as u32 - 3;
        let d =
            Diagnostic::error("e", Span::new(at, at + 3), "m").with_label(Span::new(0, 1), "first");
        let out = render(&src, &d);
        assert!(out.contains("\n  --> 13:1\n"), "{out}");
        assert!(out.contains("\n 1 | a\n   | - first\n"), "{out}");
        assert!(out.contains("\n13 | bad\n   | ^^^\n"), "{out}");
        assert!(!out.contains('\r'));
        // ECMAScript's separators end lines too, with that index.
        let src = "a\u{2028}b;";
        let d = Diagnostic::error("e", Span::new(4, 5), "m");
        let out = d.render(src, &LineIndex::ecmascript(src), &RenderOptions::default());
        assert!(out.contains("2 | b;\n  | ^\n"), "{out}");
    }

    #[test]
    fn spans_out_of_range_are_clamped() {
        let src = "ab";
        let d = Diagnostic::error("e", Span::new(40, 50), "m");
        assert!(render(src, &d).ends_with("1 | ab\n  |   ^\n"));
        assert!(render("", &Diagnostic::error("e", Span::at(0), "m")).ends_with("1 | \n  | ^\n"));
    }
}
