//! Stamps (page numbers, watermarks) and crop boxes written by `recto-core`, as PDFium shows
//! them. Needs `cargo xtask fetch-pdfium`. One test per file: PDFium has process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::geom::{PageRect, Point, Rect, Size};
use recto_core::op::Op;
use recto_core::stamp::Stamp;
use recto_core::text::PageText;
use recto_core::{Document, PageId};
use recto_render::{DocInfo, PdfiumRenderer, RenderThread, Response};

fn open(thread: &RenderThread, bytes: Vec<u8>) -> DocInfo {
    thread.open(Arc::from(bytes), None);
    match thread.responses().recv().unwrap() {
        Response::Opened(info) => info.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn text(thread: &RenderThread, index: usize) -> PageText {
    thread.text(index);
    match thread.responses().recv().unwrap() {
        Response::Text { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// Box around the first occurrence of `word`.
fn bounds(page: &PageText, word: &str) -> PageRect {
    let chars: String = page.chars.iter().collect();
    let start = chars.find(word).unwrap_or_else(|| panic!("{word} not in {chars:?}"));
    let start = chars[..start].chars().count();
    page.boxes[start..start + word.chars().count()].iter().fold(PageRect::default(), |all, item| all.union(item))
}

#[test]
fn stamps_sit_where_the_page_shows_them() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/layout.pdf");
    let mut document = Document::open(&fixture).unwrap();
    let page = |index| PageId { source: 0, index };
    document.apply(Op::AddStamp(Stamp::page_numbers()));
    document.apply(Op::AddStamp(Stamp { pages: Some(vec![page(0)]), ..Stamp::watermark("DRAFT") }));
    let crop = Rect { origin: Point { x: 100.0, y: 100.0 }, width: 300.0, height: 400.0 };
    document.apply(Op::CropPages { pages: vec![page(3)], area: crop });
    let info = open(&thread, document.build().unwrap());
    assert_eq!(info.page_sizes[3], Size { width: 300.0, height: 400.0 }, "the crop box is what shows");

    // Every number at the bottom centre of the page as displayed, reading left to right, also
    // on page 3 (A4 portrait with /Rotate 90, shown landscape) and on the cropped page 4.
    for (index, label) in ["1 / 4", "2 / 4", "3 / 4", "4 / 4"].into_iter().enumerate() {
        let size = info.page_sizes[index];
        let found = bounds(&text(&thread, index), label);
        let center = found.x + found.width / 2.0;
        assert!((center - size.width / 2.0).abs() < 3.0, "page {index}: {found:?} not centred in {size:?}");
        assert!(found.bottom() < size.height - 20.0 && found.bottom() > size.height - 40.0, "page {index}: {found:?} not at the bottom");
        assert!(found.width > found.height, "page {index}: {found:?} does not read across");
    }
    // The watermark crosses the middle of page 1.
    let size = info.page_sizes[0];
    let draft = bounds(&text(&thread, 0), "DRAFT");
    let (x, y) = (draft.x + draft.width / 2.0, draft.y + draft.height / 2.0);
    assert!((x - size.width / 2.0).abs() < 5.0 && (y - size.height / 2.0).abs() < 5.0, "{draft:?}");
    assert!((draft.width - draft.height).abs() < 0.2 * draft.width, "45 degrees: {draft:?}");
}
