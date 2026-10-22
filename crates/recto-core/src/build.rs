//! Materialisation: the edit log replayed into a new PDF with `lopdf`.
//!
//! The opened file is the base. Merged files are renumbered and copied in, blank pages are
//! created, and a single new page tree lists the pages in their current order. Encrypted
//! originals are written encrypted again, with the same password.
//!
//! Known limit: outline entries and form fields that point at deleted pages keep those pages'
//! objects in the file (they are no longer in the page tree, so readers do not show them).

use std::collections::{BTreeSet, HashMap};

use lopdf::{Dictionary, Object, ObjectId};

use crate::PageId;
use crate::security::{Security, SecurityError};
use crate::textedit::{EditError, TextEdit, TextRun};
use crate::textlayer::TextLayer;
use crate::{annot, compress, embed, form, gray, redact, stamp, textedit, textlayer};
use crate::geom::{PageGeometry, Point, Rect};
use crate::document::{Document, Source};
use crate::op::{Op, PageState};
use crate::meta::Metadata;
use crate::stamp::Stamp;

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("cannot read a source document: {0}")]
    Source(#[from] lopdf::Error),
    #[error("page {0:?} is missing from its source")]
    MissingPage(PageId),
    #[error("the document has no pages left")]
    NoPages,
    #[error(transparent)]
    Security(#[from] SecurityError),
    #[error(transparent)]
    Edit(#[from] EditError),
    #[error("cannot write the PDF: {0}")]
    Write(std::io::Error),
}

/// How to build.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildOptions {
    /// Draw form fields into the pages and remove the form (values can no longer be changed).
    pub flatten_forms: bool,
    /// Colors and images turned to shades of gray.
    pub grayscale: bool,
    /// Images resampled and recompressed, objects packed into compressed streams.
    pub compress: bool,
    pub security: Security,
}

/// Page attributes a page can inherit from its ancestors in the page tree (PDF 32000, 7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// Deeper page trees are treated as broken.
const MAX_TREE_DEPTH: usize = 64;

impl Document {
    /// The current document as a PDF file. Without edits, the original bytes, untouched.
    pub fn build(&self) -> Result<Vec<u8>, BuildError> {
        self.build_with(&BuildOptions::default())
    }

    /// `build`, with options (flattening the form).
    pub fn build_with(&self, options: &BuildOptions) -> Result<Vec<u8>, BuildError> {
        if self.ops().is_empty() && *options == BuildOptions::default() {
            return Ok(self.bytes().to_vec());
        }
        self.build_pages_with(&self.pages(), options)
    }

    /// A PDF holding exactly `pages`, in that order (used to extract and split).
    pub fn build_pages(&self, pages: &[PageState]) -> Result<Vec<u8>, BuildError> {
        self.build_pages_with(pages, &BuildOptions::default())
    }

    /// The current form field values set by `FillField` ops, latest first wins.
    pub fn filled_values(&self) -> HashMap<String, String> {
        let mut values = HashMap::new();
        for op in self.ops() {
            if let Op::FillField { name, value } = op {
                values.insert(name.clone(), value.clone());
            }
        }
        values
    }

    /// The form fields of the opened file, with the values filled so far.
    pub fn form_fields(&self) -> Result<Vec<form::FormField>, BuildError> {
        let base = load(&self.sources()[0])?;
        let filled = self.filled_values();
        let mut fields = form::fields(&base, 0);
        for field in &mut fields {
            if let Some(value) = filled.get(&field.name) {
                field.value = value.clone();
            }
        }
        Ok(fields)
    }

    fn build_pages_with(&self, pages: &[PageState], options: &BuildOptions) -> Result<Vec<u8>, BuildError> {
        if pages.is_empty() {
            return Err(BuildError::NoPages);
        }

        let mut base = load(&self.sources()[0])?;
        // Set when the original was decrypted on load: the output is encrypted the same way.
        let encryption = base.encryption_state.take();
        let mut objects: HashMap<PageId, ObjectId> = HashMap::new();
        flatten_inherited(&mut base);
        for (index, id) in base.get_pages().into_values().enumerate() {
            objects.insert(PageId { source: 0, index: index as u32 }, id);
        }

        let used: BTreeSet<u32> = pages.iter().map(|page| page.id.source).filter(|&source| source != 0).collect();
        for source in used {
            match &self.sources()[source as usize] {
                file @ Source::File { .. } => {
                    let mut merged = load(file)?;
                    flatten_inherited(&mut merged);
                    merged.renumber_objects_with(base.max_id + 1);
                    for (index, id) in merged.get_pages().into_values().enumerate() {
                        objects.insert(PageId { source, index: index as u32 }, id);
                    }
                    base.max_id = base.max_id.max(merged.max_id);
                    base.objects.extend(merged.objects);
                }
                Source::Blank { size } => {
                    let media_box = vec![0.into(), 0.into(), size.width.into(), size.height.into()];
                    let mut page = Dictionary::new();
                    page.set("Type", Object::Name(b"Page".to_vec()));
                    page.set("MediaBox", Object::Array(media_box));
                    page.set("Resources", Object::Dictionary(Dictionary::new()));
                    objects.insert(PageId { source, index: 0 }, base.add_object(page));
                }
            }
        }

        let tree = base.new_object_id();
        let mut kids = Vec::with_capacity(pages.len());
        for page in pages {
            let id = *objects.get(&page.id).ok_or(BuildError::MissingPage(page.id))?;
            let crop = self.crop_of(page.id).map(|area| {
                let media = attribute(&base, id, b"MediaBox").and_then(|value| rectangle(&base, value)).unwrap_or(DEFAULT_MEDIA_BOX);
                intersect(area, media).unwrap_or(media)
            });
            let dictionary = base.get_dictionary_mut(id)?;
            dictionary.set("Parent", Object::Reference(tree));
            let own = dictionary.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
            let rotation = (own + i64::from(page.rotation)).rem_euclid(360);
            if rotation != 0 || dictionary.has(b"Rotate") {
                dictionary.set("Rotate", Object::Integer(rotation));
            }
            if let Some(crop) = crop {
                let corners = [crop.origin.x, crop.origin.y, crop.origin.x + crop.width, crop.origin.y + crop.height];
                dictionary.set("CropBox", Object::Array(corners.into_iter().map(Object::Real).collect()));
            }
            kids.push(Object::Reference(id));
        }
        let mut node = Dictionary::new();
        node.set("Type", Object::Name(b"Pages".to_vec()));
        node.set("Count", Object::Integer(kids.len() as i64));
        node.set("Kids", Object::Array(kids));
        base.objects.insert(tree, Object::Dictionary(node));
        base.catalog_mut()?.set("Pages", Object::Reference(tree));

        // Annotations and redactions on pages that are part of this output, in log order: a
        // redaction deletes the annotations made before it.
        let mut redactions: Vec<(ObjectId, Vec<Rect>)> = Vec::new();
        for (serial, op) in self.ops().iter().enumerate() {
            let (Op::AddAnnotation { page, .. } | Op::Redact { page, .. }) = op else { continue };
            if !pages.iter().any(|state| state.id == *page) {
                continue;
            }
            let Some(&id) = objects.get(page) else { continue };
            match op {
                Op::AddAnnotation { annotation, .. } => {
                    let written = annotation.write(&mut base, id, serial);
                    annot::attach(&mut base, id, written)?;
                }
                Op::Redact { area, .. } => {
                    redact::remove_annotations(&mut base, id, &[*area])?;
                    match redactions.iter_mut().find(|(page, _)| *page == id) {
                        Some((_, areas)) => areas.push(*area),
                        None => redactions.push((id, vec![*area])),
                    }
                }
                _ => {}
            }
        }
        // Edited text first: edits address the page's original content.
        let edits: Vec<(PageId, &TextEdit)> = self
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::EditText { page, edit } if pages.iter().any(|state| state.id == *page) => Some((*page, edit)),
                _ => None,
            })
            .collect();
        if !edits.is_empty() {
            let mut needed: HashMap<usize, Vec<char>> = HashMap::new();
            for (_, edit) in &edits {
                if let Some(font) = edit.font {
                    needed.entry(font).or_default().extend(edit.text.chars());
                }
            }
            let mut embedded = HashMap::new();
            for (font, mut chars) in needed {
                chars.sort_unstable();
                chars.dedup();
                let file = self.fonts().get(font).ok_or_else(|| EditError::Embed("unknown font".into()))?;
                embedded.insert(font, embed::embed(&mut base, file, &chars).map_err(EditError::Embed)?);
            }
            for page in pages {
                let here: Vec<&TextEdit> = edits.iter().filter(|(id, _)| *id == page.id).map(|(_, edit)| *edit).collect();
                if let (false, Some(&id)) = (here.is_empty(), objects.get(&page.id)) {
                    textedit::apply(&mut base, id, &here, &embedded)?;
                }
            }
        }
        // Recognised text goes in before redaction, so redaction removes it too.
        for page in pages {
            let layers: Vec<&TextLayer> = self
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::AddTextLayer { page: target, layer } if *target == page.id => Some(layer),
                    _ => None,
                })
                .collect();
            if let (false, Some(&id)) = (layers.is_empty(), objects.get(&page.id)) {
                textlayer::apply(&mut base, id, &layers)?;
            }
        }
        form::fill(&mut base, &self.filled_values());
        let mut originals = Vec::new();
        for (page, areas) in &redactions {
            originals.extend(redact::redact_page(&mut base, *page, areas)?);
        }
        redact::forget(&mut base, &originals)?;
        let stamps: Vec<(&Stamp, Vec<usize>)> = self
            .ops()
            .iter()
            .filter_map(|op| if let Op::AddStamp(stamp) = op { Some(stamp) } else { None })
            .map(|stamp| {
                let on = pages.iter().enumerate().filter(|(_, page)| stamp.pages.as_ref().is_none_or(|only| only.contains(&page.id)));
                (stamp, on.map(|(index, _)| index).collect())
            })
            .collect();
        if !stamps.is_empty() {
            let placed: Vec<(ObjectId, PageGeometry)> = pages
                .iter()
                .filter_map(|page| objects.get(&page.id))
                .map(|&id| (id, page_geometry(&base, id, 0)))
                .collect();
            stamp::apply(&mut base, &placed, &stamps)?;
        }
        if let Some(metadata) = self.ops().iter().rev().find_map(|op| if let Op::SetMetadata(metadata) = op { Some(metadata) } else { None }) {
            metadata.write(&mut base, std::time::SystemTime::now())?;
        }
        if options.flatten_forms {
            form::flatten(&mut base)?;
        }
        if options.grayscale {
            gray::convert(&mut base);
        }
        if options.compress {
            compress::images(&mut base);
        }
        base.prune_objects();

        let encryption = match &options.security {
            Security::Keep => encryption,
            Security::Remove => None,
            Security::Protect(protection) => Some(protection.state()?),
        };
        let encrypted = encryption.is_some();
        if let Some(state) = encryption {
            base.encrypt(&state)?;
        }
        let mut bytes = Vec::new();
        base.save_to(&mut bytes).map_err(BuildError::Write)?;
        // Object streams pack the small objects together; kept only when smaller. (lopdf does
        // not encrypt object streams, so encrypted files are written without them.)
        if options.compress && !encrypted {
            let save = lopdf::SaveOptions::builder().use_object_streams(true).use_xref_streams(true).compression_level(9).build();
            let mut packed = Vec::new();
            if base.save_with_options(&mut packed, save).is_ok() && packed.len() < bytes.len() {
                bytes = packed;
            }
        }
        Ok(bytes)
    }
}

