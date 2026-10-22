//! Annotations added by Recto: the model stored in `Op`s (user space) and how each one is written
//! as a real PDF annotation, with an explicit appearance stream so every reader shows it as Recto
//! does.

use std::sync::Arc;

use lopdf::{Dictionary, Object, ObjectId, Stream};
use pdf_writer::{Content, Name, Str};

use crate::geom::{Point, Rect};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
}

impl Color {
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0 };
    pub const RED: Color = Color { r: 0.86, g: 0.15, b: 0.15 };
    pub const BLUE: Color = Color { r: 0.15, g: 0.35, b: 0.85 };
    pub const GREEN: Color = Color { r: 0.1, g: 0.6, b: 0.25 };
    pub const YELLOW: Color = Color { r: 1.0, g: 0.85, b: 0.1 };

    fn array(self) -> Object {
        Object::Array(vec![self.r.into(), self.g.into(), self.b.into()])
    }
}

/// Base-14 fonts only: nothing is embedded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Helvetica,
    /// For typed signatures.
    TimesItalic,
}

impl Font {
    fn base_name(self) -> &'static [u8] {
        match self {
            Font::Helvetica => b"Helvetica",
            Font::TimesItalic => b"Times-Italic",
        }
    }
}

/// An image to place, decoded: 8-bit RGB, with an optional 8-bit alpha channel.
#[derive(Clone, Debug, PartialEq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    pub alpha: Option<Vec<u8>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    /// Text in a box. `upright` is the page's display rotation when it was placed, so the text
    /// reads upright on screen.
    Text { rect: Rect, text: String, size: f32, font: Font, upright: u16 },
    /// An image (or image signature) stretched over `rect`, upright like text.
    Image { rect: Rect, image: Arc<Raster>, upright: u16 },
    Rectangle { rect: Rect },
    Ellipse { rect: Rect },
    Line { from: Point, to: Point, arrow: bool },
    /// Freehand strokes; drawn signatures too.
    Ink { strokes: Vec<Vec<Point>> },
    /// Text markup over the boxes of the marked text.
    Highlight { quads: Vec<Rect> },
    Underline { quads: Vec<Rect> },
    StrikeOut { quads: Vec<Rect> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub kind: Kind,
    pub color: Color,
    /// Stroke width in points, for shapes, lines and ink.
    pub width: f32,
}

fn rect_object(rect: Rect) -> Object {
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    Object::Array(vec![x0.into(), y0.into(), (x0 + rect.width).into(), (y0 + rect.height).into()])
}

fn bounds(points: impl IntoIterator<Item = Point>, margin: f32) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for point in points {
        x0 = x0.min(point.x);
        y0 = y0.min(point.y);
        x1 = x1.max(point.x);
        y1 = y1.max(point.y);
    }
    if x0 > x1 {
        return Rect { origin: Point { x: 0.0, y: 0.0 }, width: 0.0, height: 0.0 };
    }
    Rect { origin: Point { x: x0 - margin, y: y0 - margin }, width: x1 - x0 + 2.0 * margin, height: y1 - y0 + 2.0 * margin }
}

fn corners(rect: &Rect) -> [Point; 4] {
    let (x0, y0, x1, y1) = (rect.origin.x, rect.origin.y, rect.origin.x + rect.width, rect.origin.y + rect.height);
    [Point { x: x0, y: y0 }, Point { x: x1, y: y0 }, Point { x: x0, y: y1 }, Point { x: x1, y: y1 }]
}

/// A PDF text string: PDFDocEncoding when ASCII, UTF-16BE with a byte order mark otherwise.
pub fn text_string(text: &str) -> Object {
    if text.is_ascii() {
        return Object::String(text.as_bytes().to_vec(), lopdf::StringFormat::Literal);
    }
    let mut bytes = vec![0xFE, 0xFF];
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_be_bytes());
    }
    Object::String(bytes, lopdf::StringFormat::Hexadecimal)
}

/// Bytes for a base-14 font with WinAnsiEncoding; characters it lacks become `?`.
pub(crate) fn win_ansi(text: &str) -> Vec<u8> {
    text.chars().map(|c| if (c as u32) < 256 { c as u32 as u8 } else { b'?' }).collect()
}

/// Appearance matrix turning upright content by the page's display rotation.
pub(crate) fn upright_matrix(upright: u16) -> [f32; 6] {
    match upright % 360 {
        90 => [0.0, 1.0, -1.0, 0.0, 0.0, 0.0],
        180 => [-1.0, 0.0, 0.0, -1.0, 0.0, 0.0],
        270 => [0.0, -1.0, 1.0, 0.0, 0.0, 0.0],
        _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
    }
}

