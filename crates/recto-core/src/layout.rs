//! Continuous page layout: where each page sits in the scrollable content, in logical pixels
//! (window space before scrolling). Pure math, shared by the viewer, search highlights and selection.

use std::ops::Range;

use crate::geom::{Point, Size};

/// Space around the pages and between two pages, in logical pixels.
pub const PADDING: f32 = 24.0;
pub const GAP: f32 = 12.0;

/// A page's box in content space (origin top-left, logical pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageBox {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PageBox {
    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }
}

/// Pages stacked vertically and centred horizontally, at one zoom level.
#[derive(Clone, Debug, PartialEq)]
pub struct PageLayout {
    /// Logical pixels per PDF point.
    pub zoom: f32,
    pub boxes: Vec<PageBox>,
    pub content: Size,
    /// Viewport width the layout was made for.
    pub viewport_width: f32,
}

impl PageLayout {
    /// `sizes` are displayed page sizes in points; `viewport_width` centres pages narrower than the view.
    pub fn new(sizes: &[Size], zoom: f32, viewport_width: f32) -> Self {
        let widest = sizes.iter().map(|s| s.width * zoom).fold(0.0, f32::max);
        let content_width = (widest + 2.0 * PADDING).max(viewport_width);
        let mut y = PADDING;
        let boxes = sizes
            .iter()
            .map(|size| {
                let (width, height) = (size.width * zoom, size.height * zoom);
                let page = PageBox { x: ((content_width - width) / 2.0).round(), y, width, height };
                y += height + GAP;
                page
            })
            .collect::<Vec<_>>();
        let content_height = if boxes.is_empty() { 2.0 * PADDING } else { y - GAP + PADDING };
        Self { zoom, boxes, content: Size { width: content_width, height: content_height }, viewport_width }
    }

    /// Pages that intersect the vertical band `[top, top + height)`.
    pub fn visible(&self, top: f32, height: f32) -> Range<usize> {
        let bottom = top + height;
        let start = self.boxes.partition_point(|page| page.bottom() <= top);
        let end = self.boxes.partition_point(|page| page.y < bottom);
        start..end.max(start)
    }

    /// The page shown at content height `y`: the one containing it, or the next one in a gap.
    pub fn page_at(&self, y: f32) -> usize {
        self.boxes.partition_point(|page| page.bottom() + GAP <= y).min(self.boxes.len().saturating_sub(1))
    }

    /// Converts a content-space point to (page index, PDF-point offset from the page's top-left).
    pub fn to_page(&self, point: Point) -> Option<(usize, Point)> {
        let index = self.page_at(point.y);
        let page = self.boxes.get(index)?;
        Some((index, Point { x: (point.x - page.x) / self.zoom, y: (point.y - page.y) / self.zoom }))
    }

    /// Inverse of `to_page`.
    pub fn from_page(&self, index: usize, offset: Point) -> Option<Point> {
        let page = self.boxes.get(index)?;
        Some(Point { x: page.x + offset.x * self.zoom, y: page.y + offset.y * self.zoom })
    }

    /// Largest valid scroll offset for a viewport of `viewport` size.
    pub fn max_scroll(&self, viewport: Size) -> Point {
        Point { x: (self.content.width - viewport.width).max(0.0), y: (self.content.height - viewport.height).max(0.0) }
    }

    /// Scroll offset in `self` that keeps the content under `anchor` (a viewport position) in place,
    /// given the offset `scroll` it had in `previous`. Used when zooming.
    pub fn rescroll(&self, previous: &PageLayout, scroll: Point, anchor: Point, viewport: Size) -> Point {
        let target = Point { x: scroll.x + anchor.x, y: scroll.y + anchor.y };
        let Some(moved) = previous.to_page(target).and_then(|(index, offset)| self.from_page(index, offset)) else {
            return Point { x: 0.0, y: 0.0 };
        };
        let max = self.max_scroll(viewport);
        Point { x: (moved.x - anchor.x).clamp(0.0, max.x), y: (moved.y - anchor.y).clamp(0.0, max.y) }
    }

    /// Scroll offset that puts the top of page `index` just below the top of the viewport
    /// (after the gap, so the previous page does not peek in).
    pub fn scroll_to_page(&self, index: usize, viewport: Size) -> Point {
        let Some(page) = self.boxes.get(index) else { return Point { x: 0.0, y: 0.0 } };
        let margin = if index == 0 { PADDING } else { GAP };
        // Horizontally: centre the page if it fits, else show its left edge.
        let x = if page.width + 2.0 * PADDING <= viewport.width {
            page.x + page.width / 2.0 - viewport.width / 2.0
        } else {
            page.x - PADDING
        };
        let max = self.max_scroll(viewport);
        Point { x: x.clamp(0.0, max.x), y: (page.y - margin).clamp(0.0, max.y) }
    }
}

/// Zoom at which the widest page fills the viewport width.
pub fn fit_width(sizes: &[Size], viewport: Size) -> f32 {
    let widest = sizes.iter().map(|s| s.width).fold(0.0, f32::max);
    if widest <= 0.0 { 1.0 } else { ((viewport.width - 2.0 * PADDING) / widest).max(0.01) }
}

