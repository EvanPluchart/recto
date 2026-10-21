//! Renders the generated fixtures through PDFium. Needs `cargo xtask fetch-pdfium` first.
//! Keep every PDFium check in this one test: the library has process-wide state.

use std::sync::Arc;

use recto_core::PageId;
use recto_render::{DocInfo, PdfiumRenderer, RenderRequest, RenderThread, Response, Size};

fn fixture(name: &str) -> Arc<[u8]> {
    let path = format!("{}/../../tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    Arc::from(std::fs::read(path).unwrap())
}

fn open(thread: &RenderThread, name: &str) -> DocInfo {
    thread.open(fixture(name), None);
    match thread.responses().recv().unwrap() {
        Response::Opened(info) => info.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

fn size(width: f32, height: f32) -> Size {
    Size { width, height }
}

#[test]
fn renders_the_fixtures() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");

    let info = open(&thread, "sample.pdf");
    assert_eq!(info.page_sizes, vec![size(595.0, 842.0); 3]);

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
    // The blue band drawn at the top of page 1, in BGRA order.
    let [b, g, r, a] = pixel(300, 110);
    assert!(b > r + 100 && b > g + 50 && a == 255, "expected blue, got {:?}", [r, g, b, a]);

    // Past the last page: an error, not a panic.
    let generation = thread.next_generation();
    thread.request(RenderRequest { index: 3, generation, ..request });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => assert!(result.is_err()),
        other => panic!("unexpected {other:?}"),
    }

    // Displayed sizes: landscape media box, `/Rotate 90` swaps the sides, US Letter.
    let info = open(&thread, "layout.pdf");
    assert_eq!(info.page_sizes, vec![size(595.0, 842.0), size(842.0, 595.0), size(842.0, 595.0), size(612.0, 792.0)]);

    // The rotated page renders with the displayed size, at a non-integer scale.
    let generation = thread.next_generation();
    thread.request(RenderRequest { page: PageId { source: 0, index: 2 }, index: 2, scale: 1.5, generation });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => {
            let bitmap = result.unwrap();
            assert_eq!((bitmap.width, bitmap.height), (1263, 893));
            assert_eq!(bitmap.pixels.len(), 1263 * 893 * 4);
        }
        other => panic!("unexpected {other:?}"),
    }
}
