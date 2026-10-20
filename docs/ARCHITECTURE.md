# Architecture

Recto is a Cargo workspace with three library layers and one binary. The rule that shapes everything: **the UI never touches PDF bytes, and the PDF layers never know about the UI.**

```
crates/
  recto-core     document model, edit operations, undo/redo, save        (pure Rust, no UI)
  recto-render   Renderer trait + backends (PDFium now, hayro later)     (no UI)
  recto-app      GPUI application, binary "recto"                        (depends on both)
xtask/           fetch-pdfium, bundle, release                            (M0)
tests/fixtures/  small PDFs used by integration tests
```

## Layers

### recto-core: the document model

* `Document` holds the **original bytes** plus an ordered **operation log** (`Vec<Op>`). The original file is never modified in memory.
* `Op` is an enum of edits: `RotatePages`, `DeletePages`, `MovePages`, `InsertPages { from: SourceDoc }`, `AddAnnotation`, `FillField`, `Redact`, … Each op is serialisable, so an edit session can be saved and replayed.
* **Undo/redo** moves a cursor in the op log. No snapshots.
* **Page identity**: pages are addressed by a stable `PageId` (source document + original index), never by their current position. Reordering then never invalidates selections, thumbnails or caches.
* **Materialisation**: `Document::build() -> Vec<u8>` replays the op log on a `lopdf::Document` and writes a new PDF. The renderer is fed the materialised bytes (cached, rebuilt in the background after each op).
* **Saving** is atomic: write to a temp file in the same folder, fsync, rename over the target.
* Features that need new PDF content (watermarks, page numbers, stamped text) generate content streams with `pdf-writer` and merge them through `lopdf`.

### recto-render: the renderer

```rust
pub trait Renderer: Send {
    fn open(&mut self, bytes: Arc<[u8]>, password: Option<&str>) -> Result<DocInfo>;
    fn render(&mut self, req: RenderRequest) -> Result<Bitmap>;      // page, scale, rotation, clip
    fn text(&mut self, page: usize) -> Result<Vec<TextSpan>>;        // glyph boxes for selection/search
    fn form_fields(&mut self, page: usize) -> Result<Vec<FormField>>;
}
```

* **PDFium is not thread-safe.** One dedicated render thread owns the `Pdfium` instance. The UI sends `RenderRequest`s over a `flume` channel and receives bitmaps back. Requests carry a generation number; stale ones (old zoom, scrolled away) are dropped before rendering.
* **Tiles**: at high zoom a page is rendered as 512×512 tiles, so only the visible part is rasterised. An LRU cache keyed by `(PageId, scale bucket, tile)` holds results.
* **Backends** are cargo features: `pdfium` (default), `hayro` (pure Rust, experimental). Switching backend must need zero changes outside this crate.

### recto-app: the interface

* One `Workspace` entity per window, holding one or more `DocumentTab`s.
* A `DocumentTab` owns a `recto_core::Document` and a handle to the render thread. Views:
  * `PageCanvas`: the virtualised, continuously scrolling page list. Only visible pages (plus a margin) are requested.
  * `ThumbnailRail`: the sidebar, also the drag-and-drop surface for reordering and merging.
  * `Toolbar` + `Inspector`: the active tool and its options.
* **Tools** are a state machine (`Tool::Select | Text | Highlight | Shape | Sign | Redact | …`). A tool turns pointer events into `Op`s. It never edits the document directly.
* **Slow work** (build, save, OCR, compress) runs on `cx.background_executor()` and reports back with `this.update(cx, …)`.
* **Text** goes through `i18n::t`. Source strings are English. French is the first translation.

## Coordinates

Three spaces, one conversion module (`recto-core::geom`):

| Space | Unit | Origin | Used by |
|---|---|---|---|
| PDF user space | point (1/72 in) | bottom-left, per page `MediaBox`/`CropBox` | everything stored in `Op`s |
| Page pixel space | px at render scale | top-left | bitmaps, tiles |
| Window space | GPUI logical px | top-left of the window | pointer events, layout |

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
* Irreversible actions (overwrite original, redact) ask first. Redaction removes the content, never only covers it.
* Test flags on the binary, for screenshots and agents: `recto [file.pdf] [--page=N] [--zoom=PCT] [--tool=NAME] [--dark|--light]`.

## Platform notes

| | Windows | macOS | Linux |
|---|---|---|---|
| PDFium | `pdfium.dll` next to `recto.exe` | `libpdfium.dylib` in `Recto.app/Contents/Frameworks` | `libpdfium.so` next to the binary or in `/usr/lib/recto` |
| Package | MSI or Inno Setup | `.dmg`, signed and notarised | AppImage, then Flatpak |
| File association | HKCU registry | `Info.plist` document types | `.desktop` file + MIME |

Prebuilt PDFium binaries come from [bblanchon/pdfium-binaries](https://github.com/bblanchon/pdfium-binaries), version pinned in `xtask`.
