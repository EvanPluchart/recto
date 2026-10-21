//! Page rendering behind a backend-agnostic trait. PDFium is the default backend.
//! PDFium is not thread-safe: a single render thread owns the `Renderer` (see `RenderThread`).

#[cfg(feature = "pdfium")]
mod pdfium;
mod thread;

#[cfg(feature = "pdfium")]
pub use pdfium::{PdfiumRenderer, RenderError};
pub use thread::{RenderThread, Response};

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
}

/// Premultiplied RGBA, row-major.
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

    /// Loads a document, replacing the previous one. Returns its page count.
    fn open(&mut self, bytes: std::sync::Arc<[u8]>, password: Option<&str>) -> Result<usize, Self::Error>;
    fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, Self::Error>;
}
