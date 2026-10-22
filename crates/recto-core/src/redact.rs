//! Redaction: what lies under an area is removed from the file, then the area is painted black.
//!
//! The page content is interpreted (`content.rs`) to find where each glyph, image and form lands:
//! * glyphs whose box meets an area are taken out of their string; when the glyph width is known,
//!   a `TJ` adjustment keeps the following text in place;
//! * strings in a font whose code lengths are unknown are removed whole when they may reach an
//!   area;
//! * images are copied with the covered pixels set to zero when they can be decoded (8-bit,
//!   uncompressed, Flate, LZW or baseline JPEG in gray or RGB), and removed otherwise; inline
//!   images that meet an area are removed;
//! * form XObjects that meet an area are copied and redacted recursively;
//! * paths lying entirely inside an area are not painted; paths crossing its edge are kept and
//!   covered by the black box;
//! * annotations and form fields that meet an area are deleted, with their values;
//! * alternate text of marked content (`/ActualText`, `/Alt`) on the page is dropped, as are the
//!   page thumbnail and the document's structure tree, which may repeat the text.
//!
//! Shared objects are never modified: the page gets new content, resources and copies.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::content::{FontMetrics, Matrix, Pixels, bounds_of, decode_image, dictionary, overlaps, raw_copy, resolve};
use crate::geom::{Point, Rect};

/// Nested forms deeper than this are removed instead of interpreted.
const MAX_DEPTH: usize = 12;
/// Most form XObjects rewritten for one page; past it, forms over an area are removed.
const MAX_FORMS: usize = 10_000;

/// Removes what lies under the areas (user space) on `page` and paints each one black, or white
/// when its flag is set (erasing).
pub fn redact_page(document: &mut lopdf::Document, page: ObjectId, painted: &[(Rect, bool)]) -> lopdf::Result<Vec<ObjectId>> {
    let areas: Vec<Rect> = painted.iter().map(|(area, _)| *area).collect();
    let areas = areas.as_slice();
    if areas.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = crate::content::page_content(document, page);
    let operations = Content::decode(&bytes).map(|content| content.operations).unwrap_or_default();
    let resources = document
        .get_dictionary(page)?
        .get(b"Resources")
        .ok()
        .and_then(|value| dictionary(document, value))
        .cloned()
        .unwrap_or_default();

    let mut redactor = Redactor { document, areas, created: Vec::new(), next_id: document.max_id + 1, serial: 0, originals: Vec::new(), forms_left: MAX_FORMS };
    let (operations, resources) = redactor.run(operations, resources, Matrix::IDENTITY, 0);
    let created = std::mem::take(&mut redactor.created);
    let originals = std::mem::take(&mut redactor.originals);

    let mut content = b"q\n".to_vec();
    content.extend(Content { operations }.encode()?);
    content.extend_from_slice(b"\nQ\n");
    for (area, white) in painted {
        let gray = if *white { 1 } else { 0 };
        content.extend(format!("q {gray} g {} {} {} {} re f Q\n", area.origin.x, area.origin.y, area.width, area.height).into_bytes());
    }

    for (id, object) in created {
        document.objects.insert(id, object);
        document.max_id = document.max_id.max(id.0);
    }
    let mut stream = Stream::new(Dictionary::new(), content);
    let _ = stream.compress();
    let contents = document.add_object(stream);
    let dictionary = document.get_dictionary_mut(page)?;
    dictionary.set("Contents", Object::Reference(contents));
    dictionary.set("Resources", Object::Dictionary(resources));
    for key in [b"Thumb".as_slice(), b"PieceInfo", b"Metadata", b"StructParents"] {
        dictionary.remove(key);
    }
    // The structure tree can hold the page's text as alternate descriptions.
    if let Ok(catalog) = document.catalog_mut() {
        catalog.remove(b"StructTreeRoot");
        catalog.remove(b"MarkInfo");
    }
    Ok(originals)
}

