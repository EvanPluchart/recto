//! Text stamped on pages: watermarks, page numbers, headers and footers. One model covers the
//! three: a line of Helvetica placed relative to the page as displayed, so "bottom centre" is
//! the bottom of the page on screen even when the page is rotated.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::PageId;
use crate::annot::{Color, Font, text_width, win_ansi};
use crate::content::{Matrix, dictionary};
use crate::geom::{PageGeometry, Point};

/// Where the text sits on the displayed page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position {
    TopLeft,
    TopCenter,
    TopRight,
    Center,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    /// `{page}` becomes the page number and `{pages}` the page count.
    pub text: String,
    pub position: Position,
    /// Points.
    pub size: f32,
    pub color: Color,
    /// 0 (invisible) to 1 (opaque).
    pub opacity: f32,
    /// Counter-clockwise degrees, around the text's centre.
    pub angle: f32,
    /// Distance from the page edges, in points.
    pub margin: f32,
    /// Number of the first page.
    pub first_number: u32,
    /// Pages to stamp; `None` for every page.
    pub pages: Option<Vec<PageId>>,
}

impl Stamp {
    /// A diagonal, translucent line across the middle of every page.
    pub fn watermark(text: &str) -> Stamp {
        Stamp {
            text: text.to_owned(),
            position: Position::Center,
            size: 72.0,
            color: Color { r: 0.6, g: 0.6, b: 0.6 },
            opacity: 0.35,
            angle: 45.0,
            margin: 0.0,
            first_number: 1,
            pages: None,
        }
    }

    /// "N / total" at the bottom centre of every page.
    pub fn page_numbers() -> Stamp {
        Stamp {
            text: "{page} / {pages}".to_owned(),
            position: Position::BottomCenter,
            size: 10.0,
            color: Color::BLACK,
            opacity: 1.0,
            angle: 0.0,
            margin: 28.0,
            first_number: 1,
            pages: None,
        }
    }

    /// Whether the text changes with the page's position.
    pub fn is_numbered(&self) -> bool {
        self.text.contains("{page}") || self.text.contains("{pages}")
    }

    /// The text shown on page `index` (0-based) of `count`.
    pub fn text_for(&self, index: usize, count: usize) -> String {
        let number = (self.first_number as usize + index).to_string();
        let total = (self.first_number as usize + count).saturating_sub(1).to_string();
        self.text.replace("{pages}", &total).replace("{page}", &number)
    }

    /// Content drawing the stamp on a page shown as `geometry`, with font `font` and graphics
    /// state `state` (resource names).
    fn content(&self, text: &str, geometry: &PageGeometry, font: &[u8], state: &[u8]) -> lopdf::Result<Vec<u8>> {
        // Upright space: origin at the bottom-left corner of the displayed page, y up.
        let size = geometry.size();
        let display = |x: f32, y: f32| geometry.to_user(Point { x, y: size.height - y });
        let origin = display(0.0, 0.0);
        let (x_axis, y_axis) = (display(1.0, 0.0), display(0.0, 1.0));
        let upright = Matrix([x_axis.x - origin.x, x_axis.y - origin.y, y_axis.x - origin.x, y_axis.y - origin.y, origin.x, origin.y]);

        let width = text_width(text, self.size, Font::Helvetica);
        let (w, h, m) = (size.width, size.height, self.margin);
        let center_x = match self.position {
            Position::TopLeft | Position::BottomLeft => m + width / 2.0,
            Position::TopRight | Position::BottomRight => w - m - width / 2.0,
            _ => w / 2.0,
        };
        // Cap height of Helvetica is about 0.72 em: centre the capitals on the line.
        let half = 0.36 * self.size;
        let center_y = match self.position {
            Position::TopLeft | Position::TopCenter | Position::TopRight => h - m - half,
            Position::Center => h / 2.0,
            _ => m + half,
        };
        let (sin, cos) = self.angle.to_radians().sin_cos();
        // The text's centre at (center_x, center_y), turned by `angle`.
        let (dx, dy) = (-width / 2.0, -half);
        let text_matrix = [cos, sin, -sin, cos, center_x + cos * dx - sin * dy, center_y + sin * dx + cos * dy];

        let number = |value: f32| Object::Real(value);
        let operations = vec![
            Operation::new("q", vec![]),
            Operation::new("cm", upright.0.iter().copied().map(number).collect()),
            Operation::new("gs", vec![Object::Name(state.to_vec())]),
            Operation::new("rg", vec![number(self.color.r), number(self.color.g), number(self.color.b)]),
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name(font.to_vec()), number(self.size)]),
            Operation::new("Tm", text_matrix.iter().copied().map(number).collect()),
            Operation::new("Tj", vec![Object::String(win_ansi(text), lopdf::StringFormat::Literal)]),
            Operation::new("ET", vec![]),
            Operation::new("Q", vec![]),
        ];
        Content { operations }.encode()
    }
}

