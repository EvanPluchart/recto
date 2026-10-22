//! Editing existing text (beta). A page's text is read as runs: show operations of one text
//! object, in one font and size, on one line. An edit replaces a run's text in place: the new
//! text starts where the run started, in the run's font when every character can be shown with
//! it (`fonts::Charset`), or else in a fallback font the app chose and added to the document
//! (`Document::add_font`), embedded as a subset (`embed.rs`). Nothing reflows: longer text runs
//! over what follows it on the line.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::content::{FontMetrics, Matrix, bounds_of, dictionary, resolve};
use crate::fonts::{Charset, Code};
use crate::geom::{Point, Rect};

/// A run of text on a page, as found in the page's own content.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    /// Indices of the first and last show operations in the page's content.
    pub first: usize,
    pub last: usize,
    /// The text shown, with the edits made so far (`Document::text_runs`).
    pub text: String,
    /// The text in the page's own content: what `TextEdit::original` refers to.
    pub original: String,
    /// User space.
    pub bounds: Rect,
    pub size: f32,
    /// Whether the text was decoded reliably and can be edited.
    pub editable: bool,
    pub style: FontStyle,
}

/// What the run's font looks like, to pick a fallback font.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontStyle {
    /// `/BaseFont` without the subset prefix: `Arial-BoldMT`.
    pub name: String,
    pub bold: bool,
    pub italic: bool,
    pub serif: bool,
    pub monospace: bool,
}

/// An edit of one run: what it showed when it was edited, and the new text.
#[derive(Clone, Debug, PartialEq)]
pub struct TextEdit {
    pub first: usize,
    pub last: usize,
    pub original: String,
    pub text: String,
    /// A font added with `Document::add_font`, when the run's own cannot show the text.
    pub font: Option<usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("the text to edit has changed in the page")]
    Mismatch,
    #[error("the font of this text cannot show: {0}")]
    Unencodable(String),
    #[error("the replacement font cannot show: {0}")]
    MissingGlyphs(String),
    #[error("cannot embed the replacement font: {0}")]
    Embed(String),
}

/// TJ adjustment, in thousandths of an em, read as a space between words.
const SPACE_GAP: f32 = -200.0;
/// Width of a space written as an adjustment when the font has no space.
const SPACE_ADJUSTMENT: f32 = -300.0;

/// A font of the page's resources, read once.
pub(crate) struct PageFont {
    pub charset: Charset,
    metrics: FontMetrics,
    style: FontStyle,
    type3: bool,
}

#[derive(Clone)]
struct State {
    ctm: Matrix,
    font: Vec<u8>,
    size: f32,
    char_spacing: f32,
    word_spacing: f32,
    scale: f32,
    leading: f32,
    rise: f32,
}

/// One show operation, located.
struct Shown {
    index: usize,
    /// The text object (BT…ET) it belongs to.
    block: usize,
    font: Vec<u8>,
    size: f32,
    text: Option<String>,
    codes: Vec<Code>,
    bounds: Rect,
    /// Text space to user space at its start, and where it ends along its line.
    line: Matrix,
    end: f32,
}

fn style_of(document: &lopdf::Document, font: &Dictionary) -> FontStyle {
    let base = font.get(b"BaseFont").ok().and_then(|value| resolve(document, value).as_name().ok()).unwrap_or_default();
    let base = String::from_utf8_lossy(base).into_owned();
    // Subset fonts are named `ABCDEF+Name`.
    let name = match base.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest.to_owned(),
        _ => base,
    };
    let descriptor = font
        .get(b"FontDescriptor")
        .ok()
        .and_then(|value| dictionary(document, value))
        .or_else(|| {
            let descendant = font.get(b"DescendantFonts").ok().map(|value| resolve(document, value))?.as_array().ok()?.first()?;
            dictionary(document, descendant)?.get(b"FontDescriptor").ok().and_then(|value| dictionary(document, value))
        });
    let flags = descriptor.and_then(|descriptor| descriptor.get(b"Flags").ok()).and_then(|value| value.as_i64().ok()).unwrap_or(0);
    let weight = descriptor.and_then(|descriptor| descriptor.get(b"FontWeight").ok()).and_then(|value| value.as_float().ok()).unwrap_or(400.0);
    let angle = descriptor.and_then(|descriptor| descriptor.get(b"ItalicAngle").ok()).and_then(|value| value.as_float().ok()).unwrap_or(0.0);
    let lower = name.to_ascii_lowercase();
    FontStyle {
        bold: weight >= 600.0 || flags & (1 << 18) != 0 || lower.contains("bold") || lower.contains("black") || lower.contains("heavy"),
        italic: angle != 0.0 || flags & 64 != 0 || lower.contains("italic") || lower.contains("oblique"),
        serif: flags & 2 != 0 || lower.contains("times") || lower.contains("serif") && !lower.contains("sans") || lower.contains("georgia") || lower.contains("garamond"),
        monospace: flags & 1 != 0 || lower.contains("courier") || lower.contains("mono") || lower.contains("consolas"),
        name,
    }
}