/// Takes `originals` (XObjects that redacted pages no longer draw) out of the resources of the
/// other pages that list them without drawing them, so a shared resource dictionary does not
/// keep redacted images and forms in the file. Pages that draw them keep them.
pub fn forget(document: &mut lopdf::Document, originals: &[ObjectId]) -> lopdf::Result<()> {
    if originals.is_empty() {
        return Ok(());
    }
    let pages: Vec<ObjectId> = document.get_pages().into_values().collect();
    for page in pages {
        let Some(resources) = document.get_dictionary(page)?.get(b"Resources").ok().and_then(|value| dictionary(document, value)) else {
            continue;
        };
        let Some(xobjects) = resources.get(b"XObject").ok().and_then(|value| dictionary(document, value)) else { continue };
        let listed: Vec<Vec<u8>> = xobjects
            .iter()
            .filter(|(_, value)| value.as_reference().is_ok_and(|id| originals.contains(&id)))
            .map(|(name, _)| name.clone())
            .collect();
        if listed.is_empty() {
            continue;
        }
        let drawn: HashSet<Vec<u8>> = Content::decode(&crate::content::page_content(document, page))
            .map(|content| {
                content
                    .operations
                    .into_iter()
                    .filter(|operation| operation.operator == "Do")
                    .filter_map(|operation| operation.operands.first().and_then(|value| value.as_name().ok()).map(<[u8]>::to_vec))
                    .collect()
            })
            .unwrap_or_default();
        // Forms without resources of their own use the page's: any name may be drawn.
        let inheriting = xobjects.iter().any(|(_, value)| {
            dictionary(document, value).is_some_and(|form| {
                form.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Form") && !form.has(b"Resources")
            })
        });
        if inheriting {
            continue;
        }
        let mut resources = resources.clone();
        let mut xobjects = xobjects.clone();
        for name in listed.iter().filter(|name| !drawn.contains(*name)) {
            xobjects.remove(name);
        }
        resources.set("XObject", Object::Dictionary(xobjects));
        document.get_dictionary_mut(page)?.set("Resources", Object::Dictionary(resources));
    }
    Ok(())
}

/// Deletes the annotations of `page` that meet `areas`, with their popups and, for widgets,
/// their form fields when no other widget is left.
pub fn remove_annotations(document: &mut lopdf::Document, page: ObjectId, areas: &[Rect]) -> lopdf::Result<()> {
    let annots = match document.get_dictionary(page)?.get(b"Annots") {
        Ok(value) => resolve(document, value).as_array().cloned().unwrap_or_default(),
        Err(_) => return Ok(()),
    };
    let mut removed: Vec<ObjectId> = Vec::new();
    for item in &annots {
        let Ok(id) = item.as_reference() else { continue };
        let Ok(annotation) = document.get_dictionary(id) else { continue };
        let rect = annotation.get(b"Rect").ok().map(|value| resolve(document, value)).and_then(|value| value.as_array().ok()).and_then(|items| {
            let numbers: Vec<f32> = items.iter().filter_map(|value| resolve(document, value).as_float().ok()).collect();
            (numbers.len() == 4).then(|| {
                bounds_of([Point { x: numbers[0], y: numbers[1] }, Point { x: numbers[2], y: numbers[3] }])
            })
        });
        // An annotation without a usable rectangle could be anywhere: removed.
        if rect.is_none_or(|rect| areas.iter().any(|area| overlaps(&grow(rect), area))) {
            removed.push(id);
        }
    }
    // Popups of removed annotations.
    for item in &annots {
        let Ok(id) = item.as_reference() else { continue };
        let parent = document.get_dictionary(id).ok().and_then(|annotation| annotation.get(b"Parent").ok()).and_then(|value| value.as_reference().ok());
        let is_popup = document
            .get_dictionary(id)
            .ok()
            .and_then(|annotation| annotation.get(b"Subtype").ok())
            .is_some_and(|value| value.as_name().is_ok_and(|name| name == b"Popup"));
        if is_popup && parent.is_some_and(|parent| removed.contains(&parent)) && !removed.contains(&id) {
            removed.push(id);
        }
    }
    let kept: Vec<Object> = annots.into_iter().filter(|item| item.as_reference().map_or(true, |id| !removed.contains(&id))).collect();
    document.get_dictionary_mut(page)?.set("Annots", Object::Array(kept));
    for id in removed {
        let parent = document.get_dictionary(id).ok().and_then(|annotation| annotation.get(b"Parent").ok()).and_then(|value| value.as_reference().ok());
        document.delete_object(id);
        // A field left without widgets still holds its value: delete it too, up the tree.
        let mut parent = parent;
        while let Some(field) = parent {
            let empty = document
                .get_dictionary(field)
                .ok()
                .is_some_and(|field| field.get(b"Kids").ok().and_then(|kids| kids.as_array().ok()).is_some_and(|kids| kids.is_empty()));
            if !empty {
                break;
            }
            parent = document.get_dictionary(field).ok().and_then(|field| field.get(b"Parent").ok()).and_then(|value| value.as_reference().ok());
            document.delete_object(field);
        }
    }
    Ok(())
}


