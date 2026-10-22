//! Save-time options of `recto-core` (protection, grayscale, compression), as PDFium opens and
//! draws the results. Needs `cargo xtask fetch-pdfium`. One test per file: PDFium has
//! process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::security::{Allowed, Protection, Security};
use recto_core::{BuildOptions, Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

fn opens(thread: &RenderThread, bytes: &[u8], password: Option<&str>) -> bool {
    thread.open(Arc::from(bytes), password);
    matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_)))
}

fn render(thread: &RenderThread, index: usize) -> Bitmap {
    let generation = thread.next_generation();
    let page = PageId { source: 0, index: index as u32 };
    thread.request(RenderRequest { page, index, scale: 1.0, generation, purpose: Purpose::Page, tile: None });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// Pixels whose channels differ by more than `tolerance`.
fn colored(bitmap: &Bitmap, tolerance: u8) -> usize {
    bitmap
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| {
            let (low, high) = (pixel[..3].iter().min().unwrap(), pixel[..3].iter().max().unwrap());
            high - low > tolerance
        })
        .count()
}

#[test]
fn saved_options_open_and_draw_in_pdfium() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let document = Document::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/sample.pdf")).unwrap();

    let protection = Protection { user_password: "user".into(), owner_password: "owner".into(), allowed: Allowed::default() };
    let protected = document.build_with(&BuildOptions { security: Security::Protect(protection), ..BuildOptions::default() }).unwrap();
    assert!(!opens(&thread, &protected, None), "no password");
    assert!(!opens(&thread, &protected, Some("wrong")));
    assert!(opens(&thread, &protected, Some("user")));
    assert!(opens(&thread, &protected, Some("owner")));

    assert!(opens(&thread, &document.build().unwrap(), None));
    assert!(colored(&render(&thread, 0), 8) > 1000, "the sample has color to remove");
    let gray = document.build_with(&BuildOptions { grayscale: true, ..BuildOptions::default() }).unwrap();
    assert!(opens(&thread, &gray, None));
    let bitmap = render(&thread, 0);
    assert_eq!(colored(&bitmap, 2), 0, "every pixel is gray");
    assert!(bitmap.pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[0] < 128), "content is still drawn");

    let compressed = document.build_with(&BuildOptions { compress: true, ..BuildOptions::default() }).unwrap();
    assert!(opens(&thread, &compressed, None));
    let reference = {
        assert!(opens(&thread, &document.build().unwrap(), None));
        render(&thread, 0)
    };
    assert!(opens(&thread, &compressed, None));
    let small = render(&thread, 0);
    // The sample's image is drawn below 150 ppi, so it is kept: pages look the same.
    let differing = reference.pixels.iter().zip(&small.pixels).filter(|(a, b)| a.abs_diff(**b) > 8).count();
    assert!(differing < reference.pixels.len() / 100, "{differing} channels differ");
}
