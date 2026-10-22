# Architecture

Recto is a Cargo workspace with three library layers and one binary. The rule that shapes everything: **the UI never touches PDF bytes, and the PDF layers never know about the UI.**

```
crates/
  recto-core     document model, edit operations, undo/redo, save        (pure Rust, no UI)
  recto-render   Renderer trait + backends (PDFium now, hayro later)     (no UI)
  recto-ocr      text recognition on page images (ocrs, decision 0004)  (no UI, no PDF)
  recto-app      GPUI application, binary "recto"                        (depends on both)
xtask/           fetch-pdfium, fetch-ocr-models, gen-fixtures, gen-large; bundle, release (M6)
tests/fixtures/  small PDFs used by integration tests
```

## Layers

### recto-core: the document model

* `Document` holds the **original bytes** plus an ordered **operation log** (`Vec<Op>`). The original file is never modified in memory.
* `Op` is an enum of edits: `RotatePages`, `DeletePages`, `MovePages`, `InsertPages` (pages of a merged file or a blank page, each a `Source` of the document), `AddAnnotation` (`annot::Annotation`, user space), `FillField`, `Redact` (`redact.rs`), `CropPages`, `AddStamp` (`stamp.rs`: watermark, page numbers, header, footer), `AddTextLayer` (`textlayer.rs`, OCR words), `EditText` (`textedit.rs`: a run of text rewritten in its own font or in a font added with `Document::add_font`, decision 0005), `SetMetadata` (`meta.rs`). Each op is plain data, so an edit session can be saved and replayed (serialisation is not written yet).
* **Undo/redo** moves a cursor in the op log. No snapshots.
* **Page identity**: pages are addressed by a stable `PageId` (source document + original index), never by their current position. Reordering then never invalidates selections, thumbnails or caches.
* **Materialisation**: `Document::build() -> Vec<u8>` replays the op log on a `lopdf::Document` and writes a new PDF (`build.rs`): one new page tree, inherited page attributes copied down, merged files renumbered in, annotations written with explicit appearance streams (`annot.rs`), form values with generated appearances and optional flattening (`form.rs`, decision 0003), encrypted originals re-encrypted with their password. `Document::geometry()` gives each page's visible area and display rotation (`geom::PageGeometry`), which the app uses to turn pointer positions into user space. The renderer is fed the materialised bytes, rebuilt in the background after each op.
* **Saving** is atomic: write to a temp file in the same folder, fsync, rename over the target.
* **Save-time options** (`BuildOptions`) are not ops, because they are too slow to run on every rebuild or change what the renderer needs: `flatten_forms`, `grayscale` (`gray.rs`), `compress` (`compress.rs`: images resampled to 150 ppi where drawn larger, object streams) and `security` (`security.rs`: keep, remove, or protect with AES-256 and permissions). The app uses them for Export.
* **Content interpretation** (`content.rs`): matrices, font widths (`/Widths`, `/W`, base-14 metrics) and image decoding, used by redaction to locate glyphs, images and forms in user space. Redaction never modifies a shared object: the page gets new content and copies.
* Features that need new PDF content (watermarks, page numbers, stamped text) generate content streams with `pdf-writer` and merge them through `lopdf`.

### recto-render: the renderer

```rust
pub trait Renderer {                                                 // built on the render thread, never leaves it
    fn open(&mut self, bytes: Arc<[u8]>, password: Option<&str>) -> Result<DocInfo>; // page sizes, outline
    fn render(&mut self, req: &RenderRequest) -> Result<Bitmap>;     // page, scale, optional tile; BGRA
    fn text(&mut self, page: usize) -> Result<PageText>;             // chars + boxes, for search/selection
    fn form_fields(&mut self, page: usize) -> Result<Vec<FormField>>; // M3
}
```

* **PDFium is not thread-safe.** One dedicated render thread owns the `Pdfium` instance. `RenderThread::spawn(PdfiumRenderer::new)` takes a constructor and runs it *on* the thread, so the backend is never moved across threads and needs no `Send` bound. The UI sends `RenderRequest`s over a `flume` channel and receives bitmaps back. Requests carry a generation number (`RenderThread::next_generation`); stale ones (old zoom, scrolled away) are dropped before rendering.
* **Loading PDFium**: `RECTO_PDFIUM_DIR` if set, then the executable's folder, then (debug builds only) `target/pdfium/` as filled by `cargo xtask fetch-pdfium`.
* **Requests** carry a `purpose` (page, thumbnail), echoed back so the UI can route results. Text requests (`RenderThread::text`) have no generation and are never dropped.
* **Tiles**: a page whose longest side exceeds 4096 device pixels is drawn as a backdrop of at most 2048 px plus 512×512 tiles over the part on screen (`RenderRequest::tile`), so only the visible part is rasterised at full resolution. The UI keeps tiles keyed by `(page, scale, tile)` while they are on screen and at the current zoom.
* **Text boxes** are returned in displayed page space (see Coordinates), mapped through PDFium's own page-to-device transform so crop boxes and `/Rotate` match the bitmaps.
* **Backends** are cargo features: `pdfium` (default), `hayro` (pure Rust, experimental). Switching backend must need zero changes outside this crate.

### recto-app: the interface

