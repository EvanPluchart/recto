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
