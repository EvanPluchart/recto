# Recto

A fast, offline PDF viewer and editor for Windows, macOS and Linux, written in Rust.

Recto aims at the everyday PDF jobs that usually send people to an online service: merge, split, reorder, rotate, annotate, sign, fill forms, compress, watermark, protect, redact. Files never leave your machine, and there is no daily task limit.

> **Status:** architecture stage. Nothing is usable yet. See [docs/ROADMAP.md](docs/ROADMAP.md).

## Planned features

* **View**: smooth scrolling, zoom, thumbnails, outline, text search and selection.
* **Organize**: merge, split, reorder by drag and drop, rotate, delete, extract, insert blank pages.
* **Annotate**: text, images, shapes, highlight, freehand, drawn signatures, form filling.
* **Tools**: compress, watermark, page numbers, headers and footers, password protect and unlock, redact, OCR.
* **Edit**: change existing text in place (late milestone).

The full feature matrix is in [docs/FEATURES.md](docs/FEATURES.md).

## Stack

* [GPUI](https://crates.io/crates/gpui) for the interface (GPU-rendered, cross-platform).
* [PDFium](https://pdfium.googlesource.com/pdfium/) through [`pdfium-render`](https://crates.io/crates/pdfium-render) for page rendering and forms.
* [`lopdf`](https://crates.io/crates/lopdf) and [`pdf-writer`](https://crates.io/crates/pdf-writer) for document manipulation, in pure Rust.

Why these choices: [docs/decisions](docs/decisions).

## Building

```sh
cargo run -p recto -- path/to/file.pdf
```

PDFium is fetched by `cargo xtask fetch-pdfium` (milestone M0, not written yet).

## Documentation

* [Architecture](docs/ARCHITECTURE.md)
* [Roadmap](docs/ROADMAP.md)
* [Feature matrix](docs/FEATURES.md)
* [Decision records](docs/decisions)

## License

MIT. See [LICENSE](LICENSE).
