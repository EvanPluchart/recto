//! PDF user space: points (1/72 inch), origin at the bottom-left of the page.
//! Every coordinate stored in an `Op` uses this space.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub origin: Point,
    pub width: f32,
    pub height: f32,
}

/// Page size in PDF points, with the page's `/Rotate` already applied (as displayed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

/// A rectangle on a displayed page: points, origin at the top-left corner, page rotation applied.
/// Used for text boxes and highlights; edits are stored in user space (`Rect`) instead.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PageRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl PageRect {
    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &PageRect) -> PageRect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        PageRect { x, y, width: self.right().max(other.right()) - x, height: self.bottom().max(other.bottom()) - y }
    }
}

/// How a page is shown: its visible area in user space (the crop box clipped to the media box)
/// and the clockwise rotation applied on display. Converts between user space and displayed
/// page space, as PDFium draws the page.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageGeometry {
    /// Lower-left corner and size of the visible area, in user space.
    pub area: Rect,
    /// 0, 90, 180 or 270.
    pub rotate: u16,
}

impl PageGeometry {
    /// Size as displayed.
    pub fn size(&self) -> Size {
        let (width, height) = (self.area.width, self.area.height);
        if self.rotate % 180 == 90 { Size { width: height, height: width } } else { Size { width, height } }
    }

    /// User space to displayed page space.
    pub fn to_display(&self, point: Point) -> Point {
        let (w, h) = (self.area.width, self.area.height);
        let (x, y) = (point.x - self.area.origin.x, point.y - self.area.origin.y);
        let (dx, dy) = match self.rotate {
            90 => (y, x),
            180 => (w - x, y),
            270 => (h - y, w - x),
            _ => (x, h - y),
        };
        Point { x: dx, y: dy }
    }

    /// Displayed page space to user space.
    pub fn to_user(&self, point: Point) -> Point {
        let (w, h) = (self.area.width, self.area.height);
        let (x, y) = match self.rotate {
            90 => (point.y, point.x),
            180 => (w - point.x, point.y),
            270 => (w - point.y, h - point.x),
            _ => (point.x, h - point.y),
        };
        Point { x: x + self.area.origin.x, y: y + self.area.origin.y }
    }

    /// A displayed rectangle in user space (rotations are quarter turns, so it stays a rectangle).
    pub fn rect_to_user(&self, rect: PageRect) -> Rect {
        let a = self.to_user(Point { x: rect.x, y: rect.y });
        let b = self.to_user(Point { x: rect.right(), y: rect.bottom() });
        Rect { origin: Point { x: a.x.min(b.x), y: a.y.min(b.y) }, width: (a.x - b.x).abs(), height: (a.y - b.y).abs() }
    }

    /// A user-space rectangle on the displayed page.
    pub fn rect_to_display(&self, rect: Rect) -> PageRect {
        let a = self.to_display(rect.origin);
        let b = self.to_display(Point { x: rect.origin.x + rect.width, y: rect.origin.y + rect.height });
        PageRect { x: a.x.min(b.x), y: a.y.min(b.y), width: (a.x - b.x).abs(), height: (a.y - b.y).abs() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(rotate: u16) -> PageGeometry {
        PageGeometry { area: Rect { origin: Point { x: 10.0, y: 20.0 }, width: 600.0, height: 800.0 }, rotate }
    }

    #[test]
    fn sizes_follow_rotation() {
        assert_eq!(geometry(0).size(), Size { width: 600.0, height: 800.0 });
        assert_eq!(geometry(90).size(), Size { width: 800.0, height: 600.0 });
        assert_eq!(geometry(270).size(), Size { width: 800.0, height: 600.0 });
    }

    #[test]
    fn corners_land_where_the_page_turns() {
        // The user-space top-left corner of the visible area.
        let top_left = Point { x: 10.0, y: 820.0 };
        assert_eq!(geometry(0).to_display(top_left), Point { x: 0.0, y: 0.0 });
        // Turned a quarter clockwise, the top edge becomes the right edge.
        assert_eq!(geometry(90).to_display(top_left), Point { x: 800.0, y: 0.0 });
        assert_eq!(geometry(180).to_display(top_left), Point { x: 600.0, y: 800.0 });
        assert_eq!(geometry(270).to_display(top_left), Point { x: 0.0, y: 600.0 });
    }

    #[test]
    fn conversions_are_inverse() {
        let point = Point { x: 123.5, y: 456.25 };
        for rotate in [0, 90, 180, 270] {
            let page = geometry(rotate);
            assert_eq!(page.to_user(page.to_display(point)), point, "rotate {rotate}");
            let rect = Rect { origin: point, width: 30.0, height: 40.0 };
            assert_eq!(page.rect_to_user(page.rect_to_display(rect)), rect, "rotate {rotate}");
        }
    }
}