fn load(source: &Source) -> Result<lopdf::Document, lopdf::Error> {
    match source {
        Source::File { bytes, password: Some(password), .. } => {
            lopdf::Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password))
        }
        Source::File { bytes, password: None, .. } => lopdf::Document::load_mem(bytes),
        Source::Blank { .. } => unreachable!("blank pages are created, not loaded"),
    }
}

/// Copies inherited attributes onto every page, so pages can be moved to a new page tree.
fn flatten_inherited(document: &mut lopdf::Document) {
    let pages: Vec<ObjectId> = document.get_pages().into_values().collect();
    for page in pages {
        let found: Vec<(&[u8], Object)> = INHERITED
            .into_iter()
            .filter_map(|key| attribute(document, page, key).map(|value| (key, value.clone())))
            .collect();
        if let Ok(dictionary) = document.get_dictionary_mut(page) {
            for (key, value) in found {
                if !dictionary.has(key) {
                    dictionary.set(key, value);
                }
            }
        }
    }
}

/// A page attribute, from the page itself or the nearest ancestor that sets it.
fn attribute<'a>(document: &'a lopdf::Document, page: ObjectId, key: &[u8]) -> Option<&'a Object> {
    let mut node = document.get_dictionary(page).ok()?;
    for _ in 0..MAX_TREE_DEPTH {
        if let Ok(value) = node.get(key) {
            return Some(value);
        }
        let parent = node.get(b"Parent").and_then(Object::as_reference).ok()?;
        node = document.get_dictionary(parent).ok()?;
    }
    None
}