/// Removes the topmost image drawn at `at` (user space) by the content of `page` itself:
/// image XObjects and inline images. Returns whether one was found. Shared objects are not
/// modified: the page gets new content.
pub fn remove_image_at(document: &mut lopdf::Document, page: ObjectId, at: Point) -> lopdf::Result<bool> {
    let bytes = crate::content::page_content(document, page);
    let Ok(content) = Content::decode(&bytes) else { return Ok(false) };
    let mut operations = content.operations;
    let resources = document.get_dictionary(page)?.get(b"Resources").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
    let xobjects = resources.get(b"XObject").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
    let unit = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 1.0, height: 1.0 };
    let mut stack = Vec::new();
    let mut ctm = Matrix::IDENTITY;
    let mut hit = None;
    for (index, operation) in operations.iter().enumerate() {
        match operation.operator.as_str() {
            "q" => stack.push(ctm),
            "Q" => ctm = stack.pop().unwrap_or(ctm),
            "cm" => {
                if let Some(matrix) = Matrix::from_operands(&operation.operands) {
                    ctm = matrix.then(ctm);
                }
            }
            "Do" | "BI" => {
                let image = operation.operator == "BI"
                    || operation
                        .operands
                        .first()
                        .and_then(|value| value.as_name().ok())
                        .and_then(|name| xobjects.get(name).ok())
                        .and_then(|value| resolve(document, value).as_stream().ok())
                        .is_some_and(|stream| stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Image"));
                let shown = ctm.bounds(unit);
                let inside = at.x >= shown.origin.x && at.x <= shown.origin.x + shown.width && at.y >= shown.origin.y && at.y <= shown.origin.y + shown.height;
                if image && inside {
                    hit = Some(index);
                }
            }
            _ => {}
        }
    }
    let Some(index) = hit else { return Ok(false) };
    operations.remove(index);
    let mut stream = Stream::new(Dictionary::new(), Content { operations }.encode()?);
    let _ = stream.compress();
    let contents = document.add_object(stream);
    document.get_dictionary_mut(page)?.set("Contents", Object::Reference(contents));
    Ok(true)
}
/// Rectangles from files can be degenerate (a line has no width); give them some extent.
fn grow(rect: Rect) -> Rect {
    Rect { origin: Point { x: rect.origin.x - 0.5, y: rect.origin.y - 0.5 }, width: rect.width + 1.0, height: rect.height + 1.0 }
}

#[derive(Clone)]
struct Graphics {
    ctm: Matrix,
    font: Option<Rc<FontMetrics>>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    /// Horizontal scaling, as a fraction.
    scale: f32,
    leading: f32,
    rise: f32,
}

impl Graphics {
    fn new(ctm: Matrix) -> Graphics {
        Graphics { ctm, font: None, size: 0.0, char_spacing: 0.0, word_spacing: 0.0, scale: 1.0, leading: 0.0, rise: 0.0 }
    }
}

struct Text {
    /// The text matrix at the start of the current run of glyphs.
    matrix: Matrix,
    line: Matrix,
    /// Horizontal position since `matrix`, in text space: an interval when widths are guessed.
    offset: (f32, f32),
}

impl Text {
    fn new() -> Text {
        Text { matrix: Matrix::IDENTITY, line: Matrix::IDENTITY, offset: (0.0, 0.0) }
    }

    fn move_line(&mut self, x: f32, y: f32) {
        self.line = Matrix::translate(x, y).then(self.line);
        self.matrix = self.line;
        self.offset = (0.0, 0.0);
    }
}

struct Redactor<'a> {
    document: &'a lopdf::Document,
    areas: &'a [Rect],
    /// New objects (copies of images and forms), added to the document at the end.
    created: Vec<(ObjectId, Object)>,
    next_id: u32,
    serial: usize,
    /// XObjects the page drew that were replaced by copies or removed.
    originals: Vec<ObjectId>,
    /// Forms still allowed to be rewritten: a form drawing itself many times, nested, would
    /// otherwise be copied an exponential number of times.
    forms_left: usize,
}

