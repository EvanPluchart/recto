//! Choosing pages to extract and how to split a document. Page numbers typed by people are
//! 1-based; everything returned is 0-based positions in the current page order.

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RangeError {
    #[error("“{0}” is not a page or a range of pages")]
    Syntax(String),
    #[error("page {page} does not exist (the document has {count} pages)")]
    OutOfRange { page: usize, count: usize },
    #[error("no pages selected")]
    Empty,
}

/// One part of a spec: `5`, `2-7`, `9-` (to the end) or `-3` (from the start).
fn parse_part(part: &str, count: usize) -> Result<Vec<usize>, RangeError> {
    let number = |text: &str, default: usize| -> Result<usize, RangeError> {
        let text = text.trim();
        if text.is_empty() {
            return Ok(default);
        }
        let page: usize = text.parse().map_err(|_| RangeError::Syntax(part.trim().to_owned()))?;
        if page == 0 || page > count {
            return Err(RangeError::OutOfRange { page, count });
        }
        Ok(page)
    };
    let (first, last) = match part.split_once('-') {
        Some((from, to)) => (number(from, 1)?, number(to, count)?),
        None if part.trim().is_empty() => return Ok(Vec::new()),
        None => {
            let page = number(part, 0)?;
            (page, page)
        }
    };
    if first > last {
        return Err(RangeError::Syntax(part.trim().to_owned()));
    }
    Ok((first - 1..last).collect())
}

/// Groups of pages from a spec like `1-3, 5, 8-`: one group per comma-separated part.
pub fn parse_ranges(spec: &str, count: usize) -> Result<Vec<Vec<usize>>, RangeError> {
    let groups: Vec<Vec<usize>> = spec
        .split([',', ';'])
        .map(|part| parse_part(part, count))
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|group| !group.is_empty())
        .collect();
    if groups.is_empty() { Err(RangeError::Empty) } else { Ok(groups) }
}

/// All pages of a spec, in the order typed, without repeats (for extraction).
pub fn parse_pages(spec: &str, count: usize) -> Result<Vec<usize>, RangeError> {
    let mut pages = Vec::new();
    for page in parse_ranges(spec, count)?.into_iter().flatten() {
        if !pages.contains(&page) {
            pages.push(page);
        }
    }
    Ok(pages)
}

/// Consecutive groups of `size` pages (the last one may be shorter).
pub fn every(size: usize, count: usize) -> Vec<Vec<usize>> {
    let size = size.max(1);
    (0..count).collect::<Vec<_>>().chunks(size).map(<[usize]>::to_vec).collect()
}

/// Groups starting at each of `starts` (e.g. the pages of top-level bookmarks). Pages before
/// the first start form their own group.
pub fn at_starts(starts: &[usize], count: usize) -> Vec<Vec<usize>> {
    let mut cuts: Vec<usize> = starts.iter().copied().filter(|&start| start > 0 && start < count).collect();
    cuts.sort_unstable();
    cuts.dedup();
    let mut groups = Vec::new();
    let mut from = 0;
    for cut in cuts.into_iter().chain([count]) {
        if cut > from {
            groups.push((from..cut).collect());
        }
        from = cut;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ranges() {
        assert_eq!(parse_ranges("1-3, 5", 10), Ok(vec![vec![0, 1, 2], vec![4]]));
        assert_eq!(parse_ranges("8-", 10), Ok(vec![vec![7, 8, 9]]));
        assert_eq!(parse_ranges("-2;10", 10), Ok(vec![vec![0, 1], vec![9]]));
        assert_eq!(parse_ranges(" , ", 10), Err(RangeError::Empty));
        assert_eq!(parse_ranges("3-1", 10), Err(RangeError::Syntax("3-1".into())));
        assert_eq!(parse_ranges("a", 10), Err(RangeError::Syntax("a".into())));
        assert_eq!(parse_ranges("11", 10), Err(RangeError::OutOfRange { page: 11, count: 10 }));
        assert_eq!(parse_ranges("0", 10), Err(RangeError::OutOfRange { page: 0, count: 10 }));
    }

    #[test]
    fn pages_keep_typed_order_without_repeats() {
        assert_eq!(parse_pages("5, 1-3, 2", 10), Ok(vec![4, 0, 1, 2]));
    }

    #[test]
    fn splits_every_n() {
        assert_eq!(every(2, 5), vec![vec![0, 1], vec![2, 3], vec![4]]);
        assert_eq!(every(0, 2), vec![vec![0], vec![1]]);
        assert!(every(3, 0).is_empty());
    }

    #[test]
    fn splits_at_bookmarks() {
        assert_eq!(at_starts(&[2, 0, 5, 2], 7), vec![vec![0, 1], vec![2, 3, 4], vec![5, 6]]);
        assert_eq!(at_starts(&[], 3), vec![vec![0, 1, 2]]);
        assert_eq!(at_starts(&[1, 9], 2), vec![vec![0], vec![1]]);
    }
}
