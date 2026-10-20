//! Page rendering behind a backend-agnostic trait. PDFium is the default backend (M0).
//! PDFium is not thread-safe: a single render thread owns the `Renderer`.

use recto_core::PageId;

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

pub trait Renderer: Send {
    type Error: std::error::Error + Send + Sync + 'static;

    fn open(&mut self, bytes: std::sync::Arc<[u8]>, password: Option<&str>) -> Result<usize, Self::Error>;
    fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, Self::Error>;
}
