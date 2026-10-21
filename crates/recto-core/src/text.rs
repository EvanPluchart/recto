//! Page text as extracted by the renderer, and search over it.

use std::ops::Range;

use crate::geom::{PageRect, Point};

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

    /// Caret position (a char boundary, `0..=len`) nearest to `point`, in displayed page points.
    /// The nearest character wins; the caret goes before or after it depending on which half of
    /// its box the point is in. `None` when the page has no positioned text.
    pub fn caret_at(&self, point: Point) -> Option<usize> {
        let distance = |rect: &PageRect| {
            let dx = (rect.x - point.x).max(point.x - rect.right()).max(0.0);
            let dy = (rect.y - point.y).max(point.y - rect.bottom()).max(0.0);
            // Being on the right line matters more than being near horizontally.
            dx + 4.0 * dy
        };
        let (index, rect) = self
            .boxes
            .iter()
            .enumerate()
            .filter(|(_, rect)| !rect.is_empty())
            .min_by(|(_, a), (_, b)| distance(a).total_cmp(&distance(b)))?;
        Some(if point.x > rect.x + rect.width / 2.0 { index + 1 } else { index })
    }

    /// The text of `range`, with the extractor's `\r\n` line breaks as `\n`.
    pub fn slice(&self, range: Range<usize>) -> String {
        let end = range.end.min(self.chars.len());
        self.chars[range.start.min(end)..end].iter().filter(|&&c| c != '\r').collect()
    }

    /// Boxes covering `range`, one per line of text. Lines end at the line breaks the extractor
    /// inserted, which works for any text direction.
    pub fn rects(&self, range: Range<usize>) -> Vec<PageRect> {
        let end = range.end.min(self.chars.len()).min(self.boxes.len());
        let mut lines: Vec<PageRect> = Vec::new();
        let mut current = PageRect::default();
        for index in range.start.min(end)..end {
            if matches!(self.chars[index], '\r' | '\n') {
                if !current.is_empty() {
                    lines.push(current);
                }
                current = PageRect::default();
            } else {
                current = current.union(&self.boxes[index]);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
        lines
    }
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
    fn carets_follow_the_pointer() {
        let text = page("ab\ncd");
        let at = |x, y| text.caret_at(Point { x, y });
        assert_eq!(at(2.0, 5.0), Some(0)); // left half of 'a'
        assert_eq!(at(8.0, 5.0), Some(1)); // right half of 'a'
        assert_eq!(at(500.0, 5.0), Some(2)); // past the end of line 1
        assert_eq!(at(-50.0, 25.0), Some(3)); // before 'c' on line 2
        assert_eq!(at(18.0, 1000.0), Some(5)); // below everything: nearest is 'd', right half
        assert_eq!(PageText::default().caret_at(Point { x: 0.0, y: 0.0 }), None);
    }

    #[test]
    fn slices_drop_carriage_returns() {
        let text = page("ab\r\ncd");
        assert_eq!(text.slice(1..6), "b\ncd");
        assert_eq!(text.slice(4..99), "cd");
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
