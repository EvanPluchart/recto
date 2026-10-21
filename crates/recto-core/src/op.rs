//! Edit operations. Every user action becomes one `Op`; undo/redo moves a cursor in the log.

use crate::PageId;
use crate::annot::Annotation;
use crate::geom::Rect;

/// Clockwise rotation in quarter turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    R90,
    R180,
    R270,
}

impl Rotation {
    pub fn degrees(self) -> u16 {
        match self {
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    RotatePages { pages: Vec<PageId>, rotation: Rotation },
    DeletePages { pages: Vec<PageId> },
    /// Takes `pages` out (keeping their relative order) and puts them back so the first one lands
    /// at position `to` of the remaining pages. `to` past the end appends.
    MovePages { pages: Vec<PageId>, to: usize },
    /// Inserts `pages` (from a merged file or a blank page source) at position `at`.
    /// Pages already in the document are skipped.
    InsertPages { pages: Vec<PageId>, at: usize },
    /// A new annotation on `page`, in the page's user space.
    AddAnnotation { page: PageId, annotation: Annotation },
    /// Redaction removes content under `area`; it never only paints over it. (M4)
    Redact { page: PageId, area: Rect },
}

/// A page of the current document: which original page it is, and the rotation added to it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PageState {
    pub id: PageId,
    /// Clockwise degrees added to the page's own `/Rotate`: 0, 90, 180 or 270.
    pub rotation: u16,
}

/// Replays `ops` over the `initial` pages.
pub fn replay(initial: &[PageId], ops: &[Op]) -> Vec<PageState> {
    let mut pages: Vec<PageState> = initial.iter().map(|&id| PageState { id, rotation: 0 }).collect();
    for op in ops {
        match op {
            Op::RotatePages { pages: targets, rotation } => {
                for page in pages.iter_mut().filter(|page| targets.contains(&page.id)) {
                    page.rotation = (page.rotation + rotation.degrees()) % 360;
                }
            }
            Op::DeletePages { pages: targets } => pages.retain(|page| !targets.contains(&page.id)),
            Op::MovePages { pages: targets, to } => {
                let (moved, mut rest): (Vec<PageState>, Vec<PageState>) =
                    pages.into_iter().partition(|page| targets.contains(&page.id));
                let at = (*to).min(rest.len());
                rest.splice(at..at, moved);
                pages = rest;
            }
            Op::InsertPages { pages: new, at } => {
                let fresh: Vec<PageState> = new
                    .iter()
                    .filter(|id| !pages.iter().any(|page| page.id == **id))
                    .map(|&id| PageState { id, rotation: 0 })
                    .collect();
                let at = (*at).min(pages.len());
                pages.splice(at..at, fresh);
            }
            Op::AddAnnotation { .. } | Op::Redact { .. } => {}
        }
    }
    pages
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u32) -> PageId {
        PageId { source: 0, index }
    }

    fn ids(pages: &[PageState]) -> Vec<u32> {
        pages.iter().map(|page| page.id.index + 100 * page.id.source).collect()
    }

    fn initial(count: u32) -> Vec<PageId> {
        (0..count).map(id).collect()
    }

    #[test]
    fn rotations_accumulate() {
        let ops = [
            Op::RotatePages { pages: vec![id(1)], rotation: Rotation::R90 },
            Op::RotatePages { pages: vec![id(1), id(2)], rotation: Rotation::R270 },
        ];
        let pages = replay(&initial(3), &ops);
        assert_eq!(pages.iter().map(|page| page.rotation).collect::<Vec<_>>(), vec![0, 0, 270]);
    }

    #[test]
    fn deletes() {
        let pages = replay(&initial(4), &[Op::DeletePages { pages: vec![id(0), id(2)] }]);
        assert_eq!(ids(&pages), vec![1, 3]);
    }

    #[test]
    fn moves_relative_to_the_remaining_pages() {
        // Remaining after taking out 1 and 3: [0, 2, 4]; insert at 0.
        let pages = replay(&initial(5), &[Op::MovePages { pages: vec![id(3), id(1)], to: 0 }]);
        assert_eq!(ids(&pages), vec![1, 3, 0, 2, 4]);
        // To the end.
        let pages = replay(&initial(5), &[Op::MovePages { pages: vec![id(0)], to: 99 }]);
        assert_eq!(ids(&pages), vec![1, 2, 3, 4, 0]);
        // Page 4 moved between 1 and 2: remaining [0, 1, 2, 3], at 2.
        let pages = replay(&initial(5), &[Op::MovePages { pages: vec![id(4)], to: 2 }]);
        assert_eq!(ids(&pages), vec![0, 1, 4, 2, 3]);
    }

    #[test]
    fn inserts_new_pages_once() {
        let merged = [PageId { source: 1, index: 0 }, PageId { source: 1, index: 1 }];
        let ops = [
            Op::InsertPages { pages: merged.to_vec(), at: 1 },
            Op::InsertPages { pages: vec![merged[0], id(0)], at: 99 },
        ];
        assert_eq!(ids(&replay(&initial(2), &ops)), vec![0, 100, 101, 1]);
    }
}
