//! An opened PDF: the original bytes, other sources merged into it, and the log of edits.
//! The original file is never modified; `build` writes a new PDF from the log.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::PageId;
use crate::geom::Size;
use crate::op::{Op, PageState, replay};

#[derive(Debug, thiserror::Error)]
pub enum OpenError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("not a valid PDF: {0}")]
    Invalid(#[from] lopdf::Error),
    #[error("the PDF has no pages")]
    Empty,
    #[error("password-protected files cannot be merged yet")]
    Protected,
}

/// Where pages come from. Sources are only ever added, so a `PageId` stays valid for the
/// document's life.
#[derive(Clone, Debug)]
pub enum Source {
    File {
        bytes: Arc<[u8]>,
        /// The password that opened it, if it is encrypted.
        password: Option<String>,
        page_count: usize,
    },
    /// One empty page.
    Blank { size: Size },
}

#[derive(Clone)]
pub struct Document {
    path: Option<PathBuf>,
    /// Source 0 is the opened file.
    sources: Vec<Source>,
    ops: Vec<Op>,
    /// Number of ops in effect; the ones after it can be redone.
    cursor: usize,
    /// Cursor at the last save (or at opening).
    saved_at: Option<usize>,
    /// Encrypted with a user password (the empty one does not open it), not unlocked yet.
    needs_password: bool,
}

impl Document {
    /// Reads the file and checks that `lopdf` can parse it. The file itself is never modified.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        let path = path.as_ref();
        let mut document = Self::from_bytes(read(path)?)?;
        document.path = Some(path.to_owned());
        Ok(document)
    }

    /// Validates in-memory bytes, as `open` does for a file.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, OpenError> {
        let parsed = lopdf::Document::load_mem(&bytes)?;
        // A locked document shows no pages until it is unlocked; its pages are counted then.
        let needs_password = parsed.is_encrypted() && parsed.authenticate_password("").is_err();
        let page_count = parsed.get_pages().len();
        if page_count == 0 && !needs_password {
            return Err(OpenError::Empty);
        }
        let source = Source::File { bytes: bytes.into(), password: None, page_count };
        Ok(Self { path: None, sources: vec![source], ops: Vec::new(), cursor: 0, saved_at: Some(0), needs_password })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// After "save as": the document now stands for that file.
    pub fn set_path(&mut self, path: PathBuf) {
        self.path = Some(path);
    }

    /// The original bytes, shared with the renderer without copying.
    pub fn bytes(&self) -> Arc<[u8]> {
        match &self.sources[0] {
            Source::File { bytes, .. } => bytes.clone(),
            Source::Blank { .. } => unreachable!("source 0 is always the opened file"),
        }
    }

    /// The password that unlocked the original file, if any.
    pub fn password(&self) -> Option<&str> {
        match &self.sources[0] {
            Source::File { password, .. } => password.as_deref(),
            Source::Blank { .. } => None,
        }
    }

    pub fn sources(&self) -> &[Source] {
        &self.sources
    }

    /// Pages of the original file; 0 while it needs a password.
    pub fn page_count(&self) -> usize {
        self.source_pages(0).len()
    }

    /// Whether the document must be unlocked before it can be shown.
    pub fn needs_password(&self) -> bool {
        self.needs_password
    }

    /// Whether `password` (user or owner) unlocks the document.
    pub fn check_password(&self, password: &str) -> bool {
        lopdf::Document::load_mem(&self.bytes()).is_ok_and(|parsed| parsed.authenticate_password(password).is_ok())
    }

    /// Unlocks an encrypted document; its pages become known.
    pub fn unlock(&mut self, password: &str) -> bool {
        let bytes = self.bytes();
        let options = lopdf::LoadOptions::with_password(password);
        let Ok(parsed) = lopdf::Document::load_mem_with_options(&bytes, options) else { return false };
        let page_count = parsed.get_pages().len();
        self.sources[0] = Source::File { bytes, password: Some(password.to_owned()), page_count };
        self.needs_password = false;
        true
    }

    /// Adds a PDF whose pages can then be inserted (merge). Returns its pages.
    pub fn add_file(&mut self, bytes: Vec<u8>) -> Result<Vec<PageId>, OpenError> {
        let parsed = lopdf::Document::load_mem(&bytes)?;
        if parsed.is_encrypted() && parsed.authenticate_password("").is_err() {
            return Err(OpenError::Protected);
        }
        let page_count = parsed.get_pages().len();
        if page_count == 0 {
            return Err(OpenError::Empty);
        }
        self.sources.push(Source::File { bytes: bytes.into(), password: None, page_count });
        Ok(self.source_pages(self.sources.len() - 1))
    }

    /// Reads and adds a PDF file, as `add_file`.
    pub fn add_path(&mut self, path: impl AsRef<Path>) -> Result<Vec<PageId>, OpenError> {
        self.add_file(read(path.as_ref())?)
    }

    /// Adds a blank page source and returns its page.
    pub fn add_blank(&mut self, size: Size) -> PageId {
        self.sources.push(Source::Blank { size });
        PageId { source: (self.sources.len() - 1) as u32, index: 0 }
    }

    fn source_pages(&self, source: usize) -> Vec<PageId> {
        let count = match &self.sources[source] {
            Source::File { page_count, .. } => *page_count,
            Source::Blank { .. } => 1,
        };
        (0..count as u32).map(|index| PageId { source: source as u32, index }).collect()
    }

    /// The current pages, in order: the log replayed up to the cursor.
    pub fn pages(&self) -> Vec<PageState> {
        replay(&self.source_pages(0), &self.ops[..self.cursor])
    }

    /// Records an edit. Anything that could be redone is dropped.
    pub fn apply(&mut self, op: Op) {
        self.ops.truncate(self.cursor);
        if self.saved_at.is_some_and(|saved| saved > self.cursor) {
            self.saved_at = None;
        }
        self.ops.push(op);
        self.cursor += 1;
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.ops.len()
    }

    pub fn undo(&mut self) -> bool {
        let can = self.can_undo();
        if can {
            self.cursor -= 1;
        }
        can
    }

    pub fn redo(&mut self) -> bool {
        let can = self.can_redo();
        if can {
            self.cursor += 1;
        }
        can
    }

    /// Ops in effect.
    pub fn ops(&self) -> &[Op] {
        &self.ops[..self.cursor]
    }

    /// Whether there are changes since opening or the last save.
    pub fn is_modified(&self) -> bool {
        self.saved_at != Some(self.cursor)
    }

    pub fn mark_saved(&mut self) {
        self.saved_at = Some(self.cursor);
    }

    /// Position in the log; with `mark_saved_at`, records a save made from an earlier snapshot.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Records that the state at `cursor` was saved (it may be behind the current one).
    pub fn mark_saved_at(&mut self, cursor: usize) {
        if cursor <= self.ops.len() {
            self.saved_at = Some(cursor);
        }
    }
}

