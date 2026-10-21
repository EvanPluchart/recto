//! An opened PDF: the original bytes, validated by `lopdf`. The op log arrives with M2.

use std::path::{Path, PathBuf};
use std::sync::Arc;

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
}

pub struct Document {
    path: Option<PathBuf>,
    bytes: Arc<[u8]>,
    page_count: usize,
}

impl Document {
    /// Reads the file and checks that `lopdf` can parse it. The file itself is never modified.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|source| OpenError::Io { path: path.to_owned(), source })?;
        let mut document = Self::from_bytes(bytes)?;
        document.path = Some(path.to_owned());
        Ok(document)
    }

    /// Validates in-memory bytes, as `open` does for a file.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, OpenError> {
        let parsed = lopdf::Document::load_mem(&bytes)?;
        let page_count = parsed.get_pages().len();
        if page_count == 0 {
            return Err(OpenError::Empty);
        }
        Ok(Self { path: None, bytes: bytes.into(), page_count })
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The original bytes, shared with the renderer without copying.
    pub fn bytes(&self) -> Arc<[u8]> {
        self.bytes.clone()
    }

    pub fn page_count(&self) -> usize {
        self.page_count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample.pdf")
    }

    #[test]
    fn opens_the_fixture() {
        let document = Document::open(fixture()).expect("fixture opens");
        assert_eq!(document.page_count(), 3);
        assert!(document.bytes().starts_with(b"%PDF-"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(Document::from_bytes(b"not a pdf".to_vec()), Err(OpenError::Invalid(_))));
    }

    #[test]
    fn reports_missing_files() {
        assert!(matches!(Document::open("does/not/exist.pdf"), Err(OpenError::Io { .. })));
    }
}