/// The page's content operations and the page's fonts by resource name.
pub(crate) fn read_page(document: &lopdf::Document, page: ObjectId) -> (Vec<Operation>, HashMap<Vec<u8>, Rc<PageFont>>) {
    let operations = Content::decode(&document.get_page_content(page)).map(|content| content.operations).unwrap_or_default();
    let mut fonts = HashMap::new();
    let resources = document.get_dictionary(page).ok().and_then(|page| page.get(b"Resources").ok()).and_then(|value| dictionary(document, value));
    if let Some(entries) = resources.and_then(|resources| resources.get(b"Font").ok()).and_then(|value| dictionary(document, value)) {
        for (name, value) in entries.iter() {
            let Some(font) = dictionary(document, value) else { continue };
            let type3 = font.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Type3");
            let page_font = PageFont {
                charset: Charset::load(document, font),
                metrics: FontMetrics::load(document, font),
                style: style_of(document, font),
                type3,
            };
            fonts.insert(name.clone(), Rc::new(page_font));
        }
    }
    (operations, fonts)
}

/// Locates every show operation.
fn locate(operations: &[Operation], fonts: &HashMap<Vec<u8>, Rc<PageFont>>) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut stack: Vec<State> = Vec::new();
    let mut state = State { ctm: Matrix::IDENTITY, font: Vec::new(), size: 0.0, char_spacing: 0.0, word_spacing: 0.0, scale: 1.0, leading: 0.0, rise: 0.0 };
    let (mut matrix, mut line) = (Matrix::IDENTITY, Matrix::IDENTITY);
    let mut block = 0;
    for (index, operation) in operations.iter().enumerate() {
        let operands = &operation.operands;
        let number = |i: usize| operands.get(i).and_then(|value| value.as_float().ok()).unwrap_or(0.0);
        let move_line = |x: f32, y: f32, line: &mut Matrix, matrix: &mut Matrix| {
            *line = Matrix::translate(x, y).then(*line);
            *matrix = *line;
        };
        match operation.operator.as_str() {
            "q" => stack.push(state.clone()),
            "Q" => state = stack.pop().unwrap_or(state),
            "cm" => {
                if let Some(m) = Matrix::from_operands(operands) {
                    state.ctm = m.then(state.ctm);
                }
            }
            "BT" => {
                block += 1;
                matrix = Matrix::IDENTITY;
                line = Matrix::IDENTITY;
            }
            "Tc" => state.char_spacing = number(0),
            "Tw" => state.word_spacing = number(0),
            "Tz" => state.scale = number(0) / 100.0,
            "TL" => state.leading = number(0),
            "Ts" => state.rise = number(0),
            "Tf" => {
                state.font = operands.first().and_then(|value| value.as_name().ok()).unwrap_or_default().to_vec();
                state.size = number(1);
            }
            "Td" => move_line(number(0), number(1), &mut line, &mut matrix),
            "TD" => {
                state.leading = -number(1);
                move_line(number(0), number(1), &mut line, &mut matrix);
            }
            "Tm" => {
                if let Some(m) = Matrix::from_operands(operands) {
                    line = m;
                    matrix = m;
                }
            }
            "T*" => move_line(0.0, -state.leading, &mut line, &mut matrix),
            "Tj" | "TJ" | "'" | "\"" => {
                if operation.operator == "\"" {
                    state.word_spacing = number(0);
                    state.char_spacing = number(1);
                }
                if operation.operator == "'" || operation.operator == "\"" {
                    move_line(0.0, -state.leading, &mut line, &mut matrix);
                }
                let items: Vec<&Object> = match operands.last() {
                    Some(Object::Array(items)) if operation.operator == "TJ" => items.iter().collect(),
                    Some(string @ Object::String(..)) => vec![string],
                    _ => Vec::new(),
                };
                let Some(font) = fonts.get(&state.font) else { continue };
                let (size, scale) = (state.size, state.scale);
                let mut x = 0.0f32;
                let mut text = Some(String::new());
                let mut codes = Vec::new();
                for item in items {
                    match item {
                        Object::String(bytes, _) => {
                            let split = font.charset.split(bytes);
                            if split.is_none() {
                                text = None;
                            }
                            for code in split.unwrap_or_default() {
                                match (font.charset.text(&code), &mut text) {
                                    (Some(value), Some(text)) => text.push_str(value),
                                    _ => text = None,
                                }
                                let number = code.iter().fold(0u32, |n, &b| n << 8 | u32::from(b));
                                let (low, high) = font.metrics.width(number);
                                let spacing = state.char_spacing + if code == [32] { state.word_spacing } else { 0.0 };
                                x += ((low + high) / 2.0 * size + spacing) * scale;
                                codes.push(code);
                            }
                        }
                        other => {
                            let adjustment = other.as_float().unwrap_or(0.0);
                            x -= adjustment / 1000.0 * size * scale;
                            if adjustment <= SPACE_GAP
                                && let Some(text) = &mut text
                                && !text.is_empty()
                                && !text.ends_with(' ')
                            {
                                text.push(' ');
                            }
                        }
                    }
                }
                let to_user = matrix.then(state.ctm);
                let (low_y, high_y) = (state.rise + font.metrics.descent * size, state.rise + font.metrics.ascent * size);
                let corners = [(0.0, low_y), (x, low_y), (0.0, high_y), (x, high_y)];
                let bounds = bounds_of(corners.map(|(cx, cy)| to_user.apply(Point { x: cx, y: cy })));
                shown.push(Shown { index, block, font: state.font.clone(), size, text, codes, bounds, line: to_user, end: x });
                matrix = Matrix::translate(x, 0.0).then(matrix);
            }
            _ => {}
        }
    }
    shown
}

