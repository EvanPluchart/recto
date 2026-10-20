# 0001 — PDF engine: PDFium for rendering, pure Rust for editing

**Status:** accepted, 2026-10-20

## Context

Recto must render any real-world PDF correctly on three systems, and edit documents without corrupting them. The project is MIT-licensed and wants to stay as close to pure Rust as practical.

## Options

| Option | Rendering quality | Licence | Pure Rust | Notes |
|---|---|---|---|---|
| PDFium via `pdfium-render` | excellent (Chrome's engine) | BSD / Apache | no, ships a native library | also handles forms and text extraction |
| `hayro` | good, improving | MIT / Apache | yes | young; less coverage of broken files |
| MuPDF | excellent | **AGPL** or commercial | no | would force Recto to AGPL; rejected |
| Platform APIs (Windows.Data.Pdf, PDFKit, Poppler) | varies | varies | no | three code paths, three sets of bugs; rejected |

## Decision

* **Rendering, text extraction, forms:** PDFium, behind the `Renderer` trait in `recto-render`.
* **Structural editing and writing:** `lopdf` + `pdf-writer`, pure Rust.
* **`hayro`** is kept as an optional backend behind a cargo feature, to evaluate a pure-Rust build later.

## Consequences

* Each release bundles a PDFium binary per platform (pinned version from bblanchon/pdfium-binaries, fetched by `xtask`).
* PDFium is not thread-safe: one render thread owns it (see ARCHITECTURE.md).
* The document model must never depend on PDFium types, so the engine can be swapped.
