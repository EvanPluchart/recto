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

## M2 — Organize (2–3 weeks) — implemented, not hand-checked

* Op log, undo/redo, atomic save / save as.
* Rotate, delete, reorder (drag in the rail), extract, insert blank page.
* Merge: drop other PDFs into the rail. Split: by range, every N pages, by bookmark.
* Multi-select in the rail.

**Done when:** every op round-trips (save, reopen, same result) and has an integration test. **First public release (0.1).**

**Status (2026-10-22):** every op round-trips in `recto-core` tests (lopdf) and in `recto-render/tests/edits.rs` (PDFium): rotate, delete, move, merge, blank page, subsets for extract/split, inherited page attributes, encrypted originals (re-encrypted with the same password). Screenshots on Windows through `--apply` and `--save-as`. Not hand-checked: rail clicks and drags, file dialogs, the confirmation prompts. Known limits: merging a password-protected file is refused; outline entries pointing at deleted pages keep those pages' objects in the file. The 0.1 release is not cut: it waits for a hand check.

## M3 — Annotate (3–4 weeks) — implemented, not hand-checked

* Add text, image, rectangle, ellipse, line, arrow, freehand, highlight, underline, strikeout.
* Drawn and typed signatures, stored for reuse.
* Form filling (AcroForm), flatten on save as an option (in recto-core, see decision 0003).
* Annotations are real PDF annotations, editable in other readers.

**Status (2026-10-22):** every annotation kind round-trips (`recto-core` tests read back `/Subtype`, `/Rect`, `/QuadPoints`, `/InkList` and the appearance) and PDFium draws each one where it was placed (`recto-render/tests/annotations.rs` checks pixels, including upright text on a `/Rotate 90` page). Form fields are read, filled, and flattened, checked the same two ways on `tests/fixtures/forms.pdf`. Screenshots on Windows through `--apply` (shapes, arrow, freehand, text, highlight, filled form). Not hand-checked: drawing with the mouse, the text editor, image and signature placement, the signature pad, clicking form fields. Known limits: annotations made in Recto can be picked, moved, resized and deleted (edit mode, Select tool), those already in the file cannot; text annotations and typed signatures use base-14 fonts (Latin-1 only); generated field appearances are plain (no comb or multi-line wrapping); form JavaScript is not run.

## M4 — Tools (3–4 weeks) — implemented, not hand-checked

* Compress (image downsampling + stream recompression), grayscale.
* Watermark, page numbers, headers and footers.
* Protect (password, permissions) and unlock.
* Redact (true content removal), metadata editor.
* OCR with `ocrs`, producing an invisible text layer (decision 0004, proposed).
* Crop pages.

**Status (2026-10-22):** every tool round-trips in `recto-core` tests and is checked with PDFium in `recto-render/tests/` (`redaction.rs`, `stamps.rs`, `output.rs`) and `recto-ocr/tests/scan.rs`. Redaction is tested on `tests/fixtures/redact.pdf`: no marked token is left in any string or stream of the output (text, kerned text, alternate text, image pixels, inline image, form XObject, annotation), the words after a removed one keep their PDFium boxes, and a form shared with another page is copied, not changed. OCR is tested by scanning a fixture page through PDFium and checking the text layer lands on the scanned words and stays invisible. Screenshots on Windows: redaction, watermark, OCR then search on a scan (upright, and sideways with `/Rotate 90`, with an edit made while the models load). Not hand-checked: the tool forms and the export dialog, dragging redaction and crop areas, macOS/Linux. Known limits: redaction removes vector paths only when they lie entirely inside an area (crossing paths are covered by the black box), removes whole strings when a font's code lengths are unknown, and drops the document's structure tree; grayscale leaves shadings and spot colors in color; compression skips JPEG 2000, JBIG2, CCITT and CMYK JPEG images; OCR recognises ASCII only (no accents), needs its models (`cargo xtask fetch-ocr-models`), and run on a page that already has text it adds a second copy (search finds both); stamps and the text layer use Helvetica (Latin-1); XMP metadata is dropped when properties are edited. Resizing pages moved to "later".

