//! Text selection with the mouse, across pages, and copying it.

use std::ops::Range;

use gpui::{ClipboardItem, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Window};
use recto_core::geom::{PageRect, Point};

use super::{Open, State, Viewer};

/// A position between two characters of a page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Caret {
    pub page: usize,
    pub index: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct Selection {
    anchor: Caret,
    head: Caret,
    dragging: bool,
}

impl Selection {
    fn ordered(&self) -> (Caret, Caret) {
        if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) }
    }

    /// Pages the selection touches.
    pub fn pages(&self) -> std::ops::RangeInclusive<usize> {
        let (start, end) = self.ordered();
        start.page..=end.page
    }

    /// Selected characters of page `page`, given its length.
    fn range_on(&self, page: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if page < start.page || page > end.page {
            return None;
        }
        let from = if page == start.page { start.index } else { 0 };
        let to = if page == end.page { end.index } else { len };
        (from < to).then_some(from..to.min(len))
    }
}

impl Viewer {
    /// The caret under a window position, if the page there has its text loaded.
    fn caret_at(&self, position: gpui::Point<gpui::Pixels>) -> Option<Caret> {
        let open = self.open()?;
        let layout = open.layout.as_ref()?;
        let origin = self.scroll.bounds().origin;
        let scroll = self.scroll_position();
        let content = Point {
            x: f32::from(position.x - origin.x) + scroll.x,
            y: f32::from(position.y - origin.y) + scroll.y,
        };
        let (page, offset) = layout.to_page(content)?;
        let index = open.texts.get(&page)?.caret_at(offset)?;
        Some(Caret { page, index })
    }

    pub(super) fn on_select_start(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus);
        self.selection = self.caret_at(event.position).map(|caret| Selection { anchor: caret, head: caret, dragging: true });
        cx.notify();
    }

    pub(super) fn on_select_move(&mut self, event: &MouseMoveEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(selection) = self.selection else { return };
        if !selection.dragging {
            return;
        }
        if event.pressed_button != Some(MouseButton::Left) {
            self.end_selection();
            return;
        }
        if let Some(head) = self.caret_at(event.position)
            && head != selection.head
            && let Some(selection) = &mut self.selection
        {
            selection.head = head;
            cx.notify();
        }
    }

    pub(super) fn end_selection(&mut self) {
        if let Some(selection) = &mut self.selection {
            selection.dragging = false;
        }
    }

    /// Selection boxes on page `page`, in page points.
    pub(super) fn selection_rects(&self, open: &Open, page: usize) -> Vec<PageRect> {
        let (Some(selection), Some(text)) = (self.selection, open.texts.get(&page)) else { return Vec::new() };
        selection.range_on(page, text.chars.len()).map(|range| text.rects(range)).unwrap_or_default()
    }

    pub(super) fn copy_selection(&mut self, cx: &mut Context<Self>) {
        let (Some(selection), State::Open(open)) = (self.selection, &self.state) else { return };
        let (start, end) = selection.ordered();
        let mut copied = String::new();
        for page in start.page..=end.page {
            let Some(text) = open.texts.get(&page) else { continue };
            if let Some(range) = selection.range_on(page, text.chars.len()) {
                if !copied.is_empty() && !copied.ends_with('\n') {
                    copied.push('\n');
                }
                copied.push_str(&text.slice(range));
            }
        }
        if !copied.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(copied));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caret(page: usize, index: usize) -> Caret {
        Caret { page, index }
    }

    #[test]
    fn ranges_span_pages_in_either_direction() {
        let forward = Selection { anchor: caret(1, 5), head: caret(3, 2), dragging: false };
        let backward = Selection { anchor: caret(3, 2), head: caret(1, 5), dragging: false };
        for selection in [forward, backward] {
            assert_eq!(selection.range_on(0, 10), None);
            assert_eq!(selection.range_on(1, 10), Some(5..10));
            assert_eq!(selection.range_on(2, 10), Some(0..10));
            assert_eq!(selection.range_on(3, 10), Some(0..2));
            assert_eq!(selection.range_on(4, 10), None);
        }
    }

    #[test]
    fn empty_selections_select_nothing() {
        let selection = Selection { anchor: caret(2, 4), head: caret(2, 4), dragging: true };
        assert_eq!(selection.range_on(2, 10), None);
    }
}