/// A rectangle array (`[x0 y0 x1 y1]`, corners in any order), references resolved.
fn rectangle(document: &lopdf::Document, value: &Object) -> Option<Rect> {
    let resolved = |value: &Object| -> Option<f32> {
        let value = match value {
            Object::Reference(id) => document.get_object(*id).ok()?,
            other => other,
        };
        value.as_float().ok()
    };
    let array = match value {
        Object::Reference(id) => document.get_object(*id).ok()?.as_array().ok()?,
        other => other.as_array().ok()?,
    };
    let [x0, y0, x1, y1] = [resolved(array.first()?)?, resolved(array.get(1)?)?, resolved(array.get(2)?)?, resolved(array.get(3)?)?];
    let origin = Point { x: x0.min(x1), y: y0.min(y1) };
    Some(Rect { origin, width: (x1 - x0).abs(), height: (y1 - y0).abs() })
}

fn intersect(a: Rect, b: Rect) -> Option<Rect> {
    let (x0, y0) = (a.origin.x.max(b.origin.x), a.origin.y.max(b.origin.y));
    let x1 = (a.origin.x + a.width).min(b.origin.x + b.width);
    let y1 = (a.origin.y + a.height).min(b.origin.y + b.height);
    (x1 > x0 && y1 > y0).then_some(Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 })
}

