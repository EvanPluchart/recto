//! Reading content streams: transformation matrices and font metrics, enough to know where each
//! glyph and image lands on the page. Used by redaction (`redact.rs`).

use std::collections::HashMap;

use lopdf::{Dictionary, Object, Stream};

use crate::geom::{Point, Rect};

/// An affine transformation `[a b c d e f]`: `x' = a x + c y + e`, `y' = b x + d y + f`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [f32; 6]);

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    pub fn translate(x: f32, y: f32) -> Matrix {
        Matrix([1.0, 0.0, 0.0, 1.0, x, y])
    }

    /// Six numeric operands, as `cm` and `Tm` take them.
    pub fn from_operands(operands: &[Object]) -> Option<Matrix> {
        if operands.len() != 6 {
            return None;
        }
        let mut values = [0.0; 6];
        for (value, operand) in values.iter_mut().zip(operands) {
            *value = operand.as_float().ok()?;
        }
        Some(Matrix(values))
    }

    /// A matrix array (`/Matrix` of a form XObject), identity when missing or malformed.
    pub fn from_object(object: Option<&Object>) -> Matrix {
        object.and_then(|value| value.as_array().ok()).and_then(|items| Matrix::from_operands(items)).unwrap_or(Matrix::IDENTITY)
    }

    /// `self` applied first, then `other`.
    pub fn then(self, other: Matrix) -> Matrix {
        let [a1, b1, c1, d1, e1, f1] = self.0;
        let [a2, b2, c2, d2, e2, f2] = other.0;
        Matrix([
            a1 * a2 + b1 * c2,
            a1 * b2 + b1 * d2,
            c1 * a2 + d1 * c2,
            c1 * b2 + d1 * d2,
            e1 * a2 + f1 * c2 + e2,
            e1 * b2 + f1 * d2 + f2,
        ])
    }

    pub fn apply(self, point: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point { x: a * point.x + c * point.y + e, y: b * point.x + d * point.y + f }
    }

    pub fn invert(self) -> Option<Matrix> {
        let [a, b, c, d, e, f] = self.0;
        let determinant = a * d - b * c;
        if determinant.abs() < 1e-12 {
            return None;
        }
        let (ia, ib, ic, id) = (d / determinant, -b / determinant, -c / determinant, a / determinant);
        Some(Matrix([ia, ib, ic, id, -(e * ia + f * ic), -(e * ib + f * id)]))
    }

    /// Smallest rectangle holding `rect` once transformed.
    pub fn bounds(self, rect: Rect) -> Rect {
        let (x0, y0) = (rect.origin.x, rect.origin.y);
        let (x1, y1) = (x0 + rect.width, y0 + rect.height);
        bounds_of([(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(x, y)| self.apply(Point { x, y })))
    }
}

/// Smallest rectangle holding every point.
pub fn bounds_of(points: impl IntoIterator<Item = Point>) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for point in points {
        (x0, y0, x1, y1) = (x0.min(point.x), y0.min(point.y), x1.max(point.x), y1.max(point.y));
    }
    if x0 > x1 {
        return Rect { origin: Point { x: 0.0, y: 0.0 }, width: 0.0, height: 0.0 };
    }
    Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 }
}

/// Whether two rectangles share some area (touching edges do not count).
pub fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.origin.x < b.origin.x + b.width
        && b.origin.x < a.origin.x + a.width
        && a.origin.y < b.origin.y + b.height
        && b.origin.y < a.origin.y + a.height
}

/// Most bytes one stream may inflate to: a small crafted stream (a decompression bomb) must
/// not exhaust memory.
pub const MAX_STREAM_BYTES: usize = 256 << 20;

/// The decoded content of page `page`, empty when it would inflate past `MAX_STREAM_BYTES`.
pub fn page_content(document: &lopdf::Document, page: lopdf::ObjectId) -> Vec<u8> {
    document.get_page_content_with_limit(page, MAX_STREAM_BYTES).unwrap_or_default()
}

