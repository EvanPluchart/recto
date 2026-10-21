//! Annotations written by `recto-core`, as PDFium draws them. Needs `cargo xtask fetch-pdfium`.
//! One test per file: PDFium has process-wide state.

use std::path::Path;
use std::sync::Arc;

use recto_core::annot::{Annotation, Color, Font, Kind, Raster};
use recto_core::geom::{Point, Rect};
use recto_core::op::Op;
use recto_core::{Document, PageId};
use recto_render::{Bitmap, PdfiumRenderer, Purpose, RenderRequest, RenderThread, Response};

fn rect(x: f32, y: f32, width: f32, height: f32) -> Rect {
    Rect { origin: Point { x, y }, width, height }
}

fn render(thread: &RenderThread, bytes: Vec<u8>, index: usize) -> Bitmap {
    thread.open(Arc::from(bytes), None);
    assert!(matches!(thread.responses().recv().unwrap(), Response::Opened(Ok(_))));
    let page = PageId { source: 0, index: index as u32 };
    let generation = thread.next_generation();
    thread.request(RenderRequest { page, index, scale: 1.0, generation, purpose: Purpose::Page, tile: None });
    match thread.responses().recv().unwrap() {
        Response::Rendered { result, .. } => result.unwrap(),
        other => panic!("unexpected {other:?}"),
    }
}

/// RGB at a displayed position (BGRA bitmap, one pixel per point).
fn rgb(bitmap: &Bitmap, x: f32, y: f32) -> [u8; 3] {
    let i = ((y as usize) * bitmap.width as usize + x as usize) * 4;
    [bitmap.pixels[i + 2], bitmap.pixels[i + 1], bitmap.pixels[i]]
}

fn dark_pixels(bitmap: &Bitmap, x0: usize, y0: usize, x1: usize, y1: usize) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| rgb(bitmap, x as f32, y as f32).iter().all(|&c| c < 100))
        .count()
}

#[test]
fn annotations_render_where_they_were_placed() {
    let thread = RenderThread::spawn(PdfiumRenderer::new).expect("PDFium loads; run `cargo xtask fetch-pdfium`");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    let mut document = Document::open(fixture.join("sample.pdf")).unwrap();
    let page = PageId { source: 0, index: 2 };
    let add = |document: &mut Document, kind, color| {
        document.apply(Op::AddAnnotation { page, annotation: Annotation { kind, color, width: 4.0 } });
    };
    // Page 3 is A4 (842 pt high) with text only near the top: display y = 842 - user y.
    add(&mut document, Kind::Rectangle { rect: rect(100.0, 100.0, 200.0, 100.0) }, Color::RED);
    add(&mut document, Kind::Highlight { quads: vec![rect(100.0, 300.0, 200.0, 20.0)] }, Color::YELLOW);
    add(
        &mut document,
        Kind::Text { rect: rect(320.0, 100.0, 220.0, 50.0), text: "HELLO".into(), size: 30.0, font: Font::Helvetica, upright: 0 },
        Color::BLACK,
    );
    add(&mut document, Kind::Line { from: Point { x: 100.0, y: 400.0 }, to: Point { x: 300.0, y: 400.0 }, arrow: true }, Color::BLUE);
    add(&mut document, Kind::Ink { strokes: vec![vec![Point { x: 350.0, y: 400.0 }, Point { x: 500.0, y: 400.0 }]] }, Color::GREEN);
    let image = Arc::new(Raster { width: 2, height: 2, rgb: [0, 160, 255].repeat(4), alpha: None });
    add(&mut document, Kind::Image { rect: rect(400.0, 250.0, 60.0, 60.0), image, upright: 0 }, Color::BLACK);
    add(&mut document, Kind::Ellipse { rect: rect(100.0, 450.0, 200.0, 100.0) }, Color::RED);

    let bitmap = render(&thread, document.build().unwrap(), 2);
    let reddish = |[r, g, b]: [u8; 3]| r > 180 && g < 100 && b < 100;
    assert!(reddish(rgb(&bitmap, 102.0, 692.0)), "rectangle edge: {:?}", rgb(&bitmap, 102.0, 692.0));
    assert_eq!(rgb(&bitmap, 200.0, 692.0), [255, 255, 255], "the rectangle is not filled");
    let [r, g, b] = rgb(&bitmap, 200.0, 532.0);
    assert!(r > 200 && g > 170 && b < 120, "highlight: {:?}", [r, g, b]);
    assert!(dark_pixels(&bitmap, 320, 692, 540, 742) > 100, "text annotation drawn");
    let [r, g, b] = rgb(&bitmap, 200.0, 442.0);
    assert!(b > 150 && r < 120, "line: {:?}", [r, g, b]);
    let [r, g, b] = rgb(&bitmap, 420.0, 442.0);
    assert!(g > 100 && r < 100 && b < 120, "ink: {:?}", [r, g, b]);
    assert_eq!(rgb(&bitmap, 430.0, 562.0), [0, 160, 255], "image");
    assert!(reddish(rgb(&bitmap, 102.0, 342.0)), "ellipse left edge: {:?}", rgb(&bitmap, 102.0, 342.0));

    // Upright text on a /Rotate 90 page: wider than tall on screen, as typed.
    let mut rotated = Document::open(fixture.join("layout.pdf")).unwrap();
    let geometry = rotated.geometry().unwrap()[2];
    // A 300 x 60 box on screen, placed at display (100, 400).
    let area = geometry.rect_to_user(recto_core::geom::PageRect { x: 100.0, y: 400.0, width: 300.0, height: 60.0 });
    rotated.apply(Op::AddAnnotation {
        page: PageId { source: 0, index: 2 },
        annotation: Annotation {
            kind: Kind::Text { rect: area, text: "WIDE TEXT".into(), size: 36.0, font: Font::Helvetica, upright: geometry.rotate },
            color: Color::BLACK,
            width: 1.0,
        },
    });
    let bitmap = render(&thread, rotated.build().unwrap(), 2);
    // Upright text spreads along x on screen: dark pixels across the box's width, not its height.
    let left = dark_pixels(&bitmap, 100, 400, 250, 460);
    let right = dark_pixels(&bitmap, 250, 400, 400, 460);
    assert!(left > 100 && right > 100, "left {left}, right {right}");
    assert_eq!(dark_pixels(&bitmap, 100, 470, 400, 590), 0, "nothing below the box");
}
