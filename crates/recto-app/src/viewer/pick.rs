//! Picking annotations made in Recto (edit mode, Select tool): a click selects one, dragging it
//! moves it, dragging a corner handle resizes it, Delete removes it.

use gpui::{AnyElement, Context, IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement, Styled, Window, div, px};
use recto_core::annot::Annotation;
use recto_core::geom::{PageRect, Point};
use recto_core::op::Op;

use super::Viewer;
use super::annotate::Tool;

/// Half the size of a corner handle, in screen pixels.
const HANDLE: f32 = 5.0;
/// How far around an annotation a click still picks it, in points.
const SLOP: f32 = 4.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Grip {
    Move,
    /// Corner index: 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left (displayed page).
    Corner(usize),
}

pub struct Picked {
    page: usize,
    id: u64,
    annotation: Annotation,
    /// A drag in progress: what is held, and where it started and is now (displayed points).
    drag: Option<(Grip, Point, Point)>,
}

fn corners(rect: PageRect) -> [Point; 4] {
    [
        Point { x: rect.x, y: rect.y },
        Point { x: rect.right(), y: rect.y },
        Point { x: rect.right(), y: rect.bottom() },
        Point { x: rect.x, y: rect.bottom() },
    ]
}

/// `rect` with the drag applied.
fn dragged(rect: PageRect, grip: Grip, from: Point, to: Point) -> PageRect {
    let (dx, dy) = (to.x - from.x, to.y - from.y);
    match grip {
        Grip::Move => PageRect { x: rect.x + dx, y: rect.y + dy, ..rect },
        Grip::Corner(n) => {
            let opposite = corners(rect)[(n + 2) % 4];
            let moved = corners(rect)[n];
            let moved = Point { x: moved.x + dx, y: moved.y + dy };
            PageRect {
                x: opposite.x.min(moved.x),
                y: opposite.y.min(moved.y),
                width: (opposite.x - moved.x).abs().max(2.0),
                height: (opposite.y - moved.y).abs().max(2.0),
            }
        }
    }
}

impl Viewer {
    /// Picks the annotation under the pointer, or starts dragging the picked one. Returns
    /// whether the click was used.
    pub(super) fn pick_at(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) -> bool {
        if !self.editing || self.tool != Tool::Select {
            return false;
        }
        let Some((page, at)) = self.page_point(event.position) else {
            self.picked = None;
            return false;
        };
        let zoom = self.open().and_then(|open| open.layout.as_ref()).map_or(1.0, |layout| layout.zoom);
        let Some(geometry) = self.open().and_then(|open| open.geometry.get(page).copied()) else { return false };
        // A handle or the body of the picked annotation starts a drag.
        if let Some(picked) = self.picked.as_mut().filter(|picked| picked.page == page) {
            let shown = geometry.rect_to_display(picked.annotation.frame());
            let reach = (HANDLE + 2.0) / zoom;
            let corner = corners(shown).iter().position(|c| (c.x - at.x).abs() <= reach && (c.y - at.y).abs() <= reach);
            let inside = at.x >= shown.x - SLOP && at.x <= shown.right() + SLOP && at.y >= shown.y - SLOP && at.y <= shown.bottom() + SLOP;
            if let Some(n) = corner {
                picked.drag = Some((Grip::Corner(n), at, at));
                cx.notify();
                return true;
            }
            if inside {
                picked.drag = Some((Grip::Move, at, at));
                cx.notify();
                return true;
            }
        }
        let Some(id) = self.open().and_then(|open| open.page_states.get(page)).map(|state| state.id) else { return false };
        let point = geometry.to_user(at);
        let annotations = self.open().map(|open| open.document.annotations(id)).unwrap_or_default();
        // The topmost one under the pointer.
        let hit = annotations.into_iter().rev().find(|(_, annotation)| {
            let frame = annotation.frame();
            point.x >= frame.origin.x - SLOP
                && point.x <= frame.origin.x + frame.width + SLOP
                && point.y >= frame.origin.y - SLOP
                && point.y <= frame.origin.y + frame.height + SLOP
        });
        match hit {
            Some((id, annotation)) => {
                self.picked = Some(Picked { page, id, annotation, drag: Some((Grip::Move, at, at)) });
                self.selection = None;
                cx.notify();
                true
            }
            None => {
                let had = self.picked.take().is_some();
                if had {
                    cx.notify();
                }
                false
            }
        }
    }