impl Redactor<'_> {
    fn meets(&self, rect: Rect) -> bool {
        self.areas.iter().any(|area| overlaps(&rect, area))
    }

    fn add(&mut self, object: Object) -> ObjectId {
        let id = (self.next_id, 0);
        self.next_id += 1;
        self.created.push((id, object));
        id
    }

    /// Redacts `operations` drawn with `resources` under `ctm`. Returns the new operations and
    /// resources (a copy; XObjects that were redacted are replaced by their copies).
    fn run(&mut self, operations: Vec<Operation>, mut resources: Dictionary, ctm: Matrix, depth: usize) -> (Vec<Operation>, Dictionary) {
        let mut fonts: HashMap<Vec<u8>, Rc<FontMetrics>> = HashMap::new();
        let mut stack: Vec<Graphics> = Vec::new();
        let mut graphics = Graphics::new(ctm);
        let mut text = Text::new();
        let mut path: Vec<Point> = Vec::new();
        let mut out: Vec<Operation> = Vec::with_capacity(operations.len());
        // XObject names replaced by copies or dropped; removed from the resources if unused.
        let mut replaced: HashSet<Vec<u8>> = HashSet::new();
        let mut xobjects = resources.get(b"XObject").ok().and_then(|value| dictionary(self.document, value)).cloned().unwrap_or_default();

        for mut operation in operations {
            let operands = &operation.operands;
            let number = |index: usize| operands.get(index).and_then(|value| value.as_float().ok()).unwrap_or(0.0);
            match operation.operator.as_str() {
                "q" => stack.push(graphics.clone()),
                "Q" => {
                    if let Some(saved) = stack.pop() {
                        graphics = saved;
                    }
                }
                "cm" => {
                    if let Some(matrix) = Matrix::from_operands(operands) {
                        graphics.ctm = matrix.then(graphics.ctm);
                    }
                }
                "BT" => text = Text::new(),
                "Tc" => graphics.char_spacing = number(0),
                "Tw" => graphics.word_spacing = number(0),
                "Tz" => graphics.scale = number(0) / 100.0,
                "TL" => graphics.leading = number(0),
                "Ts" => graphics.rise = number(0),
                "Tf" => {
                    let name = operands.first().and_then(|value| value.as_name().ok()).unwrap_or_default().to_vec();
                    graphics.size = number(1);
                    let metrics = fonts
                        .entry(name.clone())
                        .or_insert_with(|| {
                            let font = resources
                                .get(b"Font")
                                .ok()
                                .and_then(|value| dictionary(self.document, value))
                                .and_then(|fonts| fonts.get(&name).ok())
                                .and_then(|value| dictionary(self.document, value));
                            Rc::new(font.map_or_else(FontMetrics::unknown, |font| FontMetrics::load(self.document, font)))
                        })
                        .clone();
                    graphics.font = Some(metrics);
                }
                "Td" => text.move_line(number(0), number(1)),
                "TD" => {
                    graphics.leading = -number(1);
                    text.move_line(number(0), number(1));
                }
                "Tm" => {
                    if let Some(matrix) = Matrix::from_operands(operands) {
                        text.line = matrix;
                        text.matrix = matrix;
                        text.offset = (0.0, 0.0);
                    }
                }
                "T*" => text.move_line(0.0, -graphics.leading),
                "Tj" | "TJ" | "'" | "\"" => {
                    let operator = operation.operator.clone();
                    if operator == "\"" {
                        graphics.word_spacing = number(0);
                        graphics.char_spacing = number(1);
                    }
                    if operator == "'" || operator == "\"" {
                        text.move_line(0.0, -graphics.leading);
                    }
                    let shown: Vec<Object> = match operands.last() {
                        Some(Object::Array(items)) if operator == "TJ" => items.clone(),
                        Some(string @ Object::String(..)) => vec![string.clone()],
                        _ => Vec::new(),
                    };
                    match self.show(&shown, &graphics, &mut text) {
                        None => out.push(operation),
                        Some(kept) => {
                            if operator == "\"" {
                                out.push(Operation::new("Tw", vec![operands[0].clone()]));
                                out.push(Operation::new("Tc", vec![operands[1].clone()]));
                            }
                            if operator == "'" || operator == "\"" {
                                out.push(Operation::new("T*", vec![]));
                            }
                            out.push(Operation::new("TJ", vec![Object::Array(kept)]));
                        }
                    }
                    continue;
                }
                "Do" => {
                    let name = operands.first().and_then(|value| value.as_name().ok()).unwrap_or_default().to_vec();
                    let draw = self.xobject(&name, &xobjects, &resources, graphics.ctm, depth);
                    if !matches!(draw, Draw::Keep)
                        && let Some(original) = xobjects.get(&name).ok().and_then(|value| value.as_reference().ok())
                    {
                        self.originals.push(original);
                    }
                    match draw {
                        Draw::Keep => out.push(operation),
                        Draw::Remove => {
                            replaced.insert(name);
                        }
                        Draw::Replace(id) => {
                            let fresh = loop {
                                self.serial += 1;
                                let fresh = format!("RectoRedacted{}", self.serial).into_bytes();
                                if !xobjects.has(&fresh) {
                                    break fresh;
                                }
                            };
                            xobjects.set(fresh.clone(), Object::Reference(id));
                            replaced.insert(name);
                            out.push(Operation::new("Do", vec![Object::Name(fresh)]));
                        }
                    }
                    continue;
                }
                "BI" => {
                    let unit = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 1.0, height: 1.0 };
                    if self.meets(graphics.ctm.bounds(unit)) {
                        continue;
                    }
                }
                "BDC" => {
                    if let Some(Object::Dictionary(properties)) = operation.operands.get_mut(1) {
                        for key in [b"ActualText".as_slice(), b"Alt", b"E"] {
                            properties.remove(key);
                        }
                    }
                }
                "m" | "l" => path.push(graphics.ctm.apply(Point { x: number(0), y: number(1) })),
                "c" => {
                    for index in [0, 2, 4] {
                        path.push(graphics.ctm.apply(Point { x: number(index), y: number(index + 1) }));
                    }
                }
                "v" | "y" => {
                    for index in [0, 2] {
                        path.push(graphics.ctm.apply(Point { x: number(index), y: number(index + 1) }));
                    }
                }
                "re" => {
                    let (x, y, width, height) = (number(0), number(1), number(2), number(3));
                    for (px, py) in [(x, y), (x + width, y), (x, y + height), (x + width, y + height)] {
                        path.push(graphics.ctm.apply(Point { x: px, y: py }));
                    }
                }
                "S" | "s" | "f" | "F" | "f*" | "B" | "B*" | "b" | "b*" | "n" => {
                    let inside = !path.is_empty() && {
                        let bounds = bounds_of(path.iter().copied());
                        self.areas.iter().any(|area| contains(area, &bounds))
                    };
                    path.clear();
                    if inside {
                        // Ends the path without painting it; a pending clip still applies.
                        out.push(Operation::new("n", vec![]));
                        continue;
                    }
                }
                _ => {}
            }
            out.push(operation);
        }

        let used: HashSet<Vec<u8>> = out
            .iter()
            .filter(|operation| operation.operator == "Do")
            .filter_map(|operation| operation.operands.first().and_then(|value| value.as_name().ok()).map(<[u8]>::to_vec))
            .collect();
        for name in replaced {
            if !used.contains(&name) {
                xobjects.remove(&name);
            }
        }
        if !xobjects.is_empty() {
            resources.set("XObject", Object::Dictionary(xobjects));
        }
        (out, resources)
    }

    /// Glyphs of a show operation. `None` when nothing is removed; otherwise the `TJ` array to
    /// show instead.
    fn show(&mut self, items: &[Object], graphics: &Graphics, text: &mut Text) -> Option<Vec<Object>> {
        let unknown = Rc::new(FontMetrics::unknown());
        let font = graphics.font.clone().unwrap_or(unknown);
        let (size, scale) = (graphics.size, graphics.scale);
        let to_user = text.matrix.then(graphics.ctm);
        let (low_y, high_y) = {
            let (a, b) = (graphics.rise + font.descent * size, graphics.rise + font.ascent * size);
            (a.min(b), a.max(b))
        };
        // A box in text space, from `x0` to `x1` horizontally.
        let glyph_box = |x0: f32, x1: f32| -> Rect {
            to_user.bounds(Rect { origin: Point { x: x0.min(x1), y: low_y }, width: (x1 - x0).abs(), height: high_y - low_y })
        };

        if font.vertical {
            // Positions in vertical writing are not followed: the whole column may hold the text.
            let column = to_user.bounds(Rect {
                origin: Point { x: -0.6 * size.abs(), y: -1.0e6 },
                width: 1.2 * size.abs(),
                height: 2.0e6,
            });
            return self.meets(column).then(Vec::new);
        }

        let mut changed = false;
        let mut kept: Vec<Object> = Vec::new();
        // Adds a horizontal adjustment, merged with a preceding one.
        let push_adjustment = |kept: &mut Vec<Object>, value: f32| {
            if let Some(Object::Real(previous)) = kept.last_mut() {
                *previous = round(*previous + value);
            } else {
                kept.push(Object::Real(round(value)));
            }
        };
        for item in items {
            match item {
                Object::String(bytes, format) => {
                    let Some(codes) = font.split(bytes) else {
                        // Code lengths unknown: the string is kept or removed whole.
                        let spacing = bytes.len() as f32 * (graphics.char_spacing.abs() + graphics.word_spacing.abs()) * scale.abs();
                        let reach = font.max_width_of(bytes) * (size * scale).abs() + spacing;
                        let (low, high) = text.offset;
                        let extent = glyph_box(low - spacing, high + reach);
                        text.offset = (low - spacing, high + reach);
                        if self.meets(extent) {
                            changed = true;
                        } else {
                            kept.push(item.clone());
                        }
                        continue;
                    };
                    let mut piece: Vec<u8> = Vec::new();
                    for (range, code) in codes {
                        let (low_width, high_width) = font.width(code);
                        let single_space = range.len() == 1 && code == 32;
                        let spacing = graphics.char_spacing + if single_space { graphics.word_spacing } else { 0.0 };
                        let advance_low = (low_width * size + spacing) * scale;
                        let advance_high = (high_width * size + spacing) * scale;
                        let (low, high) = text.offset;
                        let extent = glyph_box(low.min(low + advance_low), high.max(high + advance_high));
                        text.offset = (low + advance_low.min(advance_high), high + advance_low.max(advance_high));
                        if self.meets(extent) {
                            changed = true;
                            if !piece.is_empty() {
                                kept.push(Object::String(std::mem::take(&mut piece), *format));
                            }
                            // Keeps the following glyphs in place when the advance is known.
                            if low_width == high_width && size != 0.0 {
                                push_adjustment(&mut kept, -(low_width * size + spacing) * 1000.0 / size);
                            }
                        } else {
                            piece.extend_from_slice(&bytes[range]);
                        }
                    }
                    if !piece.is_empty() {
                        kept.push(Object::String(piece, *format));
                    }
                }
                adjustment => {
                    let Ok(value) = adjustment.as_float() else { continue };
                    let shift = -value / 1000.0 * size * scale;
                    text.offset = (text.offset.0 + shift, text.offset.1 + shift);
                    push_adjustment(&mut kept, value);
                }
            }
        }
        changed.then_some(kept)
    }

    fn xobject(&mut self, name: &[u8], xobjects: &Dictionary, resources: &Dictionary, ctm: Matrix, depth: usize) -> Draw {
        let Ok(entry) = xobjects.get(name) else { return Draw::Keep };
        let Object::Stream(stream) = resolve(self.document, entry) else { return Draw::Keep };
        let subtype = stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()).unwrap_or_default();
        match subtype {
            b"Image" => {
                let unit = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 1.0, height: 1.0 };
                if !self.meets(ctm.bounds(unit)) {
                    return Draw::Keep;
                }
                match self.blackout(stream, ctm) {
                    Some(copy) => Draw::Replace(self.add(Object::Stream(copy))),
                    None => Draw::Remove,
                }
            }
            b"Form" => {
                let matrix = Matrix::from_object(stream.dict.get(b"Matrix").ok());
                let to_user = matrix.then(ctm);
                let bbox = stream
                    .dict
                    .get(b"BBox")
                    .ok()
                    .map(|value| resolve(self.document, value))
                    .and_then(|value| value.as_array().ok())
                    .and_then(|items| {
                        let numbers: Vec<f32> = items.iter().filter_map(|value| value.as_float().ok()).collect();
                        (numbers.len() == 4).then(|| bounds_of([Point { x: numbers[0], y: numbers[1] }, Point { x: numbers[2], y: numbers[3] }]))
                    });
                // Without a bounding box the form could draw anywhere.
                if bbox.is_some_and(|bbox| !self.meets(to_user.bounds(bbox))) {
                    return Draw::Keep;
                }
                if depth >= MAX_DEPTH || self.forms_left == 0 {
                    return Draw::Remove;
                }
                self.forms_left -= 1;
                let Ok(bytes) = (if stream.dict.has(b"Filter") { stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES) } else { Ok(stream.content.clone()) }) else {
                    return Draw::Remove;
                };
                let operations = Content::decode(&bytes).map(|content| content.operations).unwrap_or_default();
                let own = stream.dict.get(b"Resources").ok().and_then(|value| dictionary(self.document, value)).cloned();
                let (operations, new_resources) = self.run(operations, own.unwrap_or_else(|| resources.clone()), to_user, depth + 1);
                let Ok(content) = (Content { operations }).encode() else { return Draw::Remove };
                let mut dict = stream.dict.clone();
                for key in [b"Filter".as_slice(), b"DecodeParms", b"Length"] {
                    dict.remove(key);
                }
                dict.set("Resources", Object::Dictionary(new_resources));
                let mut copy = Stream::new(dict, content);
                let _ = copy.compress();
                Draw::Replace(self.add(Object::Stream(copy)))
            }
            _ => Draw::Keep,
        }
    }

    /// A copy of an image with the pixels under the areas set to zero, or `None` when the image
    /// cannot be decoded (it is then removed).
    fn blackout(&mut self, image: &Stream, ctm: Matrix) -> Option<Stream> {
        let to_image = ctm.invert()?;
        let mut pixels = decode_image(self.document, image)?;
        self.clear(&mut pixels, to_image);
        let mut copy = raw_copy(image, pixels);
        // Masks may hold the image's shapes: a soft mask is redacted too, or dropped when it
        // cannot be decoded; stencil and color-key masks are dropped.
        copy.dict.remove(b"Mask");
        copy.dict.remove(b"SMask");
        let mask = image.dict.get(b"SMask").ok().and_then(|value| resolve(self.document, value).as_stream().ok());
        if let Some((mask, mut mask_pixels)) = mask.and_then(|mask| decode_image(self.document, mask).map(|pixels| (mask, pixels))) {
            self.clear(&mut mask_pixels, to_image);
            let id = self.add(Object::Stream(raw_copy(mask, mask_pixels)));
            copy.dict.set("SMask", Object::Reference(id));
        }
        Some(copy)
    }

    /// Zeroes the pixels of `image` under the areas. `to_image` maps user space to the image's
    /// unit square.
    fn clear(&self, image: &mut Pixels, to_image: Matrix) {
        for area in self.areas {
            let unit = to_image.bounds(*area);
            let (width, height) = (image.width as f32, image.height as f32);
            // The unit square's bottom row is the image's last row.
            let x0 = (unit.origin.x * width).floor().clamp(0.0, width) as usize;
            let x1 = ((unit.origin.x + unit.width) * width).ceil().clamp(0.0, width) as usize;
            let y0 = ((1.0 - unit.origin.y - unit.height) * height).floor().clamp(0.0, height) as usize;
            let y1 = ((1.0 - unit.origin.y) * height).ceil().clamp(0.0, height) as usize;
            for row in y0..y1 {
                let start = (row * image.width + x0) * image.components;
                let end = (row * image.width + x1) * image.components;
                image.data[start..end].fill(0);
            }
        }
    }
}