/// Size of `rect` as seen on a page displayed with `upright` rotation.
fn upright_size(rect: &Rect, upright: u16) -> (f32, f32) {
    if upright % 180 == 90 { (rect.height, rect.width) } else { (rect.width, rect.height) }
}

/// Four Bézier arcs approximating the ellipse inscribed in `rect`.
fn ellipse(content: &mut Content, rect: Rect) {
    const K: f32 = 0.552_284_8;
    let (cx, cy) = (rect.origin.x + rect.width / 2.0, rect.origin.y + rect.height / 2.0);
    let (rx, ry) = (rect.width / 2.0, rect.height / 2.0);
    content.move_to(cx + rx, cy);
    content.cubic_to(cx + rx, cy + K * ry, cx + K * rx, cy + ry, cx, cy + ry);
    content.cubic_to(cx - K * rx, cy + ry, cx - rx, cy + K * ry, cx - rx, cy);
    content.cubic_to(cx - rx, cy - K * ry, cx - K * rx, cy - ry, cx, cy - ry);
    content.cubic_to(cx + K * rx, cy - ry, cx + rx, cy - K * ry, cx + rx, cy);
    content.close_path();
}

impl Annotation {
    fn subtype(&self) -> &'static [u8] {
        match self.kind {
            Kind::Text { .. } => b"FreeText",
            Kind::Image { .. } => b"Stamp",
            Kind::Rectangle { .. } => b"Square",
            Kind::Ellipse { .. } => b"Circle",
            Kind::Line { .. } => b"Line",
            Kind::Ink { .. } => b"Ink",
            Kind::Highlight { .. } => b"Highlight",
            Kind::Underline { .. } => b"Underline",
            Kind::StrikeOut { .. } => b"StrikeOut",
        }
    }

    /// The annotation's `/Rect`, in user space.
    pub fn rect(&self) -> Rect {
        let margin = self.width.max(1.0);
        match &self.kind {
            Kind::Text { rect, .. } | Kind::Image { rect, .. } | Kind::Rectangle { rect } | Kind::Ellipse { rect } => *rect,
            Kind::Line { from, to, .. } => bounds([*from, *to], margin * 4.0 + 6.0),
            Kind::Ink { strokes } => bounds(strokes.iter().flatten().copied(), margin),
            Kind::Highlight { quads } | Kind::Underline { quads } | Kind::StrikeOut { quads } => {
                bounds(quads.iter().flat_map(corners), 0.0)
            }
        }
    }

    /// Adds the annotation and its appearance to `document`, for the page `page`; returns its id.
    pub fn write(&self, document: &mut lopdf::Document, page: ObjectId, serial: usize) -> ObjectId {
        let rect = self.rect();
        let mut annotation = Dictionary::new();
        annotation.set("Type", Object::Name(b"Annot".to_vec()));
        annotation.set("Subtype", Object::Name(self.subtype().to_vec()));
        annotation.set("Rect", rect_object(rect));
        annotation.set("P", Object::Reference(page));
        annotation.set("F", Object::Integer(4)); // print
        annotation.set("NM", text_string(&format!("recto-{serial}")));
        annotation.set("C", self.color.array());
        let mut border = Dictionary::new();
        border.set("W", Object::Real(self.width));
        annotation.set("BS", Object::Dictionary(border));

        let mut resources = Dictionary::new();
        let mut content = Content::new();
        // Content is drawn in user space with BBox = Rect, except upright text and images.
        let mut bbox = rect;
        let mut matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        let Color { r, g, b } = self.color;
        match &self.kind {
            Kind::Text { text, size, font, upright, .. } => {
                annotation.set("Contents", text_string(text));
                annotation.set("DA", Object::string_literal(format!("/F1 {size} Tf {r} {g} {b} rg")));
                let (width, height) = upright_size(&rect, *upright);
                bbox = Rect { origin: Point { x: 0.0, y: 0.0 }, width, height };
                matrix = upright_matrix(*upright);
                let mut fonts = Dictionary::new();
                fonts.set("F1", Object::Reference(add_font(document, *font)));
                resources.set("Font", Object::Dictionary(fonts));
                content.begin_text().set_font(Name(b"F1"), *size).set_fill_rgb(r, g, b);
                content.next_line(2.0, height - size * 0.95);
                for (n, line) in text.lines().enumerate() {
                    if n > 0 {
                        content.next_line(0.0, -size * 1.2);
                    }
                    content.show(Str(&win_ansi(line)));
                }
                content.end_text();
            }
            Kind::Image { image, upright, .. } => {
                let (width, height) = upright_size(&rect, *upright);
                bbox = Rect { origin: Point { x: 0.0, y: 0.0 }, width, height };
                matrix = upright_matrix(*upright);
                let mut images = Dictionary::new();
                images.set("Im0", Object::Reference(add_image(document, image)));
                resources.set("XObject", Object::Dictionary(images));
                content.save_state().transform([width, 0.0, 0.0, height, 0.0, 0.0]).x_object(Name(b"Im0")).restore_state();
            }
            Kind::Rectangle { rect } => {
                let half = self.width / 2.0;
                content.set_stroke_rgb(r, g, b).set_line_width(self.width);
                content.rect(rect.origin.x + half, rect.origin.y + half, rect.width - self.width, rect.height - self.width).stroke();
            }
            Kind::Ellipse { rect } => {
                let half = self.width / 2.0;
                content.set_stroke_rgb(r, g, b).set_line_width(self.width);
                let inner = Rect {
                    origin: Point { x: rect.origin.x + half, y: rect.origin.y + half },
                    width: rect.width - self.width,
                    height: rect.height - self.width,
                };
                ellipse(&mut content, inner);
                content.stroke();
            }
            Kind::Line { from, to, arrow } => {
                annotation.set("L", Object::Array(vec![from.x.into(), from.y.into(), to.x.into(), to.y.into()]));
                if *arrow {
                    let ending = |name: &[u8]| Object::Name(name.to_vec());
                    annotation.set("LE", Object::Array(vec![ending(b"None"), ending(b"OpenArrow")]));
                }
                content.set_stroke_rgb(r, g, b).set_line_width(self.width);
                content.set_line_cap(pdf_writer::types::LineCapStyle::RoundCap);
                content.move_to(from.x, from.y).line_to(to.x, to.y).stroke();
                let length = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt();
                if *arrow && length > 0.0 {
                    let (dx, dy) = ((to.x - from.x) / length, (to.y - from.y) / length);
                    let head = (self.width * 4.0).max(8.0);
                    let (bx, by) = (to.x - dx * head, to.y - dy * head);
                    content.move_to(bx - dy * head * 0.5, by + dx * head * 0.5);
                    content.line_to(to.x, to.y);
                    content.line_to(bx + dy * head * 0.5, by - dx * head * 0.5);
                    content.stroke();
                }
            }
            Kind::Ink { strokes } => {
                let list = strokes
                    .iter()
                    .map(|stroke| Object::Array(stroke.iter().flat_map(|p| [p.x.into(), p.y.into()]).collect()))
                    .collect();
                annotation.set("InkList", Object::Array(list));
                content.set_stroke_rgb(r, g, b).set_line_width(self.width);
                content.set_line_cap(pdf_writer::types::LineCapStyle::RoundCap);
                content.set_line_join(pdf_writer::types::LineJoinStyle::RoundJoin);
                for stroke in strokes {
                    let Some(first) = stroke.first() else { continue };
                    content.move_to(first.x, first.y);
                    for point in &stroke[1..] {
                        content.line_to(point.x, point.y);
                    }
                    if stroke.len() == 1 {
                        content.line_to(first.x + 0.01, first.y);
                    }
                    content.stroke();
                }
            }
            Kind::Highlight { quads } | Kind::Underline { quads } | Kind::StrikeOut { quads } => {
                // QuadPoints: upper-left, upper-right, lower-left, lower-right of each box.
                let points = quads
                    .iter()
                    .flat_map(|quad| {
                        let [ll, lr, ul, ur] = corners(quad);
                        [ul, ur, ll, lr].into_iter().flat_map(|p| [p.x.into(), p.y.into()])
                    })
                    .collect();
                annotation.set("QuadPoints", Object::Array(points));
                content.set_fill_rgb(r, g, b);
                if matches!(self.kind, Kind::Highlight { .. }) {
                    // Multiply keeps the text under a highlight readable.
                    let mut state = Dictionary::new();
                    state.set("Type", Object::Name(b"ExtGState".to_vec()));
                    state.set("BM", Object::Name(b"Multiply".to_vec()));
                    let mut states = Dictionary::new();
                    states.set("GS0", Object::Dictionary(state));
                    resources.set("ExtGState", Object::Dictionary(states));
                    content.set_parameters(Name(b"GS0"));
                }
                for quad in quads {
                    let thickness = (quad.height / 14.0).max(0.75);
                    match self.kind {
                        Kind::Highlight { .. } => content.rect(quad.origin.x, quad.origin.y, quad.width, quad.height),
                        Kind::Underline { .. } => {
                            content.rect(quad.origin.x, quad.origin.y + quad.height * 0.06, quad.width, thickness)
                        }
                        _ => content.rect(quad.origin.x, quad.origin.y + quad.height * 0.42, quad.width, thickness),
                    };
                    content.fill_nonzero();
                }
            }
        }

        let mut form = Dictionary::new();
        form.set("Type", Object::Name(b"XObject".to_vec()));
        form.set("Subtype", Object::Name(b"Form".to_vec()));
        form.set("BBox", rect_object(bbox));
        form.set("Matrix", Object::Array(matrix.iter().map(|&value| value.into()).collect()));
        form.set("Resources", Object::Dictionary(resources));
        let appearance = document.add_object(Stream::new(form, content.finish().to_vec()));
        let mut states = Dictionary::new();
        states.set("N", Object::Reference(appearance));
        annotation.set("AP", Object::Dictionary(states));
        document.add_object(annotation)
    }
}

