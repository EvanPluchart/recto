# Feature matrix

Reference point: the tools offered by Sejda (online and desktop). Recto does not copy its interface; this list only tracks coverage.

Legend: milestone where it lands, or `later` / `no`.

| Area | Feature | Milestone | Engine |
|---|---|---|---|
| View | Render, zoom, scroll, thumbnails | M0–M1 | PDFium |
| View | Outline, search, select and copy text | M1 | PDFium |
| Organize | Merge | M2 | lopdf |
| Organize | Split (range, every N, by bookmark, by size) | M2 | lopdf |
| Organize | Reorder, rotate, delete, extract, insert blank | M2 | lopdf |
| Organize | Alternate and mix (interleave two scans) | later | lopdf |
| Organize | Crop pages | M4 | lopdf |
| Organize | Resize pages | later | lopdf |
| Annotate | Text, images, shapes, freehand | M3 | lopdf + pdf-writer |
| Annotate | Highlight, underline, strikeout | M3 | lopdf |
| Annotate | Sign (drawn, typed, image) | M3 | lopdf + pdf-writer |
| Forms | Fill AcroForm fields | M3 | lopdf (decision 0003) |
| Forms | Flatten | M3 | lopdf (decision 0003) |
| Forms | Create form fields | later | lopdf |
| Tools | Compress | M4 | lopdf + image |
| Tools | Grayscale | M4 | lopdf |
| Tools | Watermark, page numbers, headers and footers | M4 | pdf-writer |
| Tools | Bates numbering | later | pdf-writer |
| Security | Protect, unlock | M4 | lopdf |
| Security | Redact | M4 | lopdf (content removal) |
| Tools | Document properties (metadata) | M4 | lopdf |
| Text | OCR | M4 | ocrs, in recto-ocr (decision 0004) |
| Edit | Edit existing text (beta) | M5 | lopdf, ttf-parser, subsetter; fontdb in the app (decision 0005) |
| Convert | PDF to images, images to PDF | later | PDFium / pdf-writer |
| Convert | PDF to Word / Excel | no | out of scope |
| Compare | Diff two PDFs | later | PDFium text |

## Next: gaps found against Sejda's editor (2026-10-22)

| Area | Feature | Status |
|---|---|---|
| Annotate | Select, move, resize, delete annotations after they are made | done for annotations made in Recto (Select tool, edit mode); not for those already in the file |
| Edit | Whiteout (cover in white) | missing (redaction paints black) |
| Edit | Delete an existing image | missing |
| Sign | Several handwriting styles and ink colours, signature from an image | missing (one typed style, black) |
| Text | Find and replace | missing |
| Annotate | Freehand highlighter, hide annotations | missing |
| Annotate | Stamps placed by hand ("DRAFT", "PAID") | partial (watermark only) |
| Links | Add and edit links | missing |
| Forms | Symbols (✕ ✓ •), create fields, detect fields, tab order | missing |