enum Draw {
    Keep,
    Remove,
    Replace(ObjectId),
}

/// Thousandths of text space are far below what shows; rounding keeps the content readable.
fn round(value: f32) -> f32 {
    (value * 1000.0).round() / 1000.0
}


fn contains(outer: &Rect, inner: &Rect) -> bool {
    inner.origin.x >= outer.origin.x
        && inner.origin.y >= outer.origin.y
        && inner.origin.x + inner.width <= outer.origin.x + outer.width
        && inner.origin.y + inner.height <= outer.origin.y + outer.height
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::annot::{Font, text_width};
    use crate::op::Op;
    use crate::{Document, PageId};

    fn area(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 }
    }

    /// Areas over each token of `redact.pdf` page 1 (see the fixture generator).
    fn areas() -> Vec<Rect> {
        let helvetica = |text: &str, size: f32| text_width(text, size, Font::Helvetica);
        vec![
            area(56.0 + helvetica("Name: Alice ", 14.0) + 1.0, 758.0, 56.0 + helvetica("Name: Alice ALPHATOKEN", 14.0) - 1.0, 770.0),
            area(95.0, 718.0, 165.0, 728.0),
            area(56.0 + helvetica("Hidden ", 12.0) + 1.0, 678.0, 56.0 + helvetica("Hidden GOLF", 12.0) - 1.0, 688.0),
            area(300.0, 501.0, 399.0, 599.0),
            area(60.0, 505.0, 100.0, 515.0),
            area(56.0 + helvetica("Form ", 12.0) + 1.0, 400.0, 56.0 + helvetica("Form CHARLIETOKEN", 12.0) - 1.0, 410.0),
            area(320.0, 310.0, 380.0, 320.0),
        ]
    }

    pub(crate) fn redacted() -> Vec<u8> {
        let mut document = Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/redact.pdf")).unwrap();
        for area in areas() {
            document.apply(Op::Redact { page: PageId { source: 0, index: 0 }, area });
        }
        document.build().unwrap()
    }

    /// Every string and stream (decoded when possible) in the file.
    fn all_bytes(document: &lopdf::Document) -> Vec<Vec<u8>> {
        fn visit(object: &Object, found: &mut Vec<Vec<u8>>) {
            match object {
                Object::String(bytes, _) => found.push(bytes.clone()),
                Object::Array(items) => items.iter().for_each(|item| visit(item, found)),
                Object::Dictionary(dictionary) => dictionary.iter().for_each(|(_, value)| visit(value, found)),
                Object::Stream(stream) => {
                    stream.dict.iter().for_each(|(_, value)| visit(value, found));
                    found.push(stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES).unwrap_or_else(|_| stream.content.clone()));
                }
                _ => {}
            }
        }
        let mut found = Vec::new();
        document.objects.values().for_each(|object| visit(object, &mut found));
        found
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn removes_every_token_under_the_areas() {
        let bytes = redacted();
        let document = lopdf::Document::load_mem(&bytes).unwrap();
        let everything = all_bytes(&document);
        for token in ["ALPHATOKEN", "BRAVOTOKEN", "GOLF", "DELTATOKEN", "ECHOTOKN", "FOXTROTTOKEN"] {
            assert!(!everything.iter().any(|bytes| contains(bytes, token.as_bytes())), "{token} is still in the file");
        }
        for token in ["CHARLIETOKEN", "HOTELTOKEN", "KEEPNOTE"] {
            assert!(everything.iter().any(|bytes| contains(bytes, token.as_bytes())), "{token} was removed");
        }

        let first = document.extract_text(&[1]).unwrap();
        for word in ["Alice", "Smith", "Code", "end", "Hidden", "word"] {
            assert!(first.contains(word), "{word} is missing from {first:?}");
        }
        assert!(!first.contains("CHARLIE"), "the form copy on page 1 is redacted");
        let second = document.extract_text(&[2]).unwrap();
        // lopdf does not read text inside forms; PDFium does (recto-render tests).
        assert!(second.contains("HOTELTOKEN"), "page 2 is untouched: {second:?}");

        let pages: Vec<ObjectId> = document.get_pages().into_values().collect();
        let page = document.get_dictionary(pages[0]).unwrap();
        assert_eq!(page.get(b"Annots").unwrap().as_array().unwrap().len(), 1, "only the annotation outside the areas is left");
        // The image copy: left half cleared, right half kept.
        let resources = page.get(b"Resources").unwrap().as_dict().unwrap();
        let images: Vec<Vec<u8>> = resources
            .get(b"XObject")
            .unwrap()
            .as_dict()
            .unwrap()
            .iter()
            .filter_map(|(_, value)| document.get_object(value.as_reference().ok()?).ok()?.as_stream().ok())
            .filter(|stream| stream.dict.get(b"Subtype").unwrap().as_name().unwrap() == b"Image")
            .map(|stream| stream.decompressed_content_with_limit(crate::content::MAX_STREAM_BYTES).unwrap())
            .collect();
        assert_eq!(images.len(), 1);
        for row in images[0].chunks(60) {
            assert!(row[..30].iter().all(|&byte| byte == 0) && row[30..].iter().all(|&byte| byte == 0x80));
        }
    }

    #[test]
    fn keeps_text_in_place_and_covers_the_areas() {
        let bytes = redacted();
        let document = lopdf::Document::load_mem(&bytes).unwrap();
        let page = *document.get_pages().values().next().unwrap();
        let content = String::from_utf8_lossy(&crate::content::page_content(&document, page)).into_owned();
        // ALPHATOKEN replaced by an adjustment of its width: 10 glyphs of Helvetica.
        let width = text_width("ALPHATOKEN", 1000.0, Font::Helvetica);
        assert!(content.contains(&format!("[(Name: Alice ) {}( Smith)] TJ", -width)), "{content}");
        // Courier: the kerning around BRAVOTOKEN (-200, +50) is merged with its 10 × 600.
        assert!(content.contains("[(Code ) -6150( end)] TJ"), "{content}");
        for area in areas() {
            assert!(content.contains(&format!("{} {} {} {} re f", area.origin.x, area.origin.y, area.width, area.height)));
        }
    }


    #[test]
    fn erases_in_white_and_deletes_images() {
        let mut document = Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/redact.pdf")).unwrap();
        let page = PageId { source: 0, index: 0 };
        document.apply(Op::Erase { page, area: areas()[0] });
        document.apply(Op::DeleteImage { page, at: Point { x: 400.0, y: 550.0 } });
        // Nothing there: nothing happens.
        document.apply(Op::DeleteImage { page, at: Point { x: 10.0, y: 10.0 } });
        let bytes = document.build().unwrap();
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let first = *reopened.get_pages().values().next().unwrap();
        let content = String::from_utf8_lossy(&reopened.get_page_content(first)).into_owned();
        assert!(content.contains("q 1 g") && !content.contains("q 0 g"), "painted white: {content}");
        assert!(!content.contains("/Im1 Do") && content.contains("/Fm1 Do"), "{content}");
        assert!(!reopened.extract_text(&[1]).unwrap().contains("ALPHATOKEN"));
    }
    #[test]
    fn unknown_code_lengths_remove_the_whole_string() {
        let mut fonts = Dictionary::new();
        let mut font = Dictionary::new();
        font.set("Subtype", Object::Name(b"Type0".to_vec()));
        font.set("Encoding", Object::Name(b"UniJIS-UTF8-H".to_vec()));
        fonts.set("F1", Object::Dictionary(font));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let document = lopdf::Document::with_version("1.7");
        let mut redactor = Redactor { document: &document, areas: &[area(150.0, 95.0, 160.0, 110.0)], created: Vec::new(), next_id: 1, serial: 0, originals: Vec::new(), forms_left: MAX_FORMS };
        let operations = Content::decode(b"BT /F1 10 Tf 100 100 Td (abcdefgh) Tj (ij) Tj ET BT /F1 10 Tf 100 300 Td (kept) Tj ET").unwrap().operations;
        let (out, _) = redactor.run(operations, resources, Matrix::IDENTITY, 0);
        let shown: Vec<String> = out
            .iter()
            .filter(|operation| operation.operator == "Tj" || operation.operator == "TJ")
            .map(|operation| format!("{:?}", operation.operands))
            .collect();
        // Both strings of the first line may reach the area (8 bytes × 1.5 em × 10 pt); the
        // second line is far from it.
        assert_eq!(shown.len(), 3);
        assert!(shown[0].contains("[]") && shown[1].contains("[]") && shown[2].contains("kept"), "{shown:?}");
    }
}
