//! PDFium backend, through `pdfium-render`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use pdfium_render::prelude::{
    PdfBitmap, PdfBitmapFormat, PdfBookmark, PdfColor, PdfDocument, PdfPage, PdfPageIndex, PdfPoints, PdfRect, PdfRenderConfig,
    Pdfium, PdfiumError,
};

use recto_core::geom::PageRect;

use crate::{Bitmap, DocInfo, OutlineEntry, PageText, RenderRequest, Renderer, Size, Tile};

/// Largest bitmap side, in pixels. Bigger pages must be rendered as tiles.
const MAX_SIDE: f32 = 16_384.0;
/// Largest page side, in pixels, that tiles can address.
const MAX_PAGE_PIXELS: f32 = 1_000_000.0;

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
    #[error("tile {0:?} is empty or too large")]
    InvalidTile(Tile),
    #[error("invalid render scale {0}")]
    InvalidScale(f32),
}

pub struct PdfiumRenderer {
    pdfium: &'static Pdfium,
    document: Option<PdfDocument<'static>>,
    /// Draw annotations and form fields.
    annotations: bool,
}

impl PdfiumRenderer {
    /// Loads the PDFium library on first use. Call it on the render thread (`RenderThread::spawn`).
    pub fn new() -> Result<Self, RenderError> {
        Ok(Self { pdfium: pdfium()?, document: None, annotations: true })
    }
}

impl Renderer for PdfiumRenderer {
    type Error = RenderError;

    fn open(&mut self, bytes: Arc<[u8]>, password: Option<&str>) -> Result<DocInfo, RenderError> {
        self.document = None;
        let document = self.pdfium.load_pdf_from_byte_vec(bytes.to_vec(), password)?;
        // Sizes come from the page tree without loading each page.
        let page_sizes = document
            .pages()
            .page_sizes()?
            .into_iter()
            .map(|rect| Size { width: rect.width().value, height: rect.height().value })
            .collect();
        let mut outline = Vec::new();
        walk_outline(document.bookmarks().root(), 0, &mut outline);
        self.document = Some(document);
        Ok(DocInfo { page_sizes, outline })
    }

    fn render(&mut self, request: &RenderRequest) -> Result<Bitmap, RenderError> {
        let page = self.page(request.index)?;
        let largest = page.width().value.max(page.height().value) * request.scale;
        // A whole page must fit in one bitmap; a tile only has to fit the page in i32 coordinates.
        let limit = if request.tile.is_some() { MAX_PAGE_PIXELS } else { MAX_SIDE };
        if !(request.scale > 0.0 && largest.is_finite() && largest <= limit) {
            return Err(RenderError::InvalidScale(request.scale));
        }
        let config = PdfRenderConfig::new()
            .scale_page_by_factor(request.scale)
            .set_format(PdfBitmapFormat::BGRA)
            .set_reverse_byte_order(false) // pdfium-render defaults to RGBA output
            .set_clear_color(PdfColor::WHITE)
            .render_form_data(self.annotations)
            .render_annotations(self.annotations);
        let bitmap = match request.tile {
            None => page.render_with_config(&config)?,
            Some(tile) => {
                if tile.width == 0 || tile.height == 0 || tile.width as f32 > MAX_SIDE || tile.height as f32 > MAX_SIDE {
                    return Err(RenderError::InvalidTile(tile));
                }
                // The page is placed so the tile's corner lands on the bitmap's; PDFium clips the rest.
                let mut bitmap = PdfBitmap::empty(tile.width as i32, tile.height as i32, PdfBitmapFormat::BGRA)?;
                page.render_into_bitmap_with_config(&mut bitmap, &config.set_origin(-(tile.x as i32), -(tile.y as i32)))?;
                bitmap
            }
        };
        Ok(tight(&bitmap))
    }

    fn show_annotations(&mut self, show: bool) {
        self.annotations = show;
    }

    fn text(&mut self, index: usize) -> Result<PageText, RenderError> {
        let page = self.page(index)?;
        let to_display = DisplayTransform::of(&page)?;
        let text = page.text()?;
        let mut chars = Vec::new();
        let mut boxes = Vec::new();
        for char in text.chars().iter() {
            chars.push(char.unicode_char().unwrap_or(char::REPLACEMENT_CHARACTER));
            // Characters PDFium generated (line breaks, inferred spaces) have no bounds.
            boxes.push(char.loose_bounds().map(|rect| to_display.rect(&rect)).unwrap_or_default());
        }
        Ok(PageText { chars, boxes })
    }
}