/// US Letter, the default media box when a page has none.
const DEFAULT_MEDIA_BOX: Rect = Rect { origin: Point { x: 0.0, y: 0.0 }, width: 612.0, height: 792.0 };
/// How page `id` of `document` is shown, with `added` degrees of rotation on top of its own.
fn page_geometry(document: &lopdf::Document, id: ObjectId, added: u16) -> PageGeometry {
    let media = attribute(document, id, b"MediaBox").and_then(|value| rectangle(document, value)).unwrap_or(DEFAULT_MEDIA_BOX);
    let crop = attribute(document, id, b"CropBox").and_then(|value| rectangle(document, value));
    let area = crop.and_then(|crop| intersect(crop, media)).unwrap_or(media);
    let own = attribute(document, id, b"Rotate").and_then(|value| value.as_i64().ok()).unwrap_or(0);
    // Rotations that are not quarter turns are invalid; round them like readers do.
    let quarter = ((own + i64::from(added)).rem_euclid(360) + 45) / 90 % 4;
    PageGeometry { area, rotate: (quarter * 90) as u16 }
}

impl Document {
    /// How each current page is shown, read from the sources with `lopdf`.
    pub fn geometry(&self) -> Result<Vec<PageGeometry>, BuildError> {
        self.geometry_of(&self.pages())
    }

    /// The text runs of `page`, with the edits made so far.
    pub fn text_runs(&self, page: PageId) -> Result<Vec<TextRun>, BuildError> {
        let source = &self.sources()[page.source as usize];
        let Source::File { .. } = source else { return Ok(Vec::new()) };
        let document = load(source)?;
        let id = *document.get_pages().values().nth(page.index as usize).ok_or(BuildError::MissingPage(page))?;
        let mut runs = textedit::runs(&document, id);
        for op in self.ops() {
            if let Op::EditText { page: target, edit } = op
                && *target == page
                && let Some(run) = runs.iter_mut().find(|run| run.first == edit.first && run.last == edit.last)
            {
                run.text = edit.text.clone();
            }
        }
        Ok(runs)
    }

    /// The crop area set on `page` by the latest `CropPages`, if any.
    pub fn crop_of(&self, page: PageId) -> Option<Rect> {
        self.ops().iter().rev().find_map(|op| match op {
            Op::CropPages { pages, area } if pages.contains(&page) => Some(*area),
            _ => None,
        })
    }

    /// The document properties: the latest `SetMetadata`, or the opened file's.
    pub fn metadata(&self) -> Result<Metadata, BuildError> {
        if let Some(metadata) = self.ops().iter().rev().find_map(|op| if let Op::SetMetadata(metadata) = op { Some(metadata) } else { None }) {
            return Ok(metadata.clone());
        }
        Ok(Metadata::read(&load(&self.sources()[0])?))
    }

