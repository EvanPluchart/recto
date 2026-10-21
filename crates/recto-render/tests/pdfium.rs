//! Renders the generated fixture through PDFium. Needs `cargo xtask fetch-pdfium` first.
//! Keep every PDFium check in this one test: the library has process-wide state.

use std::sync::Arc;

use recto_core::PageId;
use recto_render::{PdfiumRenderer, RenderRequest, RenderThread, Response};

#[test]
fn renders_page_one_of_the_fixture() {
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/fixtures/sample.pdf")).unwrap();
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");

    thread.open(Arc::from(bytes), None);
    match thread.responses().recv().unwrap() {
        Response::Opened(count) => assert_eq!(count.unwrap(), 3),
        other => panic!("unexpected {other:?}"),
    }

    let generation = thread.next_generation();
    let request = RenderRequest { page: PageId { source: 0, index: 0 }, index: 0, scale: 1.0, generation };
    thread.request(request);
    let bitmap = match thread.responses().recv().unwrap() {
        Response::Rendered { request: answered, result } => {
            assert_eq!(answered, request);
            result.unwrap()
        }
        other => panic!("unexpected {other:?}"),
    };

    // A4 at one pixel per point.
    assert_eq!((bitmap.width, bitmap.height), (595, 842));
    assert_eq!(bitmap.pixels.len(), 595 * 842 * 4);
    let pixel = |x: usize, y: usize| -> [u8; 4] {
        let i = (y * 595 + x) * 4;
        bitmap.pixels[i..i + 4].try_into().unwrap()
    };
    assert!(bitmap.pixels.as_chunks::<4>().0.iter().any(|p| p[..3] != [255, 255, 255]), "page 1 is blank");
    // The blue band drawn at the top of page 1, in RGBA order.
    let [r, g, b, a] = pixel(300, 110);
    assert!(b > r + 100 && b > g + 50 && a == 255, "expected blue, got {:?}", [r, g, b, a]);

    // Past the last page: an error, not a panic.
    let generation = thread.next_generation();
    thread.request(RenderRequest { index: 3, generation, ..request });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => assert!(result.is_err()),
        other => panic!("unexpected {other:?}"),
    }
}