## M5 — Edit existing text (1–2 months) — implemented as beta, not hand-checked

* Detect text runs, edit in place, re-encode with the original font when its glyphs allow it, fall back to a matching system font otherwise (decision 0005, proposed).
* This is the hardest feature. Ship it as "beta".

**Status (2026-10-22):** runs are found and rewritten in `recto-core` (`textedit.rs`, `fonts.rs`, `embed.rs`), tested on `tests/fixtures/edit.pdf` (base-14 without widths, widths, embedded simple TrueType, Type0 Identity-H subset, the `'` operator, a run of two show operations) and checked with PDFium (`recto-render/tests/textedit.rs`): the run boxes hold PDFium's character boxes, the new text reads back and starts where the old one did, other lines do not move, fallback fonts embed and show. Screenshots on Windows through `--apply=edit:P:OLD=NEW`: same-font edits, a Latin fallback (Arial for a subset that lacked the letters) and a Japanese fallback from an installed font. Not hand-checked: clicking a line with the "Edit the PDF text (beta)" tool and typing in its editor; macOS and Linux font lookup. Known limits: no reflow (longer text overlaps what follows), kerning inside an edited run is lost, `Tw` does not apply to fallback fonts, text in form XObjects and annotations cannot be edited, Type 3 and vertical fonts cannot be edited, composite fonts only with Identity-H and a ToUnicode CMap.

## Interface — design pass — done on `dev`, not hand-checked

* One app bar (files, page navigation, zoom, history, Pages and Document menus, search, Save) and one tool bar (tools as icons in groups, colour and width), replacing four rows of text buttons.
* Lucide icons (ISC, `crates/recto-app/assets/`), tooltips with shortcuts, notices as a floating toast, a start screen with the recent files, a light theme in the brand colours (docs/brand.md), checked for WCAG contrast; the dark theme was removed at the owner's request (2026-10-22).
* Documents open read-only, like an image viewer: page navigation, zoom, search, selection and form filling. An Edit button shows the tool bar, the Pages and Document menus and enables page operations; Done goes back to reading and keeps the changes. `--edit` starts in edit mode (as do `--tool`, `--apply`, `--menu`, `--dialog`).

**Done when:** at most two toolbar rows at 900 px wide in French; every action reachable from a bar, a menu or a shortcut; the theme checked by screenshot.

**Status (2026-10-22):** met on Windows by screenshots (900 px and 660 px, French, light and dark, menus and forms open through `--menu` and `--dialog`). Not hand-checked: hover states and tooltips, opening menus with the mouse, macOS and Linux.

## M6 — Distribution

* Windows installer, signed macOS `.dmg`, AppImage.
* File associations, "Open with Recto".
* Landing page (GitHub Pages).

**Status (2026-10-22):** `cargo xtask dist` builds and packages for the host (`xtask/src/dist.rs`, files in `packaging/`); `.github/workflows/release.yml` runs it on the three systems for a `v*` tag or by hand and drafts a GitHub release. Checked on Windows: the Inno Setup installer installs per user without administrator rights, registers Recto under "Open with" for `.pdf` and in Default apps (without taking over the default viewer), starts with a file, and uninstalls without leaving files or registry keys. macOS (`Recto.app` with `CFBundleDocumentTypes`, `.dmg`, Finder "open document" events handled through `on_open_urls`) and Linux (AppDir with a `.desktop` for `application/pdf`, AppImage) are written but not yet run: they need the Release workflow. Signing: macOS takes a Developer ID certificate and notary credentials from repository secrets (none set yet, so builds are unsigned); Windows code signing needs a certificate, not chosen yet. The landing page is in `site/` (French, design system in `site/DESIGN.md`, screenshots of the app on `cargo xtask gen-demo`); `.github/workflows/pages.yml` publishes it by hand, once the repository is public and Pages is enabled. Not published yet.

## Later, maybe

PDF to images, images to PDF, compare two PDFs, n-up printing, digital signatures (PAdES), batch processing from a CLI (`recto merge a.pdf b.pdf -o out.pdf`).