* **Current state (M5)**: one `Viewer` entity per window shows one document (`viewer.rs`). Submodules: `sidebar` and `rail` (thumbnails, outline, multi-select, drag to reorder), `search`, `selection`, `files` (open, recent, drop), `password`, `edit` (ops, background rebuilds, undo/redo, `--apply`), `output` (save, save as, export, extract, split), `dialog`, `annotate` (tools, including redact and crop), `signature`, `forms`, `tools` (watermark, numbers, header/footer, properties, export forms), `ocr` (pages rendered with `Purpose::Ocr`, read by `recto-ocr` in the background, one `AddTextLayer` per page), `textedit` (the Edit text tool; `fontmatch.rs` picks an installed font with `fontdb` when the run's own cannot show the new text). After each edit the document is rebuilt on the background executor; the newest build replaces the render thread, keeping the bitmaps of pages whose state and ops did not change, and lays out at once from `recto-core` geometry. The `Workspace`/`DocumentTab` structure below remains the target for tabs.
* One `Workspace` entity per window, holding one or more `DocumentTab`s.
* A `DocumentTab` owns a `recto_core::Document` and a handle to the render thread. Views:
  * `PageCanvas`: the virtualised, continuously scrolling page list. Only visible pages (plus a margin) are requested.
  * `ThumbnailRail`: the sidebar, also the drag-and-drop surface for reordering and merging.
  * `Toolbar` + `Inspector`: the active tool and its options.
* **Tools** are a state machine (`Tool::Select | Text | Highlight | Shape | Sign | Redact | …`). A tool turns pointer events into `Op`s. It never edits the document directly.
* **Slow work** (build, save, OCR, compress) runs on `cx.background_executor()` and reports back with `this.update(cx, …)`.
* **Text** goes through `i18n::t`. Source strings are English. French is the first translation (selected by `RECTO_LANG`, `LC_ALL` or `LANG` for now; system locale detection comes later).
* **Recent files** live in the per-user config folder (`%APPDATA%\Recto`, `~/Library/Application Support/Recto`, `$XDG_CONFIG_HOME/recto`); `RECTO_CONFIG_DIR` overrides it.

## Coordinates

Four spaces; the page-level ones are defined in `recto-core::geom`, the continuous layout in `recto-core::layout`:

| Space | Unit | Origin | Used by |
|---|---|---|---|
| PDF user space | point (1/72 in) | bottom-left, per page `MediaBox`/`CropBox` | everything stored in `Op`s (`geom::Rect`) |
| Displayed page space | point | top-left of the page as shown, `/Rotate` applied | text boxes, highlights (`geom::PageRect`) |
| Page pixel space | px at render scale | top-left | bitmaps, tiles |
| Window space | GPUI logical px | top-left of the window; content space adds the scroll offset | pointer events, layout (`layout::PageLayout`) |

`Op`s only ever store PDF user space, so edits survive zoom, rotation and DPI changes.

## Data flow

```
pointer / key ──> Tool ──> Op ──> Document.apply() ──> op log + undo cursor
                                         │
                                         └─> background build() ──> new bytes ──> render thread reopen
                                                                                     │
PageCanvas ──> RenderRequest (visible pages, scale) ──────────────────────────────────┘──> Bitmap ──> cx.notify()
```

## Conventions

* Code, comments and commit messages in English.
* No `unwrap()` on anything that touches a user file. Errors surface in the UI as a toast with the cause.
* Irreversible actions (overwrite original, saving redactions) ask first. Redaction removes the content, never only covers it; the one exception, vector paths crossing an area edge, is documented in `redact.rs`.
* Test flags on the binary, for screenshots and agents: `recto [file.pdf] [--edit] [--page=N] [--zoom=PCT] [--sidebar=thumbnails|outline] [--tool=NAME] [--find=TEXT] [--apply=EDITS] [--save-as=PATH] [--export=PATH …] [--ocr] [--dark|--light]`. Without `--zoom` the page fits the window width. `--apply` takes comma-separated edits: `rotate:N`, `delete:N`, `move:N>M`, `blank:N`, `undo`, `fill:NAME=VALUE`, and annotations in displayed page points (`rectangle:P:X:Y:W:H`, `ellipse:…`, `highlight:…`, `arrow:P:X1:Y1:X2:Y2`, `line:…`, `freehand:P:X:Y:X:Y…`, `text:P:X:Y:TEXT`), `redact:P:X:Y:W:H`, `crop:P:X:Y:W:H`, `watermark:TEXT`, `numbers`, `header:TEXT`, `footer:TEXT`, `title:TEXT`, `edit:P:OLD=NEW`. `--save-as` writes the result at start; `--export=PATH` writes a copy with `--compress`, `--grayscale`, `--flatten`, `--password=PASS`; `--ocr` recognises every page; `--menu=pages|document` and `--dialog=export|watermark|numbers|header|properties|extract|split` open a menu or a form at start.

## Platform notes

| | Windows | macOS | Linux |
|---|---|---|---|
| PDFium | `pdfium.dll` next to `recto.exe` | `libpdfium.dylib` in `Recto.app/Contents/Frameworks` | `libpdfium.so` next to the binary or in `/usr/lib/recto` |
| Package | MSI or Inno Setup | `.dmg`, signed and notarised | AppImage, then Flatpak |
| File association | HKCU registry | `Info.plist` document types | `.desktop` file + MIME |

Prebuilt PDFium binaries come from [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries), version pinned in `xtask`.
