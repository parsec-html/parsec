// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 James Conway

//! Delta debugging over an HTML page: the pieces a level splits the page
//! into, and ddmin, which removes as many of them as it can while the
//! failure still reproduces.
//!
//! The levels go from coarse to fine: tags and the text between them,
//! then attributes inside start tags, then words of text, then single
//! characters. Each one runs ddmin on what the one before left, and the
//! four run again until a whole round removes nothing. The order and the
//! splits depend only on the input, so the same page and the same check
//! give the same result every time.

use std::ops::Range;

/// One way of splitting a page into pieces that may be removed.
pub struct Level {
    pub name: &'static str,
    pub split: fn(&str) -> Vec<Range<usize>>,
}

pub const LEVELS: [Level; 4] = [
    Level {
        name: "tags",
        split: tags,
    },
    Level {
        name: "attributes",
        split: attributes,
    },
    Level {
        name: "text",
        split: words,
    },
    Level {
        name: "characters",
        split: chars,
    },
];

/// Every tag, comment and run of text, in order, covering the whole
/// input. A tag runs from `<` to the next `>`, a comment from `<!--` to
/// the next `-->`; one left open runs to the end.
pub fn tags(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < s.len() {
        let start = i;
        i = if s[i..].starts_with("<!--") {
            s[i + 4..].find("-->").map_or(s.len(), |e| i + 4 + e + 3)
        } else if s[i..].starts_with('<') {
            s[i + 1..].find('>').map_or(s.len(), |e| i + 1 + e + 1)
        } else {
            s[i..].find('<').map_or(s.len(), |e| i + e)
        };
        out.push(start..i);
    }
    out
}

/// Each attribute of each start tag, with the whitespace before it.
pub fn attributes(s: &str) -> Vec<Range<usize>> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    for tag in tags(s) {
        if !(b[tag.start] == b'<' && b.get(tag.start + 1).is_some_and(u8::is_ascii_alphabetic)) {
            continue;
        }
        let end = tag.end;
        let stop = |c: u8| c.is_ascii_whitespace() || c == b'>' || c == b'/';
        let mut j = tag.start + 1;
        while j < end && !stop(b[j]) {
            j += 1;
        }
        loop {
            let from = j;
            while j < end && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= end || b[j] == b'>' {
                break;
            }
            if b[j] == b'/' {
                j += 1;
                continue;
            }
            let name = j;
            while j < end && !stop(b[j]) && (b[j] != b'=' || j == name) {
                j += 1;
            }
            let mut k = j;
            while k < end && b[k].is_ascii_whitespace() {
                k += 1;
            }
            if k < end && b[k] == b'=' {
                k += 1;
                while k < end && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < end && (b[k] == b'"' || b[k] == b'\'') {
                    let quote = b[k];
                    k += 1;
                    while k < end && b[k] != quote {
                        k += 1;
                    }
                    k = (k + 1).min(end);
                } else {
                    while k < end && !b[k].is_ascii_whitespace() && b[k] != b'>' {
                        k += 1;
                    }
                }
                j = k;
            }
            out.push(from..j);
        }
    }
    out
}

/// Each word of the text between tags, with the whitespace after it.
pub fn words(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    for run in tags(s)
        .into_iter()
        .filter(|r| !s[r.clone()].starts_with('<'))
    {
        let text = &s[run.clone()];
        let mut start = 0;
        let mut in_space = false;
        for (i, c) in text.char_indices() {
            if c.is_whitespace() {
                in_space = true;
            } else if in_space {
                out.push(run.start + start..run.start + i);
                start = i;
                in_space = false;
            }
        }
        if start < text.len() {
            out.push(run.start + start..run.end);
        }
    }
    out
}

/// Each character.
pub fn chars(s: &str) -> Vec<Range<usize>> {
    s.char_indices().map(|(i, c)| i..i + c.len_utf8()).collect()
}

/// `s` without the pieces in `pieces` whose `keep` is false.
fn without(s: &str, pieces: &[Range<usize>], keep: &[bool]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut at = 0;
    for (r, &k) in pieces.iter().zip(keep) {
        out.push_str(&s[at..r.start]);
        if k {
            out.push_str(&s[r.clone()]);
        }
        at = r.end;
    }
    out.push_str(&s[at..]);
    out
}

