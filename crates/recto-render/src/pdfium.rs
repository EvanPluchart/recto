//! PDFium backend, through `pdfium-render`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pdfium_render::prelude::{PdfColor, PdfDocument, PdfPageIndex, PdfRenderConfig, Pdfium, PdfiumError};

use crate::{Bitmap, RenderRequest, Renderer};

/// Largest bitmap side, in pixels. Bigger requests must be split into tiles.
const MAX_SIDE: f32 = 16_384.0;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("PDFium library not found (looked in: {0})")]
    LibraryNotFound(String),
    #[error(transparent)]
    Pdfium(#[from] PdfiumError),
    #[error("no document is open")]
    NoDocument,
    #[error("page {index} is out of range (the document has {count} pages)")]
    PageOutOfRange { index: usize, count: usize },
    #[error("invalid render scale {0}")]
    InvalidScale(f32),
}

pub struct PdfiumRenderer {
    pdfium: &'static Pdfium,
    document: Option<PdfDocument<'static>>,
}

impl PdfiumRenderer {
    /// Loads the PDFium library on first use. Call it on the render thread (`RenderThread::spawn`).
    pub fn new() -> Result<Self, RenderError> {
        Ok(Self { pdfium: pdfium()?, document: None })
    }
}

impl Renderer for PdfiumRenderer {
    type Error = RenderError;

    fn open(&mut self, bytes: Arc<[u8]>, password: Option<&str>) -> Result<usize, RenderError> {
        self.document = None;
        let document = self.pdfium.load_pdf_from_byte_vec(bytes.to_vec(), password)?;
        let count = document.pages().len() as usize;
        self.document = Some(document);
        Ok(count)
    }

    fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, RenderError> {
        let document = self.document.as_ref().ok_or(RenderError::NoDocument)?;
        let count = document.pages().len() as usize;
        if request.index >= count {
            return Err(RenderError::PageOutOfRange { index: request.index, count });
        }
        let page = document.pages().get(request.index as PdfPageIndex)?;
        let largest = page.width().value.max(page.height().value) * request.scale;
        if !(request.scale > 0.0 && largest.is_finite() && largest <= MAX_SIDE) {
            return Err(RenderError::InvalidScale(request.scale));
        }
        let config = PdfRenderConfig::new()
            .scale_page_by_factor(request.scale)
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(true);
        let bitmap = page.render_with_config(&config)?;
        // Opaque white background: straight and premultiplied RGBA are the same here.
        Ok(Bitmap { width: bitmap.width() as u32, height: bitmap.height() as u32, pixels: bitmap.as_rgba_bytes() })
    }
}

/// PDFium is bound once per process; the bindings are global in `pdfium-render`.
fn pdfium() -> Result<&'static Pdfium, RenderError> {
    static PDFIUM: Mutex<Option<&'static Pdfium>> = Mutex::new(None);
    let mut slot = PDFIUM.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(pdfium) = *slot {
        return Ok(pdfium);
    }
    let mut tried = Vec::new();
    for dir in library_dirs() {
        let path = Pdfium::pdfium_platform_library_name_at_path(&dir);
        if path.exists() {
            let pdfium: &'static Pdfium = Box::leak(Box::new(Pdfium::new(Pdfium::bind_to_library(&path)?)));
            *slot = Some(pdfium);
            return Ok(pdfium);
        }
        tried.push(dir.display().to_string());
    }
    Err(RenderError::LibraryNotFound(tried.join(", ")))
}

/// `RECTO_PDFIUM_DIR`, then next to the executable, then (debug builds) `target/pdfium/`
/// as written by `cargo xtask fetch-pdfium`.
fn library_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("RECTO_PDFIUM_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|exe| exe.parent().map(PathBuf::from)) {
        dirs.push(dir);
    }
    if cfg!(debug_assertions) {
        dirs.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/pdfium")));
    }
    dirs
}