/// Operators that can sit inside a run without ending it.
fn inside_run(operator: &str) -> bool {
    matches!(operator, "Td" | "TD" | "Tm" | "T*" | "Tc" | "Tw" | "Tz" | "TL" | "Ts" | "Tj" | "TJ" | "'" | "\"")
}

/// Groups located show operations into runs.
fn group(operations: &[Operation], shown: &[Shown], fonts: &HashMap<Vec<u8>, Rc<PageFont>>) -> Vec<TextRun> {
    let mut runs: Vec<TextRun> = Vec::new();
    // The run being built: its first show, and its end along the first show's line.
    let mut open: Option<(usize, f32)> = None;
    for (n, show) in shown.iter().enumerate() {
        let joins = open.and_then(|(first, end)| {
            let head = &shown[first];
            let previous = &shown[n - 1];
            let between_ok = operations[previous.index + 1..show.index].iter().all(|operation| inside_run(&operation.operator));
            let same = head.block == show.block && head.font == show.font && head.size == show.size && between_ok;
            // Where this show starts, in the head's text space.
            let start = head.line.invert()?.apply(show.line.apply(Point { x: 0.0, y: 0.0 }));
            let size = show.size.abs().max(0.1);
            let on_line = start.y.abs() < 0.2 * size;
            let close = start.x > end - 0.5 * size && start.x < end + 2.0 * size;
            (same && on_line && close).then_some((first, end, start.x))
        });
        match joins {
            Some((first, previous_end, start)) => {
                open = Some((first, start + show.end));
                let run = runs.last_mut().expect("an open run exists");
                run.last = show.index;
                run.bounds = union(run.bounds, show.bounds);
                match (&mut run.editable, &show.text) {
                    (true, Some(text)) => {
                        // A visible gap between the shows reads as a space.
                        if start - previous_end > 0.15 * show.size.abs() && !run.text.ends_with(' ') && !text.starts_with(' ') {
                            run.text.push(' ');
                        }
                        run.text.push_str(text);
                    }
                    _ => run.editable = false,
                }
            }
            None => {
                let font = fonts.get(&show.font);
                let editable = show.text.is_some()
                    && font.is_some_and(|font| font.charset.editable && !font.type3 && !font.metrics.vertical);
                let text = show.text.clone().unwrap_or_default();
                let editable = editable && !text.contains('\u{fffd}') && !text.chars().any(|c| c.is_control());
                runs.push(TextRun {
                    first: show.index,
                    last: show.index,
                    original: text.clone(),
                    text,
                    bounds: show.bounds,
                    size: show.size,
                    editable,
                    style: font.map(|font| font.style.clone()).unwrap_or_default(),
                });
                open = Some((n, show.end));
            }
        }
    }
    for run in &mut runs {
        let trimmed = run.text.trim().to_owned();
        run.editable &= !trimmed.is_empty();
        run.original = trimmed.clone();
        run.text = trimmed;
    }
    runs
}