/// Zoom at which page `page` fits entirely in the viewport.
pub fn fit_page(page: Size, viewport: Size) -> f32 {
    if page.width <= 0.0 || page.height <= 0.0 {
        return 1.0;
    }
    let by_width = (viewport.width - 2.0 * PADDING) / page.width;
    let by_height = (viewport.height - 2.0 * PADDING) / page.height;
    by_width.min(by_height).max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A4: Size = Size { width: 595.0, height: 842.0 };
    const LANDSCAPE: Size = Size { width: 842.0, height: 595.0 };

    fn point(x: f32, y: f32) -> Point {
        Point { x, y }
    }

    #[test]
    fn stacks_and_centres_pages() {
        let layout = PageLayout::new(&[A4, LANDSCAPE], 1.0, 600.0);
        assert_eq!(layout.content.width, 842.0 + 2.0 * PADDING);
        assert_eq!(layout.boxes[0], PageBox { x: 148.0, y: PADDING, width: 595.0, height: 842.0 });
        assert_eq!(layout.boxes[1], PageBox { x: PADDING, y: PADDING + 842.0 + GAP, width: 842.0, height: 595.0 });
        assert_eq!(layout.content.height, PADDING + 842.0 + GAP + 595.0 + PADDING);

        // A wide viewport centres everything.
        let wide = PageLayout::new(&[A4], 1.0, 1000.0);
        assert_eq!(wide.content.width, 1000.0);
        assert_eq!(wide.boxes[0].x, ((1000.0 - 595.0) / 2.0_f32).round());
    }

    #[test]
    fn finds_visible_pages() {
        let layout = PageLayout::new(&[A4; 10], 0.5, 400.0);
        let pitch = 421.0 + GAP;
        assert_eq!(layout.visible(0.0, 300.0), 0..1);
        assert_eq!(layout.visible(0.0, PADDING + pitch + 1.0), 0..2);
        // A band that falls inside a gap shows nothing.
        assert_eq!(layout.visible(PADDING + 421.0 + 1.0, GAP - 2.0), 1..1);
        assert_eq!(layout.visible(layout.content.height - PADDING - 10.0, 100.0), 9..10);
        // The bottom padding shows no page.
        assert_eq!(layout.visible(layout.content.height - 10.0, 100.0), 10..10);
        assert_eq!(PageLayout::new(&[], 1.0, 400.0).visible(0.0, 100.0), 0..0);
    }

    #[test]
    fn maps_points_both_ways() {
        let layout = PageLayout::new(&[A4, A4], 2.0, 400.0);
        let (index, offset) = layout.to_page(point(layout.boxes[1].x + 20.0, layout.boxes[1].y + 40.0)).unwrap();
        assert_eq!((index, offset), (1, point(10.0, 20.0)));
        assert_eq!(layout.from_page(1, offset), Some(point(layout.boxes[1].x + 20.0, layout.boxes[1].y + 40.0)));
        assert_eq!(layout.page_at(0.0), 0);
        assert_eq!(layout.page_at(1e9), 1);
    }

    #[test]
    fn zoom_keeps_the_anchor_still() {
        let viewport = Size { width: 800.0, height: 600.0 };
        let sizes = [A4; 5];
        let before = PageLayout::new(&sizes, 1.0, viewport.width);
        let scroll = point(0.0, 2000.0);
        let anchor = point(400.0, 300.0);
        let (page, offset) = before.to_page(point(scroll.x + anchor.x, scroll.y + anchor.y)).unwrap();

        let after = PageLayout::new(&sizes, 2.0, viewport.width);
        let new_scroll = after.rescroll(&before, scroll, anchor, viewport);
        let (new_page, new_offset) = after.to_page(point(new_scroll.x + anchor.x, new_scroll.y + anchor.y)).unwrap();
        assert_eq!(new_page, page);
        assert!((new_offset.x - offset.x).abs() < 0.01 && (new_offset.y - offset.y).abs() < 0.01);
    }

    #[test]
    fn scrolling_is_clamped() {
        let viewport = Size { width: 800.0, height: 600.0 };
        let layout = PageLayout::new(&[A4; 3], 1.0, viewport.width);
        assert_eq!(layout.scroll_to_page(0, viewport), point(0.0, 0.0));
        assert_eq!(layout.scroll_to_page(1, viewport).y, PADDING + 842.0);
        assert_eq!(layout.scroll_to_page(2, viewport).y, PADDING + 2.0 * 842.0 + GAP);
        let tall = Size { width: 800.0, height: 2000.0 };
        assert_eq!(layout.scroll_to_page(2, tall).y, layout.max_scroll(tall).y);

        // A narrow page in content widened by a wide one: its left edge comes into view.
        let mixed = PageLayout::new(&[A4, LANDSCAPE], 4.0, viewport.width);
        let x = mixed.scroll_to_page(0, viewport).x;
        assert_eq!(x, mixed.boxes[0].x - PADDING);
        assert_eq!(mixed.scroll_to_page(1, viewport).x, 0.0);
    }

    #[test]
    fn fits() {
        let viewport = Size { width: 600.0 + 2.0 * PADDING, height: 400.0 + 2.0 * PADDING };
        assert_eq!(fit_width(&[Size { width: 300.0, height: 10.0 }, Size { width: 600.0, height: 10.0 }], viewport), 1.0);
        assert_eq!(fit_page(Size { width: 300.0, height: 800.0 }, viewport), 0.5);
        assert_eq!(fit_width(&[], viewport), 1.0);
    }
}