fn add_font(document: &mut lopdf::Document, font: Font) -> ObjectId {
    let mut dictionary = Dictionary::new();
    dictionary.set("Type", Object::Name(b"Font".to_vec()));
    dictionary.set("Subtype", Object::Name(b"Type1".to_vec()));
    dictionary.set("BaseFont", Object::Name(font.base_name().to_vec()));
    dictionary.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    document.add_object(dictionary)
}

fn add_image(document: &mut lopdf::Document, image: &Raster) -> ObjectId {
    let channel = |bytes: Vec<u8>, gray: bool, mask: Option<ObjectId>| {
        let mut dictionary = Dictionary::new();
        dictionary.set("Type", Object::Name(b"XObject".to_vec()));
        dictionary.set("Subtype", Object::Name(b"Image".to_vec()));
        dictionary.set("Width", Object::Integer(i64::from(image.width)));
        dictionary.set("Height", Object::Integer(i64::from(image.height)));
        dictionary.set("ColorSpace", Object::Name(if gray { b"DeviceGray".to_vec() } else { b"DeviceRGB".to_vec() }));
        dictionary.set("BitsPerComponent", Object::Integer(8));
        if let Some(mask) = mask {
            dictionary.set("SMask", Object::Reference(mask));
        }
        let mut stream = Stream::new(dictionary, bytes);
        let _ = stream.compress();
        stream
    };
    let mask = image.alpha.clone().map(|alpha| {
        let stream = channel(alpha, true, None);
        document.add_object(stream)
    });
    let stream = channel(image.rgb.clone(), false, mask);
    document.add_object(stream)
}