    /// Follows a drag of the picked annotation; returns whether there is one.
    pub(super) fn pick_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) -> bool {
        let page_point = self.page_point(event.position);
        let Some(picked) = self.picked.as_mut() else { return false };
        let Some((grip, from, _)) = picked.drag else { return false };
        if event.pressed_button != Some(MouseButton::Left) {
            return false;
        }
        if let Some((page, at)) = page_point.filter(|(page, _)| *page == picked.page) {
            let _ = page;
            picked.drag = Some((grip, from, at));
            cx.notify();
        }
        true
    }

    /// Ends a drag: records the new place and size. Returns whether there was one.
    pub(super) fn pick_release(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(picked) = self.picked.as_mut() else { return false };
        let Some((grip, from, to)) = picked.drag.take() else { return false };
        if (to.x - from.x).abs() < 0.5 && (to.y - from.y).abs() < 0.5 {
            cx.notify();
            return true;
        }
        let page = picked.page;
        let Some(geometry) = self.open().and_then(|open| open.geometry.get(page).copied()) else { return true };
        let picked = self.picked.as_mut().expect("checked above");
        let shown = dragged(geometry.rect_to_display(picked.annotation.frame()), grip, from, to);
        let moved = picked.annotation.mapped(geometry.rect_to_user(shown));
        picked.annotation = moved.clone();
        let target = picked.id;
        self.edit(Op::UpdateAnnotation { target, annotation: moved }, window, cx);
        true
    }

    /// Deletes the picked annotation; returns whether there was one.
    pub(super) fn delete_picked(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(picked) = self.picked.take() else { return false };
        self.edit(Op::DeleteAnnotation { target: picked.id }, window, cx);
        true
    }

    /// The outline and handles of the picked annotation over page `index`, whose box is
    /// `frame` at `zoom`.
    pub(super) fn pick_overlay(&self, index: usize, frame: (f32, f32, f32, f32), zoom: f32) -> Option<AnyElement> {
        let picked = self.picked.as_ref().filter(|picked| picked.page == index)?;
        let geometry = self.open()?.geometry.get(index).copied()?;
        let mut shown = geometry.rect_to_display(picked.annotation.frame());
        if let Some((grip, from, to)) = picked.drag {
            shown = dragged(shown, grip, from, to);
        }
        let theme = self.theme;
        let (x, y) = (frame.0 + shown.x * zoom, frame.1 + shown.y * zoom);
        let (w, h) = ((shown.width * zoom).max(1.0), (shown.height * zoom).max(1.0));
        let mut outline = div()
            .absolute()
            .left(px(x - 3.0))
            .top(px(y - 3.0))
            .w(px(w + 6.0))
            .h(px(h + 6.0))
            .border_1()
            .border_color(theme.highlight)
            .rounded(px(2.0));
        for (cx_, cy) in [(0.0, 0.0), (w + 6.0, 0.0), (w + 6.0, h + 6.0), (0.0, h + 6.0)] {
            outline = outline.child(
                div()
                    .absolute()
                    .left(px(cx_ - HANDLE - 1.0))
                    .top(px(cy - HANDLE - 1.0))
                    .size(px(HANDLE * 2.0))
                    .rounded(px(2.0))
                    .bg(theme.toolbar)
                    .border_1()
                    .border_color(theme.highlight),
            );
        }
        Some(outline.into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drags_move_and_resize() {
        let rect = PageRect { x: 10.0, y: 20.0, width: 100.0, height: 50.0 };
        let (from, to) = (Point { x: 0.0, y: 0.0 }, Point { x: 5.0, y: -5.0 });
        assert_eq!(dragged(rect, Grip::Move, from, to), PageRect { x: 15.0, y: 15.0, width: 100.0, height: 50.0 });
        // The bottom-right corner pulled out: the top-left one stays.
        assert_eq!(dragged(rect, Grip::Corner(2), from, Point { x: 20.0, y: 10.0 }), PageRect { x: 10.0, y: 20.0, width: 120.0, height: 60.0 });
        // Pulled past the opposite corner: the box flips instead of turning negative.
        let flipped = dragged(rect, Grip::Corner(2), from, Point { x: -150.0, y: 0.0 });
        assert_eq!((flipped.x, flipped.width), (-40.0, 50.0));
    }
}
