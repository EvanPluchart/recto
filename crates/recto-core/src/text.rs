//! Page text as extracted by the renderer, and search over it.

use std::ops::Range;

use crate::geom::PageRect;

/// The characters of one page in reading order, each with its box on the displayed page.
/// Characters the extractor generated (line breaks, inferred spaces) have an empty box.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageText {
    pub chars: Vec<char>,
    pub boxes: Vec<PageRect>,
}

/// Search key for a character: case-folded, every kind of whitespace as a plain space.
fn fold(c: char) -> char {
    if c.is_whitespace() || c == '\u{2}' || c == '\u{fffe}' {
        ' '
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// `chars` folded with whitespace runs collapsed, and the index in `chars` each folded char came from.
fn normalise(chars: impl Iterator<Item = char>) -> (Vec<char>, Vec<usize>) {
    let mut folded = Vec::new();
    let mut origin = Vec::new();
    for (index, c) in chars.enumerate() {
        let c = fold(c);
        if c == ' ' && folded.last() == Some(&' ') {
            continue;
        }
        folded.push(c);
        origin.push(index);
    }
    (folded, origin)
}

impl PageText {
    /// Non-overlapping occurrences of `query`, as ranges of `chars`. Case-insensitive; any
    /// whitespace in the query matches any run of whitespace, line breaks included.
    pub fn find(&self, query: &str) -> Vec<Range<usize>> {
        let (needle, _) = normalise(query.trim().chars());
        if needle.is_empty() {
            return Vec::new();
        }
        let (haystack, origin) = normalise(self.chars.iter().copied());
        let mut hits = Vec::new();
        let mut at = 0;
        while at + needle.len() <= haystack.len() {
            if haystack[at..at + needle.len()] == needle[..] {
                hits.push(origin[at]..origin[at + needle.len() - 1] + 1);
                at += needle.len();
            } else {
                at += 1;
            }
        }
        hits
    }

    /// Boxes covering `range`, one per line of text.
    pub fn rects(&self, range: Range<usize>) -> Vec<PageRect> {
        let mut lines: Vec<PageRect> = Vec::new();
        for rect in self.boxes[range.start.min(self.boxes.len())..range.end.min(self.boxes.len())].iter() {
            if rect.is_empty() {
                continue;
            }
            match lines.last_mut() {
                // Same line: the boxes overlap vertically by at least half the smaller height.
                Some(line) if overlap(line.y, line.bottom(), rect.y, rect.bottom()) >= 0.5 * line.height.min(rect.height) => {
                    *line = line.union(rect);
                }
                _ => lines.push(*rect),
            }
        }
        lines
    }
}

fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    (a1.min(b1) - a0.max(b0)).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One box per char, 10 pt wide, on lines 20 pt apart; '\n' starts a new line with no box.
    fn page(text: &str) -> PageText {
        let (mut x, mut y) = (0.0, 0.0);
        let mut boxes = Vec::new();
        for c in text.chars() {
            if c == '\n' {
                boxes.push(PageRect::default());
                x = 0.0;
                y += 20.0;
            } else {
                boxes.push(PageRect { x, y, width: 10.0, height: 12.0 });
                x += 10.0;
            }
        }
        PageText { chars: text.chars().collect(), boxes }
    }

    #[test]
    fn finds_case_insensitively() {
        let text = page("Recto reads PDF; recto writes pdf.");
        assert_eq!(text.find("RECTO"), vec![0..5, 17..22]);
        assert_eq!(text.find("pdf"), vec![12..15, 30..33]);
        assert_eq!(text.find("  "), Vec::<Range<usize>>::new());
        assert_eq!(text.find("absent"), Vec::<Range<usize>>::new());
    }

    #[test]
    fn whitespace_matches_line_breaks() {
        let text = page("end of\r\nline  two");
        assert_eq!(text.find("of line"), vec![4..12]);
        assert_eq!(text.find("line two"), vec![8..17]);
    }

    #[test]
    fn occurrences_do_not_overlap() {
        assert_eq!(page("aaaa").find("aa"), vec![0..2, 2..4]);
    }

    #[test]
    fn rects_split_by_line() {
        let text = page("ab\ncd");
        assert_eq!(text.rects(0..5), vec![
            PageRect { x: 0.0, y: 0.0, width: 20.0, height: 12.0 },
            PageRect { x: 0.0, y: 20.0, width: 20.0, height: 12.0 },
        ]);
        assert_eq!(text.rects(1..2), vec![PageRect { x: 10.0, y: 0.0, width: 10.0, height: 12.0 }]);
        assert!(text.rects(2..3).is_empty());
    }
}
