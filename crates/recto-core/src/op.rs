//! Edit operations. Every user action becomes one `Op`; undo/redo moves a cursor in the log.

use crate::PageId;
use crate::geom::Rect;

/// Clockwise rotation in quarter turns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rotation {
    R90,
    R180,
    R270,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    RotatePages { pages: Vec<PageId>, rotation: Rotation },
    DeletePages { pages: Vec<PageId> },
    /// Move `pages` so the first one lands at position `to` in the current order.
    MovePages { pages: Vec<PageId>, to: usize },
    /// Redaction removes content under `area`; it never only paints over it.
    Redact { page: PageId, area: Rect },
}
