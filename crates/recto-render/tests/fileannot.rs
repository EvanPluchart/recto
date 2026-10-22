//! Annotations already in a file, moved by `recto-core`, as PDFium draws them. Needs
//! `cargo xtask fetch-pdfium`. One test per file: PDFium has process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::fileannot::FileChange;
use recto_core::geom::{PageRect, Point, Rect};
use recto_core::op::Op;
use recto_core::{Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

fn render(thread: &RenderThread, bytes: Vec<u8>) -> Bitmap {
    thread.open(Arc::from(bytes), None);
    assert!(matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_))));
    let page = PageId { source: 0, index: 0 };
    let generation = thread.next_generation();
    thread.request(RenderRequest { page, index: 0, scale: 1.0, generation, purpose: Purpose::Page, tile: None });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// Pixels that are not white inside `area` (displayed points, one pixel per point).
fn inked(bitmap: &Bitmap, area: PageRect) -> usize {
    let (x0, y0) = (area.x.max(0.0) as usize, area.y.max(0.0) as usize);
    let (x1, y1) = ((area.right() as usize).min(bitmap.width as usize), (area.bottom() as usize).min(bitmap.height as usize));
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| {
            let i = (y * bitmap.width as usize + x) * 4;
            bitmap.pixels[i..i + 3].iter().any(|&c| c < 200)
        })
        .count()
}

#[test]
fn a_moved_annotation_of_the_file_is_drawn_at_its_new_place() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut document = Document::open(fixture.join("redact.pdf")).unwrap();
    let pages = document.pages();
    let (geometry, _, annotations) = document.geometry_and_fields(&pages).unwrap();
    let geometry = geometry[0];
    // The Square `KEEPNOTE` at [56 100 100 140], in a part of the page with nothing else.
    let square = *annotations[0].iter().find(|annotation| annotation.frame.origin.y == 100.0).expect("the square");
    let before = geometry.rect_to_display(square.frame);
    let to = Rect { origin: Point { x: 400.0, y: 100.0 }, width: 44.0, height: 40.0 };
    let after = geometry.rect_to_display(to);

    let original = render(&thread, document.build().unwrap());
    assert!(inked(&original, before) > 0, "the square is drawn");
    assert_eq!(inked(&original, after), 0, "nothing where it goes");

    document.apply(Op::EditFileAnnotation { page: PageId { source: 0, index: 0 }, index: square.index, change: FileChange::Frame(to) });
    let moved = render(&thread, document.build().unwrap());
    assert_eq!(inked(&moved, before), 0, "gone from its old place");
    assert!(inked(&moved, after) > 0, "drawn at its new place");

    document.apply(Op::EditFileAnnotation { page: PageId { source: 0, index: 0 }, index: square.index, change: FileChange::Delete });
    let deleted = render(&thread, document.build().unwrap());
    assert_eq!(inked(&deleted, after), 0, "deleted");
}
