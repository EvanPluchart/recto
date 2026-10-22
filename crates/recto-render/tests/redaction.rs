//! Redaction by `recto-core`, checked with PDFium: the text is gone, its neighbours did not move,
//! the areas are black. Needs `cargo xtask fetch-pdfium`. One test per file: PDFium has
//! process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::annot::{Font, text_width};
use recto_core::geom::{PageRect, Point, Rect};
use recto_core::op::Op;
use recto_core::text::PageText;
use recto_core::{Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

/// A4 height: displayed y = 842 - user y on the fixture's unrotated pages.
const HEIGHT: f32 = 842.0;

fn open(thread: &RenderThread, bytes: Vec<u8>) {
    thread.open(Arc::from(bytes), None);
    assert!(matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_))));
}

fn text(thread: &RenderThread, index: usize) -> PageText {
    thread.text(index);
    match thread.responses().recv().unwrap() {
        Response::Text { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
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

fn rgb(bitmap: &Bitmap, x: f32, y: f32) -> [u8; 3] {
    let i = ((y as usize) * bitmap.width as usize + x as usize) * 4;
    [bitmap.pixels[i + 2], bitmap.pixels[i + 1], bitmap.pixels[i]]
}

/// Boxes of the characters of the first occurrence of `word`.
fn boxes(page: &PageText, word: &str) -> Vec<PageRect> {
    let chars: String = page.chars.iter().collect();
    let start = chars.find(word).unwrap_or_else(|| panic!("{word} not in {chars:?}"));
    let start = chars[..start].chars().count();
    page.boxes[start..start + word.chars().count()].to_vec()
}

fn area(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
    Rect { origin: Point { x: x0, y: y0 }, width: x1 - x0, height: y1 - y0 }
}

#[test]
fn redacted_content_is_gone_and_neighbours_stay() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/redact.pdf");
    let mut document = Document::open(&fixture).unwrap();
    open(&thread, document.build().unwrap());
    let before = text(&thread, 0);

    // ALPHATOKEN's area comes from PDFium's own boxes, shrunk a little: recto-core must agree
    // with PDFium on where the glyphs are, or it removes too little or too much.
    let alpha = boxes(&before, "ALPHATOKEN").into_iter().reduce(|a, b| a.union(&b)).unwrap();
    let helvetica = |text: &str, size: f32| text_width(text, size, Font::Helvetica);
    let areas = [
        area(alpha.x + 0.5, HEIGHT - alpha.bottom() + 0.5, alpha.right() - 0.5, HEIGHT - alpha.y - 0.5),
        area(95.0, 718.0, 165.0, 728.0),
        area(56.0 + helvetica("Hidden ", 12.0) + 1.0, 678.0, 56.0 + helvetica("Hidden GOLF", 12.0) - 1.0, 688.0),
        area(300.0, 501.0, 399.0, 599.0),
        area(60.0, 505.0, 100.0, 515.0),
        area(56.0 + helvetica("Form ", 12.0) + 1.0, 400.0, 56.0 + helvetica("Form CHARLIETOKEN", 12.0) - 1.0, 410.0),
    ];
    for area in areas {
        document.apply(Op::Redact { page: PageId { source: 0, index: 0 }, area });
    }
    open(&thread, document.build().unwrap());

    let after = text(&thread, 0);
    let shown: String = after.chars.iter().collect();
    for token in ["ALPHATOKEN", "BRAVOTOKEN", "GOLF", "CHARLIETOKEN"] {
        assert!(!shown.contains(token), "{token} still shows: {shown:?}");
    }
    for word in ["Name: Alice", "Smith", "Code", "end", "Hidden", "word", "Form", "here"] {
        assert!(shown.contains(word), "{word} is missing: {shown:?}");
    }
    // The words after the removed ones did not move.
    // ("word" is not in the text before: PDFium reads the marked content's /ActualText instead.)
    for word in ["Smith", "end", "here"] {
        for (old, new) in boxes(&before, word).iter().zip(boxes(&after, word)) {
            assert!((old.x - new.x).abs() < 0.01 && (old.y - new.y).abs() < 0.01, "{word} moved: {old:?} -> {new:?}");
        }
    }
    let second: String = text(&thread, 1).chars.iter().collect();
    assert!(second.contains("CHARLIETOKEN") && second.contains("HOTELTOKEN"), "page 2 is untouched: {second:?}");

    let bitmap = render(&thread, 0);
    for area in areas {
        let center = (area.origin.x + area.width / 2.0, HEIGHT - area.origin.y - area.height / 2.0);
        assert_eq!(rgb(&bitmap, center.0, center.1), [0, 0, 0], "area {area:?} is black");
    }
    // The right half of the image is kept.
    assert_eq!(rgb(&bitmap, 450.0, HEIGHT - 550.0), [0x80, 0x80, 0x80]);
}
