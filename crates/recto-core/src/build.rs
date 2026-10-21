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
use crate::document::{Document, Source};
use crate::op::{Op, PageState};

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("cannot read a source document: {0}")]
    Source(#[from] lopdf::Error),
    #[error("redaction is not supported yet")]
    Unsupported,
    #[error("page {0:?} is missing from its source")]
    MissingPage(PageId),
    #[error("the document has no pages left")]
    NoPages,
    #[error("cannot write the PDF: {0}")]
    Write(std::io::Error),
}

/// Page attributes a page can inherit from its ancestors in the page tree (PDF 32000, 7.7.3.4).
const INHERITED: [&[u8]; 4] = [b"Resources", b"MediaBox", b"CropBox", b"Rotate"];
/// Deeper page trees are treated as broken.
const MAX_TREE_DEPTH: usize = 64;

impl Document {
    /// The current document as a PDF file. Without edits, the original bytes, untouched.
    pub fn build(&self) -> Result<Vec<u8>, BuildError> {
        if self.ops().is_empty() {
            return Ok(self.bytes().to_vec());
        }
        self.build_pages(&self.pages())
    }

    /// A PDF holding exactly `pages`, in that order (used to extract and split).
    pub fn build_pages(&self, pages: &[PageState]) -> Result<Vec<u8>, BuildError> {
        if self.ops().iter().any(|op| matches!(op, Op::Redact { .. })) {
            return Err(BuildError::Unsupported);
        }
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
            let dictionary = base.get_dictionary_mut(id)?;
            dictionary.set("Parent", Object::Reference(tree));
            let own = dictionary.get(b"Rotate").and_then(Object::as_i64).unwrap_or(0);
            let rotation = (own + i64::from(page.rotation)).rem_euclid(360);
            if rotation != 0 || dictionary.has(b"Rotate") {
                dictionary.set("Rotate", Object::Integer(rotation));
            }
            kids.push(Object::Reference(id));
        }
        let mut node = Dictionary::new();
        node.set("Type", Object::Name(b"Pages".to_vec()));
        node.set("Count", Object::Integer(kids.len() as i64));
        node.set("Kids", Object::Array(kids));
        base.objects.insert(tree, Object::Dictionary(node));
        base.catalog_mut()?.set("Pages", Object::Reference(tree));
        base.prune_objects();

        if let Some(state) = encryption {
            base.encrypt(&state)?;
        }
        let mut bytes = Vec::new();
        base.save_to(&mut bytes).map_err(BuildError::Write)?;
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
        let Ok(dictionary) = document.get_dictionary(page) else { continue };
        let mut missing: Vec<&[u8]> = INHERITED.into_iter().filter(|key| !dictionary.has(key)).collect();
        let mut parent = dictionary.get(b"Parent").and_then(Object::as_reference).ok();
        let mut found = Vec::new();
        for _ in 0..MAX_TREE_DEPTH {
            let (Some(id), false) = (parent, missing.is_empty()) else { break };
            let Ok(node) = document.get_dictionary(id) else { break };
            missing.retain(|key| match node.get(key) {
                Ok(value) => {
                    found.push((key.to_vec(), value.clone()));
                    false
                }
                Err(_) => true,
            });
            parent = node.get(b"Parent").and_then(Object::as_reference).ok();
        }
        if let Ok(dictionary) = document.get_dictionary_mut(page) {
            for (key, value) in found {
                dictionary.set(key, value);
            }
        }
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