    /// How each of `pages` is shown.
    pub fn geometry_of(&self, pages: &[PageState]) -> Result<Vec<PageGeometry>, BuildError> {
        let mut loaded: HashMap<u32, (lopdf::Document, Vec<ObjectId>)> = HashMap::new();
        let mut geometry = Vec::with_capacity(pages.len());
        for page in pages {
            let source = &self.sources()[page.id.source as usize];
            let mut shown = match source {
                Source::Blank { size } => {
                    let area = Rect { origin: Point { x: 0.0, y: 0.0 }, width: size.width, height: size.height };
                    PageGeometry { area, rotate: page.rotation }
                }
                Source::File { .. } => {
                    let (document, ids) = match loaded.entry(page.id.source) {
                        std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            let document = load(source)?;
                            let ids = document.get_pages().into_values().collect();
                            entry.insert((document, ids))
                        }
                    };
                    let id = *ids.get(page.id.index as usize).ok_or(BuildError::MissingPage(page.id))?;
                    page_geometry(document, id, page.rotation)
                }
            };
            if let Some(crop) = self.crop_of(page.id) {
                let media = match source {
                    Source::Blank { size } => Rect { origin: Point { x: 0.0, y: 0.0 }, width: size.width, height: size.height },
                    Source::File { .. } => {
                        let (document, ids) = &loaded[&page.id.source];
                        let id = ids[page.id.index as usize];
                        attribute(document, id, b"MediaBox").and_then(|value| rectangle(document, value)).unwrap_or(DEFAULT_MEDIA_BOX)
                    }
                };
                shown.area = intersect(crop, media).unwrap_or(media);
            }
            geometry.push(shown);
        }
        Ok(geometry)
    }
}