fn union(a: Rect, b: Rect) -> Rect {
    let (x0, y0) = (a.origin.x.min(b.origin.x), a.origin.y.min(b.origin.y));
    let x1 = (a.origin.x + a.width).max(b.origin.x + b.width);
    let y1 = (a.origin.y + a.height).max(b.origin.y + b.height);
    Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 }
}

/// The text runs of `page`, in content order.
pub fn runs(document: &lopdf::Document, page: ObjectId) -> Vec<TextRun> {
    let (operations, fonts) = read_page(document, page);
    let shown = locate(&operations, &fonts);
    group(&operations, &shown, &fonts)
}

/// A fallback font embedded for this build: its resource and the codes of its characters.
pub(crate) struct Embedded {
    pub font: ObjectId,
    pub codes: HashMap<char, u16>,
}

/// Applies `edits` to `page`. `embedded` holds the fallback fonts, by `TextEdit::font`.
pub(crate) fn apply(document: &mut lopdf::Document, page: ObjectId, edits: &[&TextEdit], embedded: &HashMap<usize, Embedded>) -> Result<(), EditError> {
    let (mut operations, fonts) = read_page(document, page);
    let shown = locate(&operations, &fonts);
    let found = group(&operations, &shown, &fonts);
    let used: HashMap<Vec<u8>, HashSet<Code>> = shown.iter().fold(HashMap::new(), |mut used, show| {
        used.entry(show.font.clone()).or_default().extend(show.codes.iter().cloned());
        used
    });

    // Latest edit of each run wins.
    let mut latest: Vec<&TextEdit> = Vec::new();
    for edit in edits {
        latest.retain(|other| (other.first, other.last) != (edit.first, edit.last));
        latest.push(edit);
    }
    let mut resources = document
        .get_dictionary(page)
        .ok()
        .and_then(|page| page.get(b"Resources").ok())
        .and_then(|value| dictionary(document, value))
        .cloned()
        .unwrap_or_default();
    let mut font_names = resources.get(b"Font").ok().and_then(|value| dictionary(document, value)).cloned().unwrap_or_default();
    let mut fallback_names: HashMap<usize, Vec<u8>> = HashMap::new();

    // Replacements by operation index; later indices of a run become their line moves only.
    let mut replace: HashMap<usize, Vec<Operation>> = HashMap::new();
    for edit in latest {
        let run = found.iter().find(|run| run.first == edit.first && run.last == edit.last).ok_or(EditError::Mismatch)?;
        if !run.editable || run.original != edit.original {
            return Err(EditError::Mismatch);
        }
        let head = shown.iter().find(|show| show.index == edit.first).ok_or(EditError::Mismatch)?;
        let font = fonts.get(&head.font).ok_or(EditError::Mismatch)?;
        let mut items: Vec<Object> = Vec::new();
        let mut font_switch: Option<Vec<u8>> = None;
        match edit.font {
            None => {
                let none = HashSet::new();
                let used = used.get(&head.font).unwrap_or(&none);
                // Spaces the font cannot show become gaps.
                for (n, word) in edit.text.split(' ').enumerate() {
                    if n > 0 {
                        match font.charset.encode(" ", used) {
                            Some(space) => items.push(Object::String(space, lopdf::StringFormat::Hexadecimal)),
                            None => items.push(Object::Real(SPACE_ADJUSTMENT)),
                        }
                    }
                    if word.is_empty() {
                        continue;
                    }
                    let bytes = font.charset.encode(word, used).ok_or_else(|| EditError::Unencodable(word.to_owned()))?;
                    items.push(Object::String(bytes, lopdf::StringFormat::Hexadecimal));
                }
            }
            Some(id) => {
                let fallback = embedded.get(&id).ok_or_else(|| EditError::Embed("font not loaded".into()))?;
                let mut bytes = Vec::new();
                for c in edit.text.chars() {
                    let code = fallback.codes.get(&c).ok_or_else(|| EditError::MissingGlyphs(c.to_string()))?;
                    bytes.extend_from_slice(&code.to_be_bytes());
                }
                items.push(Object::String(bytes, lopdf::StringFormat::Hexadecimal));
                let name = fallback_names
                    .entry(id)
                    .or_insert_with(|| {
                        let name = crate::stamp::fresh_name(&font_names, "RectoEdit");
                        font_names.set(name.clone(), Object::Reference(fallback.font));
                        name
                    })
                    .clone();
                font_switch = Some(name);
            }
        }

        let original = &operations[edit.first];
        let mut first_ops = line_moves(original);
        let size = Object::Real(head.size);
        if let Some(name) = &font_switch {
            first_ops.push(Operation::new("Tf", vec![Object::Name(name.clone()), size.clone()]));
        }
        first_ops.push(Operation::new("TJ", vec![Object::Array(items)]));
        if font_switch.is_some() {
            first_ops.push(Operation::new("Tf", vec![Object::Name(head.font.clone()), size]));
        }
        replace.insert(edit.first, first_ops);
        for show in shown.iter().filter(|show| show.index > edit.first && show.index <= edit.last) {
            replace.insert(show.index, line_moves(&operations[show.index]));
        }
    }

    let mut out = Vec::with_capacity(operations.len());
    for (index, operation) in operations.drain(..).enumerate() {
        match replace.remove(&index) {
            Some(new) => out.extend(new),
            None => out.push(operation),
        }
    }
    let content = Content { operations: out }.encode().map_err(|error| EditError::Embed(error.to_string()))?;
    let mut stream = Stream::new(Dictionary::new(), content);
    let _ = stream.compress();
    let contents = document.add_object(stream);
    if !fallback_names.is_empty() {
        resources.set("Font", Object::Dictionary(font_names));
    }
    let page_dictionary = document.get_dictionary_mut(page).map_err(|error| EditError::Embed(error.to_string()))?;
    page_dictionary.set("Contents", Object::Reference(contents));
    page_dictionary.set("Resources", Object::Dictionary(resources));
    Ok(())
}

