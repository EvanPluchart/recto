# 0003 — Form filling and flattening in recto-core, not through PDFium

**Status:** accepted, 2026-10-22. Amends 0001 for forms.

## Context

Decision 0001 assigns "rendering, text extraction, forms" to PDFium. When implementing M3, writing form values through PDFium turned out to clash with the rest of the architecture:

* Edits are `Op`s replayed by `recto-core` on the original bytes (`Document::build`). A form value set through PDFium would live in the render thread's in-memory document, outside the op log: undo/redo, merge, reorder and save would each need a second path.
* `recto-core` must stay free of PDFium (CLAUDE.md), and only the render thread may touch PDFium. Saving through PDFium would make the render thread a writer of user files.
* PDFium still has to *show* filled values. It does so from the field values and appearance streams in the bytes it is given.

## Decision

* `recto-core` reads AcroForm fields with `lopdf` (name, type, value, options, widget rectangles in user space) and records `Op::FillField { name, value }`.
* `Document::build` writes the value (`/V`, `/AS` for check boxes and radio buttons) and generates a plain appearance stream for text and choice widgets with `pdf-writer` (Helvetica, the field's size or 11 pt), so every reader shows the value without regenerating appearances.
* Flattening is a build option: each widget's normal appearance is drawn into the page content and the widgets and the `/AcroForm` entry are removed.
* PDFium keeps rendering pages (with form data) and is not used to write.

## Consequences

* One write path: forms follow undo/redo, reorder, merge and atomic save like every other op.
* Generated appearances are simpler than PDFium's or Acrobat's (no comb fields, no multi-line wrapping beyond line breaks, no rich text, base-14 fonts only). Values outside Latin-1 may not render with Helvetica.
* JavaScript actions (calculations, formatting) are not run.
