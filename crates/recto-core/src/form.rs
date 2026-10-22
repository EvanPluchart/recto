//! AcroForm fields (decision 0003): read with `lopdf`, filled through `Op::FillField`, written
//! with generated appearances, and optionally flattened into the page content.

use std::collections::HashMap;

use lopdf::{Dictionary, Object, ObjectId, Stream};

use crate::PageId;
use crate::annot::text_string;
use crate::geom::{Point, Rect};

#[derive(Clone, Debug, PartialEq)]
pub enum FieldKind {
    Text,
    CheckBox,
    Radio,
    /// A list or combo box; `editable` combo boxes also accept other values.
    Choice { options: Vec<String>, editable: bool },
    /// Push buttons, signature fields: shown, not filled.
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Widget {
    /// Page of the opened file the widget sits on.
    pub page: PageId,
    pub rect: Rect,
    /// For check boxes and radio buttons: the appearance state that means "on" for this widget.
    pub on_state: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FormField {
    /// Fully qualified name (`parent.child`).
    pub name: String,
    pub kind: FieldKind,
    /// Text or choice value; for check boxes and radio buttons, the "on" state chosen, or "".
    pub value: String,
    pub read_only: bool,
    pub widgets: Vec<Widget>,
}

const READ_ONLY: i64 = 1;
const RADIO: i64 = 1 << 15;
const PUSH_BUTTON: i64 = 1 << 16;
const COMBO: i64 = 1 << 17;
const EDIT: i64 = 1 << 18;
const MAX_DEPTH: usize = 32;

fn resolve<'a>(document: &'a lopdf::Document, object: &'a Object) -> &'a Object {
    match object {
        Object::Reference(id) => document.get_object(*id).unwrap_or(object),
        other => other,
    }
}

fn dictionary<'a>(document: &'a lopdf::Document, object: &'a Object) -> Option<&'a Dictionary> {
    resolve(document, object).as_dict().ok()
}

/// A text string object as Rust text (PDFDocEncoding or UTF-16BE).
fn decode(object: &Object) -> Option<String> {
    match object {
        Object::String(bytes, _) if bytes.starts_with(&[0xFE, 0xFF]) => {
            let units: Vec<u16> = bytes[2..].as_chunks::<2>().0.iter().map(|pair| u16::from_be_bytes(*pair)).collect();
            Some(String::from_utf16_lossy(&units))
        }
        Object::String(bytes, _) => Some(bytes.iter().map(|&b| b as char).collect()),
        Object::Name(name) => Some(String::from_utf8_lossy(name).into_owned()),
        _ => None,
    }
}

fn rect_of(document: &lopdf::Document, object: &Object) -> Option<Rect> {
    let numbers: Vec<f32> =
        resolve(document, object).as_array().ok()?.iter().filter_map(|n| resolve(document, n).as_float().ok()).collect();
    let [x0, y0, x1, y1] = numbers[..] else { return None };
    Some(Rect { origin: Point { x: x0.min(x1), y: y0.min(y1) }, width: (x1 - x0).abs(), height: (y1 - y0).abs() })
}

/// The "on" appearance state of a check box or radio widget: any `/AP /N` key but `Off`.
fn on_state(document: &lopdf::Document, widget: &Dictionary) -> Option<String> {
    let appearance = dictionary(document, widget.get(b"AP").ok()?)?;
    let normal = dictionary(document, appearance.get(b"N").ok()?)?;
    normal.iter().map(|(key, _)| key).find(|key| key.as_slice() != b"Off").map(|key| String::from_utf8_lossy(key).into_owned())
}

/// A field, its widgets and the ids that make it up, found while walking the field tree.
struct Found {
    field: FormField,
    /// The field dictionary holding `/V`.
    id: ObjectId,
    widget_ids: Vec<ObjectId>,
}

/// Walks the AcroForm of `document`; `pages` maps page object ids to page indices.
fn walk(document: &lopdf::Document, pages: &HashMap<ObjectId, u32>, source: u32) -> Vec<Found> {
    let Ok(catalog) = document.catalog() else { return Vec::new() };
    let Some(form) = catalog.get(b"AcroForm").ok().and_then(|form| dictionary(document, form)) else { return Vec::new() };
    let Some(fields) = form.get(b"Fields").ok().and_then(|fields| resolve(document, fields).as_array().ok()) else {
        return Vec::new();
    };
    // Widgets are found through the pages, so a widget's page is known even without /P.
    let mut widget_pages = HashMap::new();
    for (&page_id, &index) in pages {
        let Ok(page) = document.get_dictionary(page_id) else { continue };
        if let Some(annots) = page.get(b"Annots").ok().and_then(|a| resolve(document, a).as_array().ok()) {
            for annot in annots {
                if let Ok(id) = annot.as_reference() {
                    widget_pages.insert(id, index);
                }
            }
        }
    }

    #[derive(Clone, Default)]
    struct Inherited {
        name: String,
        kind: Option<Vec<u8>>,
        flags: i64,
    }
    let mut found = Vec::new();
    let mut stack: Vec<(ObjectId, Inherited, usize)> =
        fields.iter().filter_map(|f| f.as_reference().ok()).map(|id| (id, Inherited::default(), 0)).rev().collect();
    while let Some((id, parent, depth)) = stack.pop() {
        let Ok(node) = document.get_dictionary(id) else { continue };
        let mut own = parent.clone();
        if let Some(part) = node.get(b"T").ok().and_then(decode) {
            own.name = if own.name.is_empty() { part } else { format!("{}.{part}", own.name) };
        }
        if let Ok(kind) = node.get(b"FT").and_then(Object::as_name) {
            own.kind = Some(kind.to_vec());
        }
        if let Ok(flags) = node.get(b"Ff").and_then(Object::as_i64) {
            own.flags = flags;
        }
        let kids: Vec<ObjectId> = node
            .get(b"Kids")
            .ok()
            .and_then(|kids| resolve(document, kids).as_array().ok())
            .map(|kids| kids.iter().filter_map(|k| k.as_reference().ok()).collect())
            .unwrap_or_default();
        // Kids with their own names are child fields; nameless kids are this field's widgets.
        let child_fields: Vec<ObjectId> =
            kids.iter().copied().filter(|kid| document.get_dictionary(*kid).is_ok_and(|k| k.has(b"T"))).collect();
        if !child_fields.is_empty() && depth < MAX_DEPTH {
            for kid in child_fields.into_iter().rev() {
                stack.push((kid, own.clone(), depth + 1));
            }
            continue;
        }
        let widget_ids: Vec<ObjectId> = if kids.is_empty() { vec![id] } else { kids };
        let widgets: Vec<Widget> = widget_ids
            .iter()
            .filter_map(|&widget| {
                let dictionary = document.get_dictionary(widget).ok()?;
                let index = widget_pages.get(&widget).copied().or_else(|| {
                    dictionary.get(b"P").ok().and_then(|p| p.as_reference().ok()).and_then(|p| pages.get(&p).copied())
                })?;
                Some(Widget {
                    page: PageId { source, index },
                    rect: rect_of(document, dictionary.get(b"Rect").ok()?)?,
                    on_state: on_state(document, dictionary),
                })
            })
            .collect();
        let kind = match own.kind.as_deref() {
            Some(b"Tx") => FieldKind::Text,
            Some(b"Btn") if own.flags & PUSH_BUTTON != 0 => FieldKind::Other,
            Some(b"Btn") if own.flags & RADIO != 0 => FieldKind::Radio,
            Some(b"Btn") => FieldKind::CheckBox,
            Some(b"Ch") => FieldKind::Choice {
                options: node
                    .get(b"Opt")
                    .ok()
                    .and_then(|options| resolve(document, options).as_array().ok())
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(|option| match resolve(document, option) {
                                // [export display] pairs show the display text.
                                Object::Array(pair) => pair.get(1).and_then(decode),
                                other => decode(other),
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
                editable: own.flags & COMBO != 0 && own.flags & EDIT != 0,
            },
            _ => FieldKind::Other,
        };
        let value = node.get(b"V").ok().map(|v| resolve(document, v)).and_then(decode).unwrap_or_default();
        let value = if matches!(kind, FieldKind::CheckBox | FieldKind::Radio) && value == "Off" { String::new() } else { value };
        found.push(Found {
            field: FormField { name: own.name, kind, value, read_only: own.flags & READ_ONLY != 0, widgets },
            id,
            widget_ids,
        });
    }
    found
}

/// The fields of `document`, the opened file being source `source`.
pub fn fields(document: &lopdf::Document, source: u32) -> Vec<FormField> {
    let pages: HashMap<ObjectId, u32> = document.get_pages().into_iter().map(|(number, id)| (id, number - 1)).collect();
    walk(document, &pages, source).into_iter().map(|found| found.field).collect()
}

/// Font size from a default appearance string like `/Helv 12 Tf 0 g`; 0 means "auto".
fn font_size(appearance: &str) -> f32 {
    let parts: Vec<&str> = appearance.split_whitespace().collect();
    parts.windows(2).find(|pair| pair[1] == "Tf").and_then(|pair| pair[0].parse().ok()).unwrap_or(0.0)
}

/// Bytes for Helvetica with WinAnsiEncoding, with PDF string escapes.
fn escaped(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    for c in text.chars() {
        let byte = if (c as u32) < 256 { c as u32 as u8 } else { b'?' };
        if matches!(byte, b'(' | b')' | b'\\') {
            bytes.push(b'\\');
        }
        bytes.push(byte);
    }
    bytes
}

/// Writes `values` (field name → value) into `document`: `/V`, `/AS`, and generated appearances
/// for text and choice widgets.
pub fn fill(document: &mut lopdf::Document, values: &HashMap<String, String>) {
    if values.is_empty() {
        return;
    }
    let pages: HashMap<ObjectId, u32> = document.get_pages().into_iter().map(|(number, id)| (id, number - 1)).collect();
    let found = walk(document, &pages, 0);
    let mut font_id = None;
    for Found { field, id, widget_ids } in found {
        let Some(value) = values.get(&field.name) else { continue };
        match field.kind {
            FieldKind::CheckBox | FieldKind::Radio => {
                let state = if value.is_empty() { "Off".to_owned() } else { value.clone() };
                if let Ok(node) = document.get_dictionary_mut(id) {
                    node.set("V", Object::Name(state.as_bytes().to_vec()));
                }
                for (widget_id, widget) in widget_ids.iter().zip(&field.widgets) {
                    let shown = if widget.on_state.as_deref() == Some(state.as_str()) { state.as_str() } else { "Off" };
                    if let Ok(dictionary) = document.get_dictionary_mut(*widget_id) {
                        dictionary.set("AS", Object::Name(shown.as_bytes().to_vec()));
                    }
                }
            }
            FieldKind::Text | FieldKind::Choice { .. } => {
                let appearance = document
                    .get_dictionary(id)
                    .ok()
                    .and_then(|node| node.get(b"DA").ok())
                    .and_then(decode)
                    .unwrap_or_default();
                if let Ok(node) = document.get_dictionary_mut(id) {
                    node.set("V", text_string(value));
                }
                let font = *font_id.get_or_insert_with(|| {
                    let mut font = Dictionary::new();
                    font.set("Type", Object::Name(b"Font".to_vec()));
                    font.set("Subtype", Object::Name(b"Type1".to_vec()));
                    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
                    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
                    document.add_object(font)
                });
                for (widget_id, widget) in widget_ids.iter().zip(&field.widgets) {
                    let (width, height) = (widget.rect.width, widget.rect.height);
                    let size = match font_size(&appearance) {
                        size if size > 0.0 => size,
                        _ => (height * 0.65).clamp(4.0, 12.0),
                    };
                    let mut content = format!("/Tx BMC q 1 1 {} {} re W n BT /Helv {size} Tf 0 g ", width - 2.0, height - 2.0).into_bytes();
                    let lines: Vec<&str> = value.lines().collect();
                    let first = if lines.len() > 1 { height - size - 2.0 } else { (height - size) / 2.0 + size * 0.22 };
                    content.extend(format!("2 {first} Td ").as_bytes());
                    for (n, line) in lines.iter().enumerate() {
                        if n > 0 {
                            content.extend(format!("0 {} Td ", -size * 1.15).as_bytes());
                        }
                        content.push(b'(');
                        content.extend(escaped(line));
                        content.extend(b") Tj ");
                    }
                    content.extend(b"ET Q EMC");
                    let mut fonts = Dictionary::new();
                    fonts.set("Helv", Object::Reference(font));
                    let mut resources = Dictionary::new();
                    resources.set("Font", Object::Dictionary(fonts));
                    let mut form = Dictionary::new();
                    form.set("Type", Object::Name(b"XObject".to_vec()));
                    form.set("Subtype", Object::Name(b"Form".to_vec()));
                    form.set("BBox", Object::Array(vec![0.into(), 0.into(), width.into(), height.into()]));
                    form.set("Resources", Object::Dictionary(resources));
                    let stream = document.add_object(Stream::new(form, content));
                    let mut states = Dictionary::new();
                    states.set("N", Object::Reference(stream));
                    if let Ok(dictionary) = document.get_dictionary_mut(*widget_id) {
                        dictionary.set("AP", Object::Dictionary(states));
                    }
                }
            }
            FieldKind::Other => {}
        }
    }
}

/// Draws every widget's current appearance into its page and removes the form.
pub fn flatten(document: &mut lopdf::Document) -> lopdf::Result<()> {
    let pages: Vec<ObjectId> = document.get_pages().into_values().collect();
    for page in pages {
        let annots: Vec<ObjectId> = match document.get_dictionary(page)?.get(b"Annots") {
            Ok(annots) => resolve(document, annots).as_array().map(|a| a.iter().filter_map(|x| x.as_reference().ok()).collect()).unwrap_or_default(),
            Err(_) => continue,
        };
        let mut drawn = Vec::new();
        let mut kept = Vec::new();
        for annot in annots {
            let Ok(dictionary) = document.get_dictionary(annot) else { continue };
            if dictionary.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Widget".as_slice()) {
                kept.push(Object::Reference(annot));
                continue;
            }
            // The normal appearance, or the one of the current state for buttons.
            let normal = dictionary.get(b"AP").ok().and_then(|ap| dictionary_of(document, ap)).and_then(|ap| ap.get(b"N").ok().cloned());
            let stream = match normal {
                Some(Object::Reference(id)) if document.get_object(id).is_ok_and(|o| o.as_stream().is_ok()) => Some(id),
                Some(states) => {
                    let state = dictionary.get(b"AS").and_then(Object::as_name).map(<[u8]>::to_vec).unwrap_or_default();
                    dictionary_of(document, &states).and_then(|states| states.get(&state).ok()).and_then(|s| s.as_reference().ok())
                }
                None => None,
            };
            let (Some(stream), Some(rect)) = (stream, dictionary.get(b"Rect").ok().and_then(|r| rect_of(document, r))) else { continue };
            let bbox = document
                .get_object(stream)
                .ok()
                .and_then(|o| o.as_stream().ok())
                .and_then(|s| s.dict.get(b"BBox").ok().cloned())
                .and_then(|b| rect_of(document, &b));
            let Some(bbox) = bbox.filter(|b| b.width > 0.0 && b.height > 0.0) else { continue };
            drawn.push((stream, rect, bbox));
        }
        if drawn.is_empty() {
            continue;
        }
        let mut content = b"Q ".to_vec();
        let mut xobjects = Dictionary::new();
        for (n, (stream, rect, bbox)) in drawn.iter().enumerate() {
            let (sx, sy) = (rect.width / bbox.width, rect.height / bbox.height);
            let (tx, ty) = (rect.origin.x - bbox.origin.x * sx, rect.origin.y - bbox.origin.y * sy);
            content.extend(format!("q {sx} 0 0 {sy} {tx} {ty} cm /RectoField{n} Do Q ").as_bytes());
            xobjects.set(format!("RectoField{n}"), Object::Reference(*stream));
        }
        // Existing content is wrapped in q/Q so a stray transformation cannot move the fields.
        let opening = document.add_object(Stream::new(Dictionary::new(), b"q".to_vec()));
        let closing = document.add_object(Stream::new(Dictionary::new(), content));
        let resources = match document.get_dictionary(page)?.get(b"Resources") {
            Ok(resources) => dictionary_of(document, resources).cloned().unwrap_or_default(),
            Err(_) => Dictionary::new(),
        };
        let mut resources = resources;
        let mut existing = match resources.get(b"XObject") {
            Ok(x) => dictionary_of(document, x).cloned().unwrap_or_default(),
            Err(_) => Dictionary::new(),
        };
        for (key, value) in xobjects {
            existing.set(key, value);
        }
        resources.set("XObject", Object::Dictionary(existing));
        let page_dictionary = document.get_dictionary_mut(page)?;
        let contents = match page_dictionary.get(b"Contents") {
            Ok(Object::Array(items)) => items.clone(),
            Ok(single) => vec![single.clone()],
            Err(_) => Vec::new(),
        };
        let mut wrapped = vec![Object::Reference(opening)];
        wrapped.extend(contents);
        wrapped.push(Object::Reference(closing));
        page_dictionary.set("Contents", Object::Array(wrapped));
        page_dictionary.set("Resources", Object::Dictionary(resources));
        if kept.is_empty() {
            page_dictionary.remove(b"Annots");
        } else {
            page_dictionary.set("Annots", Object::Array(kept));
        }
    }
    if let Ok(catalog) = document.catalog_mut() {
        catalog.remove(b"AcroForm");
    }
    Ok(())
}

fn dictionary_of<'a>(document: &'a lopdf::Document, object: &'a Object) -> Option<&'a Dictionary> {
    dictionary(document, object)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::op::Op;
    use crate::{BuildOptions, Document};

    fn forms() -> Document {
        Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/forms.pdf")).unwrap()
    }

    fn fill(document: &mut Document, name: &str, value: &str) {
        document.apply(Op::FillField { name: name.into(), value: value.into() });
    }

    #[test]
    fn reads_the_fields() {
        let fields = forms().form_fields().unwrap();
        let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
        assert_eq!(names, ["name", "agree", "size", "color"]);
        assert_eq!(fields[0].kind, FieldKind::Text);
        assert_eq!(fields[1].kind, FieldKind::CheckBox);
        assert_eq!(fields[2].kind, FieldKind::Radio);
        assert_eq!(
            fields[3].kind,
            FieldKind::Choice { options: vec!["Red".into(), "Green".into(), "Blue".into()], editable: false }
        );
        assert!(fields.iter().all(|field| field.value.is_empty() && !field.read_only));
        assert_eq!(fields[1].widgets[0].on_state.as_deref(), Some("Yes"));
        let states: Vec<_> = fields[2].widgets.iter().map(|w| w.on_state.clone().unwrap()).collect();
        assert_eq!(states, ["Small", "Large"]);
        let name = &fields[0].widgets[0];
        assert_eq!((name.page.index, name.rect.origin.x, name.rect.origin.y, name.rect.width), (0, 150.0, 700.0, 250.0));
    }

    #[test]
    fn filled_values_round_trip() {
        let mut document = forms();
        fill(&mut document, "name", "Ada (Lovelace)");
        fill(&mut document, "agree", "Yes");
        fill(&mut document, "size", "Small");
        fill(&mut document, "size", "Large");
        fill(&mut document, "color", "Green");
        let values: Vec<String> = document.form_fields().unwrap().into_iter().map(|field| field.value).collect();
        assert_eq!(values, ["Ada (Lovelace)", "Yes", "Large", "Green"]);

        let reopened = Document::from_bytes(document.build().unwrap()).unwrap();
        let fields = reopened.form_fields().unwrap();
        let values: Vec<&str> = fields.iter().map(|field| field.value.as_str()).collect();
        assert_eq!(values, ["Ada (Lovelace)", "Yes", "Large", "Green"]);
        // Each radio widget shows its own state only when chosen.
        let parsed = lopdf::Document::load_mem(&reopened.bytes()).unwrap();
        let page = *parsed.get_pages().values().next().unwrap();
        let annots = parsed.get_dictionary(page).unwrap().get(b"Annots").unwrap().as_array().unwrap().clone();
        let state = |n: usize| {
            let widget = parsed.get_dictionary(annots[n].as_reference().unwrap()).unwrap();
            String::from_utf8(widget.get(b"AS").unwrap().as_name().unwrap().to_vec()).unwrap()
        };
        assert_eq!((state(1), state(2), state(3)), ("Yes".into(), "Off".into(), "Large".into()));
        // Clearing a check box.
        let mut cleared = reopened.clone();
        fill(&mut cleared, "agree", "");
        let again = Document::from_bytes(cleared.build().unwrap()).unwrap();
        assert_eq!(again.form_fields().unwrap()[1].value, "");
    }

    #[test]
    fn flattening_removes_the_form() {
        let mut document = forms();
        fill(&mut document, "name", "Flat");
        let bytes = document.build_with(&BuildOptions { flatten_forms: true, ..BuildOptions::default() }).unwrap();
        let parsed = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(parsed.catalog().unwrap().get(b"AcroForm").is_err());
        let page = *parsed.get_pages().values().next().unwrap();
        assert!(parsed.get_dictionary(page).unwrap().get(b"Annots").is_err());
        assert!(Document::from_bytes(bytes).unwrap().form_fields().unwrap().is_empty());
        // Unedited documents are only rebuilt when flattening is asked for.
        assert_eq!(forms().build_with(&BuildOptions::default()).unwrap(), forms().bytes().to_vec());
    }
}