/// ddmin over `pieces` of `s`: tries removing runs of half the pieces
/// still in, then quarters and so on down to one at a time, keeping
/// every removal after which `fails` still holds. What's left is
/// 1-minimal: removing any single piece more makes the failure go.
/// `progress` hears the size after each pass that removed something.
pub fn ddmin(
    s: &str,
    pieces: &[Range<usize>],
    fails: &mut dyn FnMut(&str) -> bool,
    progress: &mut dyn FnMut(usize),
) -> String {
    let mut keep = vec![true; pieces.len()];
    let mut live: Vec<usize> = (0..pieces.len()).collect();
    let mut chunk = live.len().div_ceil(2).max(1);
    while !live.is_empty() {
        let mut removed = false;
        let mut i = 0;
        while i < live.len() {
            let run = i..(i + chunk).min(live.len());
            for &p in &live[run.clone()] {
                keep[p] = false;
            }
            if fails(&without(s, pieces, &keep)) {
                live.drain(run);
                removed = true;
            } else {
                for &p in &live[run] {
                    keep[p] = true;
                }
                i += chunk;
            }
        }
        if removed {
            progress(without(s, pieces, &keep).len());
            chunk = chunk.min(live.len().div_ceil(2)).max(1);
        } else if chunk == 1 {
            break;
        } else {
            chunk = chunk.div_ceil(2);
        }
    }
    without(s, pieces, &keep)
}

/// Every level in turn, again and again, until a round removes nothing.
/// `progress` hears each level's name as it starts and the size after
/// each pass that removed something.
pub fn reduce(
    s: &str,
    fails: &mut dyn FnMut(&str) -> bool,
    progress: &mut dyn FnMut(&str, usize),
) -> String {
    let mut current = s.to_string();
    loop {
        let before = current.len();
        for level in &LEVELS {
            let pieces = (level.split)(&current);
            progress(level.name, current.len());
            current = ddmin(&current, &pieces, fails, &mut |n| progress(level.name, n));
        }
        if current.len() == before {
            return current;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pieces(s: &str, split: fn(&str) -> Vec<Range<usize>>) -> Vec<&str> {
        split(s).into_iter().map(|r| &s[r]).collect()
    }

    #[test]
    fn tags_cover_the_input() {
        let s = "a<p class=x>b<!-- c > d -->e</p><unclosed";
        assert_eq!(
            pieces(s, tags),
            [
                "a",
                "<p class=x>",
                "b",
                "<!-- c > d -->",
                "e",
                "</p>",
                "<unclosed"
            ]
        );
        assert_eq!(pieces(s, tags).concat(), s);
    }

    #[test]
    fn attributes_of_start_tags_only() {
        let s = r#"<a href="x y" b=1 c d = 'e'/></a b=2><!-- f=g -->"#;
        assert_eq!(
            pieces(s, attributes),
            [r#" href="x y""#, " b=1", " c", " d = 'e'"]
        );
        assert!(pieces("<p =x>", attributes) == [" =x"]);
        assert!(pieces("<br/>", attributes).is_empty());
    }

    #[test]
    fn words_of_text_only() {
        let s = "<p title='a b'>one  two\nthree</p> four";
        assert_eq!(pieces(s, words), ["one  ", "two\n", "three", " ", "four"]);
    }

    #[test]
    fn chars_are_whole_characters() {
        assert_eq!(pieces("aé€", chars), ["a", "é", "€"]);
    }

    #[test]
    fn ddmin_keeps_only_what_fails() {
        let s = "abcdefghij";
        let mut tests = 0;
        let out = ddmin(
            s,
            &chars(s),
            &mut |c| {
                tests += 1;
                c.contains('c') && c.contains('h')
            },
            &mut |_| {},
        );
        assert_eq!(out, "ch");
        assert!(tests < 40, "{tests} tests");
    }

    #[test]
    fn ddmin_with_nothing_to_remove() {
        assert_eq!(
            ddmin("ab", &chars("ab"), &mut |c| c == "ab", &mut |_| {}),
            "ab"
        );
        assert_eq!(ddmin("", &[], &mut |_| true, &mut |_| {}), "");
    }

    #[test]
    fn reduce_finds_the_nested_structure() {
        let mut page = String::from("<!DOCTYPE html><html><head><title>Shop</title></head><body>");
        for i in 0..300 {
            page.push_str(&format!(
                "<div class=\"item\" id=\"i{i}\"><a href=\"/p/{i}\">Item {i}</a> costs <b>${i}.00</b></div>\n"
            ));
            if i == 150 {
                page.push_str("<ul><li data-x=1><table><tr><td>bad</td></tr></table></li></ul>");
            }
        }
        page.push_str("</body></html>");
        // A stand-in for a bug: it shows on a table opened inside an li.
        let mut fails = |c: &str| {
            c.find("<li")
                .and_then(|li| c[li..].find("<table").map(|t| li + t))
                .is_some_and(|t| !c[..t].contains("</li"))
        };
        let once = reduce(&page, &mut fails, &mut |_, _| {});
        assert_eq!(once, "<li<table");
        let twice = reduce(&page, &mut fails, &mut |_, _| {});
        assert_eq!(once, twice);
    }
}