/// What a show operation does besides showing: `'` moves to the next line, `"` also sets the
/// word and character spacing.
fn line_moves(operation: &Operation) -> Vec<Operation> {
    match operation.operator.as_str() {
        "'" => vec![Operation::new("T*", vec![])],
        "\"" => {
            let operands = &operation.operands;
            vec![
                Operation::new("Tw", vec![operands.first().cloned().unwrap_or(Object::Integer(0))]),
                Operation::new("Tc", vec![operands.get(1).cloned().unwrap_or(Object::Integer(0))]),
                Operation::new("T*", vec![]),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::embed::FontFile;
    use crate::op::Op;
    use crate::{BuildError, Document, PageId};

    const PAGE: PageId = PageId { source: 0, index: 0 };

    fn fixture() -> Document {
        Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/edit.pdf")).unwrap()
    }

    fn edit(document: &mut Document, original: &str, text: &str, font: Option<usize>) {
        let run = document.text_runs(PAGE).unwrap().into_iter().find(|run| run.original == original).expect("the run exists");
        let edit = TextEdit { first: run.first, last: run.last, original: run.original, text: text.into(), font };
        document.apply(Op::EditText { page: PAGE, edit });
    }

    fn text(bytes: &[u8]) -> String {
        lopdf::Document::load_mem(bytes).unwrap().extract_text(&[1]).unwrap()
    }

    #[test]
    fn finds_the_runs() {
        let runs = fixture().text_runs(PAGE).unwrap();
        let texts: Vec<&str> = runs.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(texts, ["Hello world from Recto", "Code 1234 end", "Embedded font line", "Type zero text", "First", "Second line", "Split run"]);
        assert!(runs.iter().all(|run| run.editable));
        // Helvetica 14 at (56, 760): the run spans its text's width.
        let hello = &runs[0];
        let width = crate::annot::text_width("Hello world from Recto", 14.0, crate::annot::Font::Helvetica);
        assert!((hello.bounds.origin.x - 56.0).abs() < 0.01 && (hello.bounds.width - width).abs() < 0.5, "{:?}", hello.bounds);
        assert!(hello.bounds.origin.y < 760.0 && hello.bounds.origin.y + hello.bounds.height > 770.0);
        assert_eq!((runs[3].style.name.as_str(), runs[0].style.name.as_str()), ("RectoTest", "Helvetica"));
        assert!(runs[1].style.monospace && !runs[0].style.serif);
    }

    #[test]
    fn rewrites_runs_in_their_own_fonts() {
        let mut document = fixture();
        edit(&mut document, "Hello world from Recto", "Bonjour à tous", None);
        edit(&mut document, "Code 1234 end", "Code 9876 fin", None);
        edit(&mut document, "Embedded font line", "Ligne intégrée", None);
        edit(&mut document, "Type zero text", "Type text", None);
        edit(&mut document, "Second line", "Deuxième ligne", None);
        edit(&mut document, "Split run", "Joined", None);
        let bytes = document.build().unwrap();
        let shown = text(&bytes);
        // (lopdf does not read this Type0 font through its ToUnicode CMap: checked with the runs below.)
        for new in ["Bonjour à tous", "Code 9876 fin", "Ligne intégrée", "Deuxième ligne", "Joined", "First"] {
            assert!(shown.contains(new), "{new} missing from {shown:?}");
        }
        for old in ["Hello", "1234", "Embedded", "Second", "Split"] {
            assert!(!shown.contains(old), "{old} still in {shown:?}");
        }
        // No font was added: every run kept its own.
        let reopened = Document::from_bytes(bytes).unwrap();
        let runs = reopened.text_runs(PAGE).unwrap();
        assert_eq!(runs[3].text, "Type text");
        assert_eq!(runs[4].text, "First");
        assert_eq!(runs[5].text, "Deuxième ligne");
        // The ' operator still moved to the next line: same place as before.
        let before = fixture().text_runs(PAGE).unwrap();
        assert!((runs[5].bounds.origin.y - before[5].bounds.origin.y).abs() < 0.01);
        // And the edits show through `text_runs` before building.
        assert_eq!(document.text_runs(PAGE).unwrap()[0].text, "Bonjour à tous");
    }

    #[test]
    fn falls_back_to_an_added_font() {
        let mut document = fixture();
        // The Type0 font only maps the characters it used: Z, b and a are not among them.
        edit(&mut document, "Type zero text", "Zebra", None);
        assert!(matches!(document.build(), Err(BuildError::Edit(EditError::Unencodable(_)))));
        document.undo();
        let font = document.add_font(FontFile::new(crate::testfont::font("RectoFallback"), 0).unwrap());
        edit(&mut document, "Type zero text", "Zebra €", Some(font));
        edit(&mut document, "Hello world from Recto", "日本", None);
        assert!(document.build().is_err(), "Helvetica cannot show Japanese");
        document.undo();
        let bytes = document.build().unwrap();
        assert!(text(&bytes).contains("Zebra €"), "{}", text(&bytes));
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let embedded = reopened.objects.values().filter_map(|object| object.as_dict().ok()).any(|dict| {
            dict.get(b"BaseFont").ok().and_then(|value| value.as_name().ok()).is_some_and(|name| name.ends_with(b"+RectoFallback"))
                && dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Type0")
        });
        assert!(embedded, "the fallback font is embedded as a Type0 font");
    }

    #[test]
    fn refuses_stale_edits() {
        let mut document = fixture();
        let run = document.text_runs(PAGE).unwrap().remove(0);
        let edit = TextEdit { first: run.first, last: run.last, original: "something else".into(), text: "x".into(), font: None };
        document.apply(Op::EditText { page: PAGE, edit });
        assert!(matches!(document.build(), Err(BuildError::Edit(EditError::Mismatch))));
    }
}
