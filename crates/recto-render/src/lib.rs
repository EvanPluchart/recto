//! Page rendering behind a backend-agnostic trait. PDFium is the default backend.
//! PDFium is not thread-safe: a single render thread owns the `Renderer` (see `RenderThread`).

#[cfg(feature = "pdfium")]
mod pdfium;
mod thread;

#[cfg(feature = "pdfium")]
pub use pdfium::{PdfiumRenderer, RenderError};
pub use thread::{RenderThread, Response};

pub use recto_core::geom::Size;

use recto_core::PageId;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RenderRequest {
    pub page: PageId,
    /// Position of the page in the current materialised document.
    pub index: usize,
    /// Pixels per PDF point.
    pub scale: f32,
    /// Requests older than the latest generation are dropped before rendering.
    pub generation: u64,
    /// What the bitmap is for. Ignored by the renderer, echoed back with the result.
    pub purpose: Purpose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Purpose {
    Page,
    Thumbnail,
}

/// Premultiplied BGRA (the layout PDFium and GPUI both use), row-major, `width * 4` bytes per row.
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}x{})", self.width, self.height)
    }
}

/// A rendering backend. It is built on the render thread and never leaves it.
pub trait Renderer {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Loads a document, replacing the previous one.
    fn open(&mut self, bytes: std::sync::Arc<[u8]>, password: Option<&str>) -> Result<DocInfo, Self::Error>;
    fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, Self::Error>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocInfo {
    /// Displayed size of every page, in points, in document order.
    pub page_sizes: Vec<Size>,
    /// Bookmarks, flattened in reading order.
    pub outline: Vec<OutlineEntry>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutlineEntry {
    pub title: String,
    /// 0 for top-level entries.
    pub depth: usize,
    /// Target page, if the entry points inside the document.
    pub page: Option<usize>,
}

impl DocInfo {
    pub fn page_count(&self) -> usize {
        self.page_sizes.len()
    }
}