fn read(path: &Path) -> Result<Vec<u8>, OpenError> {
    std::fs::read(path).map_err(|source| OpenError::Io { path: path.to_owned(), source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::op::Rotation;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures").join(name)
    }

    #[test]
    fn opens_the_fixture() {
        let document = Document::open(fixture("sample.pdf")).expect("fixture opens");
        assert_eq!(document.page_count(), 3);
        assert!(document.bytes().starts_with(b"%PDF-"));
        assert!(!document.is_modified());
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(Document::from_bytes(b"not a pdf".to_vec()), Err(OpenError::Invalid(_))));
    }

    #[test]
    fn detects_checks_and_unlocks_passwords() {
        let plain = Document::open(fixture("sample.pdf")).unwrap();
        assert!(!plain.needs_password());
        let mut locked = Document::open(fixture("protected.pdf")).unwrap();
        assert!(locked.needs_password());
        assert_eq!(locked.page_count(), 0);
        assert!(!locked.check_password("wrong"));
        assert!(locked.check_password("recto"));
        assert!(locked.check_password("owner"));
        assert!(!locked.unlock("wrong"));
        assert!(locked.unlock("recto"));
        assert_eq!((locked.page_count(), locked.password()), (1, Some("recto")));
    }

    #[test]
    fn reports_missing_files() {
        assert!(matches!(Document::open("does/not/exist.pdf"), Err(OpenError::Io { .. })));
    }

    #[test]
    fn undo_redo_and_modified_state() {
        let mut document = Document::open(fixture("sample.pdf")).unwrap();
        let first = PageId { source: 0, index: 0 };
        document.apply(Op::DeletePages { pages: vec![first] });
        document.apply(Op::RotatePages { pages: vec![PageId { source: 0, index: 1 }], rotation: Rotation::R90 });
        assert_eq!(document.pages().len(), 2);
        assert!(document.is_modified());

        assert!(document.undo() && document.undo() && !document.undo());
        assert_eq!(document.pages().len(), 3);
        assert!(!document.is_modified());
        assert!(document.redo());
        assert_eq!(document.pages().len(), 2);
        document.mark_saved();
        assert!(!document.is_modified());

        // A new edit drops what could be redone; the saved state is then unreachable by redo.
        document.undo();
        document.apply(Op::MovePages { pages: vec![first], to: 9 });
        assert!(!document.can_redo());
        assert_eq!(document.ops().len(), 1);
        assert!(document.is_modified());
    }

    #[test]
    fn merges_and_blank_pages_get_their_own_sources() {
        let mut document = Document::open(fixture("sample.pdf")).unwrap();
        let merged = document.add_path(fixture("layout.pdf")).unwrap();
        assert_eq!(merged.len(), 4);
        assert!(merged.iter().all(|page| page.source == 1));
        let blank = document.add_blank(Size { width: 100.0, height: 200.0 });
        assert_eq!(blank, PageId { source: 2, index: 0 });
        assert!(matches!(document.add_path(fixture("protected.pdf")), Err(OpenError::Protected)));
        // Adding sources is not an edit.
        assert!(!document.is_modified());
    }
}
