# Roadmap

Each milestone ends with something you can run and show. Estimates are evenings of side-project time.

## M0 — Skeleton (1 week) — done 2026-10-21

* Workspace compiles on Windows, macOS and Linux (GitHub Actions matrix).
* `cargo xtask fetch-pdfium` downloads the pinned PDFium build for the host.
* `recto file.pdf` opens a window and shows page 1, rendered by the render thread.

**Done when:** the same command shows a PDF on all three systems in CI screenshots or by hand.

**Status:** CI is green on Windows, macOS and Linux (build, tests, PDFium render of the fixture). The window was checked by hand on Windows only; macOS and Linux still need a manual look.

## M1 — Viewer (2–3 weeks) — implemented, not hand-checked

* Continuous scroll, zoom (fit width, fit page, %), tiles at high zoom.
* Thumbnail rail, outline (bookmarks), page number field.
* Text search with highlighted hits, text selection and copy.
* Open recent, drag a file onto the window, password-protected files.
* Light and dark themes.

**Done when:** a 500-page PDF scrolls without stutter and search finds every hit.

**Status (2026-10-21):** every item is implemented on `dev`, CI green on the three systems. Checked by screenshots on Windows: continuous view, zoom modes, tiles at 800 %, thumbnails, outline, page field, search on a 500-page file (`cargo xtask gen-large`, 11 hits for "page 25", as expected), password prompt, start screen with recent files, light and dark themes. Not yet checked by hand: scrolling smoothness, mouse and keyboard interaction (selection, Ctrl+wheel, typing in fields, drag and drop, the file dialog), and macOS/Linux. Known limits: text fields take no IME input yet; French UI only through `RECTO_LANG`/`LANG`.

## M2 — Organize (2–3 weeks) — current

* Op log, undo/redo, atomic save / save as.
* Rotate, delete, reorder (drag in the rail), extract, insert blank page.
* Merge: drop other PDFs into the rail. Split: by range, every N pages, by bookmark.
* Multi-select in the rail.

**Done when:** every op round-trips (save, reopen, same result) and has an integration test. **First public release (0.1).**

## M3 — Annotate (3–4 weeks)

* Add text, image, rectangle, ellipse, line, arrow, freehand, highlight, underline, strikeout.
* Drawn and typed signatures, stored for reuse.
* Form filling (AcroForm) through PDFium, flatten on save as an option.
* Annotations are real PDF annotations, editable in other readers.

## M4 — Tools (3–4 weeks)

* Compress (image downsampling + stream recompression), grayscale.
* Watermark, page numbers, headers and footers.
* Protect (password, permissions) and unlock.
* Redact (true content removal), metadata editor.
* OCR with `ocrs`, producing an invisible text layer.

## M5 — Edit existing text (1–2 months)

* Detect text runs, edit in place, re-encode with the original font when its glyphs allow it, fall back to a matching system font otherwise.
* This is the hardest feature. Ship it as "beta".

## M6 — Distribution

* Windows installer, signed macOS `.dmg`, AppImage.
* File associations, "Open with Recto".
* Landing page on evanpluchart.fr.

## Later, maybe

PDF to images, images to PDF, compare two PDFs, n-up printing, digital signatures (PAdES), batch processing from a CLI (`recto merge a.pdf b.pdf -o out.pdf`).
