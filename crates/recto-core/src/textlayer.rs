//! Invisible text over scanned pages, from OCR: each recognised word is written in Helvetica
//! with rendering mode 3 (neither filled nor stroked), stretched to the word's box, so it can
//! be searched, selected and copied while the page looks the same.

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::annot::{Font, text_width, upright_matrix, win_ansi};
use crate::content::dictionary;
use crate::geom::{Point, Rect};
use crate::stamp::{append, fresh_name, helvetica};

#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    /// The word's box, in user space.
    pub rect: Rect,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextLayer {
    pub words: Vec<Word>,
    /// Display rotation of the page when it was recognised: words read along the displayed x
    /// axis.
    pub upright: u16,
}

/// Share of the box height below the baseline.
const DESCENT: f32 = 0.2;

impl TextLayer {
    fn operations(&self, font: &[u8]) -> Vec<Operation> {
        let [a, b, c, d, _, _] = upright_matrix(self.upright);
        let mut operations = vec![Operation::new("BT", vec![]), Operation::new("Tr", vec![Object::Integer(3)])];
        for word in &self.words {
            let text = word.text.trim();
            if text.is_empty() {
                continue;
            }
            let Rect { origin: Point { x: x0, y: y0 }, width, height } = word.rect;
            let (x1, y1) = (x0 + width, y0 + height);
            // Length along the reading direction, and the corner at the displayed bottom-left.
            let (along, across, corner) = match self.upright % 360 {
                90 => (height, width, (x1, y0)),
                180 => (width, height, (x1, y1)),
                270 => (height, width, (x0, y1)),
                _ => (width, height, (x0, y0)),
            };
            let size = across.max(1.0);
            let natural = text_width(text, size, Font::Helvetica);
            if natural <= 0.0 {
                continue;
            }
            // The baseline sits a little above the box's bottom edge.
            let lift = DESCENT * size;
            let origin = (corner.0 + c * lift, corner.1 + d * lift);
            let number = Object::Real;
            operations.push(Operation::new("Tf", vec![Object::Name(font.to_vec()), number(size)]));
            operations.push(Operation::new("Tz", vec![number(along / natural * 100.0)]));
            operations.push(Operation::new("Tm", vec![number(a), number(b), number(c), number(d), number(origin.0), number(origin.1)]));
            operations.push(Operation::new("Tj", vec![Object::String(win_ansi(text), lopdf::StringFormat::Literal)]));
        }
        operations.push(Operation::new("ET", vec![]));
        operations
    }
}

/// Adds `layers` (in order) to `page`.
pub fn apply(document: &mut lopdf::Document, page: ObjectId, layers: &[&TextLayer]) -> lopdf::Result<()> {
    if layers.iter().all(|layer| layer.words.is_empty()) {
        return Ok(());
    }
    let font = helvetica(document);
    let open = document.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let page_dictionary = document.get_dictionary(page)?;
    let mut resources = page_dictionary.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
    let mut fonts = resources.get(b"Font").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
    let name = fresh_name(&fonts, "RectoOcr");
    fonts.set(name.clone(), Object::Reference(font));
    resources.set("Font", Object::Dictionary(fonts));
    let mut content = b"Q\n".to_vec();
    for layer in layers {
        content.extend(Content { operations: layer.operations(&name) }.encode()?);
        content.push(b'\n');
    }
    append(document, page, open, content, resources)
}
