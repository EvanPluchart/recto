//! Page selection in the thumbnail rail, and where dragged pages land. Pure logic, no GPUI.

use recto_core::PageId;

/// Selected pages, by identity so they survive reordering.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RailSelection {
    selected: Vec<PageId>,
    /// Where Shift+click ranges start.
    anchor: Option<PageId>,
}

impl RailSelection {
    pub fn contains(&self, page: PageId) -> bool {
        self.selected.contains(&page)
    }

    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
    }

    /// Selected pages in document order (`order` is the current page order).
    pub fn in_order(&self, order: &[PageId]) -> Vec<PageId> {
        order.iter().copied().filter(|page| self.selected.contains(page)).collect()
    }

    /// Drops pages that no longer exist.
    pub fn retain_existing(&mut self, order: &[PageId]) {
        self.selected.retain(|page| order.contains(page));
        if self.anchor.is_some_and(|anchor| !order.contains(&anchor)) {
            self.anchor = None;
        }
    }

    /// A click on the page at `index`: alone it selects that page, Ctrl/Cmd toggles it,
    /// Shift extends from the anchor.
    pub fn click(&mut self, order: &[PageId], index: usize, toggle: bool, extend: bool) {
        let Some(&page) = order.get(index) else { return };
        let anchor = self.anchor.and_then(|anchor| order.iter().position(|&p| p == anchor));
        match (extend, anchor) {
            (true, Some(from)) => {
                let range = from.min(index)..=from.max(index);
                if !toggle {
                    self.selected.clear();
                }
                for &page in &order[range] {
                    if !self.selected.contains(&page) {
                        self.selected.push(page);
                    }
                }
                return; // the anchor stays
            }
            _ if toggle => {
                if let Some(at) = self.selected.iter().position(|&p| p == page) {
                    self.selected.remove(at);
                } else {
                    self.selected.push(page);
                }
            }
            _ => self.selected = vec![page],
        }
        self.anchor = Some(page);
    }

    /// Pages a drag started on `index` carries: the selection if the page is part of it,
    /// otherwise that page alone.
    pub fn dragged(&self, order: &[PageId], index: usize) -> Vec<PageId> {
        match order.get(index) {
            Some(page) if self.contains(*page) => self.in_order(order),
            Some(page) => vec![*page],
            None => Vec::new(),
        }
    }
}

/// `MovePages::to` for `moved` pages dropped just before the page at `before` (or at the end
/// when `before == order.len()`): the drop position counted among the pages that stay.
pub fn move_target(order: &[PageId], moved: &[PageId], before: usize) -> usize {
    order[..before.min(order.len())].iter().filter(|page| !moved.contains(page)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use recto_core::op::{Op, replay};

    fn pages(count: u32) -> Vec<PageId> {
        (0..count).map(|index| PageId { source: 0, index }).collect()
    }

    fn indices(selection: &RailSelection, order: &[PageId]) -> Vec<u32> {
        selection.in_order(order).iter().map(|page| page.index).collect()
    }

    #[test]
    fn clicks_select_toggle_and_extend() {
        let order = pages(6);
        let mut selection = RailSelection::default();
        selection.click(&order, 1, false, false);
        assert_eq!(indices(&selection, &order), vec![1]);
        selection.click(&order, 4, false, true);
        assert_eq!(indices(&selection, &order), vec![1, 2, 3, 4]);
        // Shift again re-extends from the same anchor.
        selection.click(&order, 0, false, true);
        assert_eq!(indices(&selection, &order), vec![0, 1]);
        selection.click(&order, 5, true, false);
        assert_eq!(indices(&selection, &order), vec![0, 1, 5]);
        selection.click(&order, 0, true, false);
        assert_eq!(indices(&selection, &order), vec![1, 5]);
        selection.click(&order, 3, false, false);
        assert_eq!(indices(&selection, &order), vec![3]);
    }

    #[test]
    fn drags_carry_the_selection_or_the_page() {
        let order = pages(5);
        let mut selection = RailSelection::default();
        selection.click(&order, 3, false, false);
        selection.click(&order, 1, true, false);
        assert_eq!(selection.dragged(&order, 3), vec![order[1], order[3]]);
        assert_eq!(selection.dragged(&order, 2), vec![order[2]]);
    }

    #[test]
    fn drop_positions_match_move_pages() {
        let order = pages(5);
        // Drag page 4 before page 1, and pages 0 and 1 to the end.
        for (moved, before, expected) in [
            (vec![order[4]], 1, vec![0, 4, 1, 2, 3]),
            (vec![order[0], order[1]], 5, vec![2, 3, 4, 0, 1]),
            (vec![order[1]], 1, vec![0, 1, 2, 3, 4]),
            (vec![order[1]], 3, vec![0, 2, 1, 3, 4]),
        ] {
            let to = move_target(&order, &moved, before);
            let result = replay(&order, &[Op::MovePages { pages: moved, to }]);
            assert_eq!(result.iter().map(|page| page.id.index).collect::<Vec<_>>(), expected);
        }
    }

    #[test]
    fn deleted_pages_leave_the_selection() {
        let order = pages(3);
        let mut selection = RailSelection::default();
        selection.click(&order, 2, false, false);
        selection.retain_existing(&order[..2]);
        assert!(selection.in_order(&order).is_empty());
    }
}