#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::geom::Size;
    use crate::op::Rotation;

    fn open(name: &str) -> Document {
        Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)).unwrap()
    }

    fn id(index: u32) -> PageId {
        PageId { source: 0, index }
    }

    /// Reopens a built file: each page's text and its `/Rotate`.
    fn reopen(bytes: &[u8], password: Option<&str>) -> Vec<(String, i64)> {
        let document = match password {
            Some(password) => {
                lopdf::Document::load_mem_with_options(bytes, lopdf::LoadOptions::with_password(password)).unwrap()
            }
            None => lopdf::Document::load_mem(bytes).unwrap(),
        };
        document
            .get_pages()
            .iter()
            .map(|(&number, &page)| {
                let text = document.extract_text(&[number]).unwrap_or_default().trim().to_owned();
                let rotate = document.get_dictionary(page).unwrap().get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
                (text, rotate)
            })
            .collect()
    }

    fn titles(pages: &[(String, i64)]) -> Vec<String> {
        // First line of each page: "Recto sample, page N" or "Page N (...)".
        pages.iter().map(|(text, _)| text.lines().next().unwrap_or_default().to_owned()).collect()
    }

    #[test]
    fn without_edits_the_original_is_returned() {
        let document = open("sample.pdf");
        assert_eq!(document.build().unwrap(), document.bytes().to_vec());
    }

    #[test]
    fn rotations_round_trip() {
        let mut document = open("sample.pdf");
        document.apply(Op::RotatePages { pages: vec![id(1)], rotation: Rotation::R90 });
        document.apply(Op::RotatePages { pages: vec![id(1), id(2)], rotation: Rotation::R180 });
        let pages = reopen(&document.build().unwrap(), None);
        assert_eq!(pages.iter().map(|page| page.1).collect::<Vec<_>>(), vec![0, 270, 180]);
    }

    #[test]
    fn deletes_and_moves_round_trip() {
        let mut document = open("layout.pdf");
        document.apply(Op::DeletePages { pages: vec![id(1)] });
        document.apply(Op::MovePages { pages: vec![id(3)], to: 0 });
        let pages = reopen(&document.build().unwrap(), None);
        let titles = titles(&pages);
        assert_eq!(titles.len(), 3);
        assert!(titles[0].starts_with("Page 4") && titles[1].starts_with("Page 1") && titles[2].starts_with("Page 3"));
        // Page 3 keeps its own /Rotate 90.
        assert_eq!(pages[2].1, 90);
    }

    #[test]
    fn merges_and_blank_pages_round_trip() {
        let mut document = open("sample.pdf");
        let merged = document.add_path(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/layout.pdf")).unwrap();
        let blank = document.add_blank(Size { width: 300.0, height: 400.0 });
        document.apply(Op::InsertPages { pages: merged[..2].to_vec(), at: 1 });
        document.apply(Op::InsertPages { pages: vec![blank], at: 99 });
        let bytes = document.build().unwrap();
        let pages = reopen(&bytes, None);
        let titles = titles(&pages);
        assert_eq!(titles.len(), 6);
        assert!(titles[0].starts_with("Recto sample, page 1"));
        assert!(titles[1].starts_with("Page 1") && titles[2].starts_with("Page 2"));
        assert!(titles[3].starts_with("Recto sample, page 2") && titles[4].starts_with("Recto sample, page 3"));
        assert_eq!(titles[5], "");
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let last = *reopened.get_pages().values().last().unwrap();
        let media_box = reopened.get_dictionary(last).unwrap().get(b"MediaBox").unwrap().as_array().unwrap();
        let numbers: Vec<f32> = media_box.iter().map(|value| value.as_float().unwrap()).collect();
        assert_eq!(numbers, vec![0.0, 0.0, 300.0, 400.0]);
    }

    #[test]
    fn inherited_attributes_survive_reparenting() {
        let mut document = open("inherited.pdf");
        document.apply(Op::MovePages { pages: vec![id(2)], to: 0 });
        document.apply(Op::DeletePages { pages: vec![id(0)] });
        let bytes = document.build().unwrap();
        let pages = reopen(&bytes, None);
        assert_eq!(titles(&pages), vec!["Inherited 3", "Inherited 2"]);
        // Page 2 kept the nested node's rotation and media box; page 3 its own box.
        assert_eq!(pages.iter().map(|page| page.1).collect::<Vec<_>>(), vec![0, 90]);
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let ids: Vec<ObjectId> = reopened.get_pages().into_values().collect();
        let width = |id| reopened.get_dictionary(id).unwrap().get(b"MediaBox").unwrap().as_array().unwrap()[2].as_float().unwrap();
        assert_eq!((width(ids[0]), width(ids[1])), (842.0, 612.0));
    }

    #[test]
    fn encrypted_documents_stay_encrypted() {
        let mut document = open("protected.pdf");
        assert!(document.unlock("recto"));
        document.apply(Op::RotatePages { pages: vec![id(0)], rotation: Rotation::R90 });
        let bytes = document.build().unwrap();
        let locked = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(locked.is_encrypted() && locked.authenticate_password("").is_err());
        let pages = reopen(&bytes, Some("recto"));
        assert_eq!(pages.len(), 1);
        assert_eq!(pages[0].1, 90);
        assert!(pages[0].0.starts_with("Page 1"));
    }

    #[test]
    fn geometry_follows_boxes_rotation_and_edits() {
        let sizes = |document: &Document| -> Vec<(f32, f32, u16)> {
            document.geometry().unwrap().iter().map(|page| (page.size().width, page.size().height, page.rotate)).collect()
        };
        let mut layout = open("layout.pdf");
        assert_eq!(sizes(&layout), vec![(595.0, 842.0, 0), (842.0, 595.0, 0), (842.0, 595.0, 90), (612.0, 792.0, 0)]);
        layout.apply(Op::RotatePages { pages: vec![id(2)], rotation: Rotation::R270 });
        assert_eq!(sizes(&layout)[2], (595.0, 842.0, 0));
        let inherited = open("inherited.pdf");
        assert_eq!(sizes(&inherited), vec![(792.0, 612.0, 90), (792.0, 612.0, 90), (792.0, 505.0, 0)]);
        let crop = inherited.geometry().unwrap()[2].area;
        assert_eq!((crop.origin.x, crop.origin.y), (50.0, 40.0));
        let mut blank = open("sample.pdf");
        let page = blank.add_blank(crate::geom::Size { width: 100.0, height: 50.0 });
        blank.apply(Op::InsertPages { pages: vec![page], at: 0 });
        assert_eq!(sizes(&blank)[0], (100.0, 50.0, 0));
    }

    #[test]
    fn annotations_round_trip() {
        use crate::annot::{Annotation, Color, Font, Kind, Raster};
        use crate::geom::Point;
        let rect = |x, y, width, height| Rect { origin: Point { x, y }, width, height };
        let kinds = vec![
            Kind::Text { rect: rect(300.0, 100.0, 200.0, 50.0), text: "Héllo".into(), size: 20.0, font: Font::Helvetica, upright: 0 },
            Kind::Image {
                rect: rect(50.0, 50.0, 40.0, 40.0),
                image: std::sync::Arc::new(Raster { width: 1, height: 1, rgb: vec![255, 0, 0], alpha: Some(vec![255]) }),
                upright: 0,
            },
            Kind::Rectangle { rect: rect(100.0, 100.0, 200.0, 100.0) },
            Kind::Ellipse { rect: rect(100.0, 250.0, 100.0, 60.0) },
            Kind::Line { from: Point { x: 300.0, y: 300.0 }, to: Point { x: 400.0, y: 350.0 }, arrow: true },
            Kind::Ink { strokes: vec![vec![Point { x: 450.0, y: 300.0 }, Point { x: 500.0, y: 320.0 }]] },
            Kind::Highlight { quads: vec![rect(56.0, 660.0, 200.0, 24.0)] },
            Kind::Underline { quads: vec![rect(56.0, 630.0, 200.0, 12.0)] },
            Kind::StrikeOut { quads: vec![rect(56.0, 600.0, 200.0, 12.0)] },
        ];
        let mut document = open("sample.pdf");
        for kind in kinds.clone() {
            document.apply(Op::AddAnnotation { page: id(0), annotation: Annotation { kind, color: Color::RED, width: 2.0 } });
        }
        // On a page that is then deleted: not written.
        document.apply(Op::AddAnnotation {
            page: id(2),
            annotation: Annotation { kind: kinds[2].clone(), color: Color::BLUE, width: 1.0 },
        });
        document.apply(Op::DeletePages { pages: vec![id(2)] });
        document.apply(Op::MovePages { pages: vec![id(0)], to: 1 });

        let bytes = document.build().unwrap();
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let pages: Vec<ObjectId> = reopened.get_pages().into_values().collect();
        assert_eq!(pages.len(), 2);
        let annots = |page: ObjectId| -> Vec<Dictionary> {
            match reopened.get_dictionary(page).unwrap().get(b"Annots") {
                Ok(Object::Array(items)) => {
                    items.iter().map(|item| reopened.get_dictionary(item.as_reference().unwrap()).unwrap().clone()).collect()
                }
                _ => Vec::new(),
            }
        };
        assert!(annots(pages[0]).is_empty(), "the moved page keeps its annotations");
        let written = annots(pages[1]);
        let subtypes: Vec<String> =
            written.iter().map(|a| String::from_utf8(a.get(b"Subtype").unwrap().as_name().unwrap().to_vec()).unwrap()).collect();
        assert_eq!(subtypes, ["FreeText", "Stamp", "Square", "Circle", "Line", "Ink", "Highlight", "Underline", "StrikeOut"]);
        for (annotation, kind) in written.iter().zip(&kinds) {
            assert!(annotation.get(b"AP").is_ok(), "every annotation carries its appearance");
            let expected = Annotation { kind: kind.clone(), color: Color::RED, width: 2.0 }.rect();
            let rect: Vec<f32> = annotation.get(b"Rect").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap()).collect();
            assert_eq!(rect, vec![expected.origin.x, expected.origin.y, expected.origin.x + expected.width, expected.origin.y + expected.height]);
        }
        assert_eq!(written[6].get(b"QuadPoints").unwrap().as_array().unwrap().len(), 8);
        assert_eq!(written[5].get(b"InkList").unwrap().as_array().unwrap().len(), 1);
    }


    #[test]
    fn crops_stamps_and_metadata_round_trip() {
        use crate::meta::Metadata;
        use crate::stamp::Stamp;
        let mut document = open("layout.pdf");
        let crop = Rect { origin: Point { x: 50.0, y: 60.0 }, width: 400.0, height: 500.0 };
        document.apply(Op::CropPages { pages: vec![id(0)], area: crop });
        document.apply(Op::AddStamp(Stamp::page_numbers()));
        document.apply(Op::AddStamp(Stamp { pages: Some(vec![id(1)]), ..Stamp::watermark("DRAFT") }));
        let metadata = Metadata { title: "Été".into(), author: "Recto".into(), ..Metadata::default() };
        document.apply(Op::SetMetadata(metadata.clone()));
        document.apply(Op::MovePages { pages: vec![id(3)], to: 0 });
        assert_eq!(document.geometry().unwrap()[1].area, crop);
        assert_eq!(document.metadata().unwrap(), metadata);

        let bytes = document.build().unwrap();
        let reopened = lopdf::Document::load_mem(&bytes).unwrap();
        let text = |number: u32| reopened.extract_text(&[number]).unwrap();
        for (number, label) in [(1, "1 / 4"), (2, "2 / 4"), (3, "3 / 4"), (4, "4 / 4")] {
            assert!(text(number).contains(label), "page {number}: {:?}", text(number));
        }
        assert!(text(3).contains("DRAFT") && !text(2).contains("DRAFT"));
        // The original text is still there, under the stamps.
        assert!(text(1).contains("Page 4"));
        let pages: Vec<ObjectId> = reopened.get_pages().into_values().collect();
        let crop_box: Vec<f32> =
            reopened.get_dictionary(pages[1]).unwrap().get(b"CropBox").unwrap().as_array().unwrap().iter().map(|v| v.as_float().unwrap()).collect();
        assert_eq!(crop_box, vec![50.0, 60.0, 450.0, 560.0]);
        assert_eq!(Metadata::read(&reopened), metadata);
    }

    #[test]
    fn protects_and_removes_protection() {
        use crate::security::{Allowed, Protection, Security, allowed};
        let document = open("sample.pdf");
        let protection = Protection {
            user_password: "user".into(),
            owner_password: "owner".into(),
            allowed: Allowed { print: false, ..Allowed::default() },
        };
        for compress in [false, true] {
            let options = BuildOptions { security: Security::Protect(protection.clone()), compress, ..BuildOptions::default() };
            let bytes = document.build_with(&options).unwrap();
            let locked = lopdf::Document::load_mem(&bytes).unwrap();
            assert!(locked.is_encrypted() && locked.authenticate_password("").is_err());
            assert!(locked.authenticate_password("user").is_ok() && locked.authenticate_password("owner").is_ok());
            assert_eq!(allowed(&locked), Allowed { print: false, ..Allowed::default() });
            assert!(titles(&reopen(&bytes, Some("user")))[0].starts_with("Recto sample, page 1"));
        }
        // Without an owner password, a random one: the user password still opens the file.
        let open_only = Protection { owner_password: String::new(), ..protection };
        let bytes = document.build_with(&BuildOptions { security: Security::Protect(open_only), ..BuildOptions::default() }).unwrap();
        assert!(lopdf::Document::load_mem(&bytes).unwrap().authenticate_password("").is_err());
        assert_eq!(reopen(&bytes, Some("user")).len(), 3);

        let mut locked = open("protected.pdf");
        assert!(locked.unlock("recto"));
        let bytes = locked.build_with(&BuildOptions { security: Security::Remove, ..BuildOptions::default() }).unwrap();
        let plain = lopdf::Document::load_mem(&bytes).unwrap();
        assert!(!plain.is_encrypted() && plain.trailer.get(b"Encrypt").is_err());
        assert!(titles(&reopen(&bytes, None))[0].starts_with("Page 1"));
    }

    #[test]
    fn grayscale_and_compression() {
        let document = open("sample.pdf");
        let plain = document.build_with(&BuildOptions { grayscale: true, ..BuildOptions::default() }).unwrap();
        let reopened = lopdf::Document::load_mem(&plain).unwrap();
        let spaces: Vec<Vec<u8>> = reopened
            .objects
            .values()
            .filter_map(|object| object.as_stream().ok())
            .filter(|stream| stream.dict.get(b"Subtype").ok().and_then(|value| value.as_name().ok()) == Some(b"Image"))
            .map(|stream| stream.dict.get(b"ColorSpace").unwrap().as_name().unwrap().to_vec())
            .collect();
        assert!(!spaces.is_empty() && spaces.iter().all(|space| space == b"DeviceGray"), "{spaces:?}");
        for page in reopened.get_pages().into_values() {
            let content = String::from_utf8_lossy(&reopened.get_page_content(page)).into_owned();
            assert!(!content.contains(" rg") && !content.contains(" RG"), "{content}");
        }
        assert_eq!(titles(&reopen(&plain, None)).len(), 3);

        let compressed = document.build_with(&BuildOptions { compress: true, ..BuildOptions::default() }).unwrap();
        let normal = open("sample.pdf").build_pages(&document.pages()).unwrap();
        assert!(compressed.len() <= normal.len(), "{} > {}", compressed.len(), normal.len());
        assert!(titles(&reopen(&compressed, None))[2].starts_with("Recto sample, page 3"));
    }
    #[test]
    fn subsets_and_errors() {
        let document = open("sample.pdf");
        let pages = document.pages();
        let bytes = document.build_pages(&[pages[2], pages[0]]).unwrap();
        let titles = titles(&reopen(&bytes, None));
        assert_eq!(titles.len(), 2);
        assert!(titles[0].starts_with("Recto sample, page 3") && titles[1].starts_with("Recto sample, page 1"));
        assert!(matches!(document.build_pages(&[]), Err(BuildError::NoPages)));
    }
}