/// A resource name not used yet in `dictionary`.
pub(crate) fn fresh_name(dictionary: &Dictionary, prefix: &str) -> Vec<u8> {
    (0..).map(|n| format!("{prefix}{n}").into_bytes()).find(|name| !dictionary.has(name)).expect("an unused name")
}

/// Draws `stamps` on `pages` (the output pages in order, with their geometry). The page content
/// is wrapped in `q … Q` first, so the stamps are drawn with the default graphics state.
pub fn apply(document: &mut lopdf::Document, pages: &[(ObjectId, PageGeometry)], stamps: &[(&Stamp, Vec<usize>)]) -> lopdf::Result<()> {
    if stamps.is_empty() {
        return Ok(());
    }
    let font = helvetica(document);
    let open = document.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let count = pages.len();

    for (index, (page, geometry)) in pages.iter().enumerate() {
        let here: Vec<&Stamp> = stamps.iter().filter(|(_, on)| on.contains(&index)).map(|(stamp, _)| *stamp).collect();
        if here.is_empty() {
            continue;
        }
        // The page's own copy of its resources, so pages sharing a dictionary are not affected.
        let page_dictionary = document.get_dictionary(*page)?;
        let mut resources = page_dictionary.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        let mut fonts = resources.get(b"Font").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        let mut states = resources.get(b"ExtGState").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
        let font_name = fresh_name(&fonts, "RectoStamp");
        fonts.set(font_name.clone(), Object::Reference(font));

        let mut content = b"Q\n".to_vec();
        for stamp in here {
            let state_name = fresh_name(&states, "RectoStamp");
            let mut state = Dictionary::new();
            state.set("Type", Object::Name(b"ExtGState".to_vec()));
            state.set("ca", Object::Real(stamp.opacity.clamp(0.0, 1.0)));
            state.set("CA", Object::Real(stamp.opacity.clamp(0.0, 1.0)));
            states.set(state_name.clone(), Object::Dictionary(state));
            content.extend(stamp.content(&stamp.text_for(index, count), geometry, &font_name, &state_name)?);
            content.push(b'\n');
        }
        resources.set("Font", Object::Dictionary(fonts));
        resources.set("ExtGState", Object::Dictionary(states));

        append(document, *page, open, content, resources)?;
    }
    Ok(())
}

/// Wraps the content of `page` in `open` (a stream holding `q`) and a new stream holding
/// `content`, which must start with `Q`; sets the page's resources to `resources`.
pub(crate) fn append(document: &mut lopdf::Document, page: ObjectId, open: ObjectId, content: Vec<u8>, resources: Dictionary) -> lopdf::Result<()> {
    let mut contents: Vec<Object> = vec![Object::Reference(open)];
    match document.get_dictionary(page)?.get(b"Contents") {
        Ok(Object::Array(items)) => contents.extend(items.iter().cloned()),
        Ok(Object::Reference(id)) => match document.get_object(*id) {
            Ok(Object::Array(items)) => contents.extend(items.iter().cloned()),
            _ => contents.push(Object::Reference(*id)),
        },
        _ => {}
    }
    let mut stream = Stream::new(Dictionary::new(), content);
    let _ = stream.compress();
    contents.push(Object::Reference(document.add_object(stream)));
    let dictionary = document.get_dictionary_mut(page)?;
    dictionary.set("Contents", Object::Array(contents));
    dictionary.set("Resources", Object::Dictionary(resources));
    Ok(())
}

/// A Helvetica font object with WinAnsiEncoding.
pub(crate) fn helvetica(document: &mut lopdf::Document) -> ObjectId {
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    document.add_object(font)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_pages() {
        let mut stamp = Stamp::page_numbers();
        assert!(stamp.is_numbered());
        assert_eq!(stamp.text_for(0, 3), "1 / 3");
        stamp.first_number = 5;
        stamp.text = "Page {page} of {pages}".into();
        assert_eq!(stamp.text_for(2, 3), "Page 7 of 7");
        assert!(!Stamp::watermark("DRAFT").is_numbered());
    }
}
