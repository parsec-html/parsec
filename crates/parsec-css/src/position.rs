// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Line and column for a byte offset. Spans are byte offsets because
//! they're free to keep; lines are counted only when you ask.

/// The start of every line in a text, for turning offsets into lines and
/// columns. CR LF, CR, LF and form feed each end a line, as CSS counts.
///
/// ```
/// use parsec_css::LineIndex;
///
/// let css = "a {\r\n  color: réd;\n}";
/// let lines = LineIndex::new(css);
/// let at = css.find("réd").unwrap();
/// assert_eq!(lines.line_column(css, at), (2, 10));
/// assert_eq!(lines.line_column(css, css.len()), (3, 2));
/// ```
#[derive(Clone, Debug)]
pub struct LineIndex {
    starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(text: &str) -> LineIndex {
        let b = text.as_bytes();
        let mut starts = vec![0];
        let mut i = 0;
        while i < b.len() {
            match b[i] {
                b'\r' if b.get(i + 1) == Some(&b'\n') => {
                    i += 1;
                    starts.push(i + 1);
                }
                b'\n' | b'\r' | b'\x0c' => starts.push(i + 1),
                _ => {}
            }
            i += 1;
        }
        LineIndex { starts }
    }

    /// The 1-based line and column of `offset` in `text`, the string this
    /// index was made from. Columns count code points.
    pub fn line_column(&self, text: &str, offset: usize) -> (usize, usize) {
        let line = self.starts.partition_point(|&s| s <= offset) - 1;
        let column = text[self.starts[line]..offset].chars().count() + 1;
        (line + 1, column)
    }
}