/// Appends annotation `annotation` to the `/Annots` of `page`.
pub fn attach(document: &mut lopdf::Document, page: ObjectId, annotation: ObjectId) -> lopdf::Result<()> {
    let existing = document.get_dictionary(page)?.get(b"Annots").ok().cloned();
    match existing {
        Some(Object::Reference(array)) => {
            if let Ok(Object::Array(items)) = document.get_object_mut(array) {
                items.push(Object::Reference(annotation));
                return Ok(());
            }
            document.get_dictionary_mut(page)?.set("Annots", Object::Array(vec![Object::Reference(annotation)]));
        }
        Some(Object::Array(mut items)) => {
            items.push(Object::Reference(annotation));
            document.get_dictionary_mut(page)?.set("Annots", Object::Array(items));
        }
        _ => document.get_dictionary_mut(page)?.set("Annots", Object::Array(vec![Object::Reference(annotation)])),
    }
    Ok(())
}

/// Helvetica advance widths (thousandths of the font size) for ASCII 32..=126, from its AFM.
pub(crate) const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833,
    722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556,
    556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334,
    260, 334, 584,
];

/// Width of the widest line of `text` set in `font` at `size` points.
pub fn text_width(text: &str, size: f32, font: Font) -> f32 {
    let line_width = |line: &str| -> f32 {
        line.chars()
            .map(|c| HELVETICA.get((c as usize).wrapping_sub(32)).copied().unwrap_or(600) as f32)
            .sum::<f32>()
            / 1000.0
            * size
    };
    // Times Italic runs narrower than Helvetica.
    let factor = match font {
        Font::Helvetica => 1.0,
        Font::TimesItalic => 0.88,
    };
    text.lines().map(line_width).fold(0.0, f32::max) * factor
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_helvetica() {
        // "Hi" = 722 + 222 thousandths.
        assert!((text_width("Hi", 10.0, Font::Helvetica) - 9.44).abs() < 0.001);
        assert!((text_width("a\nlonger", 10.0, Font::Helvetica) - text_width("longer", 10.0, Font::Helvetica)).abs() < 0.001);
        assert!(text_width("WIDE", 10.0, Font::Helvetica) > text_width("wide", 10.0, Font::Helvetica));
    }
}