impl PdfiumRenderer {
    fn page(&self, index: usize) -> Result<PdfPage<'_>, RenderError> {
        let document = self.document.as_ref().ok_or(RenderError::NoDocument)?;
        let count = document.pages().len() as usize;
        if index >= count {
            return Err(RenderError::PageOutOfRange { index, count });
        }
        Ok(document.pages().get(index as PdfPageIndex)?)
    }
}

/// Maps PDF user space to the displayed page (points, top-left origin, crop box and `/Rotate`
/// applied), as an affine transform measured through PDFium's own page-to-device mapping.
struct DisplayTransform {
    /// `[a, b, c, d, e, f]`: x' = a·x + c·y + e, y' = b·x + d·y + f.
    matrix: [f32; 6],
}

impl DisplayTransform {
    fn of(page: &PdfPage<'_>) -> Result<Self, RenderError> {
        // Device coordinates are integers: measure at 16 px per point, over a 1000 pt span.
        const SCALE: f32 = 16.0;
        const SPAN: f32 = 1000.0;
        let config = PdfRenderConfig::new().scale_page_by_factor(SCALE);
        let map = |x: f32, y: f32| -> Result<(f32, f32), RenderError> {
            let (px, py) = page.points_to_pixels(PdfPoints::new(x), PdfPoints::new(y), &config)?;
            Ok((px as f32 / SCALE, py as f32 / SCALE))
        };
        let origin = map(0.0, 0.0)?;
        let x_axis = map(SPAN, 0.0)?;
        let y_axis = map(0.0, SPAN)?;
        Ok(Self {
            matrix: [
                (x_axis.0 - origin.0) / SPAN,
                (x_axis.1 - origin.1) / SPAN,
                (y_axis.0 - origin.0) / SPAN,
                (y_axis.1 - origin.1) / SPAN,
                origin.0,
                origin.1,
            ],
        })
    }

    fn point(&self, x: f32, y: f32) -> (f32, f32) {
        let [a, b, c, d, e, f] = self.matrix;
        (a * x + c * y + e, b * x + d * y + f)
    }

    /// The transform keeps axis-aligned boxes axis-aligned (rotations are multiples of 90°).
    fn rect(&self, rect: &PdfRect) -> PageRect {
        let (x0, y0) = self.point(rect.left().value, rect.bottom().value);
        let (x1, y1) = self.point(rect.right().value, rect.top().value);
        PageRect { x: x0.min(x1), y: y0.min(y1), width: (x1 - x0).abs(), height: (y1 - y0).abs() }
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
        // In a macOS app bundle, libraries live in `Contents/Frameworks`, beside `Contents/MacOS`.
        if cfg!(target_os = "macos") {
            dirs.push(dir.join("../Frameworks"));
        }
        dirs.push(dir);
    }
    if cfg!(debug_assertions) {
        dirs.push(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/pdfium")));
    }
    dirs
}

/// Outline size limits: malformed files can nest deeply or loop through siblings.
const MAX_OUTLINE_ENTRIES: usize = 20_000;
const MAX_OUTLINE_DEPTH: usize = 32;

/// Appends `first` and its following siblings, each followed by its children, in reading order.
fn walk_outline(first: Option<PdfBookmark<'_>>, depth: usize, out: &mut Vec<OutlineEntry>) {
    let mut next = first;
    while let Some(bookmark) = next {
        if out.len() >= MAX_OUTLINE_ENTRIES || depth > MAX_OUTLINE_DEPTH {
            return;
        }
        out.push(OutlineEntry { title: bookmark.title().unwrap_or_default(), depth, page: bookmark_page(&bookmark) });
        walk_outline(bookmark.first_child(), depth + 1, out);
        next = bookmark.next_sibling();
    }
}

/// Page targeted by a bookmark: its `/Dest`, or the destination of its `GoTo` action.
fn bookmark_page(bookmark: &PdfBookmark<'_>) -> Option<usize> {
    if let Some(destination) = bookmark.destination() {
        return destination.page_index().ok().map(|i| i as usize);
    }
    let action = bookmark.action()?;
    let local = action.as_local_destination_action()?;
    local.destination().ok()?.page_index().ok().map(|i| i as usize)
}

/// Copies a PDFium bitmap into a `Bitmap`, dropping any row padding.
fn tight(bitmap: &PdfBitmap<'_>) -> Bitmap {
    let (width, height) = (bitmap.width() as usize, bitmap.height() as usize);
    let mut pixels = bitmap.as_raw_bytes();
    let stride = pixels.len() / height.max(1);
    if stride != width * 4 {
        pixels = pixels.chunks_exact(stride).flat_map(|row| &row[..width * 4]).copied().collect();
    }
    // Opaque white background: straight and premultiplied alpha are the same here.
    Bitmap { width: width as u32, height: height as u32, pixels }
}