/// Follows a reference.
pub fn resolve<'a>(document: &'a lopdf::Document, object: &'a Object) -> &'a Object {
    match object {
        Object::Reference(id) => document.get_object(*id).unwrap_or(object),
        other => other,
    }
}

pub fn dictionary<'a>(document: &'a lopdf::Document, object: &'a Object) -> Option<&'a Dictionary> {
    match resolve(document, object) {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn number(document: &lopdf::Document, object: &Object) -> Option<f32> {
    resolve(document, object).as_float().ok()
}

/// How the bytes of a shown string split into character codes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Codes {
    /// Simple fonts: one byte per code.
    One,
    /// Composite fonts with a two-byte CMap (Identity-H and the UCS-2 ones).
    Two,
    /// A CMap this reader does not parse: code lengths unknown.
    Unknown,
}

/// Widths of a font, in text space units for a font size of 1. A width is an interval: exact
/// when the file gives it, a bound when it has to be guessed.
#[derive(Clone, Debug)]
pub struct FontMetrics {
    pub codes: Codes,
    /// Vertical writing: glyphs advance downwards. Positions are then not tracked.
    pub vertical: bool,
    widths: HashMap<u32, f32>,
    /// For codes missing from `widths`: the font's default width, or `None` when unknown.
    default_width: Option<f32>,
    /// Upper bound of a glyph width when it is unknown.
    max_width: f32,
    /// Extent of glyphs above and below the baseline (positive and negative).
    pub ascent: f32,
    pub descent: f32,
}

/// Bound used for widths a font does not give. Glyphs wider than 1.5 em are rare.
const UNKNOWN_WIDTH: f32 = 1.5;
/// Glyph extent used when the font descriptor gives none, generous on both sides.
const ASCENT: f32 = 1.0;
const DESCENT: f32 = -0.3;

impl FontMetrics {
    /// Metrics that know nothing: every width is between 0 and `UNKNOWN_WIDTH`.
    pub fn unknown() -> FontMetrics {
        FontMetrics {
            codes: Codes::Unknown,
            vertical: false,
            widths: HashMap::new(),
            default_width: None,
            max_width: UNKNOWN_WIDTH,
            ascent: ASCENT,
            descent: DESCENT,
        }
    }

    /// Reads a font dictionary.
    pub fn load(document: &lopdf::Document, font: &Dictionary) -> FontMetrics {
        let name = |key: &[u8]| font.get(key).ok().map(|value| resolve(document, value)).and_then(|value| value.as_name().ok());
        let subtype = name(b"Subtype").unwrap_or_default();
        let mut metrics = FontMetrics::unknown();
        if subtype == b"Type0" {
            let encoding = name(b"Encoding").unwrap_or_default();
            metrics.vertical = encoding.ends_with(b"-V");
            let two_byte = matches!(encoding, b"Identity-H" | b"Identity-V")
                || encoding.ends_with(b"UCS2-H")
                || encoding.ends_with(b"UCS2-V")
                || encoding.ends_with(b"UTF16-H")
                || encoding.ends_with(b"UTF16-V");
            metrics.codes = if two_byte { Codes::Two } else { Codes::Unknown };
            let descendant = font
                .get(b"DescendantFonts")
                .ok()
                .map(|value| resolve(document, value))
                .and_then(|value| value.as_array().ok())
                .and_then(|items| items.first())
                .and_then(|value| dictionary(document, value));
            if let Some(descendant) = descendant {
                metrics.default_width = Some(descendant.get(b"DW").ok().and_then(|value| number(document, value)).unwrap_or(1000.0) / 1000.0);
                if let Some(widths) = descendant.get(b"W").ok().map(|value| resolve(document, value)).and_then(|value| value.as_array().ok()) {
                    metrics.widths = cid_widths(document, widths);
                }
                metrics.read_descriptor(document, descendant);
            }
            return metrics;
        }

        metrics.codes = Codes::One;
        // Type 3 glyph widths are in glyph space, mapped by the font matrix.
        let scale = if subtype == b"Type3" {
            match font.get(b"FontMatrix").ok().map(|value| resolve(document, value)).and_then(|value| value.as_array().ok()) {
                Some(matrix) if !matrix.is_empty() => number(document, &matrix[0]).unwrap_or(0.001),
                _ => 0.001,
            }
        } else {
            0.001
        };
        let first = font.get(b"FirstChar").ok().and_then(|value| number(document, value)).unwrap_or(0.0) as u32;
        let widths = font.get(b"Widths").ok().map(|value| resolve(document, value)).and_then(|value| value.as_array().ok());
        match widths {
            Some(widths) => {
                for (offset, width) in widths.iter().enumerate() {
                    if let Some(width) = number(document, width) {
                        metrics.widths.insert(first + offset as u32, (width * scale).abs());
                    }
                }
                let missing = font
                    .get(b"FontDescriptor")
                    .ok()
                    .and_then(|value| dictionary(document, value))
                    .and_then(|descriptor| descriptor.get(b"MissingWidth").ok())
                    .and_then(|value| number(document, value));
                metrics.default_width = Some(missing.unwrap_or(0.0) * scale);
            }
            None => {
                // Standard 14 fonts may leave out /Widths; their metrics are known.
                let base = name(b"BaseFont").unwrap_or_default();
                let base = base.rsplit(|&byte| byte == b'+').next().unwrap_or(base);
                if base.starts_with(b"Courier") {
                    metrics.default_width = Some(0.6);
                } else if base == b"Helvetica" || base == b"Arial" || base == b"ArialMT" {
                    for (offset, width) in crate::annot::HELVETICA.iter().enumerate() {
                        metrics.widths.insert(32 + offset as u32, f32::from(*width) / 1000.0);
                    }
                }
            }
        }
        metrics.read_descriptor(document, font);
        metrics
    }

    fn read_descriptor(&mut self, document: &lopdf::Document, font: &Dictionary) {
        let Some(descriptor) = font.get(b"FontDescriptor").ok().and_then(|value| dictionary(document, value)) else { return };
        let value = |key: &[u8]| descriptor.get(key).ok().and_then(|value| number(document, value)).map(|value| value / 1000.0);
        // Keep at least the generic extent: some fonts understate it.
        if let Some(ascent) = value(b"Ascent") {
            self.ascent = self.ascent.max(ascent);
        }
        if let Some(descent) = value(b"Descent") {
            self.descent = self.descent.min(descent);
        }
    }

    /// Splits `bytes` into codes, or `None` when the code lengths are unknown.
    pub fn split(&self, bytes: &[u8]) -> Option<Vec<(std::ops::Range<usize>, u32)>> {
        match self.codes {
            Codes::One => Some(bytes.iter().enumerate().map(|(index, &byte)| (index..index + 1, u32::from(byte))).collect()),
            Codes::Two => Some(
                bytes
                    .chunks(2)
                    .enumerate()
                    .map(|(index, pair)| {
                        let code = pair.iter().fold(0u32, |code, &byte| code << 8 | u32::from(byte));
                        (index * 2..index * 2 + pair.len(), code)
                    })
                    .collect(),
            ),
            Codes::Unknown => None,
        }
    }

    /// Width of `code`: `(low, high)`, equal when exact.
    pub fn width(&self, code: u32) -> (f32, f32) {
        match self.widths.get(&code).copied().or(self.default_width) {
            Some(width) => (width, width),
            None => (0.0, self.max_width),
        }
    }

    /// Upper bound of the width of `bytes` when the codes cannot be split.
    pub fn max_width_of(&self, bytes: &[u8]) -> f32 {
        let widest = self.widths.values().copied().fold(self.max_width, f32::max);
        bytes.len() as f32 * widest
    }
}

/// The `/W` array of a CID font: `c [w1 w2 …]` or `c_first c_last w`.
fn cid_widths(document: &lopdf::Document, items: &[Object]) -> HashMap<u32, f32> {
    let mut widths = HashMap::new();
    let mut index = 0;
    while index < items.len() {
        let Some(first) = number(document, &items[index]) else { break };
        let first = first as u32;
        match items.get(index + 1).map(|value| resolve(document, value)) {
            Some(Object::Array(list)) => {
                for (offset, width) in list.iter().enumerate() {
                    if let Some(width) = number(document, width) {
                        widths.insert(first + offset as u32, width / 1000.0);
                    }
                }
                index += 2;
            }
            Some(last) => {
                let (Some(last), Some(width)) = (last.as_float().ok(), items.get(index + 2).and_then(|value| number(document, value))) else {
                    break;
                };
                // Bounded so a broken range cannot allocate without end.
                for code in first..=(last as u32).min(first.saturating_add(65_535)) {
                    widths.insert(code, width / 1000.0);
                }
                index += 3;
            }
            None => break,
        }
    }
    widths
}

/// Decoded images larger than this are removed instead of copied.
const MAX_IMAGE_BYTES: usize = 256 << 20;

pub(crate) struct Pixels {
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub data: Vec<u8>,
}


/// `stream` with its data replaced by decoded 8-bit pixels, Flate-compressed.
pub(crate) fn raw_copy(stream: &Stream, pixels: Pixels) -> Stream {
    let mut dict = stream.dict.clone();
    // Alternate versions (print, OPI proxies) would still hold the original pixels.
    for key in [b"Filter".as_slice(), b"DecodeParms", b"Length", b"Alternates", b"OPI"] {
        dict.remove(key);
    }
    dict.set("BitsPerComponent", 8);
    let mut copy = Stream::new(dict, pixels.data);
    let _ = copy.compress();
    copy
}

/// Number of color components of a color space this module can edit pixel by pixel.
pub(crate) fn components(document: &lopdf::Document, space: &Object) -> Option<usize> {
    match resolve(document, space) {
        Object::Name(name) => match name.as_slice() {
            b"DeviceGray" | b"G" | b"CalGray" => Some(1),
            b"DeviceRGB" | b"RGB" | b"CalRGB" => Some(3),
            b"DeviceCMYK" | b"CMYK" => Some(4),
            _ => None,
        },
        Object::Array(items) => {
            let family = items.first().and_then(|value| value.as_name().ok())?;
            match family {
                b"ICCBased" => {
                    let profile = resolve(document, items.get(1)?).as_stream().ok()?;
                    profile.dict.get(b"N").ok()?.as_i64().ok().map(|n| n as usize)
                }
                b"CalGray" => Some(1),
                b"CalRGB" | b"Lab" => Some(3),
                // One index per pixel.
                b"Indexed" | b"I" => Some(1),
                _ => None,
            }
        }
        _ => None,
    }
}

/// The pixels of an 8-bit image, or `None` when its format is not handled here.
pub(crate) fn decode_image(document: &lopdf::Document, image: &Stream) -> Option<Pixels> {
    let dict = &image.dict;
    let integer = |key: &[u8]| dict.get(key).ok().map(|value| resolve(document, value)).and_then(|value| value.as_i64().ok());
    let (width, height) = (usize::try_from(integer(b"Width")?).ok()?, usize::try_from(integer(b"Height")?).ok()?);
    if dict.get(b"ImageMask").ok().and_then(|value| value.as_bool().ok()) == Some(true) {
        return None;
    }
    // Soft masks have no color space and are gray.
    let components = match dict.get(b"ColorSpace") {
        Ok(space) => components(document, space)?,
        Err(_) if dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Image") => 1,
        Err(_) => return None,
    };
    let size = width.checked_mul(height)?.checked_mul(components)?;
    if size > MAX_IMAGE_BYTES {
        return None;
    }
    let filters: Vec<Vec<u8>> = match dict.get(b"Filter").ok().map(|value| resolve(document, value)) {
        None => Vec::new(),
        Some(Object::Name(name)) => vec![name.clone()],
        Some(Object::Array(names)) => names.iter().filter_map(|name| name.as_name().ok().map(<[u8]>::to_vec)).collect(),
        Some(_) => return None,
    };
    let data = match filters.as_slice() {
        [last] if last == b"DCTDecode" || last == b"DCT" => {
            if components == 4 {
                return None;
            }
            let decoded = image::load_from_memory_with_format(&image.content, image::ImageFormat::Jpeg).ok()?;
            match components {
                1 => decoded.into_luma8().into_raw(),
                _ => decoded.into_rgb8().into_raw(),
            }
        }
        _ => {
            if integer(b"BitsPerComponent") != Some(8) {
                return None;
            }
            let supported = |name: &Vec<u8>| matches!(name.as_slice(), b"FlateDecode" | b"Fl" | b"LZWDecode" | b"LZW" | b"ASCII85Decode" | b"ASCIIHexDecode" | b"RunLengthDecode");
            if !filters.iter().all(supported) {
                return None;
            }
            if filters.is_empty() { image.content.clone() } else { image.decompressed_content_with_limit(size.saturating_add(1 << 20)).ok()? }
        }
    };
    (data.len() >= size).then(|| Pixels { width, height, components, data: data[..size].to_vec() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrices_compose_and_invert() {
        let scale = Matrix([2.0, 0.0, 0.0, 3.0, 0.0, 0.0]);
        let shift = Matrix::translate(10.0, 20.0);
        let point = Point { x: 1.0, y: 1.0 };
        assert_eq!(scale.then(shift).apply(point), Point { x: 12.0, y: 23.0 });
        assert_eq!(shift.then(scale).apply(point), Point { x: 22.0, y: 63.0 });
        let turn = Matrix([0.0, 1.0, -1.0, 0.0, 5.0, 7.0]);
        let back = turn.invert().unwrap().apply(turn.apply(point));
        assert!((back.x - 1.0).abs() < 1e-5 && (back.y - 1.0).abs() < 1e-5);
        let unit = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 1.0, height: 1.0 };
        assert_eq!(turn.bounds(unit), Rect { origin: Point { x: 4.0, y: 7.0 }, width: 1.0, height: 1.0 });
    }

    #[test]
    fn reads_widths() {
        let document = lopdf::Document::with_version("1.7");
        let mut simple = Dictionary::new();
        simple.set("Subtype", Object::Name(b"Type1".to_vec()));
        simple.set("FirstChar", 65);
        simple.set("Widths", Object::Array(vec![500.into(), 250.into()]));
        let metrics = FontMetrics::load(&document, &simple);
        assert_eq!((metrics.width(65), metrics.width(66), metrics.width(67)), ((0.5, 0.5), (0.25, 0.25), (0.0, 0.0)));

        let mut helvetica = Dictionary::new();
        helvetica.set("Subtype", Object::Name(b"Type1".to_vec()));
        helvetica.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        let metrics = FontMetrics::load(&document, &helvetica);
        assert_eq!(metrics.width(u32::from(b'H')), (0.722, 0.722));
        assert_eq!(metrics.width(200), (0.0, UNKNOWN_WIDTH));

        let mut cid = Dictionary::new();
        cid.set("DW", 1000);
        cid.set("W", Object::Array(vec![1.into(), Object::Array(vec![500.into(), 600.into()]), 10.into(), 12.into(), 300.into()]));
        let mut composite = Dictionary::new();
        composite.set("Subtype", Object::Name(b"Type0".to_vec()));
        composite.set("Encoding", Object::Name(b"Identity-H".to_vec()));
        composite.set("DescendantFonts", Object::Array(vec![Object::Dictionary(cid)]));
        let metrics = FontMetrics::load(&document, &composite);
        assert_eq!(metrics.codes, Codes::Two);
        assert_eq!((metrics.width(2).0, metrics.width(11).0, metrics.width(99).0), (0.6, 0.3, 1.0));
        assert_eq!(metrics.split(&[0, 1, 0, 2]).unwrap().iter().map(|(_, code)| *code).collect::<Vec<_>>(), vec![1, 2]);

        composite.set("Encoding", Object::Name(b"90ms-RKSJ-H".to_vec()));
        assert!(FontMetrics::load(&document, &composite).split(b"ab").is_none());
    }
}
