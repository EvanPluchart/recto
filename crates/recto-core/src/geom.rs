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
